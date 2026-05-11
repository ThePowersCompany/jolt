#![doc = "jolt-macros: proc-macro support for Jolt (endpoint registration, middleware, patch queries, TS typegen)."]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, ItemFn};

mod auto_middleware;
mod endpoint;
mod patch_query;

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

/// `#[derive(AutoMiddleware)]` proc-macro derive — phase10 entry point.
///
/// JOLT-RS-046 (current): parses the struct's named fields and their types via
/// [`auto_middleware::parse_auto_middleware_input`], then emits a hidden
/// `__JOLT_AUTO_MIDDLEWARE_FIELD_COUNT` const so an integration test can
/// observe that the derive ran.
///
/// Subsequent phase10/11 items extend this expansion:
/// - 047 (landed): classifies `body`-named fields as `FieldKind::Body`,
/// - 048 (landed): classifies `QueryParams<T>` (any name) and `query_params:
///   HashMap<String, String>` as `FieldKind::QueryParams`,
/// - 049 (landed): classifies `Request` and `&Request` (any name, with or
///   without lifetime) as `FieldKind::Request`,
/// - 050 (landed): detects the struct-level `#[cors]` attribute and emits a
///   second hidden marker `__JOLT_AUTO_MIDDLEWARE_CORS: bool`. The
///   `attributes(cors)` opt-in tells the compiler to treat `#[cors]` as a
///   helper attribute owned by this derive (without it, the compiler would
///   reject `#[cors]` as an unknown macro at the user's source site before
///   the derive runs).
/// - 051 (landed): emits a real `::jolt_core::tower::Layer` impl on the user
///   struct plus a `#[doc(hidden)]` wrapper `__JoltAutoMiddleware<Ident>Service`
///   that delegates to the inner service. The wrapper is the seam JOLT-RS-052
///   (middleware-ordering chain) and JOLT-RS-053 (per-field extraction code)
///   slot into.
/// - 052 (landed): splices canonical-order step markers into the wrapper's
///   `call()` body via `middleware_chain` + `MiddlewareStep`. Each active
///   step (cors / parse-query / parse-body) renders as a stable
///   `let _: &::core::primitive::str = "jolt::middleware::step::<name>";`
///   statement in canonical order BEFORE the existing inner.call delegation.
///   Auth/log/user steps are future PRD items.
/// - 053 (landed): emits a per-derive
///   `__jolt_extract_from(req: &::jolt_core::Request) -> Self` method on the
///   user's struct via `expand_extraction`. The method constructs `Self { ... }`
///   with each field initialised by an expression matched to its `FieldKind`
///   (Body via `req.json::<T>()`, HashMap-shaped QueryParams via clone of
///   `req.query_params`, by-value Request via clone, Other via
///   `<T as Default>::default()`). Typed `QueryParams<T>` and by-ref
///   `&Request` are emitted as `unimplemented!(...)` placeholders pending
///   future PRD items. The 052 chain markers in the wrapper's `call()` body
///   stay as marker statements — replacing them with calls into the helper
///   would break 051's generic-over-`__Req` design and is deferred.
///
/// On parse failure the emission is a single `compile_error!` token (no
/// partial codegen), so the user gets a single targeted diagnostic instead of
/// a cascade of "use of undeclared type" errors from later code.
#[proc_macro_derive(AutoMiddleware, attributes(cors))]
pub fn auto_middleware_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    auto_middleware::expand_auto_middleware(input).into()
}

/// `#[derive(PatchQuery)]` proc-macro derive — phase26 entry point.
///
/// JOLT-RS-110 (current): parses the struct's named fields and their types via
/// [`patch_query::parse_patch_query_input`], then emits a hidden
/// `__JOLT_PATCH_QUERY_FIELD_COUNT: usize` const so an integration test can
/// observe that the derive ran on `Optional<T>`-containing structs.
///
/// Subsequent phase26/27 items extend this expansion:
/// - 111: parse the struct-level `#[patch("table_name")]` attribute. The
///   `attributes(patch)` opt-in below tells the compiler to treat `#[patch]`
///   as a helper attribute owned by this derive (without it the compiler
///   would reject `#[patch("...")]` as an unknown macro at the user's source
///   site before the derive runs).
/// - 112: detect `Optional<T>` fields and extract inner `T`.
/// - 113: build the `Vec<PatchField>` internal representation
///   (`name`/`column_name`/`is_optional`/`inner_type`).
/// - 114-116: emit `fn to_patch_query(&self, id_column, id_value) ->
///   (String, Vec<&dyn ToSql>)` and the `$N`-parameterized SET clause
///   builder.
/// - 117: closing-test bundle for phase27.
///
/// On parse failure the emission is a single `compile_error!` token — no
/// partial codegen. Mirrors `#[derive(AutoMiddleware)]`'s contract from
/// JOLT-RS-046.
#[proc_macro_derive(PatchQuery, attributes(patch))]
pub fn patch_query_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    patch_query::expand_patch_query(input).into()
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
