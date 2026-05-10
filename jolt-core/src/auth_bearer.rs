//! Bearer-token authentication `tower::Layer` (JOLT-RS-071..074).
//!
//! [`AuthBearerLayer`] (JOLT-RS-071) extracts the `Authorization` request
//! header, validates it carries the canonical `Bearer <token>` shape, and
//! stashes the extracted [`BearerToken`] into request extensions so a
//! downstream consumer (the upcoming JOLT-RS-072 JWT validator) can pull it
//! back out.
//!
//! On a missing or malformed header, the layer short-circuits with a
//! `401 Unauthorized` carrying a `WWW-Authenticate: Bearer` challenge plus a
//! `text/plain` body describing the rejection. The layer also flips
//! [`RequestExt::mark_finished`](crate::RequestExt::mark_finished) so any
//! composed observers see the same finished-flag contract Router relies on
//! for its own short-circuit path. The 401 is returned directly from
//! [`AuthBearerService::call`] (NOT stashed via
//! [`RequestExt::set_response`](crate::RequestExt::set_response)) — same shape
//! as [`ParseBodyService`](crate::ParseBodyService)'s 400 path, since this
//! layer typically sits OUTSIDE Router and an inner service is never invoked
//! on the failure branch.
//!
//! Architectural decisions pinned here for JOLT-RS-072..074 to build on:
//!
//! 1. **Layer scope is ONLY format validation.** "Authorization header
//!    present AND prefixed with `Bearer `" is the entire 071 contract. JWT
//!    decoding/expiry/signature checks land in JOLT-RS-072; claims extraction
//!    into request extensions lands in JOLT-RS-073. Splitting the slices
//!    keeps each iteration independently verifiable and lets 072+ depend on
//!    a [`BearerToken`] handle rather than re-extracting the token themselves.
//!
//! 2. **Extracted token lands in request extensions as
//!    [`BearerToken`](crate::auth_bearer::BearerToken), NOT as a bare
//!    [`String`].** Request extensions are keyed by [`std::any::TypeId`], so a
//!    bare `String` would collide with
//!    [`ParseBodyStringService`](crate::ParseBodyStringService)'s raw-text-body
//!    stash. The newtype gives this layer a unique extension key while keeping
//!    consumers one method call away (`token.as_str()`) from the underlying
//!    string.
//!
//! 3. **Scheme name match is case-insensitive; separator is a single ASCII
//!    space.** RFC 7235 §2.1 declares the auth-scheme name case-insensitive,
//!    so a header of `bearer eyJ...` or `BEARER eyJ...` is accepted as
//!    semantically identical to `Bearer eyJ...`. The single-space separator
//!    matches the canonical RFC 6750 example; the layer does not collapse
//!    multiple spaces or accept tabs (an over-permissive split would let
//!    `Bearer\t<token>` through, which most upstream WAFs flag as malformed).
//!
//! 4. **Empty token after the prefix is rejected as 401.** A header of
//!    `Bearer ` (with the trailing space and nothing else) is structurally
//!    parseable but semantically meaningless — there is no token to validate.
//!    A dedicated [`BearerRejectReason::EmptyToken`] variant surfaces a
//!    distinct 401 body so the caller can disambiguate "I forgot the header"
//!    (`MissingHeader`) from "I sent the header but with no token"
//!    (`EmptyToken`).
//!
//! 5. **Non-ASCII header values are rejected as 401, NOT 400.** A header
//!    that fails [`HeaderValue::to_str`] (because it carries non-visible
//!    bytes) is not a valid `Authorization: Bearer <token>` payload from the
//!    layer's perspective — the auth contract failed, so 401 with an
//!    explanatory body is the right shape. 400 would imply the rest of the
//!    request's body is the issue, which is misleading.
//!
//! 6. **Preserve-or-inject `Arc<RequestExt>` mirrors
//!    [`ParseBodyService`](crate::ParseBodyService) and
//!    [`CorsService`](crate::CorsService).** A flipped `finished` latch on
//!    rejection is observable to whoever holds the same Arc, and the contract
//!    holds even when the layer composes outside Router.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use tower::{Layer, Service};

use crate::request_ext::RequestExt;

/// Bearer token extracted from an inbound `Authorization: Bearer <token>`
/// header. Stashed into request extensions by [`AuthBearerService`] on
/// successful format validation; consumed by JOLT-RS-072's JWT validator and
/// JOLT-RS-073's claims extractor.
///
/// Wrapped in a dedicated newtype rather than a bare [`String`] so the
/// extension key is unique — request extensions are keyed by
/// [`std::any::TypeId`], so a bare `String` would collide with
/// [`ParseBodyStringService`](crate::ParseBodyStringService)'s raw-text-body
/// stash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BearerToken(pub String);

impl BearerToken {
    /// Borrow the underlying token string. Provided so callers don't have to
    /// reach for `.0` to get at the bytes.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// `tower::Layer` that validates the `Authorization: Bearer <token>` header
/// shape and stashes the extracted [`BearerToken`] into request extensions
/// (JOLT-RS-071). See module docs for the architectural contract (scope,
/// extension key, case-insensitive scheme match, rejection variants).
///
/// Carries no runtime state; cloning produces a functionally identical layer.
#[derive(Default, Clone, Debug)]
pub struct AuthBearerLayer;

impl AuthBearerLayer {
    /// Construct a bearer-auth layer. The layer carries no runtime state, so a
    /// fresh layer is functionally identical to any other.
    pub fn new() -> Self {
        Self
    }
}

impl<S> Layer<S> for AuthBearerLayer {
    type Service = AuthBearerService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthBearerService { inner }
    }
}

