#![doc = "jolt-macros: proc-macro support for Jolt (endpoint registration, middleware, patch queries, TS typegen)."]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, ItemFn};

mod endpoint;

/// `#[endpoint("/path")]` attribute macro.
///
/// As of JOLT-RS-043 the macro:
/// 1. parses the route-path string literal from the attribute (038),
/// 2. scans the impl block for `#[get]`/`#[post]`/`#[put]`/`#[patch]`/
///    `#[delete]` methods (039),
/// 3. validates each method's signature is `&self -> Response<T>` /
///    `Result<Response<T>, E>` (040),
/// 4. strips the magic-marker verb attributes from the re-emitted impl,
/// 5. emits one `::jolt_core::inventory::submit!` block per discovered method
///    so `JoltServer::start` (JOLT-RS-044) can collect the routes via
///    `inventory::iter::<RegisteredEndpoint>()` (042), and
/// 6. emits one `__jolt_handler_<name>` axum-compatible async wrapper per
///    discovered method on a sibling `impl <SelfTy>` block (043). Each wrapper
///    takes `::jolt_core::Request` and returns `::jolt_core::EndpointFuture`,
///    constructing `Self` via `Default::default` and bridging the user's
///    return value to `axum::response::Response` via axum's `IntoResponse`.
///
/// Inventory-based auto-registration (044) and the e2e dispatch test (045)
/// layer on top in subsequent PRD items.
///
/// On any parse / scan / validate failure the original item is preserved AND
/// a `compile_error!` is appended, so the user gets a single targeted
/// diagnostic instead of a cascade of "use of undeclared type" errors from
/// later code that names the item.
#[proc_macro_attribute]
pub fn endpoint(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr2: proc_macro2::TokenStream = attr.into();
    let item2: proc_macro2::TokenStream = item.into();
    endpoint::expand_endpoint(attr2, item2).into()
}

/// Placeholder attribute macro. Future PRD items will expand this into
/// auto-middleware and patch-query attributes.
#[proc_macro_attribute]
pub fn jolt_placeholder(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let output = quote! { #input };
    output.into()
}

/// Placeholder derive macro. Future PRD items will expand this into TypeScript
/// typegen and request/response body derivations.
#[proc_macro_derive(JoltPlaceholder)]
pub fn jolt_placeholder_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let output = quote! {
        impl #name {
            #[doc(hidden)]
            pub fn __jolt_placeholder() {}
        }
    };
    output.into()
}
