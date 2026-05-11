//! WebSocket subprotocol JWT-token extraction (JOLTR-RS-075..077).
//!
//! [`extract_jwt_token`] (JOLTR-RS-075) parses an inbound
//! `Sec-WebSocket-Protocol` header value carrying the canonical
//! `joltr-jwt, <token>` shape and returns the bare JWT string. The validator
//! tower::Layer that consumes this helper before allowing a WebSocket upgrade
//! lands in JOLTR-RS-076; the JwtClaims-into-handler injection lands in
//! JOLTR-RS-077.
//!
//! Architectural decisions pinned here for JOLTR-RS-076..077 to build on:
//!
//! 1. **Layer scope is ONLY format extraction.** "Header is present, parses as
//!    `joltr-jwt, <token>`, and yields a non-empty token" is the entire 075
//!    contract. JWT decoding/expiry/signature checks land in JOLTR-RS-076 (which
//!    will compose this helper with [`joltr_utils::jwt::decode`] inside a
//!    `tower::Layer` that runs BEFORE the WS upgrade); claims injection into
//!    the WS handler lands in JOLTR-RS-077. Splitting the slices keeps each
//!    iteration independently verifiable and lets 076 depend on a pure
//!    extraction helper rather than re-parsing the header itself.
//!
//! 2. **Canonical "joltr-jwt, <token>" shape: exactly two comma-separated
//!    subprotocols.** Browser clients calling
//!    `new WebSocket(url, ["joltr-jwt", "<token>"])` emit
//!    `Sec-WebSocket-Protocol: joltr-jwt, eyJ...` — exactly two values. The
//!    helper requires exactly two parts and rejects anything else with
//!    [`WsTokenRejectReason::MalformedSubprotocols`]. Allowing extra trailing
//!    subprotocols would make the second value ambiguous (token vs. another
//!    offered protocol) since JWTs themselves cannot contain commas (they are
//!    `base64url(.)base64url(.)base64url`, three dot-separated chunks).
//!
//! 3. **`joltr-jwt` literal match is case-sensitive.** Per RFC 6455 §11.5,
//!    WebSocket subprotocol identifiers are matched as opaque tokens against
//!    the IANA registry (or, for vendor protocols like ours, a case-sensitive
//!    string compare). A header of `JOLTR-JWT, eyJ...` is rejected with
//!    [`WsTokenRejectReason::MissingJoltRJwtPrefix`] — distinct from the
//!    case-insensitive match [`AuthBearerLayer`](crate::AuthBearerLayer)
//!    performs on the HTTP `Authorization` scheme name (where RFC 7235 §2.1
//!    explicitly mandates case-insensitivity).
//!
//! 4. **Whitespace around the comma is tolerated.** RFC 7230 §7 (which RFC
//!    6455 inherits for header value parsing) allows optional whitespace
//!    around list element separators. A header of `joltr-jwt,eyJ...` (no
//!    space) and `joltr-jwt , eyJ...` (extra space) both yield the same
//!    extracted token. Trim is `.trim()` (matches RFC 7230's BWS) rather than
//!    a single `.trim_start_matches(' ')` so embedded tab characters are also
//!    handled.
//!
//! 5. **Empty token after the comma is rejected as
//!    [`WsTokenRejectReason::EmptyToken`].** A header of `joltr-jwt,` (or
//!    `joltr-jwt, ` with trailing whitespace) is structurally parseable but
//!    semantically meaningless — there is no token for 076 to validate. The
//!    dedicated variant lets 076 surface a distinct 401 body so the caller
//!    can disambiguate "I forgot the protocol header" (`MissingHeader`) from
//!    "I sent the marker but with no token" (`EmptyToken`).
//!
//! 6. **Non-ASCII header values are rejected as
//!    [`WsTokenRejectReason::NotAscii`].** A `Sec-WebSocket-Protocol` value
//!    that fails [`HeaderValue::to_str`] cannot be parsed as a subprotocol
//!    list (subprotocol identifiers are restricted to RFC 7230 token chars,
//!    a strict ASCII subset). The dedicated variant lets 076's 401 body name
//!    the actual cause rather than rendering a generic "malformed" message.
//!
//! 7. **Extension-key handle for 076 lives in this module.** [`WsJwtToken`]
//!    is a newtype wrapper around the extracted JWT string, returned by 076
//!    after a successful pre-upgrade decode and stashed into request
//!    extensions for 077 to read. Defining the type here (rather than in
//!    076's future module) means the type's existence is settled by 075 and
//!    076 only adds the wiring; mirrors the JOLTR-RS-071 [`BearerToken`] /
//!    JOLTR-RS-072 [`AuthJwtLayer`] split. The newtype gives the layer a
//!    unique extension key (request extensions are keyed by
//!    [`std::any::TypeId`], so a bare `String` would collide with
//!    [`BearerToken`](crate::auth_bearer::BearerToken)).
//!
//! [`AuthBearerLayer`]: crate::AuthBearerLayer

use axum::http::HeaderValue;

/// Subprotocol identifier signaling that the next subprotocol value in the
/// `Sec-WebSocket-Protocol` header is a JWT to validate. Browsers calling
/// `new WebSocket(url, ["joltr-jwt", "<token>"])` produce a header of the
/// canonical form `joltr-jwt, <token>` that this module extracts.
///
/// Held as a `pub const` (rather than inlined as a string literal in
/// [`extract_jwt_token`]) so JOLTR-RS-076's tower::Layer and the test module
/// can refer to the marker by name; a future rename of the marker is
/// localized to this constant.
pub const JOLTR_JWT_SUBPROTOCOL: &str = "joltr-jwt";

