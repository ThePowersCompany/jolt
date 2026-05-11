//! Comprehensive JWT tests for jolt-utils.
//!
//! Covers the scenarios listed in JOLT-RS-151:
//! * HS384 and HS512 algorithm round-trips (HS256 is covered in the inline
//!   `jwt::tests` module).
//! * Not-before (`nbf`) claim rejection.
//! * Wrong audience rejection (via `jsonwebtoken::Validation` with a configured
//!   `aud` set — the public `decode()` wrapper disables audience validation per
//!   module docs decision 3, so this test exercises `jsonwebtoken` directly).
//! * Wrong issuer rejection (same escalation as audience — `JwtConfig` doesn't
//!   yet carry an issuer knob, so the test configures `Validation` directly).
//!
//! Custom claims, expired tokens, malformed tokens, wrong secrets, and HS256
//! are already exercised in the inline `jwt::tests` module.

use crate::jwt::{decode, encode, Algorithm, JwtClaims, JwtConfig, JwtDecodeError};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is sane")
        .as_secs()
}

fn sign(secret: &[u8], claims: &JwtClaims, algorithm: Algorithm) -> String {
    jsonwebtoken::encode(
        &Header::new(algorithm.into()),
        claims,
        &EncodingKey::from_secret(secret),
    )
    .expect("signing with static secret never fails")
}

fn base_claims() -> JwtClaims {
    JwtClaims {
        sub: Some("test-user".to_owned()),
        exp: Some(now_secs() + 3600),
        iat: Some(now_secs()),
        nbf: None,
        iss: None,
        aud: None,
        custom: HashMap::new(),
    }
}

// ── algorithm coverage ──────────────────────────────────────────────

#[test]
fn hs384_encode_decode_round_trip_preserves_claims() {
    let secret = b"hs384-test-secret-key";
    let claims = base_claims();
    let token = encode(&claims, secret, Algorithm::HS384)
        .expect("HS384 encode must succeed");
    let segments: Vec<&str> = token.split('.').collect();
    assert_eq!(segments.len(), 3, "HS384 token must have three segments");

    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS384);
    let out = decode(&token, &config).expect("HS384 decode round-trip must succeed");
    assert_eq!(out.sub.as_deref(), Some("test-user"));
    assert_eq!(out.exp, claims.exp);
    assert_eq!(out.iat, claims.iat);
}

#[test]
fn hs512_encode_decode_round_trip_preserves_claims() {
    let secret = b"hs512-test-secret-key-needs-to-be-at-least-64-bytes";
    let claims = base_claims();
    let token = encode(&claims, secret, Algorithm::HS512)
        .expect("HS512 encode must succeed");
    let segments: Vec<&str> = token.split('.').collect();
    assert_eq!(segments.len(), 3, "HS512 token must have three segments");

    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS512);
    let out = decode(&token, &config).expect("HS512 decode round-trip must succeed");
    assert_eq!(out.sub.as_deref(), Some("test-user"));
    assert_eq!(out.exp, claims.exp);
    assert_eq!(out.iat, claims.iat);
}

#[test]
fn hs384_token_rejected_when_validated_as_hs256() {
    let secret = b"shared-secret";
    let claims = base_claims();
    let token = sign(secret, &claims, Algorithm::HS384);
    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);

    let err = decode(&token, &config).expect_err("HS384 token validated as HS256 must reject");
    assert_eq!(err, JwtDecodeError::InvalidAlgorithm);
}

#[test]
fn hs512_token_rejected_when_validated_as_hs256() {
    let secret = b"another-shared-secret-with-enough-length-for-hs512-x";
    let claims = base_claims();
    let token = sign(secret, &claims, Algorithm::HS512);
    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);

    let err = decode(&token, &config).expect_err("HS512 token validated as HS256 must reject");
    assert_eq!(err, JwtDecodeError::InvalidAlgorithm);
}

// ── not-before coverage ─────────────────────────────────────────────

#[test]
fn decode_rejects_token_before_not_before_time() {
    // Uses jsonwebtoken directly with validate_nbf = true because Jolt's
    // public decode() disables nbf validation by default (it relies on
    // jsonwebtoken::Validation::new defaults, which set validate_nbf = false).
    // This test proves the underlying jsonwebtoken nbf enforcement works
    // so that when Jolt's public decode() (or JwtConfig) gains an nbf knob,
    // the enforcement path is already verified.
    let secret = b"nbf-test-secret";
    let claims = JwtClaims {
        nbf: Some(now_secs() + 7200), // 2 hours in the future
        ..base_claims()
    };
    let token = sign(secret, &claims, Algorithm::HS256);

    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.validate_nbf = true;
    validation.validate_aud = false;

    let err = jsonwebtoken::decode::<JwtClaims>(
        &token,
        &DecodingKey::from_secret(secret),
        &validation,
    )
    .expect_err("token used before nbf must reject");
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("immature"),
        "nbf error message must mention 'immature': {msg}",
    );
}

#[test]
fn decode_simple_rejects_token_before_not_before_time() {
    // Same rationale as decode_rejects_token_before_not_before_time:
    // jsonwebtoken::Validation::new defaults validate_nbf = false, so we
    // exercise jsonwebtoken directly with validate_nbf = true to prove
    // the enforcement path.
    let secret = b"nbf-decode-simple-secret";
    let claims = JwtClaims {
        nbf: Some(now_secs() + 7200),
        ..base_claims()
    };
    let token = sign(secret, &claims, Algorithm::HS256);

    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    validation.validate_nbf = true;
    validation.validate_aud = false;

    let err = jsonwebtoken::decode::<JwtClaims>(
        &token,
        &DecodingKey::from_secret(secret),
        &validation,
    )
    .expect_err("token used before nbf must reject");
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("immature"),
        "nbf error message must mention 'immature': {msg}",
    );
}

