//! `#[endpoint("/path")]` attribute macro — phase08 parsing surface.
//!
//! Phase08 ladder:
//! - JOLT-RS-038: parse the path string literal from the attribute tokens. (landed)
//! - JOLT-RS-039: scan the impl block for `#[get]`/`#[post]`/`#[put]`/
//!   `#[patch]`/`#[delete]` methods, collect their signatures. (landed)
//! - JOLT-RS-040 (this iteration): validate handler signatures — first arg is
//!   `&self`, return type is `Response<T>` or `Result<Response<T>, E>`.
//! - JOLT-RS-041: emit the (Method, path) -> handler match.
//!
//! The parsing entry points are split out from `lib.rs` so they can be
//! unit-tested against a `proc_macro2::TokenStream` / parsed `syn::ItemImpl`
//! (proc-macro entry points themselves cannot be invoked outside the compiler).
//!
//! Verb attributes (`#[get]`, `#[post]`, ...) are treated as **magic markers**:
//! `#[endpoint]` recognizes them when scanning, but they are not registered as
//! their own proc-macro attributes. JOLT-RS-041 will strip them from the
//! re-emitted impl so rustc never sees them.

use syn::{
    parse2, Attribute, FnArg, GenericArgument, ImplItem, ItemImpl, LitStr, Meta, PathArguments,
    ReturnType, Signature, Type,
};

/// Parsed shape of `#[endpoint("/path")]`'s attribute argument.
#[allow(dead_code)] // `path` is read by tests this iteration; JOLT-RS-041 wires it into codegen.
pub(crate) struct EndpointAttr {
    pub(crate) path: LitStr,
}

/// Parse the attribute tokens of `#[endpoint(...)]` into an [`EndpointAttr`].
///
/// The single positional argument MUST be a string literal — the route path.
/// Empty input or non-string-literal input is rejected with a `syn::Error`
/// pointing at the offending span (or at call-site for empty input).
pub(crate) fn parse_endpoint_attr(
    tokens: proc_macro2::TokenStream,
) -> syn::Result<EndpointAttr> {
    let path: LitStr = parse2(tokens)?;
    Ok(EndpointAttr { path })
}

/// HTTP verb tagged on a method via a magic-marker attribute. Mirrors
/// `jolt_core::Method` but lives in the macro crate to avoid a cyclic dep
/// (jolt-core depends on jolt-macros at the workspace level once macros are
/// re-exported). Codegen in JOLT-RS-041 will emit `::jolt_core::Method::Get`
/// etc. from this enum via [`HttpMethod::as_jolt_core_variant_ident`] (added
/// when 041 lands).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // variants are constructed via `from_attr_name`; tests read them.
pub(crate) enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethod {
    /// Returns `Some(verb)` if `name` is one of the recognized verb-attribute
    /// names (`get`, `post`, `put`, `patch`, `delete`). Returns `None` for any
    /// other identifier — including verbs we don't currently route (`head`,
    /// `options`) which are handled by jolt-core's auto-OPTIONS / Allow-header
    /// machinery rather than by user-defined endpoints.
    #[allow(dead_code)] // tests consume it; lib.rs wires it in once JOLT-RS-041 lands.
    fn from_attr_name(name: &str) -> Option<Self> {
        match name {
            "get" => Some(Self::Get),
            "post" => Some(Self::Post),
            "put" => Some(Self::Put),
            "patch" => Some(Self::Patch),
            "delete" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// One method discovered inside an `impl` block by [`scan_methods`].
///
/// `sig` is cloned from the source `ImplItemFn` so callers can inspect it
/// without holding a borrow on the input `ItemImpl`. JOLT-RS-040 will read
/// `sig.inputs` (first arg must be `&self`) and `sig.output` (must be
/// `Response<T>` or `Result<Response<T>, E>`); JOLT-RS-041 will read
/// `sig.ident` and `http_method` to emit the dispatch match arm.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields are read by tests this iteration; JOLT-RS-040/041 read them in codegen.
pub(crate) struct DiscoveredMethod {
    pub(crate) http_method: HttpMethod,
    pub(crate) sig: Signature,
}

/// Walk `item.items`, find each `ImplItem::Fn` whose attribute list contains
/// exactly one of the recognized verb attributes, and collect the discovered
/// method's verb + signature.
///
/// Methods with no verb attribute are skipped (an impl block can have helper
/// methods alongside endpoint handlers). Non-fn impl items (consts, type
/// aliases, sub-macros) are ignored entirely.
///
/// A method tagged with two or more verb attributes (e.g. both `#[get]` and
/// `#[post]`) is a parse-time error — the second verb attribute carries the
/// span. Routing the same handler under multiple verbs is not currently
/// supported; if a future PRD wants it, this check relaxes into a `Vec<HttpMethod>`
/// per discovered method.
#[allow(dead_code)] // tests consume it; lib.rs wires it in once JOLT-RS-041 lands.
pub(crate) fn scan_methods(item: &ItemImpl) -> syn::Result<Vec<DiscoveredMethod>> {
    let mut discovered = Vec::new();
    for impl_item in &item.items {
        let ImplItem::Fn(method_fn) = impl_item else {
            continue;
        };
        let mut matched: Option<HttpMethod> = None;
        for attr in &method_fn.attrs {
            let Some(verb) = verb_from_attr(attr) else {
                continue;
            };
            if matched.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "method has more than one HTTP verb attribute (use only one of #[get], #[post], #[put], #[patch], #[delete])",
                ));
            }
            matched = Some(verb);
        }
        if let Some(http_method) = matched {
            discovered.push(DiscoveredMethod {
                http_method,
                sig: method_fn.sig.clone(),
            });
        }
    }
    Ok(discovered)
}

