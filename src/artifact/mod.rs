//! Artifact format detection and a unified [`Artifact`] view over Hardhat and
//! Foundry build outputs.

pub mod build_info;
pub mod foundry;
pub mod hardhat;

use crate::Error;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Which build tool produced an artifact. Detected structurally, not by filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactFormat {
    Hardhat,
    Foundry,
}

/// Where a contract's immutable offsets were obtained — for reporting and to
/// decide whether the heuristic is allowed to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImmutableSource {
    /// Foundry inline `immutableReferences` (or a contract with none).
    ArtifactInline,
    /// Resolved exactly via the Hardhat `.dbg.json` → build-info chain.
    BuildInfoViaDbg,
    /// Resolved exactly via an explicit `--build-info` override.
    BuildInfoOverride,
    /// Inferred by the address/word-shaped heuristic (opt-in, unsound).
    Heuristic,
    /// Could not be resolved exactly (no dbg/build-info, or no source name).
    Unresolved,
}

impl ImmutableSource {
    /// Short human label for reports.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ArtifactInline => "exact (inline)",
            Self::BuildInfoViaDbg => "exact (build-info via .dbg.json)",
            Self::BuildInfoOverride => "exact (--build-info)",
            Self::Heuristic => "heuristic (inferred)",
            Self::Unresolved => "unresolved",
        }
    }
}

/// A masked-region reference recovered from an artifact: a byte range plus the
/// source identifier it came from (AST id for immutables, lib name for links).
#[derive(Debug, Clone)]
pub struct Reference {
    pub offset: usize,
    pub length: usize,
    pub identifier: String,
}

/// The normalized, tool-agnostic artifact the engine consumes.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub contract: String,
    pub format: ArtifactFormat,
    /// solc source unit path (e.g. `contracts/Foo.sol`), used to key into
    /// build-info. Present for Hardhat; `None` when unavailable.
    pub source_name: Option<String>,
    /// Runtime (deployed) bytecode, hex-decoded.
    pub deployed_bytecode: Vec<u8>,
    /// Immutable byte ranges. Inline for Foundry; resolved via build-info for
    /// Hardhat (see §6). Empty until resolved.
    pub immutable_refs: Vec<Reference>,
    /// Linked-library placeholder byte ranges.
    pub link_refs: Vec<Reference>,
}

/// Load and parse an artifact from an explicit path.
pub fn load_from_path(path: &Path) -> Result<Artifact, Error> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| Error::Artifact(format!("read {}: {e}", path.display())))?;
    let json: serde_json::Value = serde_json::from_str(&data)
        .map_err(|e| Error::Artifact(format!("parse {}: {e}", path.display())))?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    match detect(&json) {
        ArtifactFormat::Foundry => foundry::parse(&json, &stem),
        ArtifactFormat::Hardhat => hardhat::parse(&json, &stem),
    }
}

/// Resolve an artifact by contract name by searching the given roots for a
/// `<name>.json` file. Returns the parsed artifact and the path it was found at
/// (needed to locate a sibling `.dbg.json`).
pub fn resolve_by_name(name: &str, dirs: &[PathBuf]) -> Result<(Artifact, PathBuf), Error> {
    let target = format!("{name}.json");
    for dir in dirs {
        if let Some(found) = find_file(dir, &target) {
            let artifact = load_from_path(&found)?;
            return Ok((artifact, found));
        }
    }
    Err(Error::Artifact(format!(
        "no artifact named {target} found under {:?}",
        dirs
    )))
}

/// Default search roots used when `--artifacts-dir` is not supplied.
pub fn default_dirs() -> Vec<PathBuf> {
    ["build", "out", "artifacts"]
        .iter()
        .map(PathBuf::from)
        .collect()
}

/// Foundry artifacts store `deployedBytecode` as an object (`{ "object": ... }`);
/// Hardhat stores it as a hex string.
fn detect(json: &serde_json::Value) -> ArtifactFormat {
    if json
        .get("deployedBytecode")
        .and_then(|v| v.get("object"))
        .is_some()
    {
        ArtifactFormat::Foundry
    } else {
        ArtifactFormat::Hardhat
    }
}

