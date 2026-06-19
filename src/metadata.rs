//! Decode the solc CBOR metadata trailer appended to runtime bytecode.
//!
//! The trailer is a CBOR map followed by a 2-byte big-endian length. A typical
//! map is `{ "ipfs": <34-byte multihash>, "solc": 0x000813 }`. We decode the
//! small, fixed subset solc emits (a definite-length map of text keys → byte
//! string / uint / bool) with a minimal reader rather than pulling in a CBOR
//! crate — it keeps the dependency surface small and is fully golden-testable.

use serde::Serialize;

/// Decoded provenance fields from a metadata trailer.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Metadata {
    /// Whether a trailer was present at all.
    pub present: bool,
    /// Compiler version, e.g. `0.8.19` (a string for nightly builds).
    pub solc: Option<String>,
    /// Source-hash algorithm: `ipfs`, `bzzr0`, or `bzzr1`.
    pub hash_kind: Option<String>,
    /// Source-metadata hash, hex-encoded (no `0x`).
    pub hash: Option<String>,
    /// solc `experimental` flag, when present.
    pub experimental: Option<bool>,
}

impl Metadata {
    /// Human-facing field names that differ between two trailers.
    pub fn diff_fields(&self, other: &Metadata) -> Vec<String> {
        let mut fields = Vec::new();
        if self.solc != other.solc {
            fields.push("compiler".to_string());
        }
        if self.hash != other.hash || self.hash_kind != other.hash_kind {
            fields.push("source hash".to_string());
        }
        if self.experimental != other.experimental {
            fields.push("experimental".to_string());
        }
        fields
    }
}

/// Decode a metadata trailer (CBOR map + trailing 2-byte length). Best-effort:
/// unrecognized or malformed input yields a `present` record with empty fields.
pub fn parse(trailer: &[u8]) -> Metadata {
    let mut md = Metadata {
        present: !trailer.is_empty(),
        ..Default::default()
    };
    if trailer.len() < 2 {
        return md;
    }
    let cbor = &trailer[..trailer.len() - 2];
    let mut c = Cursor { buf: cbor, pos: 0 };

    let Some((5, pairs)) = c.head() else {
        return md;
    };
    for _ in 0..pairs {
        let Some(key) = c.text() else { break };
        match key.as_str() {
            "solc" => md.solc = c.solc_version(),
            "ipfs" => {
                md.hash_kind = Some("ipfs".to_string());
                md.hash = c.byte_string_hex();
            }
            "bzzr0" => {
                md.hash_kind = Some("bzzr0".to_string());
                md.hash = c.byte_string_hex();
            }
            "bzzr1" => {
                md.hash_kind = Some("bzzr1".to_string());
                md.hash = c.byte_string_hex();
            }
            "experimental" => md.experimental = c.bool_value(),
            _ => c.skip_value(),
        }
    }
    md
}

/// A minimal CBOR reader covering only the item types solc metadata uses.
struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    /// Read a CBOR head, returning `(major type, argument)`.
    fn head(&mut self) -> Option<(u8, u64)> {
        let b = self.byte()?;
        let major = b >> 5;
        let info = b & 0x1f;
        let arg = match info {
            0..=23 => info as u64,
            24 => self.byte()? as u64,
            25 => {
                let mut v = 0u64;
                for _ in 0..2 {
                    v = (v << 8) | self.byte()? as u64;
                }
                v
            }
            26 => {
                let mut v = 0u64;
                for _ in 0..4 {
                    v = (v << 8) | self.byte()? as u64;
                }
                v
            }
            27 => {
                let mut v = 0u64;
                for _ in 0..8 {
                    v = (v << 8) | self.byte()? as u64;
                }
                v
            }
            _ => return None,
        };
        Some((major, arg))
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    /// Read a text string (major type 3).
    fn text(&mut self) -> Option<String> {
        let (3, n) = self.head()? else { return None };
        let bytes = self.take(n as usize)?;
        std::str::from_utf8(bytes).ok().map(str::to_string)
    }

    /// Read a byte string (major type 2) as hex.
    fn byte_string_hex(&mut self) -> Option<String> {
        let (2, n) = self.head()? else { return None };
        let bytes = self.take(n as usize)?;
        Some(alloy::hex::encode(bytes))
    }

    /// The `solc` value: a 3-byte string → `maj.min.patch`, or a text string.
    fn solc_version(&mut self) -> Option<String> {
        let (major, n) = self.head()?;
        let bytes = self.take(n as usize)?;
        match major {
            2 if bytes.len() == 3 => Some(format!("{}.{}.{}", bytes[0], bytes[1], bytes[2])),
            3 => std::str::from_utf8(bytes).ok().map(str::to_string),
            _ => None,
        }
    }

    /// Read a boolean (major type 7, simple values 20/21).
    fn bool_value(&mut self) -> Option<bool> {
        let (7, v) = self.head()? else { return None };
        match v {
            20 => Some(false),
            21 => Some(true),
            _ => None,
        }
    }

    /// Best-effort skip of one item (only the length-prefixed kinds need it).
    fn skip_value(&mut self) {
        if let Some((major @ (2 | 3), n)) = self.head() {
            let _ = major;
            let _ = self.take(n as usize);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_solc_version() {
        // { "solc": h'000813' } then its 2-byte length (0x000a).
        let cbor = [0xa1, 0x64, 0x73, 0x6f, 0x6c, 0x63, 0x43, 0x00, 0x08, 0x13];
        let mut trailer = cbor.to_vec();
        trailer.extend_from_slice(&[0x00, 0x0a]);
        let md = parse(&trailer);
        assert!(md.present);
        assert_eq!(md.solc.as_deref(), Some("0.8.19"));
    }

    #[test]
    fn diff_fields_reports_compiler() {
        let a = Metadata {
            solc: Some("0.8.19".into()),
            ..Default::default()
        };
        let b = Metadata {
            solc: Some("0.8.20".into()),
            ..Default::default()
        };
        assert_eq!(a.diff_fields(&b), vec!["compiler".to_string()]);
    }

    #[test]
    fn empty_trailer_is_absent() {
        assert!(!parse(&[]).present);
    }
}
