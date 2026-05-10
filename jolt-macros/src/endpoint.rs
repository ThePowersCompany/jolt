//! `#[endpoint("/path")]` attribute macro — phase08 parsing + phase09 codegen.
//!
//! Phase08 ladder:
//! - JOLT-RS-038: parse the path string literal from the attribute tokens. (landed)
//! - JOLT-RS-039: scan the impl block for `#[get]`/`#[post]`/`#[put]`/
//!   `#[patch]`/`#[delete]` methods, collect their signatures. (landed)
//! - JOLT-RS-040: validate handler signatures — first arg is `&self`, return
//!   type is `Response<T>` or `Result<Response<T>, E>`. (landed)
//! - JOLT-RS-041: emit the (Method, path) -> handler match. (landed)
//!
//! Phase09 ladder:
//! - JOLT-RS-042: wire the parsing + codegen pipeline into the proc-macro
//!   entry point and emit one `inventory::submit!` block per discovered
//!   method, registering a [`jolt_core::RegisteredEndpoint`] so
//!   `JoltServer::start` (JOLT-RS-044) can collect them via
//!   `inventory::iter::<RegisteredEndpoint>()`. (landed)
//! - JOLT-RS-043 (this iteration): emit one `__jolt_handler_<name>` axum-
//!   compatible async wrapper per discovered method via
//!   [`generate_handler_wrappers`]. Each wrapper takes a `::jolt_core::Request`,
//!   constructs `Self` via `Default::default`, calls the user method, and
//!   bridges the return type to `axum::response::Response` through axum's
//!   [`IntoResponse`] trait. JOLT-RS-044 will extend
//!   [`crate::registered_endpoint::RegisteredEndpoint`] with a
//!   `handler: fn(Request) -> EndpointFuture` field pointing at these wrappers.
//!
//! The parsing entry points are split out from `lib.rs` so they can be
//! unit-tested against a `proc_macro2::TokenStream` / parsed `syn::ItemImpl`
//! (proc-macro entry points themselves cannot be invoked outside the compiler).
//!
//! Verb attributes (`#[get]`, `#[post]`, ...) are treated as **magic markers**:
//! `#[endpoint]` recognizes them when scanning, but they are not registered as
//! their own proc-macro attributes. JOLT-RS-042 wires [`strip_verb_attrs`] into
//! the proc-macro entry point so rustc never sees them on the re-emitted impl.

