//! Device ID handling.
//!
//! Implements generation, parsing, and validation of Syncthing device IDs
//! (Base32 per RFC 4648 plus Luhn-32 check characters).
//!
//! See "docs/design/ADR-20261105-device-id-format.md".

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// Base32 alphabet (RFC 4648, no padding).
const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Device ID (32-byte SHA-256 hash of the device certificate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DeviceId(pub [u8; 32]);

impl DeviceId {
    /// Creates a device ID from a byte slice.
    ///
    /// # Errors
    /// Returns a device ID error unless `bytes` is exactly 32 bytes long.
    pub fn from_bytes(bytes: &[u8]) -> crate::Result<Self> {
        if bytes.len() != 32 {
            return Err(crate::SyncthingError::device_id("Invalid device ID length"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }

    /// Creates a device ID from a 32-byte array.
    pub const fn from_bytes_array(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Generates a random device ID (for testing).
    pub fn random() -> Self {
        use rand::RngExt;
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes);
        Self(bytes)
    }

    /// Returns the raw 32-byte identifier.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns the short ID (first 8 bytes, hex-encoded).
    pub fn short_id(&self) -> String {
        hex::encode(&self.0[..8])
    }

    /// Returns the version-vector counter ID (first 8 bytes as big-endian u64).
    ///
    /// Every device must use a unique counter ID in version vectors; otherwise
    /// concurrent edits on both ends are misread as linear history and the
    /// deleter's version "dominates", silently overwriting the peer's offline
    /// changes. Matches Go Syncthing's `ShortID` semantics.
    pub fn counter_id(&self) -> u64 {
        u64::from_be_bytes(self.0[..8].try_into().expect("first 8 bytes of 32"))
    }

    /// Returns the string representation (kept for compatibility with the
    /// legacy hex-format API).
    pub fn to_string_formatted(&self) -> String {
        self.to_string()
    }

    /// Validates the device ID's string form, including Luhn-32 check characters.
    pub fn is_valid(&self) -> bool {
        let id_str = self.to_string();
        let cleaned: String = id_str.chars().filter(|c| c.is_alphanumeric()).collect();

        if cleaned.len() != 56 {
            return false;
        }

        Self::verify_luhn32_checksum(&cleaned)
    }

    /// Verifies the Luhn-32 check characters (4 groups of 14: 13 data chars
    /// + 1 check char each).
    fn verify_luhn32_checksum(s: &str) -> bool {
        if s.len() != 56 {
            return false;
        }

        for i in 0..4 {
            let start = i * 14;
            let end = start + 13;
            let group = &s[start..end];
            let Some(check_char) = s.chars().nth(end) else {
                return false;
            };

            let expected_check = luhn32_char(group);
            if check_char != expected_check {
                return false;
            }
        }

        true
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let base32 = to_base32(&self.0);

        let data = if base32.len() >= 52 {
            base32[..52].to_string()
        } else {
            format!("{:0<52}", base32) // right-pad with '0' (i.e. 'A' in Base32)
        };

        let with_luhn = luhnify(&data);

        let parts: Vec<&str> = with_luhn
            .as_bytes()
            .chunks(7)
            // SAFETY: chunks come from a base32 String which is pure ASCII.
            .map(|chunk| unsafe { std::str::from_utf8_unchecked(chunk) })
            .collect();

        write!(f, "{}", parts.join("-"))
    }
}

impl FromStr for DeviceId {
    type Err = crate::SyncthingError;

    fn from_str(s: &str) -> crate::Result<Self> {
        let cleaned: String = s.chars().filter(|c| c.is_alphanumeric()).collect();

        if cleaned.len() == 64 {
            let bytes = hex::decode(cleaned).map_err(|e| {
                crate::SyncthingError::device_id(format!("Invalid hex encoding: {}", e))
            })?;
            Self::from_bytes(&bytes)
        } else if cleaned.len() == 56 {
            let mut base32_data = String::with_capacity(52);
            for i in 0..4 {
                let start = i * 14;
                let end = start + 13;
                base32_data.push_str(&cleaned[start..end]);
            }

            if !DeviceId::verify_luhn32_checksum(&cleaned) {
                return Err(crate::SyncthingError::device_id("Invalid Luhn-32 checksum"));
            }

            let bytes = from_base32(&base32_data)?;
            Self::from_bytes(&bytes)
        } else {
            Err(crate::SyncthingError::device_id(
                format!("Invalid device ID length: expected 64 (hex) or 56 (base32) alphanumeric chars, got {}", cleaned.len())
            ))
        }
    }
}

impl Serialize for DeviceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).map_err(serde::de::Error::custom)
    }
}

/// Encodes bytes as unpadded Base32.
fn to_base32(bytes: &[u8]) -> String {
    let mut result = String::new();
    let mut buffer = 0u32;
    let mut bits_left = 0;

    for &byte in bytes {
        buffer = (buffer << 8) | byte as u32;
        bits_left += 8;

        while bits_left >= 5 {
            let index = ((buffer >> (bits_left - 5)) & 0x1F) as usize;
            result.push(BASE32_ALPHABET[index] as char);
            bits_left -= 5;
        }
    }

    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1F) as usize;
        result.push(BASE32_ALPHABET[index] as char);
    }

    result
}

