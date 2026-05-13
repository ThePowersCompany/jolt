//! Comprehensive password + UUID tests for joltr-utils.
//!
//! Covers the scenarios listed in JOLTR-RS-155:
//! * hash determinism (different salts → different hashes)
//! * verify edge cases (empty password, unicode, long input, invalid hex)
//! * UUID uniqueness within 1000 generations
//!
//! The inline `#[cfg(test)]` modules in `password.rs` and `uuid.rs` already
//! cover the basic smoke tests (hash returns non-empty, correct/wrong password
//! verify, valid UUID formats). This module adds determinism, edge cases, and
//! uniqueness-at-scale tests that go beyond the inline suite.

use crate::password::{Password, PasswordError};
use crate::uuid::{uuid_v4, uuid_v7};
use std::collections::HashSet;

// ── password hash determinism ───────────────────────────────────────

#[test]
fn different_salts_produce_different_hashes() {
    let (hash1, _) = Password::hash("password");
    let (hash2, _) = Password::hash("password");
    assert_ne!(
        hash1, hash2,
        "different random salts must yield different hashes"
    );
}

#[test]
fn same_salt_same_password_produces_same_hash_in_verify() {
    let plaintext = "duplicate-test";
    let (hash_hex, salt_hex) = Password::hash(plaintext);

    let result1 = Password::verify(plaintext, &hash_hex, &salt_hex).unwrap();
    let result2 = Password::verify(plaintext, &hash_hex, &salt_hex).unwrap();

    assert!(result1, "first verify of correct password must return true");
    assert!(
        result2,
        "second verify of correct password with same salt must return true"
    );
}

#[test]
fn same_salt_different_password_produces_different_hash() {
    let (hash1, salt_hex) = Password::hash("alpha");
    let hash2_result = Password::verify("beta", &hash1, &salt_hex);
    assert_eq!(
        hash2_result,
        Ok(false),
        "different password with same salt must not match"
    );
}

// ── password edge cases ─────────────────────────────────────────────

#[test]
fn empty_password_hash_produces_valid_output() {
    let (hash, salt) = Password::hash("");
    assert!(
        !hash.is_empty(),
        "empty password must produce a non-empty hash"
    );
    assert_eq!(hash.len(), 64, "empty password hash must be 64 hex chars");
    assert_eq!(salt.len(), 32, "empty password salt must be 32 hex chars");
}

#[test]
fn empty_password_verify_round_trips() {
    let (hash_hex, salt_hex) = Password::hash("");
    assert!(Password::verify("", &hash_hex, &salt_hex).unwrap());
}

#[test]
fn long_password_verify_round_trips() {
    let long = "a".repeat(10_000);
    let (hash_hex, salt_hex) = Password::hash(&long);
    assert!(Password::verify(&long, &hash_hex, &salt_hex).unwrap());
}

#[test]
fn unicode_password_verify_round_trips() {
    let unicode = "パスワード🔐🎉";
    let (hash_hex, salt_hex) = Password::hash(unicode);
    assert!(Password::verify(unicode, &hash_hex, &salt_hex).unwrap());
}

#[test]
fn unicode_password_case_sensitive() {
    let (hash_hex, salt_hex) = Password::hash("Café");
    assert!(!Password::verify("café", &hash_hex, &salt_hex).unwrap());
}

#[test]
fn verify_with_odd_length_hash_hex_returns_error() {
    let (_, salt_hex) = Password::hash("test");
    let result = Password::verify("test", "abc", &salt_hex);
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        PasswordError::InvalidHex { field: "hash" }
    );
}

#[test]
fn verify_with_odd_length_salt_hex_returns_error() {
    let (hash_hex, _) = Password::hash("test");
    let result = Password::verify("test", &hash_hex, "f");
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        PasswordError::InvalidHex { field: "salt" }
    );
}

#[test]
fn verify_with_empty_hash_hex_returns_false() {
    let (_, salt_hex) = Password::hash("test");
    let result = Password::verify("test", "", &salt_hex);
    assert_eq!(
        result,
        Ok(false),
        "empty hash hex decodes to 0 bytes, which never matches a 32-byte hash"
    );
}

#[test]
fn verify_with_empty_salt_hex_returns_false() {
    let (hash_hex, _) = Password::hash("test");
    let result = Password::verify("test", &hash_hex, "");
    assert_eq!(
        result,
        Ok(false),
        "empty salt hex decodes to 0 bytes, producing a different PBKDF2 salt"
    );
}

#[test]
fn verify_with_non_hex_hash_chars_returns_error() {
    let (_, salt_hex) = Password::hash("test");
    let result = Password::verify(
        "test",
        "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
        &salt_hex,
    );
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        PasswordError::InvalidHex { field: "hash" }
    );
}

