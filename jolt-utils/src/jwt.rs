//! JWT signing/verification helpers (JOLT-RS-072).
//!
//! Two-half surface:
//!
//! * [`JwtConfig`] carries the verification-side parameters (symmetric secret
//!   bytes plus expected algorithm).
//! * [`decode`] consumes a `Bearer`-stripped token string and a [`JwtConfig`]
//!   and produces a typed [`JwtClaims`] on success or a typed
//!   [`JwtDecodeError`] on failure.
//!
//! Architectural decisions pinned here for JOLT-RS-073..074 to build on:
//!
//! 1. **HS-style symmetric algorithms only for the initial slice.** The
//!    [`JwtConfig`] holds the raw secret bytes and builds
//!    [`jsonwebtoken::DecodingKey::from_secret`] on each decode call. RS/ES
//!    algorithms (PEM-keyed) are deferred to a future iteration that adds a
//!    [`jsonwebtoken::DecodingKey`]-carrying variant. JOLT-RS-072's PRD slice
//!    asks for "configured secret and algorithm", which the secret + algorithm
//!    pair satisfies.
//!
//! 2. **Typed error variants on the rejection side, NOT a single
//!    `Other(String)`.** A downstream
//!    [`AuthJwtLayer`](../../../jolt_core/auth_jwt/struct.AuthJwtLayer.html)
//!    needs to disambiguate "token expired" from "signature invalid" so the
//!    401 response body names the actual contract that failed. Dedicated
//!    variants ([`Expired`](JwtDecodeError::Expired),
//!    [`InvalidSignature`](JwtDecodeError::InvalidSignature),
//!    [`InvalidAlgorithm`](JwtDecodeError::InvalidAlgorithm),
//!    [`Malformed`](JwtDecodeError::Malformed)) carry the discriminant; the
//!    catch-all [`Other(String)`](JwtDecodeError::Other) handles jsonwebtoken
//!    error kinds the framework doesn't yet recognize without forcing this
//!    module to mirror every variant in the upstream enum.
//!
//! 3. **Audience validation is disabled.** [`jsonwebtoken::Validation::new`]
//!    defaults `validate_aud` to `true`, which expects an `aud` claim to be
//!    present when an audience expectation is configured. Since [`JwtConfig`]
//!    doesn't yet expose an audience knob, the validation is disabled
//!    explicitly so tokens lacking an `aud` claim aren't rejected for the
//!    wrong reason. A future iteration adds the `audience` field to
//!    [`JwtConfig`] and flips the validation back on conditionally.
//!
//! 4. **Standard claims are all optional.** [`JwtClaims`] models `sub` +
//!    `exp` + `iat` + `nbf` + `iss` + `aud` as `Option<T>` fields so
//!    tokens minted without a given claim deserialize cleanly.
//!    jsonwebtoken's [`Validation::required_spec_claims`] still enforces
//!    presence as configured.
//!
//! 5. **Custom claims land in [`JwtClaims::custom`] via `#[serde(flatten)]`
//!    (JOLT-RS-148).** Any JWT payload key that isn't one of the explicit
//!    fields (`sub`, `exp`, `iat`, `nbf`, `iss`, `aud`) is collected into a
//!    [`HashMap<String, JsonValue>`] keyed by claim name. Callers reach role /
//!    scopes / arbitrary application claims via `claims.custom.get("role")`
//!    without having to redefine the struct. The flattened-map shape was chosen
//!    over a generic `JwtClaims<C: DeserializeOwned>` because it preserves the
//!    struct-literal construction pattern while still surfacing every custom
//!    claim through the typed [`JwtClaims`] extension key.

use jsonwebtoken::{
    decode as jwt_decode, encode as jwks_encode, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Symmetric signing algorithms supported by the framework. Mirrors
/// jsonwebtoken's HMAC-SHA variants so the module doc "HS-only" scope is
/// enforced at the type level. JOLT-RS-149 (encode) and JOLT-RS-150 (decode)
/// surface this as a public parameter on their convenience functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Algorithm {
    HS256,
    HS384,
    HS512,
}

impl From<Algorithm> for jsonwebtoken::Algorithm {
    fn from(a: Algorithm) -> Self {
        match a {
            Algorithm::HS256 => jsonwebtoken::Algorithm::HS256,
            Algorithm::HS384 => jsonwebtoken::Algorithm::HS384,
            Algorithm::HS512 => jsonwebtoken::Algorithm::HS512,
        }
    }
}