/// Decode a `0x`-prefixed (or bare) hex string into bytes.
///
/// Unlinked artifacts embed library link placeholders (`__$<34 hex>$__`, and the
/// legacy `__Name__…` form) which are not valid hex, so we first replace each
/// 40-char (20-byte) placeholder with zeros. Those slots then read as blank
/// locally, exactly matching how a linked address looks once masked.
pub(crate) fn decode_hex(s: &str) -> Result<Vec<u8>, Error> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    let cleaned = zero_link_placeholders(trimmed);
    alloy::hex::decode(cleaned.as_ref())
        .map_err(|e| Error::Artifact(format!("invalid hex bytecode: {e}")))
}

/// Replace each library link placeholder with zeros. A placeholder always
/// occupies exactly 40 hex characters (20 bytes) and begins with `_` — a
/// character that never appears in real hex bytecode — so any `_` marks the
/// start of one. Borrows unchanged when there are no placeholders.
fn zero_link_placeholders(hex: &str) -> std::borrow::Cow<'_, str> {
    if !hex.contains('_') {
        return std::borrow::Cow::Borrowed(hex);
    }
    const PLACEHOLDER_LEN: usize = 40;
    let chars: Vec<char> = hex.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '_' {
            let end = (i + PLACEHOLDER_LEN).min(chars.len());
            out.extend(std::iter::repeat_n('0', end - i));
            i = end;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    std::borrow::Cow::Owned(out)
}

/// Parse a solc-style `linkReferences` map:
/// `{ "<file>": { "<Lib>": [{ "start", "length" }] } }`.
pub(crate) fn parse_link_refs(node: Option<&serde_json::Value>) -> Vec<Reference> {
    let mut out = Vec::new();
    let Some(files) = node.and_then(|v| v.as_object()) else {
        return out;
    };
    for (_file, libs) in files {
        let Some(libs) = libs.as_object() else {
            continue;
        };
        for (lib, ranges) in libs {
            for r in ranges.as_array().into_iter().flatten() {
                if let (Some(start), Some(length)) = (
                    r.get("start").and_then(|v| v.as_u64()),
                    r.get("length").and_then(|v| v.as_u64()),
                ) {
                    out.push(Reference {
                        offset: start as usize,
                        length: length as usize,
                        identifier: lib.clone(),
                    });
                }
            }
        }
    }
    out
}

/// Parse a solc `immutableReferences` map:
/// `{ "<astId>": [{ "start", "length" }] }`. Shared by Foundry artifacts and
/// build-info output.
pub(crate) fn parse_immutable_refs(node: Option<&serde_json::Value>) -> Vec<Reference> {
    let mut out = Vec::new();
    let Some(map) = node.and_then(|v| v.as_object()) else {
        return out;
    };
    for (ast_id, ranges) in map {
        for r in ranges.as_array().into_iter().flatten() {
            if let (Some(start), Some(length)) = (
                r.get("start").and_then(|v| v.as_u64()),
                r.get("length").and_then(|v| v.as_u64()),
            ) {
                out.push(Reference {
                    offset: start as usize,
                    length: length as usize,
                    identifier: format!("immutable#{ast_id}"),
                });
            }
        }
    }
    out
}

/// Recursively search `dir` for a file named `target`, returning the first hit.
fn find_file(dir: &Path, target: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path.file_name().and_then(|n| n.to_str()) == Some(target) {
            return Some(path);
        }
    }
    for sub in subdirs {
        if let Some(found) = find_file(&sub, target) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_zeros_modern_library_placeholder() {
        // 1 byte, a 20-byte `__$<34 hex>$__` placeholder, then 1 byte.
        let s = format!("aa__${}$__bb", "0".repeat(34));
        let bytes = decode_hex(&s).unwrap();
        assert_eq!(bytes.len(), 22);
        assert_eq!(bytes[0], 0xaa);
        assert!(bytes[1..21].iter().all(|&b| b == 0));
        assert_eq!(bytes[21], 0xbb);
    }

    #[test]
    fn decode_plain_hex_unchanged() {
        assert_eq!(decode_hex("0x1234ff").unwrap(), vec![0x12, 0x34, 0xff]);
    }
}
