//! JWT validation `tower::Layer` (JOLT-RS-072..074).
//!
//! [`AuthJwtLayer`] (JOLT-RS-072) consumes the
//! [`BearerToken`](crate::auth_bearer::BearerToken) stashed by
//! [`AuthBearerLayer`](crate::AuthBearerLayer) (JOLT-RS-071), calls
//! [`jolt_utils::jwt::decode`] with the configured secret + algorithm, and
//! either stashes the parsed [`JwtClaims`] into request extensions for a
//! downstream handler to consume, or short-circuits with a
//! `401 Unauthorized` carrying a
//! `WWW-Authenticate: Bearer error="invalid_token"` challenge and a
//! `text/plain` body describing the rejection reason.
//!
//! On rejection, the layer also flips
//! [`RequestExt::mark_finished`](crate::RequestExt::mark_finished) so any
//! composed observers see the same finished-flag contract Router relies on for
//! its own short-circuit path. The 401 is returned directly from
//! [`AuthJwtService::call`] (NOT stashed via
//! [`RequestExt::set_response`](crate::RequestExt::set_response)) — same
//! pattern as [`AuthBearerService`](crate::AuthBearerService)'s 401 path.
//!
//! Architectural decisions pinned here for JOLT-RS-073..074 to build on:
//!
//! 1. **Two-layer split (`AuthBearerLayer` + `AuthJwtLayer`), NOT a single
//!    auth layer with optional JWT config.** [`AuthBearerLayer`] stays
//!    stateless and can be wired alone (e.g. for a passthrough proxy that
//!    doesn't decode tokens itself). [`AuthJwtLayer`] carries the
//!    [`JwtConfig`] and follows the format check in the stack. A caller wires
//!    [`AuthBearerLayer`] THEN [`AuthJwtLayer`] on a tower stack — order
//!    matters: the format check stashes the
//!    [`BearerToken`](crate::auth_bearer::BearerToken), the JWT check consumes
//!    it. This decision was pinned by the JOLT-RS-071 progress note's "(b)
//!    sibling layer" recommendation.
//!
//! 2. **`AuthJwtLayer` requires `AuthBearerLayer` upstream; missing
//!    `BearerToken` produces a 401.** If no
//!    [`BearerToken`](crate::auth_bearer::BearerToken) is in request
//!    extensions when [`AuthJwtService::call`] runs, the layer treats this as
//!    a missing-token rejection and surfaces a 401 with body `"Missing bearer
//!    token"`. Two cases reach this branch: (a) the operator forgot to wire
//!    [`AuthBearerLayer`] ahead of [`AuthJwtLayer`] (operator error, but the
//!    401 is ALSO the right caller-visible answer because no auth occurred);
//!    (b) a deployment that wires [`AuthJwtLayer`] standalone — the 401 is
//!    still right because no token was extracted. The dedicated
//!    [`JwtRejectReason::MissingBearerToken`] variant gives operators a
//!    distinct body so the misconfiguration is debuggable from the response.
//!
//! 3. **Parsed [`JwtClaims`] land in extensions on success.** Downstream
//!    handlers reach them via `req.extensions().get::<JwtClaims>()`. Matches
//!    the [`BearerToken`](crate::auth_bearer::BearerToken) /
//!    [`QueryParams`](crate::QueryParams) /
//!    [`ParseBodyLayer`](crate::ParseBodyLayer)-parsed-T convention. Stashing
//!    in 072 (rather than 073) avoids a wasted re-decode: doing the decode
//!    and throwing away the claims would force JOLT-RS-073 to either re-call
//!    [`jolt_utils::jwt::decode`] or expose half-validated state. The
//!    JOLT-RS-073 slice closes the contract by pinning the extension-key
//!    behavior with a dedicated test.
//!
//! 4. **`Arc<JwtConfig>` is shared across cloned services.** The layer
//!    carries `Arc<JwtConfig>` so cloning the layer (which tower requires for
//!    the standard `ServiceBuilder` path) is cheap and doesn't duplicate the
//!    secret bytes per request. [`JwtConfig`] itself is `Clone` (a `Vec<u8>`
//!    plus `Algorithm` pair), so an `Arc` wrap isn't strictly required for
//!    correctness; it keeps clone cost O(1) regardless of secret size.
//!
//! 5. **`WWW-Authenticate: Bearer error="invalid_token"` follows RFC 6750 §3.**
//!    The bare `Bearer` challenge from [`AuthBearerLayer`] indicates "no
//!    credentials provided"; the `error="invalid_token"` parameter
//!    discriminates "credentials provided but rejected". The 401 body still
//!    carries the per-reason `text/plain` message so callers can debug
//!    without parsing the `WWW-Authenticate` parameters.
//!
//! 6. **Preserve-or-inject `Arc<RequestExt>` mirrors
//!    [`AuthBearerService`](crate::AuthBearerService) and
//!    [`ParseBodyService`](crate::ParseBodyService).** Reuse an upstream
//!    `Arc<RequestExt>` so the flipped `finished` latch on rejection is
//!    observable to whoever holds the same Arc; inject a fresh one when no
//!    upstream layer set one so the contract holds in standalone tests too.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::header::{CONTENT_TYPE, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use tower::{Layer, Service};

