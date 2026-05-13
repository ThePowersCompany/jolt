//! Comprehensive JWT tests for joltr-utils.
//!
//! Covers the scenarios listed in JOLTR-RS-151:
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

const RSA_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDCjwzN+jM99d3g
wLc/BbM4oLooTBqK1Sywm1JQTZx8XJAUzfyfDmwtmSwyVjUQ0WOLFRvUIq22W6lV
cq0GHIDQZ2UnsZxb0XEJ1ym4gygVnh10g7/Y40qsOrsg1cKI6gj4uzkzSlFtGyUC
44SvJPI8fshFC6ebR2isPe04eDw33i3iJTz6BxNrbavYTJVm2uTkGRG71grwgFw8
zR3Du2o9sD6qK0sOJucmi5bOQRdlhbtHB3c04SBJ9DSp0KsLAtPph0VDDEtd5u47
Y0Vn7WKH9bGnEx71JrahcoqHUIYcd8XRUyfXB3pmWMaU1AYR9QIGfHYa0KYTxv+4
1ne0i/M3AgMBAAECggEAWOkhi2HLGAYreuHm/ByBPiApYnAA8zAfJ6gbcko9eJGe
YHuP9ioTorTsfyZpQsHFsVIYsRWV+A+kb0GkM3ZEIbkWf5DJqSYp97rFvKXnZBWp
VU0+F4IrZlDComs9Zu844V5B8iAE3Qz6GXta7+U89AtmPzNnyWzVN11ncpZzwn1v
MxEaFSSwTl9lj1Hh08BYMC0u2QUL6nPNypNryF/+dHFdWrOw6iWQeM+QJE2I0DrJ
NN8bmtneJ3V9A/mNV0xiQMRtJ8An7L8oKHKb8oMWe67VrK4GLYVnQ78a9WT+my8k
YZ67VAoxn6llNWrjUf8Kknwq1E4ELlFku+5eTgdvFQKBgQD9P+HIiRgZRu/+npLK
HM/apotq6E4REsULVK7ch5i5OD63hgYEPEBTYYGfat1rsTh29fk5jYB/wAXxbb43
zQCYCmFVyuG/kh83E/s1D7Y0waN8lyuBebewsiZaR3lr6k1FByW7+3xbSGdiU8VT
0igZG8Y3rIewwaLyZZxVQdk2wwKBgQDEq/z8dGW0lb4C/Lm2LfMW/TuaFHTO7lIq
Bi7Xx3TtIjd17x6TALz1FRER5m/ucmpGK43YgBwEzhfiz/kEmya9oiMlL6CoMT/b
2Bk0FfqcFKq1E7PXw7DvvlvWbPbNMlFY/3RUUsnKw9dbOrW73LQe8C9FzJbMWUk9
DGzpCKuSfQKBgQCaNKXlGCWGbr2AS0qSq0ydlT/bjyzKFKXLKnt3aVHDps35rjBH
r/BzVTI6wjWld7osJcbmFpWiNGjqA6sKC1hLPDbcqLchkXZUcWRLgQ+vvCEyolIp
etYxT6ku28rBvV/jomCFwLKOWt4o711+lr832sOt7u6I4L+53cl92KTNRQKBgCTe
kTjKoV5vTAXHVxFeH9pJcuj5uMQqTWDvc8yj6bmi1n7XawXn94SChIa0intLG703
4QmbSqVj9Xphvq8sXuDiCnCoMxgU04HlSyRGkoq8HRyBKw/h8cOgDhtupf3l1vY2
PZqpQocum6rQoM0tHN9H67TMG+EHRAGb2Lb/FxsJAoGAc3wALUvnMzlZ41AHFjAS
7jYuEJyFndCRbFIllSoxlG389rX5DKT1YhuFsJ20Xj20u6rBk7IbcL5+YH9nfzlu
pPAo9KTSIkVdfy8RjClnyK5PgUPk4lZblbi1aUerx3KuO+IBwKDQ0RgQfOv5MVHV
tSA7cWLhGVLDm725DxFtX9s=
-----END PRIVATE KEY-----"#;

const RSA_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAwo8MzfozPfXd4MC3PwWz
OKC6KEwaitUssJtSUE2cfFyQFM38nw5sLZksMlY1ENFjixUb1CKttlupVXKtBhyA
0GdlJ7GcW9FxCdcpuIMoFZ4ddIO/2ONKrDq7INXCiOoI+Ls5M0pRbRslAuOEryTy
PH7IRQunm0dorD3tOHg8N94t4iU8+gcTa22r2EyVZtrk5BkRu9YK8IBcPM0dw7tq
PbA+qitLDibnJouWzkEXZYW7Rwd3NOEgSfQ0qdCrCwLT6YdFQwxLXebuO2NFZ+1i
h/WxpxMe9Sa2oXKKh1CGHHfF0VMn1wd6ZljGlNQGEfUCBnx2GtCmE8b/uNZ3tIvz
NwIDAQAB
-----END PUBLIC KEY-----"#;