use proc_macro2::Ident;
use quote::{format_ident, quote};
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
/// re-exported). Codegen via [`generate_dispatch_match`] emits
/// `::jolt_core::Method::<variant>` paths using
/// [`HttpMethod::as_jolt_core_variant_ident`].
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
    #[allow(dead_code)] // tests consume it; lib.rs wires it in once a later phase08+ item lands.
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

    /// Identifier for the matching variant in `::jolt_core::Method`, suitable
    /// for splicing into a `quote!` path like `::jolt_core::Method::#ident`.
    /// The five user-routable verbs map 1:1 onto jolt-core's enum variants
    /// (`Method::Get`, `Method::Post`, ...); HEAD/OPTIONS live on jolt-core's
    /// enum but are framework-handled, so this enum has no variants for them.
    fn as_jolt_core_variant_ident(self) -> Ident {
        match self {
            Self::Get => format_ident!("Get"),
            Self::Post => format_ident!("Post"),
            Self::Put => format_ident!("Put"),
            Self::Patch => format_ident!("Patch"),
            Self::Delete => format_ident!("Delete"),
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

/// Strip the magic-marker verb attributes (`#[get]`, `#[post]`, `#[put]`,
/// `#[patch]`, `#[delete]`) from every `ImplItem::Fn` in `item`. Called from
/// the proc-macro entry point before re-emitting the impl block: rustc would
/// otherwise error with "cannot find attribute `get` in this scope" because
/// the verb attributes are recognized only by `#[endpoint]`'s scanner — they
/// are not registered as their own proc-macros (see 039's progress notes).
///
/// Non-fn impl items (consts, types) are untouched. Non-verb attributes
/// (`#[inline]`, `#[doc]`, ...) on methods are preserved.
pub(crate) fn strip_verb_attrs(item: &mut ItemImpl) {
    for impl_item in &mut item.items {
        let ImplItem::Fn(method_fn) = impl_item else {
            continue;
        };
        method_fn
            .attrs
            .retain(|attr| verb_from_attr(attr).is_none());
    }
}

/// Emit `::jolt_core::inventory::submit! { ... }` blocks — one per discovered
/// method — that register a [`::jolt_core::RegisteredEndpoint`] for the
/// (path, method) pair. JOLT-RS-044 will iterate
/// `inventory::iter::<RegisteredEndpoint>()` from `JoltServer::start` to
/// register every entry across all linked crates into an
/// [`::jolt_core::EndpointRegistry`].
///
/// This iteration emits the path + method only; JOLT-RS-043's handler-wrapper
/// codegen will extend the record (or supplement it with a sibling submit) with
/// the handler fn pointer. Splitting the metadata-only submit from the handler
/// submit keeps 042 testable in isolation: the integration test asserts
/// `inventory::iter::<RegisteredEndpoint>()` sees one entry per `#[get]` /
/// `#[post]` / etc. method, with no dependency on handler-wrapper codegen.
fn generate_inventory_submits(
    path: &LitStr,
    methods: &[DiscoveredMethod],
) -> proc_macro2::TokenStream {
    let submits = methods.iter().map(|m| {
        let variant = m.http_method.as_jolt_core_variant_ident();
        quote! {
            ::jolt_core::inventory::submit! {
                ::jolt_core::RegisteredEndpoint {
                    path: #path,
                    method: ::jolt_core::Method::#variant,
                }
            }
        }
    });
    quote! { #(#submits)* }
}

/// Emit one `__jolt_handler_<name>` associated fn per discovered method on the
/// endpoint type. Each wrapper takes a `::jolt_core::Request`, constructs a
/// `Self` instance via `Default::default`, invokes the user's `&self` method,
/// and bridges the return value to an `axum::response::Response` via axum's
/// [`IntoResponse`] trait.
///
/// Wrapper signature:
///
/// ```ignore
/// #[doc(hidden)]
/// pub fn __jolt_handler_<name>(__req: ::jolt_core::Request) -> ::jolt_core::EndpointFuture {
///     ::std::boxed::Box::pin(async move {
///         let _ = __req; // consumed by JOLT-RS-046+ auto-middleware extraction
///         let __endpoint = <Self as ::core::default::Default>::default();
///         let __result = Self::<name>(&__endpoint);
///         <_ as ::axum::response::IntoResponse>::into_response(__result)
///     })
/// }
/// ```
///
/// Decisions pinned here:
///
/// 1. **Per-request `Default::default()` construction.** The progress notes
///    for JOLT-RS-042 flagged the construct-`Self` problem as non-trivial.
///    Default is the simplest forward-compatible choice: it works for unit
///    structs (`#[derive(Default)]`), for structs whose fields all have
///    sensible defaults, and lets future PRDs swap to a state-injected
///    constructor without changing the wrapper's external signature. The
///    `Self: Default` bound is enforced by rustc on the generated code; users
///    get a "the trait `Default` is not implemented for `MyEndpoint`" error
///    at the wrapper site if they forget to derive it.
/// 2. **Wrappers are emitted as a SECOND `impl <SelfTy>` block.** Adding the
///    wrappers to the user's own impl block would mean re-parsing and
///    splicing `ImplItem::Fn` entries into `item.items`, which fights with
///    [`strip_verb_attrs`]'s in-place mutation. A separate impl block is
///    cleaner: it composes via Rust's "multiple inherent impls per type" rule,
///    keeps the user's impl block lossless except for the verb-attr strip,
///    and gives wrappers a stable name-spaced location for 044's
///    `RegisteredEndpoint.handler` to point at via `Self::__jolt_handler_<name>`.
/// 3. **`__req` is bound but unused.** Until JOLT-RS-046 (auto-middleware)
///    lands, the user's method takes `&self` only — there is no body/query
///    extraction to feed the request into. Binding `__req` and immediately
///    `let _ = __req;` keeps the wrapper signature stable for 044's fn-pointer
///    and avoids a `unused_variable` warning. When auto-middleware lands, the
///    `let _ = __req;` line is replaced with field extraction.
/// 4. **Return-shape bridging via `axum::response::IntoResponse`** (not
///    `Into<axum::response::Response>`). axum has a blanket `impl IntoResponse
///    for Result<T: IntoResponse, E: IntoResponse>`, so emitting
///    `IntoResponse::into_response(__result)` covers both `Response<T>` and
///    `Result<Response<T>, E>` shapes uniformly. `Response<T>` itself
///    implements `IntoResponse` via the impl in `jolt-core/src/response.rs`
///    that piggybacks on the existing `From<Response<T>>` conversions.
///
/// Wrappers are `#[doc(hidden)]` because they are macro-internal — no user
/// should call `MyEndpoint::__jolt_handler_xxx` directly. JOLT-RS-044 will
/// reach into them via inventory iteration.
fn generate_handler_wrappers(
    self_ty: &Type,
    methods: &[DiscoveredMethod],
) -> proc_macro2::TokenStream {
    if methods.is_empty() {
        return proc_macro2::TokenStream::new();
    }
    let wrappers = methods.iter().map(|m| {
        let user_fn = &m.sig.ident;
        let wrapper_name = format_ident!("__jolt_handler_{}", user_fn);
        quote! {
            #[doc(hidden)]
            pub fn #wrapper_name(__req: ::jolt_core::Request) -> ::jolt_core::EndpointFuture {
                ::std::boxed::Box::pin(async move {
                    let _ = __req;
                    let __endpoint: Self = <Self as ::core::default::Default>::default();
                    let __result = Self::#user_fn(&__endpoint);
                    <_ as ::axum::response::IntoResponse>::into_response(__result)
                })
            }
        }
    });
    quote! {
        #[automatically_derived]
        impl #self_ty {
            #(#wrappers)*
        }
    }
}

/// Top-level driver invoked by the proc-macro entry point in `lib.rs`. Parses
/// the attribute and impl block, runs the phase08 scan + validate passes,
/// strips magic-marker verb attributes from the re-emitted impl, appends
/// one `inventory::submit!` block per discovered method, and emits a sibling
/// impl block holding one `__jolt_handler_<name>` wrapper per method
/// (JOLT-RS-043).
///
/// Returns the combined token stream (re-emitted impl + submits + wrappers).
/// On failure returns the original `item` tokens plus a `compile_error!` so
/// the user gets a single targeted diagnostic — the same shape JOLT-RS-038
/// used for attribute-parse failures.
pub(crate) fn expand_endpoint(
    attr: proc_macro2::TokenStream,
    item: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let attr_parsed = match parse_endpoint_attr(attr) {
        Ok(a) => a,
        Err(err) => {
            let err_tokens = err.to_compile_error();
            return quote! { #item #err_tokens };
        }
    };
    let mut impl_block: ItemImpl = match parse2(item.clone()) {
        Ok(i) => i,
        Err(err) => {
            let err_tokens = err.to_compile_error();
            return quote! { #item #err_tokens };
        }
    };
    let methods = match scan_methods(&impl_block) {
        Ok(m) => m,
        Err(err) => {
            let err_tokens = err.to_compile_error();
            return quote! { #item #err_tokens };
        }
    };
    if let Err(err) = validate_methods(&methods) {
        let err_tokens = err.to_compile_error();
        return quote! { #item #err_tokens };
    }
    let self_ty = (*impl_block.self_ty).clone();
    strip_verb_attrs(&mut impl_block);
    let submits = generate_inventory_submits(&attr_parsed.path, &methods);
    let wrappers = generate_handler_wrappers(&self_ty, &methods);
    quote! {
        #impl_block
        #submits
        #wrappers
    }
}

/// Emit a dispatch `match` expression with one arm per discovered method,
/// keyed on `(::jolt_core::Method, path)` and resolving to the handler's
/// fn-path on the endpoint type.
///
/// The emitted shape is:
///
/// ```ignore
/// match (__method, __path) {
///     (::jolt_core::Method::Get,  "/api/test") => Self::list,
///     (::jolt_core::Method::Post, "/api/test") => Self::create,
///     _ => unreachable!("..."),
/// }
/// ```
///
/// The wildcard arm is intentional: this dispatch is invoked from the
/// router AFTER it has already matched the path + method against the
/// registry, so the catch-all must be unreachable in well-formed code.
/// Emitting it explicitly keeps the match exhaustive and gives a
/// span-pointing diagnostic if a future bug ever does fall through.
///
/// Match scrutinee idents (`__method`, `__path`) are leading-double-
/// underscore-prefixed so callers introducing those bindings via the
/// surrounding generated code don't collide with user-defined names.
///
/// JOLT-RS-042 will wire this into the proc-macro entry point alongside
/// inventory-style registration; JOLT-RS-043 will adapt the
/// `Self::<handler>` references into tower-compatible async fns.
#[allow(dead_code)] // tests consume it; lib.rs wires it in once JOLT-RS-042/043 land.
pub(crate) fn generate_dispatch_match(
    path: &LitStr,
    methods: &[DiscoveredMethod],
) -> proc_macro2::TokenStream {
    let arms = methods.iter().map(|m| {
        let variant = m.http_method.as_jolt_core_variant_ident();
        let fn_ident = &m.sig.ident;
        quote! {
            (::jolt_core::Method::#variant, #path) => Self::#fn_ident,
        }
    });
    quote! {
        match (__method, __path) {
            #(#arms)*
            _ => ::core::unreachable!(
                "endpoint dispatch fell through to catch-all; the router must only invoke this with a matched (method, path) pair"
            ),
        }
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

    // ----- JOLT-RS-041: generate_dispatch_match -----

    fn parse_path_lit(src: &str) -> LitStr {
        syn::parse_str::<LitStr>(src).expect("test input parses as LitStr")
    }

    fn parse_match(tokens: proc_macro2::TokenStream) -> syn::ExprMatch {
        syn::parse2::<syn::ExprMatch>(tokens)
            .expect("generated tokens parse as a match expression")
    }

    #[test]
    fn generates_two_arms_for_get_and_post() {
        // PRD-mandated verification: two methods (get + post) -> generated code
        // has two match arms.
        let item = parse_impl(
            r#"
            impl Api {
                #[get] fn list(&self) -> Response<()> { todo!() }
                #[post] fn create(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        let path = parse_path_lit(r#""/api/test""#);
        let parsed = parse_match(generate_dispatch_match(&path, &methods));

        // 2 explicit arms (Get, Post) + 1 wildcard catch-all.
        assert_eq!(parsed.arms.len(), 3);
        // First two arms are the discovered methods; the third is the wildcard.
        // Pinned via pattern shape: a tuple pattern for the discovered arms,
        // a `_` pattern for the catch-all.
        assert!(matches!(parsed.arms[0].pat, syn::Pat::Tuple(_)));
        assert!(matches!(parsed.arms[1].pat, syn::Pat::Tuple(_)));
        assert!(matches!(parsed.arms[2].pat, syn::Pat::Wild(_)));
    }

    #[test]
    fn arms_reference_jolt_core_method_variants_and_handler_fn_names() {
        // Pin the arm-body shape: each arm pattern names the right
        // ::jolt_core::Method variant and path literal, and resolves to the
        // handler fn via Self::<ident>. A regression that emitted the wrong
        // variant name (e.g. all arms as Method::Get) or that reused the same
        // handler fn for every arm would fail this test.
        let item = parse_impl(
            r#"
            impl Api {
                #[get] fn list(&self) -> Response<()> { todo!() }
                #[delete] fn destroy(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        let path = parse_path_lit(r#""/items""#);
        let tokens = generate_dispatch_match(&path, &methods);
        let rendered = tokens.to_string();

        // Variant idents on the LHS pattern.
        assert!(
            rendered.contains(":: jolt_core :: Method :: Get"),
            "missing Method::Get variant; rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: jolt_core :: Method :: Delete"),
            "missing Method::Delete variant; rendered: {rendered}"
        );
        // Path literal in the LHS pattern (quoted; tokens render with spaces).
        assert!(
            rendered.contains(r#""/items""#),
            "missing path literal; rendered: {rendered}"
        );
        // Handler fn refs on the RHS.
        assert!(
            rendered.contains("Self :: list"),
            "missing Self::list handler ref; rendered: {rendered}"
        );
        assert!(
            rendered.contains("Self :: destroy"),
            "missing Self::destroy handler ref; rendered: {rendered}"
        );
    }

    #[test]
    fn empty_methods_emits_only_catch_all_arm() {
        // An impl with no verb-tagged methods (e.g. helpers only) still
        // generates a syntactically-valid match — just the wildcard arm.
        // This pins that the generator doesn't depend on at-least-one-arm and
        // that the wildcard's unreachable! body always emits.
        let path = parse_path_lit(r#""/empty""#);
        let parsed = parse_match(generate_dispatch_match(&path, &[]));
        assert_eq!(parsed.arms.len(), 1);
        assert!(matches!(parsed.arms[0].pat, syn::Pat::Wild(_)));
    }

    #[test]
    fn arms_share_the_same_path_literal_across_all_methods() {
        // All methods on a single #[endpoint] share the path attribute. A
        // regression that re-stringified the path per arm and dropped a leading
        // slash, or that emitted the path-ident name instead of its literal
        // value, would fail this test.
        let item = parse_impl(
            r#"
            impl Api {
                #[get] fn a(&self) -> Response<()> { todo!() }
                #[post] fn b(&self) -> Response<()> { todo!() }
                #[put] fn c(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        let path = parse_path_lit(r#""/users/:id""#);
        let rendered = generate_dispatch_match(&path, &methods).to_string();
        // Path literal (with token-spacing) must appear exactly three times —
        // once per discovered method's arm pattern.
        let needle = r#""/users/:id""#;
        let count = rendered.matches(needle).count();
        assert_eq!(
            count, 3,
            "path literal must appear once per arm, got {count}; rendered: {rendered}"
        );
    }

    // ----- JOLT-RS-042: strip_verb_attrs / generate_inventory_submits / expand_endpoint -----

    #[test]
    fn strip_verb_attrs_removes_get_and_post_keeps_others() {
        // Input has a mix of verb attrs (must be stripped) and non-verb attrs
        // (`#[inline]`, `#[doc = ...]` — must survive). Verifies the strip is
        // attribute-name-narrow, not attribute-class-broad.
        let mut item = parse_impl(
            r#"
            impl Api {
                #[get]
                #[inline]
                fn list(&self) -> Response<()> { todo!() }

                #[doc = "create a thing"]
                #[post]
                fn create(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        strip_verb_attrs(&mut item);
        // Re-scan: no more verb attrs should be discoverable (since they're
        // gone from the impl block).
        let still_discovered = scan_methods(&item).expect("scan succeeds");
        assert!(
            still_discovered.is_empty(),
            "verb attrs should be gone after strip, got {} methods",
            still_discovered.len()
        );
        // Non-verb attrs survive: each method's remaining attrs list is
        // exactly the non-verb ones.
        let surviving_attr_names: Vec<Vec<String>> = item
            .items
            .iter()
            .filter_map(|i| match i {
                ImplItem::Fn(f) => Some(
                    f.attrs
                        .iter()
                        .map(|a| {
                            a.path()
                                .get_ident()
                                .map(|i| i.to_string())
                                .unwrap_or_default()
                        })
                        .collect(),
                ),
                _ => None,
            })
            .collect();
        assert_eq!(
            surviving_attr_names,
            vec![
                vec!["inline".to_string()],
                vec!["doc".to_string()],
            ],
        );
    }

    #[test]
    fn strip_verb_attrs_does_not_touch_non_fn_items() {
        // Strip walks `item.items` looking for `ImplItem::Fn` only. Non-fn
        // items (`const`, `type`) must be left alone — including any
        // attributes attached to them.
        let mut item = parse_impl(
            r#"
            impl Mixed {
                #[doc = "max"]
                const MAX: usize = 32;

                type Output = String;

                #[get]
                fn handler(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        strip_verb_attrs(&mut item);
        // Find the const and check its attribute survived.
        let const_attr_count = item
            .items
            .iter()
            .filter_map(|i| match i {
                ImplItem::Const(c) => Some(c.attrs.len()),
                _ => None,
            })
            .next()
            .expect("const item present");
        assert_eq!(const_attr_count, 1, "const's #[doc] must survive strip");
    }

    #[test]
    fn generate_inventory_submits_one_per_method() {
        // PRD-mandated verification axis: each discovered method produces an
        // independent inventory::submit! block, so JOLT-RS-044's iter() will
        // see one entry per (path, method) pair.
        let item = parse_impl(
            r#"
            impl Api {
                #[get] fn list(&self) -> Response<()> { todo!() }
                #[post] fn create(&self) -> Response<()> { todo!() }
                #[delete] fn destroy(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        let path = parse_path_lit(r#""/api/test""#);
        let rendered = generate_inventory_submits(&path, &methods).to_string();
        // One submit per method.
        let submit_count = rendered.matches(":: jolt_core :: inventory :: submit").count();
        assert_eq!(submit_count, 3, "expected 3 submits, rendered: {rendered}");
        // Each submit references the correct method variant.
        for variant in ["Get", "Post", "Delete"] {
            let needle = format!(":: jolt_core :: Method :: {variant}");
            assert!(
                rendered.contains(&needle),
                "missing variant {variant} in rendered submits: {rendered}"
            );
        }
        // Path literal appears once per submit.
        let path_count = rendered.matches(r#""/api/test""#).count();
        assert_eq!(path_count, 3, "path should appear once per submit");
    }

    #[test]
    fn generate_inventory_submits_empty_methods_emits_nothing() {
        // No verb-tagged methods → no submits. (The impl block is still
        // re-emitted by expand_endpoint, but the submits stream is empty.)
        let path = parse_path_lit(r#""/empty""#);
        let rendered = generate_inventory_submits(&path, &[]).to_string();
        assert_eq!(rendered, "", "empty methods must emit zero submits");
    }

    #[test]
    fn expand_endpoint_emits_impl_plus_submit_for_single_method() {
        // PRD-mandated verification (042): `#[endpoint("/test")]` on an impl
        // with one verb-tagged method emits the re-stripped impl AND one
        // inventory::submit! block. The integration test in
        // jolt-core/tests/inventory_registration.rs runs the same shape end-
        // to-end through cargo's compile pipeline; this unit test pins the
        // token-stream shape so a regression surfaces here first.
        let attr = proc_macro2::TokenStream::from_str(r#""/test""#).unwrap();
        let item = proc_macro2::TokenStream::from_str(
            r#"
            impl Probe {
                #[get]
                fn ping(&self) -> Response<()> { todo!() }
            }
            "#,
        )
        .unwrap();
        let rendered = expand_endpoint(attr, item).to_string();
        // Submit emitted.
        assert!(
            rendered.contains(":: jolt_core :: inventory :: submit"),
            "expected inventory::submit!, rendered: {rendered}"
        );
        assert!(
            rendered.contains(":: jolt_core :: Method :: Get"),
            "expected Method::Get, rendered: {rendered}"
        );
        assert!(
            rendered.contains(r#""/test""#),
            "expected /test path literal, rendered: {rendered}"
        );
        // Verb attribute stripped from re-emitted impl.
        assert!(
            !rendered.contains("# [get]"),
            "verb attribute should be stripped, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_endpoint_surfaces_compile_error_on_bad_signature() {
        // A method with `&self -> String` (non-Response return) must produce
        // a compile_error! token rather than silently emitting an inventory
        // submit. The user-visible diagnostic comes from validate_signature.
        let attr = proc_macro2::TokenStream::from_str(r#""/bad""#).unwrap();
        let item = proc_macro2::TokenStream::from_str(
            r#"
            impl Bad {
                #[get]
                fn handler(&self) -> String { todo!() }
            }
            "#,
        )
        .unwrap();
        let rendered = expand_endpoint(attr, item).to_string();
        assert!(
            rendered.contains("compile_error"),
            "expected compile_error! for bad signature, rendered: {rendered}"
        );
        assert!(
            !rendered.contains(":: jolt_core :: inventory :: submit"),
            "must NOT emit inventory submit when validation fails"
        );
    }

    #[test]
    fn expand_endpoint_surfaces_compile_error_on_non_impl_item() {
        // The `#[endpoint("/path")]` macro expects an impl block. A free fn
        // is not parseable as ItemImpl — expand_endpoint must surface a
        // compile_error! pointing at the bad item shape rather than panicking.
        let attr = proc_macro2::TokenStream::from_str(r#""/path""#).unwrap();
        let item = proc_macro2::TokenStream::from_str("fn not_an_impl() {}").unwrap();
        let rendered = expand_endpoint(attr, item).to_string();
        assert!(
            rendered.contains("compile_error"),
            "expected compile_error! for non-impl item, rendered: {rendered}"
        );
    }

    // ----- JOLT-RS-043: generate_handler_wrappers -----

    fn parse_self_ty(src: &str) -> Type {
        syn::parse_str::<Type>(src).expect("test input parses as Type")
    }

    #[test]
    fn generate_handler_wrappers_emits_one_fn_per_method() {
        // Two discovered methods → two wrappers in the emitted impl block.
        // Wrapper names follow the `__jolt_handler_<user_fn>` pattern so that
        // JOLT-RS-044's inventory record can reference them via
        // `Self::__jolt_handler_ping` / `Self::__jolt_handler_record`.
        let item = parse_impl(
            r#"
            impl Probe {
                #[get] fn ping(&self) -> Response<()> { todo!() }
                #[post] fn record(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        let self_ty = parse_self_ty("Probe");
        let rendered = generate_handler_wrappers(&self_ty, &methods).to_string();

        // One pub fn per method, named `__jolt_handler_<user_fn>`.
        assert!(
            rendered.contains("__jolt_handler_ping"),
            "expected __jolt_handler_ping wrapper, rendered: {rendered}"
        );
        assert!(
            rendered.contains("__jolt_handler_record"),
            "expected __jolt_handler_record wrapper, rendered: {rendered}"
        );
        // Each wrapper takes a Request and returns an EndpointFuture.
        let req_count = rendered.matches(":: jolt_core :: Request").count();
        assert_eq!(
            req_count, 2,
            "each wrapper takes one Request, so the path must appear twice; rendered: {rendered}"
        );
        let fut_count = rendered.matches(":: jolt_core :: EndpointFuture").count();
        assert_eq!(
            fut_count, 2,
            "each wrapper returns one EndpointFuture, so the path must appear twice; rendered: {rendered}"
        );
        // Bodies use IntoResponse::into_response to bridge to axum.
        let into_resp_count = rendered
            .matches(":: axum :: response :: IntoResponse")
            .count();
        assert_eq!(
            into_resp_count, 2,
            "each wrapper bridges via IntoResponse, rendered: {rendered}"
        );
        // Bodies invoke the user method via Self::<ident>(&self).
        assert!(
            rendered.contains("Self :: ping"),
            "wrapper must invoke user fn via Self::ping, rendered: {rendered}"
        );
        assert!(
            rendered.contains("Self :: record"),
            "wrapper must invoke user fn via Self::record, rendered: {rendered}"
        );
    }

    #[test]
    fn generate_handler_wrappers_constructs_self_via_default() {
        // Pin the construct-Self decision: wrapper uses `<Self as Default>::default()`
        // to instantiate the endpoint per request. A regression that switched to
        // `Self::new()`, a static, or a thread-local would fail this test.
        let item = parse_impl(
            r#"
            impl Probe {
                #[get] fn ping(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        let self_ty = parse_self_ty("Probe");
        let rendered = generate_handler_wrappers(&self_ty, &methods).to_string();
        assert!(
            rendered.contains(":: core :: default :: Default"),
            "wrapper must construct Self via Default::default; rendered: {rendered}"
        );
    }

    #[test]
    fn generate_handler_wrappers_empty_methods_emits_nothing() {
        // No verb-tagged methods → no wrapper impl block. The user's own impl
        // block is emitted unchanged by expand_endpoint; this helper only
        // contributes the wrappers, which collapse to empty when there are
        // none. (Matches the same shape as `generate_inventory_submits`.)
        let self_ty = parse_self_ty("Empty");
        let rendered = generate_handler_wrappers(&self_ty, &[]).to_string();
        assert_eq!(rendered, "", "empty methods must emit zero wrappers");
    }

    #[test]
    fn generate_handler_wrappers_threads_self_ty_through_impl_block() {
        // `self_ty` is read from the original `ItemImpl::self_ty`. Pin that
        // it appears verbatim in the generated `impl <SelfTy> { ... }` block,
        // including for path-style types (e.g. `crate::api::User`). A
        // regression that hard-coded `Self` at the top level (instead of
        // splicing `self_ty`) would fail this test.
        let item = parse_impl(
            r#"
            impl crate::api::User {
                #[get] fn fetch(&self) -> Response<()> { todo!() }
            }
            "#,
        );
        let methods = scan_methods(&item).expect("scan succeeds");
        let self_ty = (*item.self_ty).clone();
        let rendered = generate_handler_wrappers(&self_ty, &methods).to_string();
        assert!(
            rendered.contains("impl crate :: api :: User"),
            "self_ty must appear in the emitted impl block, rendered: {rendered}"
        );
    }

    #[test]
    fn expand_endpoint_emits_handler_wrapper_alongside_submits() {
        // End-to-end shape of the 043 driver path: parsing + scan + validate
        // succeed, then BOTH the inventory submit AND the handler wrapper are
        // emitted. The integration test in jolt-core/tests/inventory_registration.rs
        // exercises the same shape through cargo's compile pipeline; this unit
        // test is the fast-feedback parse-check for regressions.
        let attr = proc_macro2::TokenStream::from_str(r#""/users""#).unwrap();
        let item = proc_macro2::TokenStream::from_str(
            r#"
            impl Users {
                #[get] fn list(&self) -> Response<()> { todo!() }
            }
            "#,
        )
        .unwrap();
        let rendered = expand_endpoint(attr, item).to_string();
        // Inventory submit (042 contract) is still emitted.
        assert!(
            rendered.contains(":: jolt_core :: inventory :: submit"),
            "expected inventory::submit!, rendered: {rendered}"
        );
        // Handler wrapper (043 contract) is emitted alongside.
        assert!(
            rendered.contains("__jolt_handler_list"),
            "expected __jolt_handler_list wrapper, rendered: {rendered}"
        );
        // The wrapper's impl block targets the user's self_ty (`Users`).
        assert!(
            rendered.contains("impl Users"),
            "wrapper impl block must target Users, rendered: {rendered}"
        );
    }
}
