use pbkdf2::pbkdf2_hmac;
use rand::Rng;
use sha2::Sha256;

const SALT_LEN: usize = 16;
const HASH_LEN: usize = 32;
const ITERATIONS: u32 = 100_000;

pub struct Password;

impl Password {
    pub fn hash(plaintext: &str) -> (String, String) {
        let mut rng = rand::thread_rng();
        let salt: [u8; SALT_LEN] = rng.gen();
        let mut hash_bytes = [0u8; HASH_LEN];

        pbkdf2_hmac::<Sha256>(plaintext.as_bytes(), &salt, ITERATIONS, &mut hash_bytes);

        (hex_encode(&hash_bytes), hex_encode(&salt))
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
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
}