// `JwtClaims` is referenced in rustdoc links throughout this module but isn't
// named in the code — it's only the type-inference target of `jwt::decode`'s
// `Result<JwtClaims, _>` return. Bringing it into scope keeps the intra-doc
// links resolvable without a fully-qualified path on each reference.
#[allow(unused_imports)]
use jolt_utils::jwt::JwtClaims;
use jolt_utils::jwt::{self, JwtConfig, JwtDecodeError};

use crate::auth_bearer::BearerToken;
use crate::request_ext::RequestExt;

/// `tower::Layer` that validates the JWT carried by the
/// [`BearerToken`](crate::auth_bearer::BearerToken) extension and stashes the
/// parsed [`JwtClaims`] into request extensions (JOLT-RS-072). See module docs
/// for the architectural contract (two-layer split, missing-token-as-401,
/// claims-stash, `Arc<JwtConfig>` sharing, RFC 6750 challenge parameter).
///
/// Carries an `Arc<JwtConfig>` so cloning the layer (required by tower's
/// `ServiceBuilder` path) is cheap regardless of secret size.
#[derive(Clone, Debug)]
pub struct AuthJwtLayer {
    config: Arc<JwtConfig>,
}

impl AuthJwtLayer {
    /// Construct a JWT-auth layer with the given config. The config is wrapped
    /// in an `Arc` so cloned layers share the same secret bytes; callers that
    /// already hold an `Arc<JwtConfig>` can use [`Self::from_arc`] to avoid a
    /// re-wrap.
    pub fn new(config: JwtConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    /// Construct a JWT-auth layer from an already-wrapped `Arc<JwtConfig>`.
    /// Useful when the caller has a single config shared across multiple
    /// routes or layers and wants to avoid a fresh `Arc` allocation per
    /// layer.
    pub fn from_arc(config: Arc<JwtConfig>) -> Self {
        Self { config }
    }
}

impl<S> Layer<S> for AuthJwtLayer {
    type Service = AuthJwtService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthJwtService {
            inner,
            config: Arc::clone(&self.config),
        }
    }
}

/// Inner-service wrapper produced by [`AuthJwtLayer::layer`]. Reads the
/// [`BearerToken`](crate::auth_bearer::BearerToken) from request extensions,
/// validates the JWT via [`jolt_utils::jwt::decode`], and either stashes a
/// parsed [`JwtClaims`] into extensions before delegating to the inner service
/// or short-circuits with a 401 (with [`RequestExt::mark_finished`] flipped)
/// on decode failure. See [`AuthJwtLayer`] for the architectural contract.
#[derive(Clone, Debug)]
pub struct AuthJwtService<S> {
    inner: S,
    config: Arc<JwtConfig>,
}