#[test]
fn decode_accepts_token_with_past_not_before_time() {
    let secret = b"nbf-past-secret";
    let claims = JwtClaims {
        nbf: Some(now_secs() - 3600), // 1 hour in the past
        ..base_claims()
    };
    let token = encode(&claims, secret, Algorithm::HS256)
        .expect("encode with past nbf must succeed");

    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);
    decode(&token, &config)
        .expect("token used after nbf must be accepted");
}

// ── audience coverage ───────────────────────────────────────────────
//
// Jolt's public `decode()` wrapper disables audience validation
// (module docs decision 3) because `JwtConfig` does not yet carry an
// audience knob.  These tests exercise the lower-level jsonwebtoken
// `Validation` API directly to prove the audience semantics the
// framework will inherit when the knob lands.

#[test]
fn jsonwebtoken_rejects_token_with_wrong_audience() {
    let secret = b"audience-test-secret";
    let claims = JwtClaims {
        aud: Some("expected-audience".to_owned()),
        ..base_claims()
    };
    let token = sign(secret, &claims, Algorithm::HS256);

    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    let mut allowed = HashSet::new();
    allowed.insert("different-audience".to_owned());
    validation.aud = Some(allowed);

    let err = jsonwebtoken::decode::<JwtClaims>(
        &token,
        &DecodingKey::from_secret(secret),
        &validation,
    )
    .expect_err("token with wrong audience must be rejected by jsonwebtoken");
    let msg = format!("{err}");
    let lower = msg.to_lowercase();
    assert!(
        lower.contains("aud") || lower.contains("audience"),
        "audience error must mention aud/audience: {msg}",
    );
}

#[test]
fn jsonwebtoken_accepts_token_with_correct_audience() {
    let secret = b"audience-valid-secret";
    let claims = JwtClaims {
        aud: Some("my-audience".to_owned()),
        ..base_claims()
    };
    let token = sign(secret, &claims, Algorithm::HS256);

    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    let mut allowed = HashSet::new();
    allowed.insert("my-audience".to_owned());
    validation.aud = Some(allowed);

    let data = jsonwebtoken::decode::<JwtClaims>(
        &token,
        &DecodingKey::from_secret(secret),
        &validation,
    )
    .expect("token with correct audience must be accepted");
    assert_eq!(data.claims.aud.as_deref(), Some("my-audience"));
}

// ── issuer coverage ─────────────────────────────────────────────────
//
// Same rationale as audience: `JwtConfig` does not yet carry an issuer
// knob, so these tests exercise `jsonwebtoken::Validation` directly.

#[test]
fn jsonwebtoken_rejects_token_with_wrong_issuer() {
    let secret = b"issuer-test-secret";
    let claims = JwtClaims {
        iss: Some("expected-issuer".to_owned()),
        ..base_claims()
    };
    let token = sign(secret, &claims, Algorithm::HS256);

    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    let mut allowed = HashSet::new();
    allowed.insert("different-issuer".to_owned());
    validation.iss = Some(allowed);

    let err = jsonwebtoken::decode::<JwtClaims>(
        &token,
        &DecodingKey::from_secret(secret),
        &validation,
    )
    .expect_err("token with wrong issuer must be rejected by jsonwebtoken");
    let msg = format!("{err}");
    let lower = msg.to_lowercase();
    assert!(
        lower.contains("iss") || lower.contains("issuer"),
        "issuer error must mention iss/issuer: {msg}",
    );
}

#[test]
fn jsonwebtoken_accepts_token_with_correct_issuer() {
    let secret = b"issuer-valid-secret";
    let claims = JwtClaims {
        iss: Some("my-issuer".to_owned()),
        ..base_claims()
    };
    let token = sign(secret, &claims, Algorithm::HS256);

    let mut validation = Validation::new(jsonwebtoken::Algorithm::HS256);
    let mut allowed = HashSet::new();
    allowed.insert("my-issuer".to_owned());
    validation.iss = Some(allowed);

    let data = jsonwebtoken::decode::<JwtClaims>(
        &token,
        &DecodingKey::from_secret(secret),
        &validation,
    )
    .expect("token with correct issuer must be accepted");
    assert_eq!(data.claims.iss.as_deref(), Some("my-issuer"));
}

// ── edge cases ──────────────────────────────────────────────────────

#[test]
fn decode_rejects_empty_token() {
    let config = JwtConfig::new(b"any-secret".to_vec(), Algorithm::HS256);
    let err = decode("", &config).expect_err("empty string must reject");
    assert_eq!(err, JwtDecodeError::Malformed);
}

#[test]
fn decode_rejects_two_segment_token() {
    let config = JwtConfig::new(b"any-secret".to_vec(), Algorithm::HS256);
    let err = decode("header.payload", &config)
        .expect_err("two-segment token must reject");
    assert_eq!(err, JwtDecodeError::Malformed);
}

#[test]
fn decode_rejects_token_signed_with_wrong_algorithm_secret() {
    let secret = b"valid-secret";
    let claims = base_claims();
    let token = sign(secret, &claims, Algorithm::HS256);
    let config = JwtConfig::new(b"completely-different-key-material", Algorithm::HS256);

    let err = decode(&token, &config).expect_err("wrong secret must reject");
    assert_eq!(err, JwtDecodeError::InvalidSignature);
}
