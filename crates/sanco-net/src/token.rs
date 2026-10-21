//! Workspace sync key and the bearer token derived from it.

use hkdf::Hkdf;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::{Error, Result};

const SALT: &[u8] = b"sanco-sync";
const INFO: &[u8] = b"bearer-token/v1";

/// The 32-byte secret every PC of a workspace shares.
#[derive(Clone, PartialEq, Eq)]
pub struct SyncKey([u8; 32]);

impl std::fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SyncKey(..)")
    }
}

impl SyncKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_hex(hex: &str) -> Result<Self> {
        let hex = hex.trim().as_bytes();
        if hex.len() != 64 {
            return Err(Error::BadJoinCode("sync key must be 64 hex digits"));
        }
        let mut out = [0u8; 32];
        for (i, pair) in hex.chunks(2).enumerate() {
            let digit = |c: u8| {
                (c as char)
                    .to_digit(16)
                    .ok_or(Error::BadJoinCode("bad hex"))
            };
            out[i] = (digit(pair[0])? * 16 + digit(pair[1])?) as u8;
        }
        Ok(Self(out))
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The bearer token peers send. Derived, so the key itself never goes
    /// over the wire after joining.
    pub fn token(&self) -> Token {
        let mut okm = [0u8; 32];
        Hkdf::<Sha256>::new(Some(SALT), &self.0)
            .expand(INFO, &mut okm)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        Token(okm.iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// Bearer token for `Authorization: Bearer …`.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

impl Token {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Constant-time check of an `Authorization` header value.
    pub fn matches_header(&self, header: &str) -> bool {
        let presented = header.strip_prefix("Bearer ").unwrap_or("");
        presented.len() == self.0.len() && bool::from(presented.as_bytes().ct_eq(self.0.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_stable_and_key_specific() {
        let a = SyncKey::from_bytes([7; 32]);
        let b = SyncKey::from_bytes([8; 32]);
        assert_eq!(a.token(), a.token());
        assert_ne!(a.token(), b.token());
        assert_eq!(a.token().as_str().len(), 64);

        let header = format!("Bearer {}", a.token().as_str());
        assert!(a.token().matches_header(&header));
        assert!(!b.token().matches_header(&header));
        assert!(!a.token().matches_header(a.token().as_str()));
        assert!(!a.token().matches_header("Bearer "));
    }

    #[test]
    fn hex_round_trip() {
        let hex = "00ff".repeat(16);
        let key = SyncKey::from_hex(&hex).unwrap();
        assert_eq!(key.as_bytes()[1], 0xff);
        assert!(SyncKey::from_hex("abc").is_err());
        assert!(SyncKey::from_hex(&"zz".repeat(32)).is_err());
    }
}
