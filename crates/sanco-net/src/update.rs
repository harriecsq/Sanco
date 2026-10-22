//! App updates without a server: a PC that runs a newer version shares its
//! installer with peers over the sync port.
//!
//! The NSIS installer copies itself next to the app (`sanco-setup.exe`), so
//! every installed PC can hand its version on. A PC that sees a peer with a
//! newer `app_version` (from `/hello`, also when the schemas differ and sync
//! is refused) downloads `GET /update/installer`, checks size and SHA-256
//! against `GET /update`, and the shell runs it.

use std::cmp::Ordering;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// File name of the installer copy next to the app.
pub const INSTALLER_NAME: &str = "sanco-setup.exe";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    /// App version of the installer (the serving PC's version).
    pub version: String,
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

/// A newer version a peer can send.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateOffer {
    pub version: String,
    /// `host:port` of the peer.
    pub addr: String,
    pub name: Option<String>,
}

/// Compares dotted versions numerically ("0.10.0" > "0.9.3"). Anything after
/// a `-` or `+` is ignored; missing parts count as 0.
pub fn compare(a: &str, b: &str) -> Ordering {
    let parts = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let o = a.get(i).unwrap_or(&0).cmp(b.get(i).unwrap_or(&0));
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    compare(candidate, current) == Ordering::Greater
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads an installer and describes it as `version`.
pub fn describe(path: &Path, version: &str) -> Result<(UpdateInfo, Vec<u8>)> {
    let bytes = std::fs::read(path)?;
    let info = UpdateInfo {
        version: version.to_string(),
        size: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
    };
    Ok((info, bytes))
}

/// Checks downloaded bytes against what the peer announced.
pub fn verify(addr: &str, info: &UpdateInfo, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != info.size || sha256_hex(bytes) != info.sha256 {
        return Err(Error::Protocol {
            addr: addr.into(),
            reason: "the installer download is damaged (size or checksum differs)".into(),
        });
    }
    if !bytes.starts_with(b"MZ") {
        return Err(Error::Protocol {
            addr: addr.into(),
            reason: "the download is not a Windows program".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.0"));
        assert!(is_newer("1.0.1", "1.0"));
        assert!(!is_newer("1.0.0", "1.0"));
        assert!(!is_newer("1.0.0-beta", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert_eq!(compare("v2.1", "2.1.0"), Ordering::Equal);
    }

    #[test]
    fn verify_checks_size_hash_and_kind() {
        let exe = b"MZ fake installer".to_vec();
        let info = UpdateInfo {
            version: "1.0.0".into(),
            size: exe.len() as u64,
            sha256: sha256_hex(&exe),
        };
        verify("x", &info, &exe).unwrap();
        let mut bad = exe.clone();
        bad[5] = b'!';
        assert!(verify("x", &info, &bad).is_err());
        let text = b"not an exe at all".to_vec();
        let info = UpdateInfo {
            size: text.len() as u64,
            sha256: sha256_hex(&text),
            ..info
        };
        assert!(verify("x", &info, &text).is_err());
    }
}
