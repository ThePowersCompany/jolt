//! Query string parsing `tower::Layer` (JOLTR-RS-063) plus typed field-level
//! extractors (JOLTR-RS-064+).
//!
//! [`ParseQueryLayer`] reads the inbound request's URI query string, splits it
//! into key/value pairs, and stashes the resulting
//! [`QueryParams`] (a `HashMap<String, String>` newtype) into request
//! extensions so a downstream service (or the AutoMiddleware-derived struct
//! consuming the request) can pull it back out with
//! `req.extensions().get::<QueryParams>()`.
//!
//! [`extract`] (JOLTR-RS-064) consumes the parsed map: given a key and a
//! target type `T: FromStr`, it returns `Result<T, QueryExtractError>` so
//! callers can route a missing-key or unparseable-value case to a
//! `400 Bad Request` via [`bad_request_for_query_error`]. The extractor is a
//! free function rather than another tower layer because the typed shape only
//! exists once a specific field's `T` is known — the natural caller is the
//! AutoMiddleware codegen, which knows the per-field type at compile time.
//!
//! [`extract_bool`], [`extract_string`], and [`extract_enum`] (JOLTR-RS-065)
//! are sibling helpers for the three target types whose parser shape doesn't
//! match [`extract`]'s `T: FromStr` bound:
//!
//! - [`extract_bool`] adds the `"1"`/`"0"` aliases (and case-insensitive
//!   match) on top of [`bool`]'s `FromStr`, which only accepts `"true"` and
//!   `"false"`.
//! - [`extract_string`] is a pass-through that skips the
//!   `Result<String, Infallible>` chain that `extract::<String>` would force
//!   the caller through.
//! - [`extract_enum`] targets [`TryFrom<&str>`] instead of [`FromStr`] so
//!   user enums with hand-rolled (or `#[derive(strum::EnumString)]`-style)
//!   string-to-variant maps can wire in.
//!
//! [`extract_vec`] (JOLTR-RS-066) splits the value on commas and parses each
//! element as `T: FromStr`. Element-level failures surface as the new
//! [`QueryExtractError::InvalidElement`] variant which carries the failing
//! position so a caller debugging `?ids=1,abc,3` can read off `index 1` from
//! the 400 body without guessing.
//!
//! Architectural decisions pinned here for JOLTR-RS-064..067 to build on:
//!
//! 1. **Layer carries no type parameter; output is always
//!    [`QueryParams`].** JOLTR-RS-063 mandates "key-value pairs" only — typed
//!    extraction (int/float, bool, enum, `Vec<T>`) lands in 064–066 as
//!    *consumers* of this map. The map is the foundation, not a typed shape.
//!    A future `ParseQueryLayer<T>` over a deserializable struct (mirroring
//!    [`ParseBodyLayer<T>`](crate::ParseBodyLayer)) is a sibling layer, not a
//!    parameterization of this one — it would target a different decoding
//!    surface (`serde_urlencoded::from_str`) and have a different failure mode.
//!
//! 2. **`QueryParams` is a newtype over `HashMap<String, String>`, not the
//!    raw `HashMap` itself.** Inserting a bare `HashMap<String, String>` into
//!    request extensions would collide with any other layer or handler that
//!    happens to stash one for a different purpose (request extensions are
//!    keyed by `TypeId`). The newtype gives this layer a unique extension key
//!    while still being one `Deref` from the underlying map for ergonomics.
//!
//! 3. **Empty / missing query string inserts an empty [`QueryParams`].** The
//!    extension is ALWAYS present after the layer runs; downstream consumers
//!    can call `.get::<QueryParams>().unwrap()` (or expect-with-message) without
//!    a `?query=` upstream. Making the extension conditional on a non-empty
//!    query would force every consumer to handle two shapes (present-empty vs.
//!    absent) for the same logical state.
//!
//! 4. **Parsing is infallible.** Malformed pairs (no `=`, repeated `&`, etc.)
//!    are silently dropped — same shape as
//!    [`endpoint_registry::parse_query`](crate::endpoint_registry) which has
//!    been the framework's de-facto query parser since JOLTR-RS-034. Rejecting
//!    a `?foo&bar=1` query as 400 would be more strict than the existing JoltR
//!    [`Request::query`](crate::Request::query) contract; a future
//!    typed-extractor layer (064+) can choose to surface 400 on per-value type
//!    errors without changing the foundational map's permissive shape.
//!
//! 5. **No body buffering, no `Arc<RequestExt>` preserve-or-inject.** Unlike
//!    [`ParseBodyLayer`](crate::ParseBodyLayer) and
//!    [`CorsLayer`](crate::CorsLayer), this layer doesn't fail and doesn't
//!    short-circuit, so it has no reason to flip a finished latch. The
//!    request flows through unchanged except for the inserted extension.
//!    064+ typed extractors that DO surface 400s will need the preserve-or-
//!    inject dance; this foundational layer does not.
//!
//!    Note (JOLTR-RS-078): the layer still *reads* an upstream
//!    [`Arc<RequestExt>`](crate::RequestExt)'s `finished` latch — if an
//!    earlier layer has already finished the request, this layer skips the
//!    query parse + extension insert and delegates to inner so the
//!    already-determined response propagates. The check is read-only; the
//!    layer does not inject a fresh ext when none is present (no
//!    mark_finished side effect to observe on the active path).

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use tower::{Layer, Service};