impl<S> Service<AxumRequest> for AuthJwtService<S>
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
        // `AuthBearerService::call` (JOLT-RS-071), `ParseBodyService::call`
        // (JOLT-RS-059), and `CorsService::call` (JOLT-RS-056).
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);
        let config = Arc::clone(&self.config);

        // Mirror AuthBearerService / ParseBodyService / CorsService's
        // preserve-or-inject contract for `Arc<RequestExt>`: reuse an
        // upstream-supplied ext so a flipped `finished` latch is observable
        // to whoever holds the same Arc; inject a fresh one when no upstream
        // layer has so the rejection-branch mark_finished call is always
        // sound.
        let request_ext: Arc<RequestExt> = match req.extensions().get::<Arc<RequestExt>>() {
            Some(existing) => Arc::clone(existing),
            None => {
                let fresh = Arc::new(RequestExt::new());
                req.extensions_mut().insert(Arc::clone(&fresh));
                fresh
            }
        };

        // The BearerToken is stashed by AuthBearerLayer's format check. If
        // it's absent, EITHER the operator forgot to wire AuthBearerLayer
        // upstream OR this layer is wired standalone — both cases produce a
        // 401 with the dedicated MissingBearerToken body (see module docs
        // decision 2).
        let token = match req.extensions().get::<BearerToken>() {
            Some(t) => t.as_str().to_owned(),
            None => {
                request_ext.mark_finished();
                let response = unauthorized_response(JwtRejectReason::MissingBearerToken);
                return Box::pin(async move { Ok(response) });
            }
        };

        match jwt::decode(&token, &config) {
            Ok(claims) => {
                req.extensions_mut().insert(claims);
                Box::pin(async move { inner.call(req).await })
            }
            Err(err) => {
                request_ext.mark_finished();
                let response = unauthorized_response(JwtRejectReason::DecodeFailed(err));
                Box::pin(async move { Ok(response) })
            }
        }
    }
}

/// Reason a request's JWT was rejected by the layer. Held as a dedicated enum
/// so each rejection path produces a distinct 401 body (the caller can
/// disambiguate "bearer token absent" from "token expired" from "signature
/// invalid" without parsing free-form prose). Module-private since the
/// rejection mechanism is an implementation detail; the contractual surface
/// is the 401 body strings, which the tests pin verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
enum JwtRejectReason {
    MissingBearerToken,
    DecodeFailed(JwtDecodeError),
}

impl JwtRejectReason {
    /// `text/plain` body emitted in the 401 response for each rejection
    /// variant. Owned `String` (rather than `&'static str`) because the
    /// `Other` variant of [`JwtDecodeError`] carries a runtime error message
    /// that the body interpolates.
    fn body(&self) -> String {
        match self {
            Self::MissingBearerToken => "Missing bearer token".to_owned(),
            Self::DecodeFailed(err) => match err {
                JwtDecodeError::Expired => "Token has expired".to_owned(),
                JwtDecodeError::InvalidSignature => "Invalid token signature".to_owned(),
                JwtDecodeError::InvalidAlgorithm => "Invalid token algorithm".to_owned(),
                JwtDecodeError::Malformed => "Malformed token".to_owned(),
                JwtDecodeError::Other(detail) => format!("Token rejected: {detail}"),
            },
        }
    }
}

/// Build the `401 Unauthorized` response for a rejected JWT. Carries
/// `WWW-Authenticate: Bearer error="invalid_token"` (per RFC 6750 §3 — the
/// `error` parameter discriminates "credentials provided but rejected" from
/// AuthBearerLayer's bare `Bearer` challenge for "no credentials provided")
/// plus a `text/plain` body whose contents come from
/// [`JwtRejectReason::body`].
fn unauthorized_response(reason: JwtRejectReason) -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            WWW_AUTHENTICATE,
            HeaderValue::from_static(r#"Bearer error="invalid_token""#),
        )
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(reason.body()))
        .expect("401 response builder always succeeds with static headers + owned body")
}
