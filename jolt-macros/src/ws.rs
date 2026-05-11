//! `ws!` function-like proc-macro — phase29 entry point.
//!
//! Phase29 ladder:
//! - JOLT-RS-122 (this iteration): parse the `ws!(path, HandlerType,
//!   subprotocol = "proto", auth_fn = fn_name)` call form and emit a witness
//!   expression that type-checks every argument. The two positional args
//!   (path, handler type) come first; the two named args (`subprotocol`,
//!   `auth_fn`) may appear in either order after them. On success the
//!   expansion is a block expression that:
//!   1. emits a `const _: fn() = ...` closure containing a
//!      `fn _check<T: ::jolt_core::WebSocketHandler>()` invocation on the
//!      parsed handler type — this is a compile-time-only trait-bound check,
//!      not a runtime call,
//!   2. emits a `let _ = #auth_fn;` so the resolved auth-fn path is
//!      type-checked as a value at the call site (123 will tighten this to
//!      `fn(&str) -> Result<JwtClaims, _>`),
//!   3. returns a `::jolt_core::__WsMacroWitness { path, subprotocol }`
//!      literal carrying the two string-literal args. The witness is the
//!      observable surface that tests in `jolt-core/tests/ws_macro.rs` read
//!      to verify the macro expanded with the right values.
//! - JOLT-RS-123: tighten the `auth_fn` signature check to
//!   `fn(&str) -> Result<JwtClaims, AuthError>` (the trait-check closure is
//!   the natural place to splice that in), and route an `Err(_)` to a 401
//!   response.
//! - JOLT-RS-124: replace the witness-struct return with the real axum WS
//!   upgrade-handler — extract subprotocol, validate token via auth_fn, drive
//!   the [`jolt_core::WebSocketHandler`] lifecycle through 120's `WsMessage`
//!   variants.
//! - JOLT-RS-125: phase29 closing integration test (full connect / send /
//!   receive roundtrip against a running axum server).
//!
//! The parse entry point is split out from `lib.rs` so it can be unit-tested
//! against a `proc_macro2::TokenStream` directly. Mirrors the same split
//! established by [`crate::auto_middleware::parse_auto_middleware_input`]
//! (JOLT-RS-046) and [`crate::patch_query::parse_patch_query_input`]
//! (JOLT-RS-110).

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Path, Token, Type};

/// Parsed shape of a `ws!(...)` invocation.
///
/// All four fields are required at JOLT-RS-122. Later slices may add optional
/// fields (e.g. a `state = state_type` parameter to thread an `Arc<AppState>`
/// into the handler constructor) but the four anchored here form the minimum
/// invocation for the route + auth + upgrade pipeline.
#[derive(Debug)]
pub(crate) struct WsMacroInput {
    pub(crate) path: LitStr,
    pub(crate) handler_type: Type,
    pub(crate) subprotocol: LitStr,
    pub(crate) auth_fn: Path,
}