/// Verification-side JWT configuration. Carries the symmetric secret bytes and
/// the expected algorithm; the same instance is reused across requests by
/// the [`AuthJwtLayer`](../../../jolt_core/auth_jwt/struct.AuthJwtLayer.html)
/// wrapper (which holds an `Arc<JwtConfig>` so cloning the layer is cheap).
///
/// See module docs decision 1 for the HS-only scope rationale.
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// Symmetric secret bytes for HS-style algorithms. Used to build
    /// [`DecodingKey::from_secret`] on each decode call.
    pub secret: Vec<u8>,
    /// Expected algorithm. A token whose header carries a different `alg`
    /// will be rejected with [`JwtDecodeError::InvalidAlgorithm`].
    pub algorithm: jsonwebtoken::Algorithm,
}

impl JwtConfig {
    /// Construct a config from a secret + algorithm. `secret` accepts any
    /// `Into<Vec<u8>>` (e.g. `&[u8]`, `&str`). `algorithm` accepts our own
    /// [`Algorithm`] (recommended) or a bare
    /// [`jsonwebtoken::Algorithm`] for backward compatibility.
    pub fn new(
        secret: impl Into<Vec<u8>>,
        algorithm: impl Into<jsonwebtoken::Algorithm>,
    ) -> Self {
        Self {
            secret: secret.into(),
            algorithm: algorithm.into(),
        }
    }
}

/// Standard JWT claims surfaced by [`decode`] on success. All standard
/// fields (`sub`, `exp`, `iat`, `nbf`, `iss`, `aud`) are optional; custom
/// claims land in [`custom`](JwtClaims::custom) via `#[serde(flatten)]`.
/// See module docs decision 5 for the custom-claims rationale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Subject (typically a user identifier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// Expiry (UNIX seconds). The validator rejects expired tokens
    /// with [`JwtDecodeError::Expired`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    /// Issued-at (UNIX seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iat: Option<u64>,
    /// Not-before (UNIX seconds). The validator rejects tokens used
    /// before this time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,
    /// Issuer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    /// Audience.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    /// Custom (application-defined) claims captured by `#[serde(flatten)]`.
    /// Any JWT payload key that isn't one of the explicit fields above
    /// (`sub`, `exp`, `iat`, `nbf`, `iss`, `aud`) is collected here. Callers
    /// reach a custom claim via `claims.custom.get("role")`; minting a token
    /// with custom claims is symmetric (populate `custom` before passing the
    /// struct to the encoder). An empty map serializes to no additional JSON
    /// fields.
    #[serde(flatten)]
    pub custom: HashMap<String, JsonValue>,
}

/// Reason a JWT decode call rejected the token. Dedicated variants so a
/// downstream
/// [`AuthJwtLayer`](../../../jolt_core/auth_jwt/struct.AuthJwtLayer.html)
/// produces distinct 401 bodies per failure mode (see module docs decision 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JwtDecodeError {
    /// Token's `exp` claim is in the past.
    Expired,
    /// Token signature did not verify against the configured secret.
    InvalidSignature,
    /// Token's header carries an algorithm other than the configured one
    /// (or jsonwebtoken cannot parse the algorithm name).
    InvalidAlgorithm,
    /// Token does not have the canonical three-segment `header.payload.sig`
    /// shape, or the base64/JSON decode of those segments failed.
    Malformed,
    /// Any other rejection reason from
    /// [`jsonwebtoken::errors::ErrorKind`] not explicitly mapped above. The
    /// string carries the upstream error's `Display` rendering so operators
    /// can pin the actual cause without losing detail.
    Other(String),
}

impl std::fmt::Display for JwtDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expired => f.write_str("token has expired"),
            Self::InvalidSignature => f.write_str("token signature is invalid"),
            Self::InvalidAlgorithm => f.write_str("token algorithm does not match config"),
            Self::Malformed => f.write_str("token is malformed"),
            Self::Other(detail) => write!(f, "token decode failed: {detail}"),
        }
    }
}

impl std::error::Error for JwtDecodeError {}

/// Error returned by [`encode`] when JWT signing fails. Wraps the underlying
/// [`jsonwebtoken::errors::Error`]; callers that need detail can inspect the
/// [`Display`] rendering.
#[derive(Debug, Clone)]
pub struct JwtEncodeError(pub String);

impl std::fmt::Display for JwtEncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JWT encode failed: {}", self.0)
    }
}

