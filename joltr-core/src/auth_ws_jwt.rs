//! WebSocket JWT validation `tower::Layer` (JOLTR-RS-076..077).
//!
//! [`AuthWsJwtLayer`] (JOLTR-RS-076) composes
//! [`extract_jwt_token`](crate::auth_websocket::extract_jwt_token) (JOLTR-RS-075)
//! with [`joltr_utils::jwt::decode`] (JOLTR-RS-072). It runs as a pre-upgrade
//! gate on a WebSocket-bearing route: on a valid `Sec-WebSocket-Protocol:
//! joltr-jwt, <token>` header carrying a token that decodes against the
//! configured [`JwtConfig`], the layer stashes both a [`WsJwtToken`] AND a
//! [`JwtClaims`] into request extensions and delegates to the inner service
//! (which performs the actual WebSocket upgrade in JOLTR-RS-077). On any
//! rejection — extraction or decode — the layer short-circuits with a `401
//! Unauthorized` carrying a `text/plain` body describing the reason and the
//! upgrade is NOT performed.
//!
//! On rejection the layer also flips
//! [`RequestExt::mark_finished`](crate::RequestExt::mark_finished) so any
//! composed observers see the same finished-flag contract Router relies on for
//! its own short-circuit path. The 401 is returned directly from
//! [`AuthWsJwtService::call`] (NOT stashed via
//! [`RequestExt::set_response`](crate::RequestExt::set_response)) — same
//! pattern as [`AuthJwtService`](crate::AuthJwtService) and
//! [`AuthBearerService`](crate::AuthBearerService).
//!
//! Architectural decisions pinned here for JOLTR-RS-077 to build on:
//!
//! 1. **Single-layer scope, NOT a two-layer split.** Unlike the HTTP path
//!    where [`AuthBearerLayer`](crate::AuthBearerLayer) (071) and
//!    [`AuthJwtLayer`](crate::AuthJwtLayer) (072) split format check and
//!    decode into separate layers, the WS path's
//!    [`extract_jwt_token`](crate::auth_websocket::extract_jwt_token) is a
//!    pure helper (JOLTR-RS-075) and 076 composes it with the decode in a
//!    single layer. The HTTP split exists because the HTTP `Authorization`
//!    header's `Bearer` shape is reusable across many auth schemes (JWT,
//!    OAuth2 reference token, etc.); the WS `Sec-WebSocket-Protocol:
//!    joltr-jwt, <token>` shape is JWT-specific by construction, so there is
//!    no caller that wants format-check-only without decode.
//!
//! 2. **NO `WWW-Authenticate` challenge header on the 401.** RFC 6750 §3
//!    `WWW-Authenticate: Bearer` is a contract for the HTTP `Authorization`
//!    header re-prompting flow. WebSocket clients (browsers, native clients)
//!    do not honor this challenge during a failed handshake — the upgrade
//!    simply fails — so emitting one would be misleading. The 401 carries a
//!    `text/plain` body and `Content-Type: text/plain; charset=utf-8` only.
//!
//! 3. **No selected subprotocol echoed back on the 401.** When a WebSocket
//!    upgrade succeeds, the server echoes back the selected subprotocol via a
//!    `Sec-WebSocket-Protocol` response header. On the 401 path no upgrade
//!    occurs, so no subprotocol is selected and the response carries no
//!    `Sec-WebSocket-Protocol` header. (077's success path is responsible
//!    for emitting `Sec-WebSocket-Protocol: joltr-jwt` on the upgrade
//!    response so the client's offered subprotocol list is honored.)
//!
//! 4. **Both [`WsJwtToken`] and [`JwtClaims`] are stashed on success.**
//!    The token handle gives 077's WS handler a way to log/echo the
//!    authenticated subject without re-formatting the claims; the
//!    [`JwtClaims`] gives the handler typed access to the parsed payload
//!    (sub, exp, iat, extra). Both share the [`Arc<RequestExt>`] scope of
//!    the request so 077's downstream observers see them via the standard
//!    `req.extensions().get::<T>()` lookup.
//!
//! 5. **`Arc<JwtConfig>` is shared across cloned services.** Mirrors
//!    [`AuthJwtLayer`](crate::AuthJwtLayer)'s decision 4: cloning the layer
//!    (which tower's `ServiceBuilder` requires) is cheap regardless of secret
//!    size. The same [`JwtConfig`] instance can be reused by both the HTTP
//!    [`AuthJwtLayer`] and the WS [`AuthWsJwtLayer`] via [`Self::from_arc`]
//!    so a single deployment-wide secret only allocates once.
//!
//! 6. **Preserve-or-inject `Arc<RequestExt>` mirrors
//!    [`AuthJwtService`](crate::AuthJwtService).** Reuse an upstream-supplied
//!    ext so the flipped `finished` latch on rejection is observable to
//!    whoever holds the same Arc; inject a fresh one when no upstream layer
//!    has so the contract holds in standalone tests too.
//!
//! 7. **The per-reason 401 body strings are part of the contract.** The
//!    extraction-failure bodies come verbatim from
//!    [`WsTokenRejectReason::message`] (075); the decode-failure bodies
//!    mirror [`AuthJwtLayer`](crate::AuthJwtLayer)'s
//!    `JwtRejectReason::body` map ("Token has expired", "Invalid token
//!    signature", "Invalid token algorithm", "Malformed token", "Token
//!    rejected: <detail>"). 077's WS handler can rely on the body shape if
//!    it wants to surface the rejection reason in a higher-level error
//!    channel.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::header::{CONTENT_TYPE, SEC_WEBSOCKET_PROTOCOL};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use tower::{Layer, Service};

