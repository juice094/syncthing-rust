use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key as AesKey, Nonce,
};
use aes_siv::{Aes256SivAead, Key as SivKey};
use argon2::Argon2;
use base32::Alphabet;
use hkdf::Hkdf;
use sha2::Sha256;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zeroize::{Zeroize, ZeroizeOnDrop};

const CONTENT_INFO: &[u8] = b"obsidian-vault-content-v1";
const FILENAME_INFO: &[u8] = b"obsidian-vault-filename-v1";
const FORMAT_VERSION: u8 = 0x01;
const NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("argon2 error")]
    Argon2,
    #[error("walkdir error: {0}")]
    Walkdir(#[from] walkdir::Error),
    #[error("encryption failed: {0}")]
    Encrypt(String),
    #[error("decryption failed: {0}")]
    Decrypt(String),
    #[error("invalid ciphertext format")]
    InvalidFormat,
    #[error("unsupported format version: {0}")]
    UnsupportedVersion(u8),
    #[error("invalid encrypted filename")]
    InvalidFilename,
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct CryptoAdapter {
    #[zeroize(skip)]
    content_key: AesKey<Aes256Gcm>,
    #[zeroize(skip)]
    filename_key: SivKey<Aes256SivAead>,
}

impl CryptoAdapter {
    pub fn from_password(password: &str, salt: &[u8]) -> Result<Self> {
        let argon2 = Argon2::default();
        let mut master_key = [0u8; 32];
        argon2
            .hash_password_into(password.as_bytes(), salt, &mut master_key)
            .map_err(|_| Error::Argon2)?;

        let hkdf = Hkdf::<Sha256>::new(Some(salt), &master_key);
        let mut content_key = [0u8; 32];
        hkdf.expand(CONTENT_INFO, &mut content_key)
            .map_err(|e| Error::Encrypt(format!("hkdf expand failed: {e}")))?;

        let mut filename_key = [0u8; 64];
        hkdf.expand(FILENAME_INFO, &mut filename_key)
            .map_err(|e| Error::Encrypt(format!("hkdf expand failed: {e}")))?;

        master_key.zeroize();

        Ok(Self {
            content_key: content_key.into(),
            filename_key: filename_key.into(),
        })
    }

    pub fn encrypt_filename(&self, name: &str) -> Result<String> {
        let cipher = Aes256SivAead::new(&self.filename_key);
        let encrypted = cipher
            .encrypt(&Default::default(), name.as_bytes())
            .map_err(|e| Error::Encrypt(format!("filename encrypt failed: {e}")))?;
        let alphabet = Alphabet::Rfc4648 { padding: false };
        Ok(base32::encode(alphabet, &encrypted).to_lowercase())
    }

    pub fn decrypt_filename(&self, encrypted_name: &str) -> Result<String> {
        let cipher = Aes256SivAead::new(&self.filename_key);
        let alphabet = Alphabet::Rfc4648 { padding: false };
        let decoded = base32::decode(alphabet, &encrypted_name.to_uppercase())
            .ok_or(Error::InvalidFilename)?;
        let plaintext = cipher
            .decrypt(&Default::default(), decoded.as_ref())
            .map_err(|e| Error::Decrypt(format!("filename decrypt failed: {e}")))?;
        String::from_utf8(plaintext).map_err(|_| Error::InvalidFilename)
    }

    pub fn encrypt_content(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new(&self.content_key);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| Error::Encrypt(format!("content encrypt failed: {e}")))?;

        let mut out = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        out.push(FORMAT_VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    pub fn decrypt_content(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        if ciphertext.len() < 1 + NONCE_LEN {
            return Err(Error::InvalidFormat);
        }
        let version = ciphertext[0];
        if version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion(version));
        }
        let nonce = Nonce::from_slice(&ciphertext[1..1 + NONCE_LEN]);
        let encrypted = &ciphertext[1 + NONCE_LEN..];

        let cipher = Aes256Gcm::new(&self.content_key);
        cipher
            .decrypt(nonce, encrypted)
            .map_err(|e| Error::Decrypt(format!("content decrypt failed: {e}")))
    }

    pub fn encrypt_dir(&self, src: &Path, dst: &Path) -> Result<()> {
        std::fs::create_dir_all(dst)?;

        for entry in WalkDir::new(src).min_depth(1) {
            let entry = entry?;
            let rel = entry.path().strip_prefix(src).expect("prefix validated");
            if rel.components().any(|c| {
                let name = c.as_os_str().to_string_lossy();
                name.starts_with(".") && name != ".obsidian"
            }) {
                // ponytail: 先跳过隐藏文件，避免 .git/.DS_Store 等污染 sync-folder
                // 升级路径：配置化 ignore 规则
                continue;
            }

            let mut encrypted_rel = PathBuf::new();
            for comp in rel.components() {
                let name = comp.as_os_str().to_string_lossy();
                let enc = self.encrypt_filename(&name)?;
                encrypted_rel.push(enc);
            }

            let dst_path = dst.join(encrypted_rel);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&dst_path)?;
            } else {
                if let Some(parent) = dst_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let plaintext = std::fs::read(entry.path())?;
                let encrypted = self.encrypt_content(&plaintext)?;
                std::fs::write(&dst_path, encrypted)?;
            }
        }
        Ok(())
    }

    pub fn decrypt_dir(&self, src: &Path, dst: &Path) -> Result<()> {
        std::fs::create_dir_all(dst)?;

        for entry in WalkDir::new(src).min_depth(1) {
            let entry = entry?;
            let rel = entry.path().strip_prefix(src).expect("prefix validated");

            let mut decrypted_rel = PathBuf::new();
            for comp in rel.components() {
                let name = comp.as_os_str().to_string_lossy();
                let dec = self.decrypt_filename(&name)?;
                decrypted_rel.push(dec);
            }

            let dst_path = dst.join(decrypted_rel);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&dst_path)?;
            } else {
                if let Some(parent) = dst_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let encrypted = std::fs::read(entry.path())?;
                let plaintext = self.decrypt_content(&encrypted)?;
                std::fs::write(&dst_path, plaintext)?;
            }
        }
        Ok(())
    }
}