use crate::request_ext::RequestExt;

/// Newtype wrapper over `HashMap<String, String>` used as the request-extension
/// key for the parsed query map (JOLTR-RS-063). The newtype shields downstream
/// consumers from collisions with any other `HashMap<String, String>` that
/// might be stashed in extensions for unrelated reasons. `Deref` exposes the
/// underlying map's read API directly; no separate forwarding methods needed.
///
/// `Default` is derived so [`ParseQueryService::call`] can produce an empty
/// instance without manual construction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryParams<T = HashMap<String, String>>(pub T);

impl<T> std::ops::Deref for QueryParams<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for QueryParams<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> From<T> for QueryParams<T> {
    fn from(value: T) -> Self {
        Self(value)
    }
}

pub type QueryErrorResponse = Response;

/// `tower::Layer` that parses the request URI's query string into a
/// [`QueryParams`] map and stashes it in request extensions. See module docs
/// for the architectural contract (extension key, infallibility, always-insert
/// shape).
///
/// Carries no runtime state; cloning produces a functionally identical layer.
#[derive(Default, Clone, Debug)]
pub struct ParseQueryLayer;

impl ParseQueryLayer {
    /// Construct a query parser layer. The layer carries no runtime state, so
    /// a fresh layer is functionally identical to any other.
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for ParseQueryLayer {
    type Service = ParseQueryService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ParseQueryService { inner }
    }
}

/// Inner-service wrapper produced by [`ParseQueryLayer::layer`]. Inserts the
/// parsed [`QueryParams`] into request extensions before delegating to the
/// inner service. See [`ParseQueryLayer`] for the architectural contract.
#[derive(Clone, Debug)]
pub struct ParseQueryService<S> {
    inner: S,
}

impl<S> Service<AxumRequest> for ParseQueryService<S>
where
    S: Service<AxumRequest, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response, S::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut req: AxumRequest) -> Self::Future {
        // Standard tower delegation pattern: poll_ready was driven on the
        // current `self.inner`, so `call` must use that same instance. Replace
        // it with a clone we DON'T call; the caller's next poll_ready readies
        // that slot. Same idiom as `ParseBodyService::call` (JOLTR-RS-059) and
        // `CorsService::call` (JOLTR-RS-056).
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);

        // JOLTR-RS-078 early-termination check: if an upstream layer has
        // already finished the request, skip the query parse + extension
        // insert and delegate to inner. Read-only check — this layer never
        // flips the latch itself (decision 5 in the module rustdoc), but it
        // still observes the latch so a finished request avoids the wasted
        // map allocation.
        if let Some(ext) = req.extensions().get::<Arc<RequestExt>>() {
            if ext.is_finished() {
                return Box::pin(async move { inner.call(req).await });
            }
        }

        let params = QueryParams(parse_query(req.uri().query()));
        req.extensions_mut().insert(params);

        Box::pin(async move { inner.call(req).await })
    }
}