/// Decodes unpadded Base32.
fn from_base32(s: &str) -> crate::Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut buffer = 0u32;
    let mut bits_left = 0;

    for c in s.chars() {
        let c = c.to_ascii_uppercase();
        let index = BASE32_ALPHABET
            .iter()
            .position(|&b| b as char == c)
            .ok_or_else(|| {
                crate::SyncthingError::device_id(format!("Invalid Base32 character: {}", c))
            })?;

        buffer = (buffer << 5) | index as u32;
        bits_left += 5;

        while bits_left >= 8 {
            result.push(((buffer >> (bits_left - 8)) & 0xFF) as u8);
            bits_left -= 8;
        }
    }

    Ok(result)
}

/// Computes the Luhn-32 check character, matching Go Syncthing's luhn32 algorithm.
fn luhn32_char(s: &str) -> char {
    let n = 32u32;
    let mut factor = 1u32;
    let mut sum = 0u32;

    for c in s.chars() {
        let code = base32_char_to_value(c);
        let mut addend = factor * code;
        factor = if factor == 2 { 1 } else { 2 };
        addend = (addend / n) + (addend % n);
        sum += addend;
    }

    let remainder = sum % n;
    let check = (n - remainder) % n;
    BASE32_ALPHABET[check as usize] as char
}

/// Maps a Base32 character to its numeric value.
fn base32_char_to_value(c: char) -> u32 {
    let c = c.to_ascii_uppercase();
    BASE32_ALPHABET
        .iter()
        .position(|&b| b as char == c)
        .map(|i| i as u32)
        .unwrap_or(0)
}

/// Appends Luhn-32 check characters to a 52-char Base32 string (4 groups of 13).
///
/// Groups are processed in reverse so that inserting a check character does
/// not shift the offsets of earlier groups.
fn luhnify(s: &str) -> String {
    let mut result = s.to_string();

    for i in (0..4).rev() {
        let start = i * 13;
        let end = start + 13;
        let group = &result[start..end];
        let check_char = luhn32_char(group);
        result.insert(end, check_char);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_base32() {
        // 测试全0
        let bytes = [0u8; 32];
        let base32 = to_base32(&bytes);
        // 32字节 * 8位 = 256位，256/5 = 51.2，所以需要52个字符
        assert_eq!(base32.len(), 52);
        assert!(base32.chars().all(|c| c == 'A'));
    }

    #[test]
    fn test_from_base32() {
        let bytes = [0u8; 32];
        let base32 = to_base32(&bytes);
        let decoded = from_base32(&base32).unwrap();
        assert_eq!(decoded, bytes.to_vec());
    }

    #[test]
    fn test_luhn32() {
        // 测试简单的校验位计算
        let group = "AAAAAAAAAAAAA"; // 13个A
        let check = luhn32_char(group);
        assert!(BASE32_ALPHABET.contains(&(check as u8)));
    }

    #[test]
    fn test_luhnify() {
        let data = "A".repeat(52);
        let with_luhn = luhnify(&data);
        assert_eq!(with_luhn.len(), 56); // 52 + 4个校验位
    }

    #[test]
    fn test_device_id_format() {
        let bytes = [0u8; 32]; // 测试用全0
        let device_id = DeviceId::from_bytes_array(bytes);
        let s = device_id.to_string();

        // 检查格式: 8组，每组7字符，用-连接
        assert_eq!(s.len(), 7 * 8 + 7); // 56字符 + 7个连字符
        assert_eq!(s.matches('-').count(), 7);

        // 检查只包含合法字符 (A-Z, 2-7，以及-)
        for c in s.chars() {
            if c != '-' {
                assert!(
                    (c.is_ascii_uppercase() && !"0189".contains(c)) || ('2'..='7').contains(&c),
                    "非法字符: {}",
                    c
                );
            }
        }
    }

    #[test]
    fn test_device_id_parsing() {
        // 生成一个有效的设备ID
        let id = DeviceId::from_bytes_array([0u8; 32]);
        let id_str = id.to_string();

        // 解析应该成功
        let parsed = DeviceId::from_str(&id_str).unwrap();
        assert_eq!(id.0, parsed.0);
    }

    #[test]
    fn test_device_id_short_id() {
        let id = DeviceId::random();
        let short = id.short_id();
        assert_eq!(short.len(), 16); // 8 bytes = 16 hex chars
    }

    #[test]
    fn test_is_valid() {
        let id = DeviceId::from_bytes_array([0u8; 32]);
        assert!(id.is_valid());
    }

    #[test]
    fn test_roundtrip() {
        for _ in 0..10 {
            let original = DeviceId::random();
            let id_str = original.to_string();
            let parsed = DeviceId::from_str(&id_str).unwrap();
            assert_eq!(original.0, parsed.0);
        }
    }

    #[test]
    fn test_device_id_valid_chars() {
        let bytes = [0u8; 32];
        let id = DeviceId::from_bytes_array(bytes);
        let s = id.to_string();

        // 检查只包含合法字符 (A-Z 除了 O,I, 以及 2-7 和 -)
        for c in s.chars() {
            if c != '-' {
                assert!(
                    (c.is_ascii_uppercase() && !"OI".contains(c)) || // A-Z 除了 O,I
                    ('2'..='7').contains(&c),
                    "非法字符: {}",
                    c
                );
            }
        }
    }
}
