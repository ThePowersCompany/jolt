//! `ws!` function-like proc-macro.
//!
//! The macro parses `ws!(path, HandlerType, subprotocol = "proto", auth_fn = fn)`
//! and emits an axum-compatible async route handler. The expansion:
//! 1. compile-time-checks `HandlerType: WebSocketHandler + Default + Send`,
//! 2. compile-time-checks `auth_fn: Fn(&str) -> Result<JwtClaims, AuthError>`,
//! 3. extracts a JWT from `Sec-WebSocket-Protocol: joltr-jwt, <token>`,
//! 4. returns `401 Unauthorized` for missing/invalid tokens or auth failures,
//! 5. upgrades the WebSocket and spawns a writer task for handler sends,
//! 6. drives `set_claims -> on_open -> on_ready -> on_message* -> on_close`,
//! 7. signals the writer, drains queued frames, closes the sink, and waits for
//!    the writer before `on_shutdown` returns.
//!
//! The parse entry point is split out from `lib.rs` so it can be unit-tested
//! against a `proc_macro2::TokenStream` directly, matching the other macro
//! modules' parser/codegen split.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitStr, Path, Token, Type};

/// Parsed shape of a `ws!(...)` invocation.
///
/// All four fields are required. Unknown named arguments are rejected so the
/// route + auth + upgrade pipeline has a single stable invocation shape.
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
/// Parses via [`WsMacroInput`]'s `Parse` impl and emits an async-closure
/// expression that axum can wire as a route handler. On parse failure the
/// emission is a single `compile_error!` token (with the span the parser
/// attached) — no partial codegen.
///
/// The emitted closure:
/// 1. extract JWT from `Sec-WebSocket-Protocol: joltr-jwt, <token>` via
///    [`joltr_core::extract_ws_jwt_token`],
/// 2. validate the token via `#auth_fn`, returning 401 on rejection,
/// 3. upgrade the WebSocket and drive the [`joltr_core::WebSocketHandler`]
///    lifecycle through set_claims → on_open → on_ready → on_message loop →
///    on_close → writer drain → on_shutdown.
pub(crate) fn expand_ws_macro(input: TokenStream) -> TokenStream {
    let parsed: WsMacroInput = match syn::parse2(input) {
        Ok(p) => p,
        Err(err) => return err.to_compile_error(),
    };
    let WsMacroInput {
        path: _path,
        handler_type,
        subprotocol: _subprotocol,
        auth_fn,
    } = parsed;

    quote! {
        {
            // Compile-time trait-bound check on the handler type. Lives inside
            // a `const _: fn() = || { ... };` so the closure body is type-
            // checked at compile time but never invoked at runtime. A handler
            // type that doesn't implement `WebSocketHandler + Default + Send`
            // surfaces here as a `the trait bound ... is not satisfied`
            // diagnostic at the call site of `ws!`. The `Send` bound is
            // required because the handler is moved into the `on_upgrade`
            // callback which axum requires to be `Send + 'static`.
            const _: fn() = || {
                fn __jolt_ws_assert_handler<
                    __T: ::joltr_core::WebSocketHandler + Default + Send,
                >() {}
                __jolt_ws_assert_handler::<#handler_type>();
            };
            // JOLTR-RS-123 signature check: auth_fn must match
            // `fn(&str) -> Result<JwtClaims, AuthError>`. Uses a const block
            // so the check runs at compile time with zero runtime cost.
            const _: fn() = || {
                fn __jolt_ws_assert_auth_fn<
                    __F: Fn(&str)
                        -> Result<
                            ::joltr_core::JwtClaims,
                            ::joltr_core::AuthError,
                        >,
                >(__f: __F) {
                    let _ = __f;
                }
                __jolt_ws_assert_auth_fn(#auth_fn);
            };
            // Return an async closure that axum can wire directly as a route
            // handler. The closure takes `WebSocketUpgrade` + `Request` as
            // extractors, handles auth, and returns a `Response`.
            |__ws: ::axum::extract::ws::WebSocketUpgrade,
             __req: ::axum::extract::Request| async move {
                // Extract the JWT from the Sec-WebSocket-Protocol header
                // (canonical form: `joltr-jwt, <token>`). On any extraction
                // failure (missing header, non-ASCII, wrong format, empty
                // token) short-circuit with 401.
                let __token: ::std::string::String =
                    match ::joltr_core::extract_ws_jwt_token(
                        __req
                            .headers()
                            .get(::axum::http::header::SEC_WEBSOCKET_PROTOCOL),
                    ) {
                        ::std::result::Result::Ok(t) => t,
                        ::std::result::Result::Err(reason) => {
                            return ::axum::response::Response::builder()
                                .status(::axum::http::StatusCode::UNAUTHORIZED)
                                .header(
                                    ::axum::http::header::CONTENT_TYPE,
                                    ::axum::http::HeaderValue::from_static(
                                        "text/plain; charset=utf-8",
                                    ),
                                )
                                .body(::axum::body::Body::from(reason.message()))
                                .expect(
                                    "401 response builder always succeeds with static headers",
                                );
                        }
                    };
                // Validate the JWT via the user-supplied auth_fn. On failure
                // (expired, bad signature, custom rejection) short-circuit with
                // 401 carrying the auth error's Display text as the body.
                let __claims: ::joltr_core::JwtClaims = match #auth_fn(&__token) {
                    ::std::result::Result::Ok(c) => c,
                    ::std::result::Result::Err(err) => {
                        return ::axum::response::Response::builder()
                            .status(::axum::http::StatusCode::UNAUTHORIZED)
                            .header(
                                ::axum::http::header::CONTENT_TYPE,
                                ::axum::http::HeaderValue::from_static(
                                    "text/plain; charset=utf-8",
                                ),
                            )
                            .body(::axum::body::Body::from(
                                err.to_string(),
                            ))
                            .expect(
                                "401 response builder always succeeds with static headers",
                            );
                    }
                };
                // Perform the WebSocket upgrade. The callback receives the raw
                // WebSocket, splits it into a sender/receiver pair, spawns a
                // writer task, creates the handler, and drives the full
                // lifecycle.
                __ws.on_upgrade(
                    move |__socket: ::axum::extract::ws::WebSocket| async move {
                        use ::joltr_core::futures_util::{
                            SinkExt as __JoltRSinkExt,
                            StreamExt as __JoltRStreamExt,
                        };
                        let (mut __tx, mut __rx) = __socket.split();
                        let (__sender, __writer_rx) =
                            ::joltr_core::WebSocketSender::channel();
                        let (__writer_shutdown_tx, __writer_shutdown_rx) =
                            ::tokio::sync::oneshot::channel::<()>();
                        // Writer task: reads frames from the mpsc channel
                        // (fed by handler callbacks) and forwards them into
                        // the axum sink. Shutdown closes the receiver, drains
                        // already queued frames, then closes the sink.
                        let __writer = ::tokio::spawn(async move {
                            let mut __writer_rx = __writer_rx;
                            let mut __writer_shutdown_rx = __writer_shutdown_rx;
                            loop {
                                ::tokio::select! {
                                    __msg = __writer_rx.recv() => {
                                        let ::std::option::Option::Some(__msg) = __msg else {
                                            break;
                                        };
                                        if __tx.send(__msg).await.is_err() {
                                            return;
                                        }
                                    }
                                    _ = &mut __writer_shutdown_rx => {
                                        __writer_rx.close();
                                        while let ::std::option::Option::Some(__msg) =
                                            __writer_rx.recv().await
                                        {
                                            if __tx.send(__msg).await.is_err() {
                                                return;
                                            }
                                        }
                                        break;
                                    }
                                }
                            }
                            let _ = __tx.close().await;
                        });
                        // Construct the handler and drive the lifecycle.
                        let mut __handler = <#handler_type as ::std::default::Default>::default();
                        __handler.set_claims(__claims);
                        __handler.on_open(__sender.clone()).await;
                        __handler.on_ready(__sender.clone()).await;
                        // Read loop: receive frames from the client, map
                        // axum → WsMessage, and dispatch to the handler.
                        // A Close frame breaks the loop; the handler's
                        // on_close and on_shutdown run afterwards.
                        while let ::std::option::Option::Some(
                            ::std::result::Result::Ok(__msg),
                        ) = __rx.next().await
                        {
                            let __ws_msg =
                                ::joltr_core::WsMessage::from(__msg);
                            let __is_close =
                                ::std::matches!(
                                    &__ws_msg,
                                    ::joltr_core::WsMessage::Close(_)
                                );
                            __handler
                                .on_message(__ws_msg, __sender.clone())
                                .await;
                            if __is_close {
                                break;
                            }
                        }
                        __handler.on_close().await;
                        let _ = __writer_shutdown_tx.send(());
                        let _ = __writer.await;
                        __handler.on_shutdown().await;
                    },
                )
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
    fn expand_emits_handler_trait_check_with_send_bound() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("__jolt_ws_assert_handler"),
            "emission must include the handler trait-bound check, got: {out}"
        );
        assert!(
            out.contains("WebSocketHandler"),
            "handler check must reference WebSocketHandler, got: {out}"
        );
        assert!(
            out.contains("Default") && out.contains("Send"),
            "handler check must include Default + Send bounds, got: {out}"
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
    fn expand_emits_web_socket_upgrade_and_lifecycle() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("on_upgrade"),
            "emission must call WebSocketUpgrade::on_upgrade, got: {out}"
        );
        assert!(
            out.contains("extract_ws_jwt_token"),
            "emission must call the WS JWT extractor, got: {out}"
        );
        assert!(
            out.contains("set_claims"),
            "emission must call handler.set_claims, got: {out}"
        );
        assert!(
            out.contains("on_open"),
            "emission must call handler.on_open, got: {out}"
        );
        assert!(
            out.contains("on_ready"),
            "emission must call handler.on_ready, got: {out}"
        );
        assert!(
            out.contains("on_message"),
            "emission must call handler.on_message, got: {out}"
        );
        assert!(
            out.contains("on_close"),
            "emission must call handler.on_close, got: {out}"
        );
        assert!(
            out.contains("on_shutdown"),
            "emission must call handler.on_shutdown, got: {out}"
        );
    }

    #[test]
    fn expand_emits_unauthorized_on_extraction_or_auth_failure() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("StatusCode :: UNAUTHORIZED")
                || out.contains("StatusCode::UNAUTHORIZED"),
            "emission must return 401 on auth failure, got: {out}"
        );
    }

    #[test]
    fn expand_emits_writer_task_spawn() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            out.contains("spawn"),
            "emission must spawn a writer task, got: {out}"
        );
        assert!(
            out.contains("WebSocketSender"),
            "emission must use WebSocketSender, got: {out}"
        );
    }

    #[test]
    fn expand_uses_split_for_ws_read_write() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        // quote renders `.split()` as `. split ()` (with spaces). Match
        // on the constituent tokens rather than the formatting-sensitive
        // canonical string.
        assert!(
            out.contains("split") && out.contains("()"),
            "emission must split the WebSocket for read/write, got: {out}"
        );
    }

    #[test]
    fn expand_no_longer_emits_witness_struct() {
        let tokens = TokenStream::from_str(
            r#""/chat", ChatHandler, subprotocol = "chat-v1", auth_fn = validate_token"#,
        )
        .expect("test input parses as TokenStream");
        let out = expand_ws_macro(tokens).to_string();
        assert!(
            !out.contains("__WsMacroWitness"),
            "JOLTR-RS-124 removes the witness struct; emission must not contain it, got: {out}"
        );
        assert!(
            !out.contains("check_auth"),
            "JOLTR-RS-124 removes the check_auth method, got: {out}"
        );
    }
}