impl Parse for WsMacroInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let path: LitStr = input.parse().map_err(|e| {
            syn::Error::new(
                e.span(),
                "ws! expects a string-literal path as its first argument, e.g. ws!(\"/chat\", ...)",
            )
        })?;
        input.parse::<Token![,]>().map_err(|e| {
            syn::Error::new(
                e.span(),
                "ws! expects a comma after the path argument",
            )
        })?;
        let handler_type: Type = input.parse().map_err(|e| {
            syn::Error::new(
                e.span(),
                "ws! expects a handler type as its second argument, e.g. ws!(\"/chat\", ChatHandler, ...)",
            )
        })?;
        input.parse::<Token![,]>().map_err(|e| {
            syn::Error::new(
                e.span(),
                "ws! expects a comma after the handler-type argument, followed by subprotocol = \"...\" and auth_fn = fn_name",
            )
        })?;

        let mut subprotocol: Option<LitStr> = None;
        let mut auth_fn: Option<Path> = None;
        let mut first_named = true;
        while !input.is_empty() {
            if !first_named {
                input.parse::<Token![,]>()?;
                if input.is_empty() {
                    break;
                }
            }
            first_named = false;
            let name: Ident = input.parse()?;
            input.parse::<Token![=]>().map_err(|e| {
                syn::Error::new(
                    e.span(),
                    format!("ws! named argument `{}` requires `= value`", name),
                )
            })?;
            match name.to_string().as_str() {
                "subprotocol" => {
                    if subprotocol.is_some() {
                        return Err(syn::Error::new(
                            name.span(),
                            "ws! `subprotocol` was already provided; specify it exactly once",
                        ));
                    }
                    subprotocol = Some(input.parse::<LitStr>().map_err(|e| {
                        syn::Error::new(
                            e.span(),
                            "ws! `subprotocol` value must be a string literal, e.g. subprotocol = \"chat-v1\"",
                        )
                    })?);
                }
                "auth_fn" => {
                    if auth_fn.is_some() {
                        return Err(syn::Error::new(
                            name.span(),
                            "ws! `auth_fn` was already provided; specify it exactly once",
                        ));
                    }
                    auth_fn = Some(input.parse::<Path>().map_err(|e| {
                        syn::Error::new(
                            e.span(),
                            "ws! `auth_fn` value must be a path to a function, e.g. auth_fn = validate_token",
                        )
                    })?);
                }
                other => {
                    return Err(syn::Error::new(
                        name.span(),
                        format!(
                            "ws! unknown named argument `{}` (expected `subprotocol` or `auth_fn`)",
                            other
                        ),
                    ));
                }
            }
        }

        let subprotocol = subprotocol.ok_or_else(|| {
            syn::Error::new(
                path.span(),
                "ws! missing required `subprotocol = \"...\"` argument",
            )
        })?;
        let auth_fn = auth_fn.ok_or_else(|| {
            syn::Error::new(
                path.span(),
                "ws! missing required `auth_fn = fn_name` argument",
            )
        })?;

        Ok(WsMacroInput {
            path,
            handler_type,
            subprotocol,
            auth_fn,
        })
    }
}