impl std::error::Error for JwtEncodeError {}

/// Convenience wrapper for [`decode`] that constructs a [`JwtConfig`] from
/// individual `secret` + `algorithm` parameters and validates the token with
/// the standard jsonwebtoken defaults (`exp`, `nbf`, `iss`, `aud` enabled).
/// Prefer this over the config-based [`decode`] when you don't need
/// fine-grained control over validation flags. JOLT-RS-150.
pub fn decode_simple(
    token: &str,
    secret: &[u8],
    algorithm: Algorithm,
) -> Result<JwtClaims, JwtDecodeError> {
    let key = DecodingKey::from_secret(secret);
    let validation = Validation::new(algorithm.into());
    jwt_decode::<JwtClaims>(token, &key, &validation)
        .map(|data| data.claims)
        .map_err(map_error)
}

/// Sign `claims` with `secret` using `algorithm` and return the compact
/// JWT string (`header.payload.signature`). JOLT-RS-149.
///
/// This is the signing-side convenience entrypoint. Callers that need a full
/// [`JwtConfig`] (e.g. for round-trip encode→decode verification) should
/// construct a config and call [`decode`] separately.
pub fn encode(
    claims: &JwtClaims,
    secret: &[u8],
    algorithm: Algorithm,
) -> Result<String, JwtEncodeError> {
    let header = Header::new(algorithm.into());
    jwks_encode(&header, claims, &EncodingKey::from_secret(secret))
        .map_err(|e| JwtEncodeError(e.to_string()))
}

/// Validate `token` against `config` and return the parsed [`JwtClaims`] on
/// success or a typed [`JwtDecodeError`] on failure.
///
/// The token must be the bare JWT string (the value of an
/// `Authorization: Bearer <token>` header with the `Bearer ` prefix already
/// stripped — JOLT-RS-071's [`AuthBearerLayer`] handles that stripping).
///
/// Validation honors `exp` (rejects expired tokens via
/// [`JwtDecodeError::Expired`]), the configured algorithm (rejects token-vs-
/// config algorithm mismatches via [`JwtDecodeError::InvalidAlgorithm`]),
/// and the signature (rejects bad signatures via
/// [`JwtDecodeError::InvalidSignature`]). Audience validation is disabled
/// for the initial slice; see module docs decision 3.
///
/// [`AuthBearerLayer`]: ../../../jolt_core/auth_bearer/struct.AuthBearerLayer.html
pub fn decode(token: &str, config: &JwtConfig) -> Result<JwtClaims, JwtDecodeError> {
    let key = DecodingKey::from_secret(&config.secret);

    // jsonwebtoken's `Validation::new` defaults `validate_aud` to true with
    // an empty audience set, which would reject tokens lacking an `aud`
    // claim for the WRONG reason (since no audience expectation has been
    // configured). Disable it explicitly until `JwtConfig` grows an audience
    // knob (deferred per module docs decision 3).
    let mut validation = Validation::new(config.algorithm);
    validation.validate_aud = false;

    jwt_decode::<JwtClaims>(token, &key, &validation)
        .map(|data| data.claims)
        .map_err(map_error)
}

