use std::path::Path;

use rand::Rng;
use sha2::{Digest, Sha256};

use crate::error::{KaidaDbError, Result};

const KEY_FILENAME: &str = ".server_key";
const KEY_LENGTH: usize = 32;

/// Generate a random alphanumeric password.
pub fn generate_key() -> String {
    let mut rng = rand::thread_rng();
    (0..KEY_LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..62);
            match idx {
                0..=9 => (b'0' + idx) as char,
                10..=35 => (b'a' + idx - 10) as char,
                _ => (b'A' + idx - 36) as char,
            }
        })
        .collect()
}

/// Hash a plaintext key with SHA-256, returning hex-encoded hash.
pub fn hash_key(plaintext: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hex::encode(hasher.finalize())
}

/// Verify a plaintext key against a stored hash.
pub fn verify_key(provided: &str, stored_hash: &str) -> bool {
    hash_key(provided) == stored_hash
}

/// Load the server key hash from disk, or create a new one if it doesn't exist.
///
/// Returns `(key_hash, Option<plaintext>)`. The plaintext is only returned
/// when a new key is generated (so it can be displayed to the user once).
pub fn load_or_create_key(data_dir: &Path) -> Result<(String, Option<String>)> {
    let key_path = data_dir.join(KEY_FILENAME);

    if key_path.exists() {
        let contents = std::fs::read_to_string(&key_path).map_err(|e| {
            KaidaDbError::Config(format!("failed to read server key: {e}"))
        })?;
        let hash = contents.trim().to_string();
        if hash.len() != 64 {
            return Err(KaidaDbError::Config(
                "server key file is corrupt (expected 64-char hex hash)".into(),
            ));
        }
        Ok((hash, None))
    } else {
        // Ensure data dir exists
        std::fs::create_dir_all(data_dir).map_err(|e| {
            KaidaDbError::Config(format!("failed to create data directory: {e}"))
        })?;

        let plaintext = generate_key();
        let hash = hash_key(&plaintext);
        std::fs::write(&key_path, &hash).map_err(|e| {
            KaidaDbError::Config(format!("failed to write server key: {e}"))
        })?;
        Ok((hash, Some(plaintext)))
    }
}

/// Regenerate the server key. Returns the new plaintext key.
pub fn regenerate_key(data_dir: &Path) -> Result<String> {
    let key_path = data_dir.join(KEY_FILENAME);

    std::fs::create_dir_all(data_dir).map_err(|e| {
        KaidaDbError::Config(format!("failed to create data directory: {e}"))
    })?;

    let plaintext = generate_key();
    let hash = hash_key(&plaintext);
    std::fs::write(&key_path, &hash).map_err(|e| {
        KaidaDbError::Config(format!("failed to write server key: {e}"))
    })?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_key_length() {
        let key = generate_key();
        assert_eq!(key.len(), KEY_LENGTH);
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_generate_key_uniqueness() {
        let k1 = generate_key();
        let k2 = generate_key();
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_hash_key_deterministic() {
        let hash1 = hash_key("test-password");
        let hash2 = hash_key("test-password");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_verify_key() {
        let plaintext = "my-secret-key-123";
        let hash = hash_key(plaintext);
        assert!(verify_key(plaintext, &hash));
        assert!(!verify_key("wrong-key", &hash));
    }

    #[test]
    fn test_load_or_create_key() {
        let dir = tempfile::tempdir().unwrap();

        // First call creates a new key
        let (hash1, plaintext) = load_or_create_key(dir.path()).unwrap();
        assert!(plaintext.is_some());
        assert_eq!(hash1.len(), 64);

        // Verify the plaintext matches
        let pt = plaintext.unwrap();
        assert!(verify_key(&pt, &hash1));

        // Second call loads existing key
        let (hash2, plaintext2) = load_or_create_key(dir.path()).unwrap();
        assert!(plaintext2.is_none());
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_regenerate_key() {
        let dir = tempfile::tempdir().unwrap();

        let (hash1, _) = load_or_create_key(dir.path()).unwrap();
        let new_plaintext = regenerate_key(dir.path()).unwrap();
        let (hash2, _) = load_or_create_key(dir.path()).unwrap();

        assert_ne!(hash1, hash2);
        assert!(verify_key(&new_plaintext, &hash2));
    }
}