const RSA_WRONG_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA5x9kq09y9IxtNMil3g7Q
Psw62QFcyKMQh+WaMzX2CbRuOB5UekqEHIMTHrWY6l0MRMaoM9Jf+OEcA2WruqgQ
EoptVAfmsIKwygFuR2fS1rRNyGZYjDSGPC4BewyCXkbhGKK5Kb+M2qNITG4e4o8s
teMTGGYtfFujLP2fmQyaGDLRNNZNtpaiC55v3Kt1dnI9oXEkE+eEWTBvaJ5amNXM
6UTcSTEPUPTRqnRTOiLrvAcnQVltmLg/z1fzlbEJF9Ie0llSOtJy31+l9MwV49JF
HbM/c+ukyIvWEwr9KH6Gi1x+611qaJHDjq1XuTqzv/S7C4GR71gaFaB4YaMIriKD
BQIDAQAB
-----END PUBLIC KEY-----"#;

const ES256_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgRFKTMW+0q3f6ebqG
4vFgfCj4FOKktP5AQoDoX0A/f5GhRANCAARyOd6gl9+AQ01qm7ggYuuKdLYSOa7h
f6vueRLTLfMD3rKo1bZgPxoknR3LD+pcjZlpf6qIa3zT3nX2VzYI/Lr2
-----END PRIVATE KEY-----"#;

const ES256_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEcjneoJffgENNapu4IGLrinS2Ejmu
4X+r7nkS0y3zA96yqNW2YD8aJJ0dyw/qXI2ZaX+qiGt809519lc2CPy69g==
-----END PUBLIC KEY-----"#;

const ES256_WRONG_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEjfCzgLNtAJxAvaCY1SSzHiYhytVc
xLWXdECEyopbSU0jUD8yFHl4nhTOJWU87AZRmI2kfryUHgRo4n2s90KrIg==
-----END PUBLIC KEY-----"#;

const ES384_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIG2AgEAMBAGByqGSM49AgEGBSuBBAAiBIGeMIGbAgEBBDD4xtiQ5xeYjiXWbacp
SyyZ+do0G7rTZy7Eth7YzBVevsVOGkUzEEWCnbCQfhky4POhZANiAASk/xtsTN7n
cvDYzIQSCzhEp+zpxcK0pdZweaatfKhBKQ9nDBtSGQ23BB0cwRA6zgGQsc7JRnMl
BkPP21NvtCGQzD3SiwpAQEJOrNzzV1XwPDD7EtqKw8uwcXikySmrAIg=
-----END PRIVATE KEY-----"#;

const ES384_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MHYwEAYHKoZIzj0CAQYFK4EEACIDYgAEpP8bbEze53Lw2MyEEgs4RKfs6cXCtKXW
cHmmrXyoQSkPZwwbUhkNtwQdHMEQOs4BkLHOyUZzJQZDz9tTb7QhkMw90osKQEBC
Tqzc81dV8Dww+xLaisPLsHF4pMkpqwCI
-----END PUBLIC KEY-----"#;

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
    let token = encode(&claims, secret, Algorithm::HS384).expect("HS384 encode must succeed");
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
    let token = encode(&claims, secret, Algorithm::HS512).expect("HS512 encode must succeed");
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

fn assert_rsa_round_trip(algorithm: Algorithm) {
    let claims = base_claims();
    let token = encode(&claims, RSA_PRIVATE_KEY.as_bytes(), algorithm)
        .expect("RSA encode must succeed with private PEM");
    let segments: Vec<&str> = token.split('.').collect();
    assert_eq!(segments.len(), 3, "RSA token must have three segments");

    let config = JwtConfig::new(RSA_PUBLIC_KEY.as_bytes(), algorithm);
    let out = decode(&token, &config).expect("RSA decode must succeed with public PEM");
    assert_eq!(out.sub.as_deref(), Some("test-user"));
    assert_eq!(out.exp, claims.exp);
    assert_eq!(out.iat, claims.iat);
}

fn assert_ecdsa_round_trip(algorithm: Algorithm, private_key: &str, public_key: &str) {
    let claims = base_claims();
    let token = encode(&claims, private_key.as_bytes(), algorithm)
        .expect("ECDSA encode must succeed with private PEM");
    let segments: Vec<&str> = token.split('.').collect();
    assert_eq!(segments.len(), 3, "ECDSA token must have three segments");

    let config = JwtConfig::new(public_key.as_bytes(), algorithm);
    let out = decode(&token, &config).expect("ECDSA decode must succeed with public PEM");
    assert_eq!(out.sub.as_deref(), Some("test-user"));
    assert_eq!(out.exp, claims.exp);
    assert_eq!(out.iat, claims.iat);
}