/// Top-level driver for `ws!(...)`.
///
/// Parses via [`WsMacroInput`]'s `Parse` impl and emits a block expression
/// witness. On parse failure the emission is a single `compile_error!` token
/// (with the span the parser attached) — no partial codegen. Mirrors
/// [`crate::patch_query::expand_patch_query`]'s contract from JOLT-RS-110.
pub(crate) fn expand_ws_macro(input: TokenStream) -> TokenStream {
    let parsed: WsMacroInput = match syn::parse2(input) {
        Ok(p) => p,
        Err(err) => return err.to_compile_error(),
    };
    let WsMacroInput {
        path,
        handler_type,
        subprotocol,
        auth_fn,
    } = parsed;

    quote! {
        {
            // Compile-time trait-bound check on the handler type. Lives inside
            // a `const _: fn() = || { ... };` so the closure body is type-
            // checked at compile time but never invoked at runtime. A handler
            // type that doesn't implement `WebSocketHandler + Default` surfaces
            // here as a `the trait bound ... is not satisfied` diagnostic at the
            // call site of `ws!`.
            const _: fn() = || {
                fn __jolt_ws_assert_handler<__T: ::jolt_core::WebSocketHandler + Default>() {}
                __jolt_ws_assert_handler::<#handler_type>();
            };
            // Tightened JOLT-RS-123 signature check: auth_fn must be a callable
            // matching `fn(&str) -> Result<JwtClaims, AuthError>`. Uses a const
            // block so the check runs at compile time with zero runtime cost.
            // If auth_fn's signature doesn't match exactly, the compiler
            // surfaces a type-mismatch diagnostic at the ws! call site.
            const _: fn() = || {
                fn __jolt_ws_assert_auth_fn<__F: Fn(&str) -> Result<::jolt_core::JwtClaims, ::jolt_core::AuthError>>(__f: __F) {
                    let _ = __f;
                }
                __jolt_ws_assert_auth_fn(#auth_fn);
            };
            // Anonymous struct carrying both the witness fields for
            // backward-compatible path/subprotocol assertions AND a testable
            // auth-check method that exercises the auth_fn → claims →
            // handler.on_open dispatch path JOLT-RS-123 requires.
            struct __JoltWsExpanded {
                witness: ::jolt_core::__WsMacroWitness,
            }
            impl __JoltWsExpanded {
                pub async fn check_auth(self, token: &str) -> ::axum::response::Response {
                    match #auth_fn(token) {
                        Err(err) => {
                            ::axum::response::Response::builder()
                                .status(::axum::http::StatusCode::UNAUTHORIZED)
                                .header(
                                    ::axum::http::header::CONTENT_TYPE,
                                    ::axum::http::HeaderValue::from_static("text/plain; charset=utf-8"),
                                )
                                .body(::axum::body::Body::from(err.to_string()))
                                .expect("401 response builder always succeeds with static headers")
                        }
                        Ok(claims) => {
                            let mut handler = <#handler_type>::default();
                            handler.set_claims(claims);
                            let (sender, _rx) = ::jolt_core::WebSocketSender::channel();
                            handler.on_open(sender).await;
                            ::axum::response::Response::builder()
                                .status(::axum::http::StatusCode::OK)
                                .body(::axum::body::Body::empty())
                                .expect("200 response builder always succeeds with empty body")
                        }
                    }
                }
            }
            __JoltWsExpanded {
                witness: ::jolt_core::__WsMacroWitness {
                    path: #path,
                    subprotocol: #subprotocol,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn parse_input(src: &str) -> syn::Result<WsMacroInput> {
        let tokens = TokenStream::from_str(src).expect("test input parses as TokenStream");
        syn::parse2::<WsMacroInput>(tokens)
    }

    #[test]
    fn parses_canonical_invocation_with_named_args_in_doc_order() {
        let input = parse_input(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("canonical invocation parses");
        assert_eq!(input.path.value(), "/chat");
        assert_eq!(input.subprotocol.value(), "chat-v1");
        let auth_fn = &input.auth_fn;
        assert_eq!(quote! { #auth_fn }.to_string(), "validate_token");
        let handler_ty = &input.handler_type;
        assert_eq!(quote! { #handler_ty }.to_string(), "ChatHandler");
    }

    #[test]
    fn parses_named_args_in_reverse_order() {
        // Named args may appear in either order after the two positional args.
        // The parser is positional-first, named-flexible.
        let input = parse_input(
            r#""/ws", MyHandler, auth_fn = my_auth, subprotocol = "v2""#,
        )
        .expect("reverse-order named args parse");
        assert_eq!(input.path.value(), "/ws");
        assert_eq!(input.subprotocol.value(), "v2");
        let auth_fn = &input.auth_fn;
        assert_eq!(quote! { #auth_fn }.to_string(), "my_auth");
    }

    #[test]
    fn parses_qualified_path_for_auth_fn() {
        // `auth_fn` accepts any `syn::Path`, not just a bare identifier.
        // A module-qualified function is a routine use case (the user defines
        // auth helpers in a sibling module).
        let input = parse_input(
            r#""/chat", H, subprotocol = "v1", auth_fn = crate::auth::validate"#,
        )
        .expect("qualified path parses");
        let auth_fn = &input.auth_fn;
        assert_eq!(
            quote! { #auth_fn }.to_string(),
            "crate :: auth :: validate"
        );
    }

    #[test]
    fn parses_complex_handler_type_with_generics() {
        // The handler type is a full `syn::Type`. A user might wrap a base
        // handler in a generic adapter — pin that the parser doesn't choke on
        // generic args.
        let input = parse_input(
            r#""/chat", Adapter<ChatHandler, AppState>, subprotocol = "v1", auth_fn = auth"#,
        )
        .expect("generic handler type parses");
        let ty = &input.handler_type;
        let rendered = quote! { #ty }.to_string();
        assert!(rendered.contains("Adapter"), "got: {rendered}");
        assert!(rendered.contains("ChatHandler"), "got: {rendered}");
        assert!(rendered.contains("AppState"), "got: {rendered}");
    }

    #[test]
    fn rejects_missing_path() {
        let err = parse_input(r#""#).expect_err("empty input must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("string-literal path"),
            "diagnostic must mention the path, got: {msg}"
        );
    }

    #[test]
    fn rejects_non_string_path() {
        let err = parse_input(r#"42, Handler, subprotocol = "v1", auth_fn = auth"#)
            .expect_err("non-string path must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("string-literal path"),
            "diagnostic must mention the path, got: {msg}"
        );
    }

    #[test]
    fn rejects_missing_subprotocol() {
        let err = parse_input(r#""/chat", Handler, auth_fn = auth"#)
            .expect_err("missing subprotocol must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("subprotocol"),
            "diagnostic must name the missing arg, got: {msg}"
        );
    }

    #[test]
    fn rejects_missing_auth_fn() {
        let err = parse_input(r#""/chat", Handler, subprotocol = "v1""#)
            .expect_err("missing auth_fn must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("auth_fn"),
            "diagnostic must name the missing arg, got: {msg}"
        );
    }

    #[test]
    fn rejects_unknown_named_argument() {
        // `state = ...` is reserved for a future slice; today it's an unknown
        // argument and must be rejected with a targeted diagnostic.
        let err = parse_input(
            r#""/chat", Handler, subprotocol = "v1", auth_fn = auth, state = StateType"#,
        )
        .expect_err("unknown named arg must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("unknown named argument") || msg.contains("state"),
            "diagnostic must mention the unknown arg name, got: {msg}"
        );
    }

    #[test]
    fn rejects_duplicate_subprotocol() {
        let err = parse_input(
            r#""/chat", H, subprotocol = "v1", auth_fn = auth, subprotocol = "v2""#,
        )
        .expect_err("duplicate subprotocol must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("already provided") || msg.contains("subprotocol"),
            "diagnostic must explain the duplicate, got: {msg}"
        );
    }

    #[test]
    fn expand_emits_witness_struct_literal_on_well_formed_input() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("__WsMacroWitness"),
            "emission must construct the witness struct, got: {out}"
        );
        assert!(
            out.contains("\"/chat\""),
            "emission must splice the path literal, got: {out}"
        );
        assert!(
            out.contains("\"chat-v1\""),
            "emission must splice the subprotocol literal, got: {out}"
        );
        assert!(
            out.contains("validate_token"),
            "emission must reference the auth_fn path, got: {out}"
        );
        assert!(
            out.contains("__jolt_ws_assert_handler"),
            "emission must include the trait-bound check, got: {out}"
        );
        assert!(
            out.contains("WebSocketHandler"),
            "emission must reference the WebSocketHandler bound, got: {out}"
        );
    }

    #[test]
    fn expand_emits_compile_error_on_bad_input() {
        let tokens = TokenStream::from_str(r#"42, Handler"#)
            .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("compile_error"),
            "parse failure must emit compile_error!, got: {out}"
        );
        assert!(
            !out.contains("__WsMacroWitness"),
            "no partial codegen on parse failure, got: {out}"
        );
    }

    #[test]
    fn expand_emits_auth_fn_signature_check() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("__jolt_ws_assert_auth_fn"),
            "emission must include the auth_fn signature check, got: {out}"
        );
        assert!(
            out.contains("JwtClaims"),
            "auth_fn check must reference JwtClaims, got: {out}"
        );
        assert!(
            out.contains("AuthError"),
            "auth_fn check must reference AuthError, got: {out}"
        );
    }

    #[test]
    fn expand_emits_check_auth_method() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("check_auth"),
            "emission must include the check_auth method, got: {out}"
        );
        assert!(
            out.contains("set_claims"),
            "check_auth must call handler.set_claims, got: {out}"
        );
        assert!(
            out.contains("on_open"),
            "check_auth must call handler.on_open, got: {out}"
        );
        assert!(
            out.contains("StatusCode :: UNAUTHORIZED")
                || out.contains("StatusCode::UNAUTHORIZED"),
            "check_auth must return 401 on auth failure, got: {out}"
        );
    }

    #[test]
    fn expand_emits_handler_default_bound_check() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("Default"),
            "handler trait check must include Default bound, got: {out}"
        );
    }

    #[test]
    fn expand_still_contains_backward_compatible_witness_struct() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("__WsMacroWitness"),
            "expansion must still construct the witness struct for backward compat, got: {out}"
        );
    }
}