/// Extract the verb from a single attribute, if it is one of the recognized
/// magic markers. Path-style attributes like `#[axum::get]` or `#[get(...)]`
/// are NOT matched: only bare `#[get]`-shape attributes whose path is a single
/// identifier matching one of the five verb names AND that carry no argument
/// list. This keeps the marker surface narrow — a user who writes `#[axum::get]`
/// or `#[get("/path")]` clearly meant a different macro (likely axum's) and we
/// should not steal it.
#[allow(dead_code)] // exercised via `scan_methods` (also dead this iteration).
fn verb_from_attr(attr: &Attribute) -> Option<HttpMethod> {
    if !matches!(attr.meta, Meta::Path(_)) {
        return None;
    }
    let ident = attr.path().get_ident()?;
    HttpMethod::from_attr_name(&ident.to_string())
}

/// Validate every discovered method's signature in turn. Stops at the first
/// failure and returns its [`syn::Error`] (so the user sees one diagnostic at
/// a time rather than a cascade — `compile_error!` already handles cascades
/// poorly). If all signatures validate, returns `Ok(())`.
///
/// JOLT-RS-041 will call this between `scan_methods` and codegen so the
/// dispatch match-arm emission can assume every signature is well-formed.
#[allow(dead_code)] // tests consume it; lib.rs wires it in once JOLT-RS-041 lands.
pub(crate) fn validate_methods(methods: &[DiscoveredMethod]) -> syn::Result<()> {
    for method in methods {
        validate_signature(&method.sig)?;
    }
    Ok(())
}

/// Validate a single handler signature.
///
/// Two contracts are checked:
///
/// 1. The first argument must be `&self` (immutable borrow of the endpoint
///    struct). `&mut self`, owned `self`, typed receivers like `self: Box<Self>`,
///    a non-receiver first arg, and a missing first arg are all rejected.
///    Endpoint impls model dispatch tables, not state machines — `&self` is the
///    one shape that lets the same `&Endpoint` serve concurrent requests.
///
/// 2. The return type must be `Response<...>` or `Result<Response<...>, ...>`,
///    matched name-only against the type's last path segment. The check is
///    deliberately lax about generic args, fully-qualified paths, and crate
///    prefixes — a user-typed `crate::Response<T>`, `jolt_core::Response<T>`,
///    or `Response` (no args) is all accepted at the macro layer; rustc's type
///    checker on the generated code is the strict gate.
///
/// On failure the [`syn::Error`] carries a span pointing at the offending arg
/// or return-type token, so `compile_error!` underlines the right code.
#[allow(dead_code)] // tests consume it; lib.rs wires it in once JOLT-RS-041 lands.
pub(crate) fn validate_signature(sig: &Signature) -> syn::Result<()> {
    validate_receiver(sig)?;
    validate_return_type(&sig.output)?;
    Ok(())
}