// `JwtClaims` is referenced in rustdoc links throughout this module and is
// also the type-inference target of `jwt::decode`'s `Result<JwtClaims, _>`
// return on the success branch (where it's inserted into request extensions).
// Importing the name here keeps the intra-doc links resolvable.
#[allow(unused_imports)]
use joltr_utils::jwt::JwtClaims;
use joltr_utils::jwt::{self, JwtConfig, JwtDecodeError};

use crate::auth_websocket::{extract_jwt_token, WsJwtToken, WsTokenRejectReason};
use crate::request_ext::RequestExt;

/// `tower::Layer` that validates the JWT carried by a WebSocket upgrade's
/// `Sec-WebSocket-Protocol: joltr-jwt, <token>` header BEFORE the upgrade is
/// allowed (JOLTR-RS-076). On success, stashes both a [`WsJwtToken`] and a
/// [`JwtClaims`] into request extensions and delegates to the inner service
/// (which performs the actual upgrade). On any rejection — extraction or
/// decode — short-circuits with a `401 Unauthorized` and the upgrade is NOT
/// performed. See module docs for the architectural contract.
///
/// Carries an `Arc<JwtConfig>` so cloning the layer (required by tower's
/// `ServiceBuilder` path) is cheap regardless of secret size.
#[derive(Clone, Debug)]
pub struct AuthWsJwtLayer {
    config: Arc<JwtConfig>,
}

impl AuthWsJwtLayer {
    /// Construct a WS-JWT-auth layer with the given config. The config is
    /// wrapped in an `Arc` so cloned layers share the same secret bytes;
    /// callers that already hold an `Arc<JwtConfig>` can use
    /// [`Self::from_arc`] to avoid a re-wrap (or to share the same `Arc`
    /// with [`AuthJwtLayer`](crate::AuthJwtLayer)).
    pub fn new(config: JwtConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    /// Construct a WS-JWT-auth layer from an already-wrapped `Arc<JwtConfig>`.
    /// Useful when the caller already holds a single config shared across the
    /// HTTP [`AuthJwtLayer`](crate::AuthJwtLayer) and this WS layer.
    pub fn from_arc(config: Arc<JwtConfig>) -> Self {
        Self { config }
    }
}

impl<S> Layer<S> for AuthWsJwtLayer {
    type Service = AuthWsJwtService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthWsJwtService {
            inner,
            config: Arc::clone(&self.config),
        }
    }
}

/// Inner-service wrapper produced by [`AuthWsJwtLayer::layer`]. Reads the
/// `Sec-WebSocket-Protocol` header, runs the 075 extractor and the 072
/// decoder, and either stashes [`WsJwtToken`] + [`JwtClaims`] into extensions
/// before delegating, or short-circuits with a 401 (with
/// [`RequestExt::mark_finished`] flipped) on any rejection. See
/// [`AuthWsJwtLayer`] for the architectural contract.
#[derive(Clone, Debug)]
pub struct AuthWsJwtService<S> {
    inner: S,
    config: Arc<JwtConfig>,
}

