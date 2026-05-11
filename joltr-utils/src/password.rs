use pbkdf2::pbkdf2_hmac;
use rand::Rng;
use sha2::Sha256;
use std::fmt;

const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;
const ITERATIONS: u32 = 100_000;

#[derive(Debug, PartialEq, Eq)]
pub enum PasswordError {
    InvalidHex { field: &'static str },
}

impl fmt::Display for PasswordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PasswordError::InvalidHex { field } => write!(f, "invalid hex encoding in {field}"),
        }
    }
}

impl std::error::Error for PasswordError {}

pub struct Password;

impl Password {
    pub fn hash(plaintext: &str) -> (String, String) {
        let mut rng = rand::thread_rng();
        let salt: [u8; SALT_LEN] = rng.gen();
        let mut hash_bytes = [0u8; HASH_LEN];

        pbkdf2_hmac::<Sha256>(plaintext.as_bytes(), &salt, ITERATIONS, &mut hash_bytes);

        (hex_encode(&hash_bytes), hex_encode(&salt))
    }

    pub fn verify(plaintext: &str, hash_hex: &str, salt_hex: &str) -> Result<bool, PasswordError> {
        let salt = hex_decode(salt_hex).map_err(|_| PasswordError::InvalidHex { field: "salt" })?;
        let stored_hash = hex_decode(hash_hex).map_err(|_| PasswordError::InvalidHex { field: "hash" })?;

        let mut computed_hash = [0u8; HASH_LEN];
        pbkdf2_hmac::<Sha256>(plaintext.as_bytes(), &salt, ITERATIONS, &mut computed_hash);

        Ok(computed_hash.as_ref() == stored_hash.as_slice())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn hex_decode(hex: &str) -> Result<Vec<u8>, ()> {
    if hex.len() % 2 != 0 {
        return Err(());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_returns_non_empty_hash_and_salt() {
        let (hash, salt) = Password::hash("my_secure_password");
        assert!(!hash.is_empty(), "hash must be non-empty");
        assert!(!salt.is_empty(), "salt must be non-empty");
        assert_eq!(hash.len(), 64, "SHA-256 hash hex is 64 chars");
        assert_eq!(salt.len(), 32, "16-byte salt hex is 32 chars");
    }

    #[test]
    fn different_salts_produce_different_hashes() {
        let (hash1, _salt1) = Password::hash("password");
        let (hash2, _salt2) = Password::hash("password");
        assert_ne!(hash1, hash2, "different random salts must yield different hashes");
    }

    #[test]
    fn verify_correct_password_returns_true() {
        let plaintext = "hunter2";
        let (hash_hex, salt_hex) = Password::hash(plaintext);
        let result = Password::verify(plaintext, &hash_hex, &salt_hex);
        assert!(result.is_ok());
        assert!(result.unwrap(), "correct password must verify as true");
    }

    #[test]
    fn verify_wrong_password_returns_false() {
        let (hash_hex, salt_hex) = Password::hash("correct-password");
        let result = Password::verify("wrong-password", &hash_hex, &salt_hex);
        assert!(result.is_ok());
        assert!(!result.unwrap(), "wrong password must verify as false");
    }

    #[test]
    fn verify_invalid_hash_hex_returns_error() {
        let (_, salt_hex) = Password::hash("password");
        let result = Password::verify("password", "not-a-valid-hex", &salt_hex);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            PasswordError::InvalidHex { field: "hash" }
        );
    }

    #[test]
    fn verify_invalid_salt_hex_returns_error() {
        let (hash_hex, _) = Password::hash("password");
        let result = Password::verify("password", &hash_hex, "zzz");
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            PasswordError::InvalidHex { field: "salt" }
        );
    }

    #[test]
    fn verify_is_deterministic() {
        let (hash_hex, salt_hex) = Password::hash("secret");
        for _ in 0..5 {
            assert!(Password::verify("secret", &hash_hex, &salt_hex).unwrap());
        }
    }
}