fn validate_receiver(sig: &Signature) -> syn::Result<()> {
    let Some(first) = sig.inputs.first() else {
        return Err(syn::Error::new_spanned(
            &sig.ident,
            "endpoint method must take `&self` as its first argument, but it has no arguments",
        ));
    };
    let receiver = match first {
        FnArg::Receiver(r) => r,
        FnArg::Typed(_) => {
            return Err(syn::Error::new_spanned(
                first,
                "endpoint method must take `&self` as its first argument",
            ));
        }
    };
    if receiver.colon_token.is_some() {
        // Typed receiver, e.g. `self: Box<Self>` or `self: &Self`. The macro
        // recognizes only the canonical `&self` shorthand.
        return Err(syn::Error::new_spanned(
            receiver,
            "endpoint method must take `&self` (typed receivers like `self: Box<Self>` are not supported)",
        ));
    }
    if receiver.reference.is_none() {
        return Err(syn::Error::new_spanned(
            receiver,
            "endpoint method must take `&self`, not owned `self` (handlers run on a shared `&Endpoint`)",
        ));
    }
    if receiver.mutability.is_some() {
        return Err(syn::Error::new_spanned(
            receiver,
            "endpoint method must take `&self`, not `&mut self` (handlers must be safe to call concurrently on the same `&Endpoint`)",
        ));
    }
    Ok(())
}

fn validate_return_type(output: &ReturnType) -> syn::Result<()> {
    let ty = match output {
        ReturnType::Default => {
            return Err(syn::Error::new_spanned(
                output,
                "endpoint method must return `Response<T>` or `Result<Response<T>, E>`, but no return type was given",
            ));
        }
        ReturnType::Type(_, ty) => ty.as_ref(),
    };
    let Some(last) = last_path_segment(ty) else {
        return Err(syn::Error::new_spanned(
            ty,
            "endpoint method must return `Response<T>` or `Result<Response<T>, E>`",
        ));
    };
    let name = last.ident.to_string();
    match name.as_str() {
        "Response" => Ok(()),
        "Result" => {
            let PathArguments::AngleBracketed(args) = &last.arguments else {
                return Err(syn::Error::new_spanned(
                    last,
                    "endpoint method returning `Result` must specify `Result<Response<T>, E>`",
                ));
            };
            let Some(GenericArgument::Type(inner)) = args.args.first() else {
                return Err(syn::Error::new_spanned(
                    last,
                    "endpoint method returning `Result` must specify `Result<Response<T>, E>` (first generic arg must be a `Response<T>` type)",
                ));
            };
            let Some(inner_seg) = last_path_segment(inner) else {
                return Err(syn::Error::new_spanned(
                    inner,
                    "endpoint method returning `Result` must wrap a `Response<T>` (got a non-path type)",
                ));
            };
            if inner_seg.ident == "Response" {
                Ok(())
            } else {
                Err(syn::Error::new_spanned(
                    inner,
                    format!(
                        "endpoint method returning `Result` must wrap a `Response<T>`, found `{}`",
                        inner_seg.ident
                    ),
                ))
            }
        }
        other => Err(syn::Error::new_spanned(
            ty,
            format!(
                "endpoint method must return `Response<T>` or `Result<Response<T>, E>`, found `{other}`"
            ),
        )),
    }
}

