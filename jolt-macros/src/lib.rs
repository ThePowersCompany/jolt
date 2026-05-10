#![doc = "jolt-macros: proc-macro support for Jolt (endpoint registration, middleware, patch queries, TS typegen)."]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, ItemFn};

mod endpoint;

/// `#[endpoint("/path")]` attribute macro.
///
/// JOLT-RS-038 lands path parsing only: this iteration validates that the
/// attribute argument is a string literal and re-emits the annotated item
/// unchanged. Method discovery (JOLT-RS-039), signature validation
/// (JOLT-RS-040), and handler-match codegen (JOLT-RS-041) layer on top in
/// subsequent PRD items.
///
/// On parse failure the original item is preserved AND a `compile_error!`
/// is appended, so the user gets a single targeted diagnostic instead of a
/// cascade of "use of undeclared type" errors from later code that names the
/// item.
#[proc_macro_attribute]
pub fn endpoint(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr2: proc_macro2::TokenStream = attr.into();
    let item2: proc_macro2::TokenStream = item.into();
    match endpoint::parse_endpoint_attr(attr2) {
        Ok(_parsed) => item2.into(),
        Err(err) => {
            let err_tokens = err.to_compile_error();
            quote! {
                #item2
                #err_tokens
            }
            .into()
        }
    }
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