/// JWT token extracted from a `Sec-WebSocket-Protocol: joltr-jwt, <token>`
/// header. Wrapped in a dedicated newtype rather than a bare [`String`] so
/// its [`std::any::TypeId`] is unique — request extensions are keyed by
/// `TypeId`, so a bare `String` would collide with
/// [`ParseBodyStringService`](crate::ParseBodyStringService)'s raw-text-body
/// stash and a bare `String` would also be indistinguishable from the HTTP
/// [`BearerToken`](crate::auth_bearer::BearerToken) extension.
///
/// JOLTR-RS-075 only defines the type — the future JOLTR-RS-076 layer is the
/// one that actually inserts a [`WsJwtToken`] into request extensions on a
/// successful pre-upgrade decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsJwtToken(pub String);

impl WsJwtToken {
    /// Borrow the underlying token string. Provided so callers don't have to
    /// reach for `.0` to get at the bytes; mirrors
    /// [`BearerToken::as_str`](crate::auth_bearer::BearerToken::as_str).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Reason a `Sec-WebSocket-Protocol` header was rejected by
/// [`extract_jwt_token`]. Held as a dedicated enum so each rejection path
/// can produce a distinct 401 body once JOLTR-RS-076 wires this helper into
/// a tower::Layer (the caller can disambiguate "header missing" from
/// "header malformed" from "token empty" without parsing free-form prose).
/// Mirrors the [`auth_bearer::BearerRejectReason`] split for the HTTP
/// `Authorization` header.
///
/// [`auth_bearer::BearerRejectReason`]: crate::auth_bearer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsTokenRejectReason {
    /// No `Sec-WebSocket-Protocol` header was present on the inbound request.
    MissingHeader,
    /// The header value carries non-ASCII bytes and cannot be parsed as a
    /// subprotocol list (subprotocol identifiers are RFC 7230 tokens, an
    /// ASCII subset).
    NotAscii,
    /// The header does not split into exactly two comma-separated
    /// subprotocols, OR the first subprotocol is not the case-sensitive
    /// literal [`JOLTR_JWT_SUBPROTOCOL`]. Both shapes are rejected with this
    /// variant since both indicate the canonical `joltr-jwt, <token>` shape
    /// is absent.
    MissingJoltRJwtPrefix,
    /// The header carries exactly two comma-separated subprotocols but the
    /// second value (the token slot) is empty after whitespace trim.
    EmptyToken,
    /// The header carries more than two comma-separated subprotocols. Since
    /// JWTs cannot contain commas (they are dot-separated base64url chunks),
    /// extra commas mean the client is offering additional subprotocols
    /// alongside the auth pair, which makes the token slot ambiguous.
    MalformedSubprotocols,
}

impl WsTokenRejectReason {
    /// Static `text/plain` body emitted in the future JOLTR-RS-076 401
    /// response for each variant. Returned by reference so the static-body
    /// caller can hand it to `Body::from` without an owned-string allocation.
    /// Pinned in 075 (rather than 076) so the rejection-side test bundle
    /// here can assert against the canonical message strings.
    pub fn message(self) -> &'static str {
        match self {
            Self::MissingHeader => "Missing Sec-WebSocket-Protocol header",
            Self::NotAscii => "Sec-WebSocket-Protocol header is not valid ASCII",
            Self::MissingJoltRJwtPrefix => {
                "Invalid Sec-WebSocket-Protocol format: expected 'joltr-jwt, <token>'"
            }
            Self::EmptyToken => "Empty WebSocket JWT token",
            Self::MalformedSubprotocols => {
                "Invalid Sec-WebSocket-Protocol: more than two subprotocols offered"
            }
        }
    }
}

/// Validate that `header` carries a `Sec-WebSocket-Protocol: joltr-jwt, <token>`
/// value and return the extracted token. Whitespace around the comma is
/// tolerated (per RFC 7230 §7 BWS); the `joltr-jwt` literal match is
/// case-sensitive (per RFC 6455 §11.5). See module docs for the full
/// architectural contract.
///
/// JOLTR-RS-075 contract: this helper is the entire surface. JOLTR-RS-076's
/// tower::Layer will compose this with [`joltr_utils::jwt::decode`] before
/// allowing the WS upgrade.
pub fn extract_jwt_token(header: Option<&HeaderValue>) -> Result<String, WsTokenRejectReason> {
    let value = header.ok_or(WsTokenRejectReason::MissingHeader)?;
    let text = value.to_str().map_err(|_| WsTokenRejectReason::NotAscii)?;

    let mut parts = text.split(',');
    let marker = parts
        .next()
        .ok_or(WsTokenRejectReason::MissingJoltRJwtPrefix)?
        .trim();
    if marker != JOLTR_JWT_SUBPROTOCOL {
        return Err(WsTokenRejectReason::MissingJoltRJwtPrefix);
    }

    let token_part = parts
        .next()
        .ok_or(WsTokenRejectReason::MissingJoltRJwtPrefix)?
        .trim();

    // A third (or further) comma-separated value means the client is
    // offering more than the auth pair; reject as ambiguous since the JWT
    // itself cannot contain a comma.
    if parts.next().is_some() {
        return Err(WsTokenRejectReason::MalformedSubprotocols);
    }

    if token_part.is_empty() {
        return Err(WsTokenRejectReason::EmptyToken);
    }
    Ok(token_part.to_owned())
}