/// Split a query string of the form `k=v&k=v` into a key→value map. Mirrors
/// [`endpoint_registry::parse_query`](crate::endpoint_registry) verbatim so
/// the layer's behavior is observably identical to the existing `Request`
/// snapshot path's interpretation of the same URL.
///
/// `None` (no `?` in the URI) returns an empty map. Malformed pairs (any chunk
/// without an `=`) are silently dropped — see module docs decision 4 for the
/// rationale.
///
/// Percent-decoding is NOT performed; the same caveat applies to the existing
/// `endpoint_registry::parse_query`. A future polish item can hoist a shared
/// decoder used by both call sites.
fn parse_query(query: Option<&str>) -> HashMap<String, String> {
    let Some(q) = query else {
        return HashMap::new();
    };
    q.split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// Typed extraction error returned by [`extract`] (JOLTR-RS-064).
///
/// Two variants kept distinct so a future caller can map them differently
/// (e.g., `Missing` → `"required parameter X missing"`,
/// `Invalid` → `"invalid value Y for X"`). Collapsing them into a single
/// variant would force every consumer to re-derive which case fired from the
/// message text. Both round-trip to a `400 Bad Request` via
/// [`bad_request_for_query_error`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryExtractError {
    /// The requested key was not present in the parsed query map.
    Missing {
        /// The key the caller asked for.
        key: String,
    },
    /// The key was present but its value failed `T::from_str` (or the
    /// equivalent typed parser used by a sibling extractor).
    Invalid {
        /// The key whose value failed to parse.
        key: String,
        /// The raw string value that was rejected.
        value: String,
        /// Human-readable detail from the underlying parser
        /// (e.g., `ParseIntError::Display`).
        message: String,
    },
    /// One element of a comma-separated list value (e.g., `?ids=1,abc,3`)
    /// failed to parse (JOLTR-RS-066). Distinct from [`Invalid`](Self::Invalid)
    /// so the failing element's zero-based position is preserved verbatim
    /// rather than being smuggled through the message text — the
    /// [`AutoMiddleware`](crate::AutoMiddleware) codegen and any future
    /// custom-renderer can branch on the variant directly.
    InvalidElement {
        /// The key whose value contained the failing element.
        key: String,
        /// Zero-based index of the element that failed.
        index: usize,
        /// The raw element string that was rejected.
        value: String,
        /// Human-readable detail from the underlying parser.
        message: String,
    },
}

impl fmt::Display for QueryExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { key } => {
                write!(f, "Missing query parameter '{key}'")
            }
            Self::Invalid {
                key,
                value,
                message,
            } => {
                write!(f, "Invalid query parameter '{key}'='{value}': {message}",)
            }
            Self::InvalidElement {
                key,
                index,
                value,
                message,
            } => {
                write!(
                    f,
                    "Invalid query parameter '{key}' element [{index}]='{value}': {message}",
                )
            }
        }
    }
}

impl std::error::Error for QueryExtractError {}

/// Look up `key` in the parsed query map and parse the associated value as
/// `T` via [`FromStr`] (JOLTR-RS-064).
///
/// Works for any `T: FromStr` whose error implements [`Display`](fmt::Display)
/// — covers the integer types the PRD calls out (`i32`, `i64`, `u32`,
/// `u64`, `usize`) and the float types (`f32`, `f64`) without per-type
/// boilerplate. The error variants in [`QueryExtractError`] distinguish a
/// missing key from a present-but-unparseable value so the caller can decide
/// how to surface each (the typical path is
/// [`bad_request_for_query_error`] for both).
///
/// The function is field-level (not another `tower::Layer`): the
/// AutoMiddleware codegen will call it from inside the per-field extraction
/// block emitted for a `query: T` field, and the typed-error result lets the
/// codegen route both shapes through the same 400-response path. A future
/// PRD that needs custom typed parsers for surfaces that aren't `FromStr`
/// (booleans with `"1"/"0"` aliases — JOLTR-RS-065 — or enum aliasing via
/// `TryFrom<&str>`) will add sibling helpers rather than parameterizing this
/// one.
pub fn extract<T>(params: &QueryParams, key: &str) -> Result<T, QueryExtractError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    let value = params.get(key).ok_or_else(|| QueryExtractError::Missing {
        key: key.to_string(),
    })?;
    value
        .parse::<T>()
        .map_err(|err| QueryExtractError::Invalid {
            key: key.to_string(),
            value: value.clone(),
            message: err.to_string(),
        })
}

/// Look up `key` in the parsed query map and parse the associated value as a
/// [`bool`] (JOLTR-RS-065).
///
/// Accepts (case-insensitively) `"true"` / `"1"` as `true`, and `"false"` /
/// `"0"` as `false`. Any other value yields [`QueryExtractError::Invalid`]
/// with a message naming the four accepted forms so the 400 body is
/// self-explanatory.
///
/// Sibling of [`extract`] rather than a use of it because [`bool`]'s
/// [`FromStr`] only accepts `"true"`/`"false"` — the `"1"`/`"0"` aliases
/// the PRD calls out for 065 would be rejected by the generic helper.
pub fn extract_bool(params: &QueryParams, key: &str) -> Result<bool, QueryExtractError> {
    let value = params.get(key).ok_or_else(|| QueryExtractError::Missing {
        key: key.to_string(),
    })?;
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(QueryExtractError::Invalid {
            key: key.to_string(),
            value: value.clone(),
            message: "expected one of true|false|1|0 (case-insensitive)".to_string(),
        }),
    }
}