#[test]
fn rs256_encode_decode_round_trip_preserves_claims() {
    assert_rsa_round_trip(Algorithm::RS256);
}

#[test]
fn rs384_encode_decode_round_trip_preserves_claims() {
    assert_rsa_round_trip(Algorithm::RS384);
}

#[test]
fn rs512_encode_decode_round_trip_preserves_claims() {
    assert_rsa_round_trip(Algorithm::RS512);
}

#[test]
fn rs256_token_rejected_with_wrong_public_key() {
    let claims = base_claims();
    let token = encode(&claims, RSA_PRIVATE_KEY.as_bytes(), Algorithm::RS256)
        .expect("RS256 encode must succeed");
    let config = JwtConfig::new(RSA_WRONG_PUBLIC_KEY.as_bytes(), Algorithm::RS256);

    let err = decode(&token, &config).expect_err("wrong RSA public key must reject");
    assert_eq!(err, JwtDecodeError::InvalidSignature);
}

#[test]
fn rs512_token_rejected_when_validated_as_rs256() {
    let claims = base_claims();
    let token = encode(&claims, RSA_PRIVATE_KEY.as_bytes(), Algorithm::RS512)
        .expect("RS512 encode must succeed");
    let config = JwtConfig::new(RSA_PUBLIC_KEY.as_bytes(), Algorithm::RS256);

    let err = decode(&token, &config).expect_err("RS512 token validated as RS256 must reject");
    assert_eq!(err, JwtDecodeError::InvalidAlgorithm);
}

#[test]
fn es256_encode_decode_round_trip_preserves_claims() {
    assert_ecdsa_round_trip(Algorithm::ES256, ES256_PRIVATE_KEY, ES256_PUBLIC_KEY);
}

#[test]
fn es384_encode_decode_round_trip_preserves_claims() {
    assert_ecdsa_round_trip(Algorithm::ES384, ES384_PRIVATE_KEY, ES384_PUBLIC_KEY);
}

#[test]
fn es256_token_rejected_with_wrong_public_key() {
    let claims = base_claims();
    let token = encode(&claims, ES256_PRIVATE_KEY.as_bytes(), Algorithm::ES256)
        .expect("ES256 encode must succeed");
    let config = JwtConfig::new(ES256_WRONG_PUBLIC_KEY.as_bytes(), Algorithm::ES256);

    let err = decode(&token, &config).expect_err("wrong ECDSA public key must reject");
    assert_eq!(err, JwtDecodeError::InvalidSignature);
}

#[test]
fn es384_token_rejected_when_validated_as_es256() {
    let claims = base_claims();
    let token = encode(&claims, ES384_PRIVATE_KEY.as_bytes(), Algorithm::ES384)
        .expect("ES384 encode must succeed");
    let config = JwtConfig::new(ES384_PUBLIC_KEY.as_bytes(), Algorithm::ES256);

    let err = decode(&token, &config).expect_err("ES384 token validated as ES256 must reject");
    assert_eq!(err, JwtDecodeError::InvalidAlgorithm);
}

// ── not-before coverage ─────────────────────────────────────────────

#[test]
fn decode_rejects_token_before_not_before_time() {
    // Uses jsonwebtoken directly with validate_nbf = true because JoltR's
    // public decode() disables nbf validation by default (it relies on
    // jsonwebtoken::Validation::new defaults, which set validate_nbf = false).
    // This test proves the underlying jsonwebtoken nbf enforcement works
    // so that when JoltR's public decode() (or JwtConfig) gains an nbf knob,
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

    let err =
        jsonwebtoken::decode::<JwtClaims>(&token, &DecodingKey::from_secret(secret), &validation)
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

    let err =
        jsonwebtoken::decode::<JwtClaims>(&token, &DecodingKey::from_secret(secret), &validation)
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
    let token =
        encode(&claims, secret, Algorithm::HS256).expect("encode with past nbf must succeed");

    let config = JwtConfig::new(secret.to_vec(), Algorithm::HS256);
    decode(&token, &config).expect("token used after nbf must be accepted");
}

// ── audience coverage ───────────────────────────────────────────────
//
// JoltR's public `decode()` wrapper disables audience validation
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

    let err =
        jsonwebtoken::decode::<JwtClaims>(&token, &DecodingKey::from_secret(secret), &validation)
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

    let data =
        jsonwebtoken::decode::<JwtClaims>(&token, &DecodingKey::from_secret(secret), &validation)
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

    let err =
        jsonwebtoken::decode::<JwtClaims>(&token, &DecodingKey::from_secret(secret), &validation)
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

    let data =
        jsonwebtoken::decode::<JwtClaims>(&token, &DecodingKey::from_secret(secret), &validation)
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
    let err = decode("header.payload", &config).expect_err("two-segment token must reject");
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
