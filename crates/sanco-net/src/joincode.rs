//! Join codes: the workspace sync key plus the inviting PC's address, as
//! Crockford base32 in dash-separated groups so it survives being pasted
//! into chat. A 4-byte checksum catches typos.

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::token::SyncKey;

const VERSION: u8 = 1;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const GROUP: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinCode {
    pub key: SyncKey,
    /// `host:port` of the PC that made the code.
    pub addr: String,
}

impl JoinCode {
    pub fn encode(&self) -> String {
        let mut bytes = vec![VERSION];
        bytes.extend_from_slice(self.key.as_bytes());
        bytes.extend_from_slice(self.addr.as_bytes());
        let sum = Sha256::digest(&bytes);
        bytes.extend_from_slice(&sum[..4]);
        let text = base32(&bytes);
        text.as_bytes()
            .chunks(GROUP)
            .map(|c| std::str::from_utf8(c).expect("ascii"))
            .collect::<Vec<_>>()
            .join("-")
    }

    pub fn decode(code: &str) -> Result<Self> {
        let bytes = unbase32(code)?;
        if bytes.len() < 1 + 32 + 3 + 4 {
            return Err(Error::BadJoinCode("too short"));
        }
        let (body, sum) = bytes.split_at(bytes.len() - 4);
        if Sha256::digest(body)[..4] != *sum {
            return Err(Error::BadJoinCode("typo or incomplete code"));
        }
        if body[0] != VERSION {
            return Err(Error::BadJoinCode("made by a different app version"));
        }
        let key: [u8; 32] = body[1..33].try_into().expect("32 bytes");
        let addr = String::from_utf8(body[33..].to_vec())
            .map_err(|_| Error::BadJoinCode("bad address"))?;
        Ok(Self {
            key: SyncKey::from_bytes(key),
            addr,
        })
    }
}

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buf >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn unbase32(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in text.chars() {
        let v = match c.to_ascii_uppercase() {
            '-' | ' ' | '\t' | '\r' | '\n' => continue,
            'O' => 0,
            'I' | 'L' => 1,
            c => ALPHABET
                .iter()
                .position(|&a| a as char == c)
                .ok_or(Error::BadJoinCode("unexpected character"))? as u32,
        };
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code() -> JoinCode {
        JoinCode {
            key: SyncKey::from_bytes(std::array::from_fn(|i| i as u8 * 7)),
            addr: "100.101.102.103:7470".into(),
        }
    }

    #[test]
    fn round_trip() {
        let text = code().encode();
        assert!(text.split('-').all(|g| g.len() <= GROUP));
        assert_eq!(JoinCode::decode(&text).unwrap(), code());
        // Pasting is forgiving: case, spaces, line breaks, O/0 and I/1.
        let messy = format!(" {}\n", text.to_lowercase().replace('-', " "));
        assert_eq!(JoinCode::decode(&messy).unwrap(), code());
        let swapped = text.replace('0', "O").replace('1', "I");
        assert_eq!(JoinCode::decode(&swapped).unwrap(), code());
    }

    #[test]
    fn typos_are_caught() {
        let text = code().encode();
        let mut chars: Vec<char> = text.chars().collect();
        let i = chars.iter().position(|c| *c != '-').unwrap() + 10;
        chars[i] = if chars[i] == 'A' { 'B' } else { 'A' };
        let typo: String = chars.into_iter().collect();
        assert!(JoinCode::decode(&typo).is_err());
        assert!(JoinCode::decode(&text[..text.len() - 6]).is_err());
        assert!(JoinCode::decode("hello world!").is_err());
    }
}
