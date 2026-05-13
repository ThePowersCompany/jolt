#![doc = "joltr-macros: proc-macro support for JoltR (endpoint registration, middleware, patch queries, TS typegen)."]

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, ItemFn};

mod auto_middleware;
mod endpoint;
mod patch_query;
mod ts_export;
mod ws;

/// `#[endpoint("/path")]` attribute macro.
///
/// As of JOLTR-RS-043 the macro:
/// 1. parses the route-path string literal from the attribute (038),
/// 2. scans the impl block for `#[get]`/`#[post]`/`#[put]`/`#[patch]`/
///    `#[delete]` methods (039),
/// 3. validates each method's signature is `&self -> Response<T>` /
///    `Result<Response<T>, E>` (040),
/// 4. strips the magic-marker verb attributes from the re-emitted impl,
/// 5. emits one `::joltr_core::inventory::submit!` block per discovered method
///    so `JoltRServer::start` (JOLTR-RS-044) can collect the routes via
///    `inventory::iter::<RegisteredEndpoint>()` (042), and
/// 6. emits one `__jolt_handler_<name>` axum-compatible async wrapper per
///    discovered method on a sibling `impl <SelfTy>` block (043). Each wrapper
///    takes `::joltr_core::Request` and returns `::joltr_core::EndpointFuture`,
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

/// `#[derive(AutoMiddleware)]` proc-macro derive.
///
/// The macro parses named structs, classifies fields as body, query params,
/// request injection, or ordinary `Default` fields, and accepts the helper
/// attribute `#[cors]` on the struct. It emits:
/// 1. hidden marker consts for field count and CORS detection,
/// 2. a `tower::Layer` impl plus generated wrapper service,
/// 3. canonical ordering marker statements for active CORS/query/body steps,
/// 4. extraction helpers that populate middleware fields from `joltr_core::Request`,
/// 5. a field-bearing service path that honors finished `RequestExt` responses,
///    runs request extraction before delegation, and returns the framework's
///    400 query-error response for invalid typed `QueryParams<T>` extraction.
///
/// By-value `Request` fields clone the active request, by-ref `&Request` fields
/// borrow it using the user's lifetime, typed `QueryParams<T>` fields deserialize
/// from the parsed query map, raw query maps are cloned, and body fields parse
/// JSON via `Request::json::<T>()`.
///
/// On parse failure the emission is a single `compile_error!` token (no
/// partial codegen), so the user gets a single targeted diagnostic instead of
/// a cascade of "use of undeclared type" errors from later code.
#[proc_macro_derive(AutoMiddleware, attributes(cors))]
pub fn auto_middleware_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    auto_middleware::expand_auto_middleware(input).into()
}

/// `#[derive(PatchQuery)]` proc-macro derive.
///
/// The macro parses named structs, the struct-level `#[patch("table_name")]`
/// helper attribute, and `Optional<T>` fields. It emits hidden marker consts
/// for field count, optional-field count, and table name, plus
/// `to_patch_query(&self, id_column, id_value) -> (String, Vec<&dyn ToSql>)`
/// when a table is configured.
///
/// Generated patch queries use PostgreSQL-style `$N` placeholders. Plain fields
/// are always included in the `SET` clause; `Optional::Some` fields bind a
/// value, `Optional::Null` writes `NULL`, and `Optional::NotProvided` skips the
/// field. The `WHERE` id value is always appended as the final bound parameter.
///
/// On parse failure the emission is a single `compile_error!` token — no
/// partial codegen. Mirrors `#[derive(AutoMiddleware)]`'s contract from
/// JOLTR-RS-046.
#[proc_macro_derive(PatchQuery, attributes(patch))]
pub fn patch_query_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    patch_query::expand_patch_query(input).into()
}

/// `#[derive(TsExport)]` proc-macro derive.
///
/// The macro accepts named/unit structs, simple enums, and data-carrying enums.
/// It emits hidden marker consts used by tests and registers a structured
/// `joltr_types::TsTypeDef` in the link-time inventory consumed by the
/// `joltr-types` renderer.
///
/// Supported type rendering covers primitives, `Vec<T>`, `JsonArray<T>`,
/// `Option<T>`, `Optional<T>`, transparent `Json<T>`, Rust type parameters, and
/// user-defined path references. Field-level `#[ts(rename = "...")]` and doc
/// comments are included in the submitted TypeScript field metadata. The
/// `#[ts(flatten)]` marker is preserved on hidden consts, but registry rendering
/// keeps the field as an ordinary property because cross-type flattening is not
/// available at macro expansion time.
///
/// On parse failure the emission is a single `compile_error!` token — no
/// partial codegen. Mirrors the contract from `#[derive(PatchQuery)]`
/// (JOLTR-RS-110).
#[proc_macro_derive(TsExport, attributes(ts))]
pub fn ts_export_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    ts_export::expand_ts_export(input).into()
}

/// `ws!(path, HandlerType, subprotocol = "...", auth_fn = fn_name)` —
/// function-like proc-macro for declaring an axum WebSocket route with a
/// JWT-subprotocol auth gate.
///
/// The macro parses a string-literal route path, a handler type, and named
/// `subprotocol` / `auth_fn` arguments. It emits an axum-compatible async route
/// handler that compile-time-checks `HandlerType: WebSocketHandler + Default +
/// Send`, requires `auth_fn: Fn(&str) -> Result<JwtClaims, AuthError>`, extracts
/// the JWT from `Sec-WebSocket-Protocol`, returns 401 for token extraction or
/// auth failures, upgrades the WebSocket, and drives the handler lifecycle:
/// `set_claims -> on_open -> on_ready -> on_message* -> on_close -> writer drain
/// -> on_shutdown`.
///
/// On parse failure the emission is a single `compile_error!` token (no
/// partial codegen) — mirrors the contract from JOLTR-RS-046 / JOLTR-RS-110.
#[proc_macro]
pub fn ws(input: TokenStream) -> TokenStream {
    let input2: proc_macro2::TokenStream = input.into();
    ws::expand_ws_macro(input2).into()
}

/// Placeholder attribute macro. Future PRD items will expand this into
/// auto-middleware and patch-query attributes.
#[proc_macro_attribute]
pub fn joltr_placeholder(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let output = quote! { #input };
    output.into()
}

/// Placeholder derive macro. Future PRD items will expand this into TypeScript
/// typegen and request/response body derivations.
#[proc_macro_derive(JoltRPlaceholder)]
pub fn joltr_placeholder_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let output = quote! {
        impl #name {
            #[doc(hidden)]
            pub fn __joltr_placeholder() {}
        }
    };
    output.into()
}