/// Inner-service wrapper produced by [`AuthBearerLayer::layer`]. Validates the
/// `Authorization` header and either stashes a [`BearerToken`] into
/// extensions before delegating, or short-circuits with a 401 (and flips
/// [`RequestExt::mark_finished`]) on a malformed header. See [`AuthBearerLayer`]
/// for the architectural contract.
#[derive(Clone, Debug)]
pub struct AuthBearerService<S> {
    inner: S,
}

impl<S> Service<AxumRequest> for AuthBearerService<S>
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
        // Standard tower delegation: poll_ready was driven on `self.inner`, so
        // `call` must use that same instance. Replace it with a clone we DON'T
        // call; the caller's next poll_ready readies that slot. Same idiom as
        // `ParseBodyService::call` (JOLT-RS-059) and `CorsService::call`
        // (JOLT-RS-056).
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);

        // Mirror ParseBodyService/CorsService's preserve-or-inject contract
        // for `Arc<RequestExt>`: reuse an upstream-supplied ext so a flipped
        // `finished` latch is observable to whoever holds the same Arc; inject
        // a fresh one when no upstream layer has so the rejection-branch
        // mark_finished call is always sound.
        let request_ext: Arc<RequestExt> = match req.extensions().get::<Arc<RequestExt>>() {
            Some(existing) => Arc::clone(existing),
            None => {
                let fresh = Arc::new(RequestExt::new());
                req.extensions_mut().insert(Arc::clone(&fresh));
                fresh
            }
        };

        match extract_bearer_token(req.headers().get(AUTHORIZATION)) {
            Ok(token) => {
                req.extensions_mut().insert(BearerToken(token));
                Box::pin(async move { inner.call(req).await })
            }
            Err(reason) => {
                request_ext.mark_finished();
                let response = unauthorized_response(reason);
                Box::pin(async move { Ok(response) })
            }
        }
    }
}

/// Reason a request's `Authorization` header was rejected by the layer. Held
/// as a dedicated enum so each rejection path produces a distinct 401 body
/// (the caller can disambiguate "header missing" from "header malformed" from
/// "token empty" without parsing free-form prose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BearerRejectReason {
    MissingHeader,
    NotAscii,
    MissingBearerPrefix,
    EmptyToken,
}

impl BearerRejectReason {
    /// Static `text/plain` body emitted in the 401 response for each variant.
    /// Returned by reference so the static-body caller can hand it to
    /// [`Body::from`] without an owned-string allocation.
    fn message(self) -> &'static str {
        match self {
            Self::MissingHeader => "Missing Authorization header",
            Self::NotAscii => "Authorization header is not valid ASCII",
            Self::MissingBearerPrefix => {
                "Invalid Authorization header format: expected 'Bearer <token>'"
            }
            Self::EmptyToken => "Empty bearer token",
        }
    }
}

/// Validate that `header` carries a `Bearer <token>` value and return the
/// extracted token. Scheme name match is case-insensitive (per RFC 7235 §2.1);
/// the separator is a single ASCII space (canonical RFC 6750 form). See
/// module docs decision 3 for the rationale.
fn extract_bearer_token(header: Option<&HeaderValue>) -> Result<String, BearerRejectReason> {
    let value = header.ok_or(BearerRejectReason::MissingHeader)?;
    let text = value.to_str().map_err(|_| BearerRejectReason::NotAscii)?;

    // RFC 7235 declares the scheme name case-insensitive, so accept "Bearer",
    // "bearer", "BEARER", etc. The separator is held strict (single space)
    // because over-permissive splitting (tabs, multiple spaces) lets shapes
    // through that most upstream WAFs already flag as malformed. The length
    // gate is `>` against "Bearer".len() (NOT "Bearer ".len()) because a
    // header of `Bearer ` (prefix + space, no token) MUST reach the
    // empty-token check below — the prefix IS valid, only the token is
    // missing.
    let after_scheme = if text.len() > "Bearer".len()
        && text.as_bytes()[.."Bearer".len()].eq_ignore_ascii_case(b"Bearer")
        && text.as_bytes()["Bearer".len()] == b' '
    {
        &text["Bearer ".len()..]
    } else {
        return Err(BearerRejectReason::MissingBearerPrefix);
    };

    if after_scheme.is_empty() {
        return Err(BearerRejectReason::EmptyToken);
    }
    Ok(after_scheme.to_owned())
}

/// Build the `401 Unauthorized` response for a rejected `Authorization`
/// header. Carries `WWW-Authenticate: Bearer` (per RFC 6750 §3) plus a
/// `text/plain` body whose contents come from
/// [`BearerRejectReason::message`].
fn unauthorized_response(reason: BearerRejectReason) -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"))
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(reason.message()))
        .expect("401 response builder always succeeds with static headers + static body")
}