pub fn generate_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    salt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filename_roundtrip() {
        let salt = generate_salt();
        let adapter = CryptoAdapter::from_password("test-password", &salt).unwrap();
        let name = "Daily Notes/2026-07-07.md";
        let encrypted = adapter.encrypt_filename(name).unwrap();
        let decrypted = adapter.decrypt_filename(&encrypted).unwrap();
        assert_eq!(decrypted, name);

        // deterministic
        let encrypted2 = adapter.encrypt_filename(name).unwrap();
        assert_eq!(encrypted, encrypted2);
    }

    #[test]
    fn test_content_roundtrip() {
        let salt = generate_salt();
        let adapter = CryptoAdapter::from_password("test-password", &salt).unwrap();
        let plaintext = b"Hello Obsidian Vault";
        let encrypted = adapter.encrypt_content(plaintext).unwrap();
        let decrypted = adapter.decrypt_content(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_content_tamper_detection() {
        let salt = generate_salt();
        let adapter = CryptoAdapter::from_password("test-password", &salt).unwrap();
        let mut encrypted = adapter.encrypt_content(b"secret").unwrap();
        encrypted[13] ^= 0xFF;
        assert!(adapter.decrypt_content(&encrypted).is_err());
    }

    #[test]
    fn test_dir_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        let sync = tmp.path().join("sync");
        let decrypted = tmp.path().join("decrypted");

        std::fs::create_dir(&vault).unwrap();
        std::fs::create_dir(vault.join("Projects")).unwrap();
        std::fs::write(vault.join("Daily.md"), "# Daily").unwrap();
        std::fs::write(vault.join("Projects").join("Idea.md"), "# Idea").unwrap();

        let salt = generate_salt();
        let adapter = CryptoAdapter::from_password("test-password", &salt).unwrap();
        adapter.encrypt_dir(&vault, &sync).unwrap();
        adapter.decrypt_dir(&sync, &decrypted).unwrap();

        assert_eq!(
            std::fs::read_to_string(decrypted.join("Daily.md")).unwrap(),
            "# Daily"
        );
        assert_eq!(
            std::fs::read_to_string(decrypted.join("Projects").join("Idea.md")).unwrap(),
            "# Idea"
        );
    }

    #[test]
    fn test_wrong_password_fails() {
        let salt = generate_salt();
        let adapter = CryptoAdapter::from_password("right-password", &salt).unwrap();
        let encrypted = adapter.encrypt_content(b"secret").unwrap();

        let wrong = CryptoAdapter::from_password("wrong-password", &salt).unwrap();
        assert!(wrong.decrypt_content(&encrypted).is_err());
    }
}