fn last_path_segment(ty: &Type) -> Option<&syn::PathSegment> {
    match ty {
        Type::Path(tp) => tp.path.segments.last(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::TokenStream;
    use std::str::FromStr;

    #[test]
    fn extracts_simple_path() {
        let tokens = TokenStream::from_str(r#""/api/test""#).unwrap();
        let parsed = parse_endpoint_attr(tokens).expect("parses");
        assert_eq!(parsed.path.value(), "/api/test");
    }

    #[test]
    fn extracts_root_path() {
        let tokens = TokenStream::from_str(r#""/""#).unwrap();
        let parsed = parse_endpoint_attr(tokens).expect("parses");
        assert_eq!(parsed.path.value(), "/");
    }

    #[test]
    fn extracts_path_with_param_placeholder() {
        let tokens = TokenStream::from_str(r#""/users/:id""#).unwrap();
        let parsed = parse_endpoint_attr(tokens).expect("parses");
        assert_eq!(parsed.path.value(), "/users/:id");
    }

    #[test]
    fn rejects_empty_attr() {
        let tokens = TokenStream::new();
        assert!(parse_endpoint_attr(tokens).is_err());
    }

    #[test]
    fn rejects_integer_literal() {
        let tokens = TokenStream::from_str("123").unwrap();
        assert!(parse_endpoint_attr(tokens).is_err());
    }

    #[test]
    fn rejects_bare_identifier() {
        let tokens = TokenStream::from_str("path").unwrap();
        assert!(parse_endpoint_attr(tokens).is_err());
    }

    #[test]
    fn rejects_trailing_tokens_after_path() {
        let tokens = TokenStream::from_str(r#""/api/test", "extra""#).unwrap();
        assert!(parse_endpoint_attr(tokens).is_err());
    }

    // ----- JOLT-RS-039: scan_methods -----

    fn parse_impl(src: &str) -> ItemImpl {
        syn::parse_str::<ItemImpl>(src).expect("test input parses as ItemImpl")
    }

    #[test]
    fn discovers_single_get_method() {
        // PRD-mandated verification: impl block with #[get] fn hello(...) ->
        // found method 'hello' tagged as GET.
        let item = parse_impl(
            r#"
            impl Hello {
                #[get]
                fn hello(&self) {}
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].http_method, HttpMethod::Get);
        assert_eq!(found[0].sig.ident.to_string(), "hello");
    }

    #[test]
    fn discovers_one_method_per_verb() {
        let item = parse_impl(
            r#"
            impl Multi {
                #[get] fn list(&self) {}
                #[post] fn create(&self) {}
                #[put] fn replace(&self) {}
                #[patch] fn update(&self) {}
                #[delete] fn destroy(&self) {}
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        let actual: Vec<(HttpMethod, String)> = found
            .iter()
            .map(|d| (d.http_method, d.sig.ident.to_string()))
            .collect();
        assert_eq!(
            actual,
            vec![
                (HttpMethod::Get, "list".to_string()),
                (HttpMethod::Post, "create".to_string()),
                (HttpMethod::Put, "replace".to_string()),
                (HttpMethod::Patch, "update".to_string()),
                (HttpMethod::Delete, "destroy".to_string()),
            ],
        );
    }

    #[test]
    fn skips_methods_without_verb_attribute() {
        // Helper methods on an endpoint impl are common — `validate`, `db`, etc.
        // They MUST be ignored, not error.
        let item = parse_impl(
            r#"
            impl Mixed {
                #[get]
                fn handler(&self) {}

                fn helper(&self) {}

                #[inline]
                fn other_helper(&self, x: i32) -> i32 { x }
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].sig.ident.to_string(), "handler");
    }

    #[test]
    fn ignores_non_fn_impl_items() {
        // Impl blocks can also hold associated consts and types. These are not
        // endpoints — they must not be scanned (and must not crash the walk).
        let item = parse_impl(
            r#"
            impl WithConsts {
                const MAX: usize = 32;
                type Output = String;

                #[get]
                fn handler(&self) {}
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].http_method, HttpMethod::Get);
    }

    #[test]
    fn empty_impl_returns_empty_vec() {
        let item = parse_impl("impl Empty {}");
        let found = scan_methods(&item).expect("scan succeeds");
        assert!(found.is_empty());
    }

    #[test]
    fn captures_signature_inputs_and_output() {
        // 040 will read `sig.inputs` (first arg `&self`) and `sig.output`
        // (return type). Pin that the signature survives intact.
        let item = parse_impl(
            r#"
            impl WithSig {
                #[post]
                fn create(&self, body: String) -> Result<(), Error> {
                    Ok(())
                }
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        assert_eq!(found.len(), 1);
        let sig = &found[0].sig;
        assert_eq!(sig.inputs.len(), 2); // &self + body
        match &sig.output {
            syn::ReturnType::Type(_, _) => {} // has explicit return type
            syn::ReturnType::Default => panic!("expected explicit return type"),
        }
    }

    #[test]
    fn rejects_method_with_two_verb_attributes() {
        let item = parse_impl(
            r#"
            impl Bad {
                #[get]
                #[post]
                fn confused(&self) {}
            }
            "#,
        );
        let err = scan_methods(&item).expect_err("two verb attributes is an error");
        let msg = err.to_string();
        assert!(
            msg.contains("more than one HTTP verb attribute"),
            "expected diagnostic about multi-verb, got: {msg}",
        );
    }

    #[test]
    fn ignores_path_style_and_argful_verb_attributes() {
        // `#[axum::get]` (multi-segment path) and `#[get("/path")]` (path with
        // arglist) are NOT magic markers — the marker surface is bare `#[get]`
        // only. This avoids stealing attributes that belong to other macros
        // (e.g. axum's `#[get("/path")]`-shape route attributes).
        let item = parse_impl(
            r#"
            impl Pathy {
                #[axum::get]
                fn axum_handler(&self) {}

                #[get("/with-path")]
                fn args_handler(&self) {}

                #[get]
                fn marker_handler(&self) {}
            }
            "#,
        );
        let found = scan_methods(&item).expect("scan succeeds");
        let names: Vec<String> = found.iter().map(|d| d.sig.ident.to_string()).collect();
        assert_eq!(names, vec!["marker_handler".to_string()]);
    }

    // ----- JOLT-RS-040: validate_signature / validate_methods -----

    fn parse_sig(method_src: &str) -> Signature {
        syn::parse_str::<syn::ImplItemFn>(method_src)
            .expect("test input parses as ImplItemFn")
            .sig
    }

    #[test]
    fn accepts_response_return_with_borrowed_self() {
        let sig = parse_sig("fn handler(&self) -> Response<()> { todo!() }");
        validate_signature(&sig).expect("&self -> Response<T> is valid");
    }

    #[test]
    fn accepts_result_response_return_with_borrowed_self() {
        let sig = parse_sig(
            "fn handler(&self) -> Result<Response<User>, AppError> { todo!() }",
        );
        validate_signature(&sig).expect("&self -> Result<Response<T>, E> is valid");
    }

    #[test]
    fn accepts_qualified_response_path() {
        // syn-level validation is name-only on the last segment, so fully-
        // qualified `crate::Response<T>` and `jolt_core::Response<T>` are both
        // accepted; rustc enforces the actual type identity on generated code.
        let sig = parse_sig(
            "fn handler(&self) -> jolt_core::response::Response<()> { todo!() }",
        );
        validate_signature(&sig).expect("qualified path is accepted");
    }

    #[test]
    fn accepts_extra_args_after_self() {
        // Auto-middleware (phase11) adds body / query / req fields after &self;
        // validation must not reject methods with multiple args.
        let sig = parse_sig(
            "fn handler(&self, body: CreateUser, id: u64) -> Response<User> { todo!() }",
        );
        validate_signature(&sig).expect("extra args after &self are valid");
    }

    #[test]
    fn rejects_method_with_no_args() {
        let sig = parse_sig("fn handler() -> Response<()> { todo!() }");
        let err = validate_signature(&sig).expect_err("no args is invalid");
        assert!(
            err.to_string().contains("&self"),
            "diagnostic should name &self, got: {err}"
        );
    }

    #[test]
    fn rejects_method_with_typed_first_arg_instead_of_self() {
        let sig = parse_sig("fn handler(req: Request) -> Response<()> { todo!() }");
        let err = validate_signature(&sig).expect_err("typed first arg is invalid");
        assert!(
            err.to_string().contains("&self"),
            "diagnostic should name &self, got: {err}"
        );
    }

    #[test]
    fn rejects_owned_self_receiver() {
        let sig = parse_sig("fn handler(self) -> Response<()> { todo!() }");
        let err = validate_signature(&sig).expect_err("owned self is invalid");
        let msg = err.to_string();
        assert!(
            msg.contains("&self") && msg.contains("owned"),
            "diagnostic should explain owned-self, got: {msg}"
        );
    }

    #[test]
    fn rejects_mut_self_receiver() {
        let sig = parse_sig("fn handler(&mut self) -> Response<()> { todo!() }");
        let err = validate_signature(&sig).expect_err("&mut self is invalid");
        let msg = err.to_string();
        assert!(
            msg.contains("&mut self") || msg.contains("concurrently"),
            "diagnostic should explain why &mut self is rejected, got: {msg}"
        );
    }

    #[test]
    fn rejects_typed_receiver() {
        let sig = parse_sig("fn handler(self: Box<Self>) -> Response<()> { todo!() }");
        let err = validate_signature(&sig).expect_err("typed receiver is invalid");
        assert!(
            err.to_string().contains("typed receiver"),
            "diagnostic should mention typed receivers, got: {err}"
        );
    }

    #[test]
    fn rejects_missing_return_type() {
        let sig = parse_sig("fn handler(&self) {}");
        let err = validate_signature(&sig).expect_err("missing return type is invalid");
        let msg = err.to_string();
        assert!(
            msg.contains("Response<T>") && msg.contains("no return type"),
            "diagnostic should name Response<T> and explain the missing type, got: {msg}"
        );
    }

    #[test]
    fn rejects_unrelated_return_type() {
        let sig = parse_sig("fn handler(&self) -> String { todo!() }");
        let err = validate_signature(&sig).expect_err("non-Response return is invalid");
        let msg = err.to_string();
        assert!(
            msg.contains("Response<T>") && msg.contains("String"),
            "diagnostic should name Response<T> and the offending type, got: {msg}"
        );
    }

    #[test]
    fn rejects_unit_return_type() {
        // `fn handler(&self) -> () { ... }` has an explicit unit type — caught
        // by the "last path segment is not Response/Result" branch (Type::Tuple,
        // not Type::Path). Pinned because Type::Tuple is one of the few non-Path
        // shapes that's syntactically valid as a return type.
        let sig = parse_sig("fn handler(&self) -> () { todo!() }");
        let err = validate_signature(&sig).expect_err("unit return is invalid");
        assert!(
            err.to_string().contains("Response<T>"),
            "diagnostic should name Response<T>, got: {err}"
        );
    }

    #[test]
    fn rejects_result_with_non_response_first_arg() {
        let sig = parse_sig(
            "fn handler(&self) -> Result<String, AppError> { todo!() }",
        );
        let err = validate_signature(&sig).expect_err("Result<String, _> is invalid");
        let msg = err.to_string();
        assert!(
            msg.contains("Response<T>") && msg.contains("String"),
            "diagnostic should name Response<T> and the wrong inner type, got: {msg}"
        );
    }

    #[test]
    fn rejects_bare_result_without_type_args() {
        // `Result` with no `<...>` is not valid Rust at the type level, but
        // syn happily parses `Result` as a Type::Path with no PathArguments.
        // Pinned so the macro surfaces a clear diagnostic instead of panicking.
        let sig = parse_sig("fn handler(&self) -> Result { todo!() }");
        let err = validate_signature(&sig).expect_err("bare Result is invalid");
        assert!(
            err.to_string().contains("Result<Response<T>, E>"),
            "diagnostic should show the expected shape, got: {err}"
        );
    }

    #[test]
    fn validate_methods_walks_each_discovered_method() {
        // Two valid handlers in the same impl — `validate_methods` should
        // succeed and walk both signatures, not stop after the first.
        let item = parse_impl(
            r#"
            impl Both {
                #[get]
                fn list(&self) -> Response<Vec<User>> { todo!() }

                #[post]
                fn create(&self, body: CreateUser) -> Result<Response<User>, AppError> { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        validate_methods(&methods).expect("both signatures are valid");
    }

    #[test]
    fn validate_methods_surfaces_error_from_offending_method() {
        // First method valid, second invalid (returns `String`). Validation
        // must surface the second method's error rather than silently passing.
        let item = parse_impl(
            r#"
            impl Mixed {
                #[get]
                fn ok_handler(&self) -> Response<()> { todo!() }

                #[post]
                fn bad_handler(&self) -> String { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        let err = validate_methods(&methods).expect_err("bad_handler signature is invalid");
        assert!(
            err.to_string().contains("Response<T>"),
            "diagnostic should name Response<T>, got: {err}"
        );
    }
}