/// Look up `key` in the parsed query map and return the raw string value
/// (JOLTR-RS-065).
///
/// Pass-through that always succeeds when the key is present (string
/// "parsing" is the identity). Distinct from `extract::<String>` because
/// [`String`]'s [`FromStr`] is `Infallible` — the generic helper would force
/// callers through a `Result<String, Infallible>` chain whose error variant
/// can never fire. The dedicated helper documents the intent at the call
/// site and skips the unused error path.
pub fn extract_string(params: &QueryParams, key: &str) -> Result<String, QueryExtractError> {
    params
        .get(key)
        .cloned()
        .ok_or_else(|| QueryExtractError::Missing {
            key: key.to_string(),
        })
}

/// Look up `key` in the parsed query map and parse the associated value via
/// [`TryFrom<&str>`] (JOLTR-RS-065).
///
/// Sibling of [`extract`] for types that implement [`TryFrom<&str>`] instead
/// of [`FromStr`]. The natural target is user-defined enums (often via
/// `#[derive(strum::EnumString)]` or a hand-rolled `TryFrom<&str>` impl).
/// The HRTB on the bound lets the caller pass any `T` whose `TryFrom<&str>`
/// works for arbitrary input lifetimes, which is the shape strum and most
/// hand-rolled enum impls already have.
pub fn extract_enum<T>(params: &QueryParams, key: &str) -> Result<T, QueryExtractError>
where
    T: for<'a> TryFrom<&'a str>,
    for<'a> <T as TryFrom<&'a str>>::Error: fmt::Display,
{
    let value = params.get(key).ok_or_else(|| QueryExtractError::Missing {
        key: key.to_string(),
    })?;
    T::try_from(value.as_str()).map_err(|err| QueryExtractError::Invalid {
        key: key.to_string(),
        value: value.clone(),
        message: err.to_string(),
    })
}

/// Look up `key` in the parsed query map, split the associated value on
/// commas, and parse each element as `T` via [`FromStr`] (JOLTR-RS-066).
///
/// `?ids=1,2,3` with `T = i32` returns `Ok(vec![1, 2, 3])`. A single value
/// (`?ids=42`) yields a one-element vec; a present-but-empty value (`?ids=`)
/// yields `Err(InvalidElement { index: 0, value: "", … })` because
/// `"".parse::<i32>()` rejects. Callers that want "absent → empty vec"
/// semantics should match on [`Missing`](QueryExtractError::Missing)
/// themselves rather than swallowing it here — the absent-vs-present split
/// is the same load-bearing distinction the rest of the typed-extractor
/// surface preserves.
///
/// Element-level failures surface as
/// [`QueryExtractError::InvalidElement`] (NOT
/// [`Invalid`](QueryExtractError::Invalid)) so the failing position is
/// available to the eventual 400 renderer without parsing it back out of
/// the message text. The element's raw string is also captured in the
/// variant's `value` field — same shape as `Invalid`'s `value` but scoped
/// to the rejected element rather than the whole comma-separated payload.
pub fn extract_vec<T>(params: &QueryParams, key: &str) -> Result<Vec<T>, QueryExtractError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    let value = params.get(key).ok_or_else(|| QueryExtractError::Missing {
        key: key.to_string(),
    })?;
    value
        .split(',')
        .enumerate()
        .map(|(index, element)| {
            element
                .parse::<T>()
                .map_err(|err| QueryExtractError::InvalidElement {
                    key: key.to_string(),
                    index,
                    value: element.to_string(),
                    message: err.to_string(),
                })
        })
        .collect()
}

/// Deserialize the parsed query map into a user-provided typed shape.
///
/// This is the full-struct companion to the field-level extractors above. The
/// map comes from the same foundational query parser, then gets re-encoded with
/// `serde_urlencoded` so serde can apply the user's `Deserialize` impl.
pub fn deserialize_query<T>(params: &HashMap<String, String>) -> Result<T, QueryExtractError>
where
    T: DeserializeOwned,
{
    let encoded =
        serde_urlencoded::to_string(params).map_err(|err| QueryExtractError::Invalid {
            key: "<query>".to_string(),
            value: String::new(),
            message: err.to_string(),
        })?;

    serde_urlencoded::from_str(&encoded).map_err(|err| QueryExtractError::Invalid {
        key: "<query>".to_string(),
        value: encoded,
        message: err.to_string(),
    })
}

/// Build the `400 Bad Request` response surfaced when a typed query
/// extractor (JOLTR-RS-064+) rejects a value or finds a missing key. Mirrors
/// [`parse_body::bad_request_for_parse_error`](crate::parse_body) in shape:
/// `text/plain` body, `Display` impl of the error as the payload.
///
/// The AutoMiddleware codegen path is the expected caller — it will invoke
/// [`extract`] (or a sibling typed parser) on each `query: T` field and
/// short-circuit with this response on `Err`.
pub fn bad_request_for_query_error(err: &QueryExtractError) -> Response {
    let body = err.to_string();
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(body))
        .expect("400 response builder always succeeds with static headers + owned body")
}