#[test]
fn verify_with_non_hex_salt_chars_returns_error() {
    let (hash_hex, _) = Password::hash("test");
    let result = Password::verify("test", &hash_hex, "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz");
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err(),
        PasswordError::InvalidHex { field: "salt" }
    );
}

#[test]
fn hash_produces_hex_encoded_output() {
    let (hash, salt) = Password::hash("password");
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash must be all hex digits"
    );
    assert!(
        salt.chars().all(|c| c.is_ascii_hexdigit()),
        "salt must be all hex digits"
    );
}

#[test]
fn passworderror_invalid_hex_displays_field_name() {
    let err = PasswordError::InvalidHex { field: "hash" };
    let display = format!("{err}");
    assert!(display.contains("hash"), "must mention the field name");
    assert!(display.contains("hex"), "must mention hex encoding");
}

// ── UUID uniqueness at scale ────────────────────────────────────────

#[test]
fn uuid_v4_1000_unique() {
    let mut seen = HashSet::new();
    for _ in 0..1000 {
        let id = uuid_v4();
        assert!(
            seen.insert(id),
            "uuid_v4 must produce unique values within 1000 generations"
        );
    }
}

#[test]
fn uuid_v7_1000_unique() {
    let mut seen = HashSet::new();
    for _ in 0..1000 {
        let id = uuid_v7();
        assert!(
            seen.insert(id),
            "uuid_v7 must produce unique values within 1000 generations"
        );
    }
}

#[test]
fn uuid_v4_and_v7_do_not_collide() {
    let mut seen = HashSet::new();
    for _ in 0..500 {
        seen.insert(uuid_v4());
        seen.insert(uuid_v7());
    }
    assert_eq!(
        seen.len(),
        1000,
        "1000 mixed v4+v7 UUIDs must all be unique"
    );
}

#[test]
fn uuid_v4_conforms_to_format() {
    let id = uuid_v4();
    let parsed = uuid::Uuid::parse_str(&id).expect("uuid_v4 must produce valid UUID string");
    assert_eq!(parsed.get_version_num(), 4, "uuid_v4 must be version 4");
    assert_eq!(
        parsed.to_string(),
        id,
        "uuid_v4 output must round-trip through Uuid"
    );
}

#[test]
fn uuid_v7_conforms_to_format() {
    let id = uuid_v7();
    let parsed = uuid::Uuid::parse_str(&id).expect("uuid_v7 must produce valid UUID string");
    assert_eq!(parsed.get_version_num(), 7, "uuid_v7 must be version 7");
    assert_eq!(
        parsed.to_string(),
        id,
        "uuid_v7 output must round-trip through Uuid"
    );
}

#[test]
fn uuid_v4_is_lowercase() {
    for _ in 0..100 {
        let id = uuid_v4();
        assert_eq!(id, id.to_lowercase(), "uuid_v4 must produce lowercase hex");
    }
}

#[test]
fn uuid_v7_is_lowercase() {
    for _ in 0..100 {
        let id = uuid_v7();
        assert_eq!(id, id.to_lowercase(), "uuid_v7 must produce lowercase hex");
    }
}

#[test]
fn uuid_v4_has_standard_hyphenation() {
    let id = uuid_v4();
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.len(), 5, "UUID must have 5 hyphen-separated segments");
    assert_eq!(parts[0].len(), 8);
    assert_eq!(parts[1].len(), 4);
    assert_eq!(parts[2].len(), 4);
    assert_eq!(parts[3].len(), 4);
    assert_eq!(parts[4].len(), 12);
}

#[test]
fn uuid_v7_has_standard_hyphenation() {
    let id = uuid_v7();
    let parts: Vec<&str> = id.split('-').collect();
    assert_eq!(parts.len(), 5, "UUID must have 5 hyphen-separated segments");
    assert_eq!(parts[0].len(), 8);
    assert_eq!(parts[1].len(), 4);
    assert_eq!(parts[2].len(), 4);
    assert_eq!(parts[3].len(), 4);
    assert_eq!(parts[4].len(), 12);
}

#[test]
fn uuid_v4_variant_is_rfc4122() {
    for _ in 0..100 {
        let id = uuid_v4();
        let parsed = uuid::Uuid::parse_str(&id).unwrap();
        assert_eq!(
            parsed.get_variant(),
            uuid::Variant::RFC4122,
            "uuid_v4 must be RFC4122 variant"
        );
    }
}

#[test]
fn uuid_v7_variant_is_rfc4122() {
    for _ in 0..100 {
        let id = uuid_v7();
        let parsed = uuid::Uuid::parse_str(&id).unwrap();
        assert_eq!(
            parsed.get_variant(),
            uuid::Variant::RFC4122,
            "uuid_v7 must be RFC4122 variant"
        );
    }
}