/// Map a [`jsonwebtoken::errors::Error`] to the framework's typed
/// [`JwtDecodeError`]. Recognized error kinds become dedicated variants; the
/// catch-all [`JwtDecodeError::Other`] preserves the upstream error's
/// [`Display`] rendering so operators can still pin the cause.
fn map_error(err: jsonwebtoken::errors::Error) -> JwtDecodeError {
    use jsonwebtoken::errors::ErrorKind;
    match err.kind() {
        ErrorKind::ExpiredSignature => JwtDecodeError::Expired,
        ErrorKind::InvalidSignature => JwtDecodeError::InvalidSignature,
        ErrorKind::InvalidAlgorithm | ErrorKind::InvalidAlgorithmName => {
            JwtDecodeError::InvalidAlgorithm
        }
        ErrorKind::InvalidToken
        | ErrorKind::Base64(_)
        | ErrorKind::Json(_)
        | ErrorKind::Utf8(_) => JwtDecodeError::Malformed,
        _ => JwtDecodeError::Other(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is sane")
            .as_secs()
    }

    fn sign_hs256(secret: &[u8], claims: &JwtClaims) -> String {
        encode(
            &Header::new(Algorithm::HS256.into()),
            claims,
            &EncodingKey::from_secret(secret),
        )
        .expect("HS256 encode with static secret never fails")
    }

    #[test]
    fn decode_valid_hs256_token_returns_claims() {
        let secret = b"jolt-rs-072-test-secret";
        let claims = JwtClaims {
            sub: Some("alice".to_owned()),
            exp: Some(now_secs() + 3600),
            iat: Some(now_secs()),
            nbf: None,
            iss: None,
            aud: None,
            custom: HashMap::new(),
        };
        let token = sign_hs256(secret, &claims);
        let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);

        let out = decode(&token, &config).expect("valid token must decode");
        assert_eq!(out.sub.as_deref(), Some("alice"));
        assert_eq!(out.exp, claims.exp);
        assert_eq!(out.iat, claims.iat);
        assert!(
            out.custom.is_empty(),
            "minting a token without custom claims must leave custom empty after round-trip"
        );
    }

    #[test]
    fn decode_expired_token_yields_expired_variant() {
        let secret = b"jolt-rs-072-test-secret";
        // exp = 1000 → 1970-01-01 00:16:40 UTC; well in the past.
        let claims = JwtClaims {
            sub: Some("alice".to_owned()),
            exp: Some(1_000),
            iat: None,
            nbf: None,
            iss: None,
            aud: None,
            custom: HashMap::new(),
        };
        let token = sign_hs256(secret, &claims);
        let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);

        let err = decode(&token, &config).expect_err("expired token must reject");
        assert_eq!(err, JwtDecodeError::Expired);
    }

    #[test]
    fn decode_with_wrong_secret_yields_invalid_signature_variant() {
        let claims = JwtClaims {
            sub: Some("alice".to_owned()),
            exp: Some(now_secs() + 3600),
            iat: None,
            nbf: None,
            iss: None,
            aud: None,
            custom: HashMap::new(),
        };
        let token = sign_hs256(b"signed-with-this-secret", &claims);
        let config = JwtConfig::new(b"verify-with-DIFFERENT-secret".to_vec(), Algorithm::HS256);

        let err = decode(&token, &config).expect_err("wrong-secret token must reject");
        assert_eq!(err, JwtDecodeError::InvalidSignature);
    }

    #[test]
    fn decode_malformed_token_yields_malformed_variant() {
        let config = JwtConfig::new(b"any-secret".to_vec(), Algorithm::HS256);
        let err = decode("definitely-not-a-jwt", &config)
            .expect_err("malformed token must reject");
        assert_eq!(err, JwtDecodeError::Malformed);
    }

    #[test]
    fn decode_algorithm_mismatch_yields_invalid_algorithm_variant() {
        let secret = b"jolt-rs-072-test-secret";
        let claims = JwtClaims {
            sub: Some("alice".to_owned()),
            exp: Some(now_secs() + 3600),
            iat: None,
            nbf: None,
            iss: None,
            aud: None,
            custom: HashMap::new(),
        };
        // Sign with HS384 but configure validation for HS256.
        let token = encode(
            &Header::new(Algorithm::HS384.into()),
            &claims,
            &EncodingKey::from_secret(secret),
        )
        .unwrap();
        let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);

        let err = decode(&token, &config).expect_err("algorithm mismatch must reject");
        assert_eq!(err, JwtDecodeError::InvalidAlgorithm);
    }

    #[test]
    fn decode_token_with_custom_claims_surfaces_them_in_custom() {
        // PRD-mandated 074 verification (custom claims): minting a token with
        // application-defined claims like `role` and `scopes` must round-trip
        // verbatim through the `custom` flattened map. Pins the contract that
        // the explicit fields (sub/exp/iat/nbf/iss/aud) are NOT also duplicated
        // into `custom` — the flatten target only captures keys serde didn't
        // bind to an explicit field.
        let secret = b"jolt-rs-074-custom-claims-secret";
        let mut custom = HashMap::new();
        custom.insert("role".to_owned(), JsonValue::String("admin".to_owned()));
        custom.insert(
            "scopes".to_owned(),
            serde_json::json!(["read", "write", "admin"]),
        );
        custom.insert("tenant_id".to_owned(), serde_json::json!(42));
        let claims = JwtClaims {
            sub: Some("user-074".to_owned()),
            exp: Some(now_secs() + 3600),
            iat: Some(now_secs()),
            nbf: None,
            iss: None,
            aud: None,
            custom,
        };
        let token = sign_hs256(secret, &claims);
        let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);

        let out = decode(&token, &config).expect("valid custom-claims token must decode");
        assert_eq!(out.sub.as_deref(), Some("user-074"));
        assert_eq!(
            out.custom.get("role"),
            Some(&JsonValue::String("admin".to_owned())),
            "custom `role` claim must surface in custom after decode round-trip",
        );
        assert_eq!(
            out.custom.get("scopes"),
            Some(&serde_json::json!(["read", "write", "admin"])),
            "array-valued custom claim must round-trip verbatim",
        );
        assert_eq!(
            out.custom.get("tenant_id"),
            Some(&serde_json::json!(42)),
            "numeric custom claim must round-trip verbatim",
        );
        assert!(
            out.custom.get("sub").is_none(),
            "explicit `sub` field must NOT double-up in the flatten target",
        );
        assert!(
            out.custom.get("exp").is_none(),
            "explicit `exp` field must NOT double-up in the flatten target",
        );
        assert!(
            out.custom.get("iat").is_none(),
            "explicit `iat` field must NOT double-up in the flatten target",
        );
    }

    #[test]
    fn encode_produces_three_segment_jwt_string() {
        // PRD-mandated 149 verification: encode() output is a compact JWT
        // with the canonical header.payload.signature shape, and the same
        // output round-trips through the existing decode() path.
        let secret = b"jolt-rs-149-encode-test";
        let claims = JwtClaims {
            sub: Some("bob".to_owned()),
            exp: Some(now_secs() + 3600),
            iat: Some(now_secs()),
            nbf: None,
            iss: None,
            aud: None,
            custom: HashMap::new(),
        };

        let token = super::encode(&claims, secret, super::Algorithm::HS256)
            .expect("must encode with HMAC-SHA256");

        let segments: Vec<&str> = token.split('.').collect();
        assert_eq!(
            segments.len(),
            3,
            "JWT must have three segments (header.payload.signature), got {token}"
        );
        assert!(
            !segments[0].is_empty(),
            "header segment must be non-empty"
        );
        assert!(
            !segments[1].is_empty(),
            "payload segment must be non-empty"
        );
        assert!(
            !segments[2].is_empty(),
            "signature segment must be non-empty"
        );

        // Round-trip: decode the same token with the same secret+algorithm.
        let config = JwtConfig::new(secret.to_vec(), jsonwebtoken::Algorithm::HS256);
        let out = super::decode(&token, &config).expect("encode→decode round-trip must succeed");
        assert_eq!(out.sub.as_deref(), Some("bob"));
        assert_eq!(out.exp, claims.exp);
    }

    #[test]
    fn encode_decode_simple_round_trip_preserves_claims() {
        // PRD-mandated 150 verification: the convenience decode_simple() that
        // takes individual (token, secret, algorithm) parameters must round-trip
        // through encode() and use standard jsonwebtoken validation defaults
        // (exp, nbf, iss, aud enabled).
        let secret = b"jolt-rs-150-round-trip-secret";
        let claims = JwtClaims {
            sub: Some("eve".to_owned()),
            exp: Some(now_secs() + 3600),
            iat: Some(now_secs()),
            nbf: None,
            iss: None,
            aud: None,
            custom: HashMap::new(),
        };

        let token = super::encode(&claims, secret, super::Algorithm::HS256)
            .expect("must encode with HMAC-SHA256");

        let out = super::decode_simple(&token, secret, super::Algorithm::HS256)
            .expect("encode→decode_simple round-trip must succeed");
        assert_eq!(out.sub.as_deref(), Some("eve"));
        assert_eq!(out.exp, claims.exp);
        assert_eq!(out.iat, claims.iat);
    }

    #[test]
    fn decode_simple_rejects_expired_token() {
        // PRD-mandated 150 verification: decode_simple() must reject expired
        // tokens with JwtDecodeError::Expired when standard validation is used.
        let secret = b"jolt-rs-150-expired";
        let claims = JwtClaims {
            sub: Some("carol".to_owned()),
            exp: Some(1_000),
            iat: None,
            nbf: None,
            iss: None,
            aud: None,
            custom: HashMap::new(),
        };

        let token = super::encode(&claims, secret, super::Algorithm::HS256)
            .expect("must encode expired token");

        let err = super::decode_simple(&token, secret, super::Algorithm::HS256)
            .expect_err("expired token must reject with decode_simple");
        assert_eq!(err, JwtDecodeError::Expired);
    }
}