impl<S> Service<AxumRequest> for AuthWsJwtService<S>
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
        // `AuthJwtService::call` (JOLTR-RS-072).
        let cloned = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, cloned);
        let config = Arc::clone(&self.config);

        // JOLTR-RS-078 early-termination check: if an upstream layer has
        // already finished the request, skip the WS JWT precheck and delegate
        // to inner so the already-determined response propagates. Read-only
        // check — preserve-or-inject still runs on the active branch for the
        // rejection-side mark_finished.
        if let Some(ext) = req.extensions().get::<Arc<RequestExt>>() {
            if ext.is_finished() {
                return Box::pin(async move { inner.call(req).await });
            }
        }

        // Mirror AuthJwtService's preserve-or-inject contract so a flipped
        // `finished` latch on rejection is observable to whoever holds the
        // same Arc (and the rejection-branch mark_finished call is always
        // sound when the layer is wired standalone).
        let request_ext: Arc<RequestExt> = match req.extensions().get::<Arc<RequestExt>>() {
            Some(existing) => Arc::clone(existing),
            None => {
                let fresh = Arc::new(RequestExt::new());
                req.extensions_mut().insert(Arc::clone(&fresh));
                fresh
            }
        };

        let token = match extract_jwt_token(req.headers().get(SEC_WEBSOCKET_PROTOCOL)) {
            Ok(t) => t,
            Err(reason) => {
                request_ext.mark_finished();
                let response = unauthorized_response(WsAuthRejectReason::Extract(reason));
                return Box::pin(async move { Ok(response) });
            }
        };

        match jwt::decode(&token, &config) {
            Ok(claims) => {
                req.extensions_mut().insert(WsJwtToken(token));
                req.extensions_mut().insert(claims);
                Box::pin(async move { inner.call(req).await })
            }
            Err(err) => {
                request_ext.mark_finished();
                let response = unauthorized_response(WsAuthRejectReason::Decode(err));
                Box::pin(async move { Ok(response) })
            }
        }
    }
}

/// Reason a WebSocket upgrade's auth precheck was rejected by the layer. Held
/// as a dedicated enum so each rejection path produces a distinct 401 body
/// (the caller can disambiguate "missing subprotocol header" from "token
/// expired" without parsing free-form prose). Module-private since the
/// rejection mechanism is an implementation detail; the contractual surface
/// is the 401 body strings, which the tests pin verbatim.
#[derive(Debug)]
enum WsAuthRejectReason {
    Extract(WsTokenRejectReason),
    Decode(JwtDecodeError),
}

impl WsAuthRejectReason {
    /// `text/plain` body emitted in the 401 response. Owned `String` because
    /// the [`JwtDecodeError::Other`] variant carries a runtime detail string
    /// that the body interpolates; the [`WsTokenRejectReason`] arm uses the
    /// static `&'static str` returned by
    /// [`WsTokenRejectReason::message`](crate::auth_websocket::WsTokenRejectReason::message)
    /// which we own-convert here so both arms share the same return type.
    fn body(&self) -> String {
        match self {
            Self::Extract(reason) => reason.message().to_owned(),
            Self::Decode(err) => match err {
                JwtDecodeError::Expired => "Token has expired".to_owned(),
                JwtDecodeError::InvalidSignature => "Invalid token signature".to_owned(),
                JwtDecodeError::InvalidAlgorithm => "Invalid token algorithm".to_owned(),
                JwtDecodeError::Malformed => "Malformed token".to_owned(),
                JwtDecodeError::Other(detail) => format!("Token rejected: {detail}"),
            },
        }
    }
}

/// Build the `401 Unauthorized` response for a rejected WebSocket upgrade.
/// Carries a `text/plain` body whose contents come from
/// [`WsAuthRejectReason::body`]; no `WWW-Authenticate` challenge header (see
/// module docs decision 2) and no `Sec-WebSocket-Protocol` echo (decision 3).
fn unauthorized_response(reason: WsAuthRejectReason) -> Response {
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header(
            CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        )
        .body(Body::from(reason.body()))
        .expect("401 response builder always succeeds with static headers + owned body")
}
