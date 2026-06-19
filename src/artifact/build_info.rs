//! solc Standard-JSON "build-info" parsing — the canonical source of exact
//! immutable offsets.
//!
//! For Hardhat, a per-contract artifact omits `immutableReferences`, but its
//! sibling `<name>.dbg.json` points at the build-info file that contains them
//! for every contract in the compilation. We follow that pointer automatically;
//! `--build-info` is an explicit override that works for either toolchain.

use super::{parse_immutable_refs, Reference};
use crate::Error;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Load and parse a build-info JSON file.
pub fn load(path: &Path) -> Result<Value, Error> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| Error::Artifact(format!("read build-info {}: {e}", path.display())))?;
    serde_json::from_str(&data)
        .map_err(|e| Error::Artifact(format!("parse build-info {}: {e}", path.display())))
}

/// Pull exact `immutableReferences` for one contract out of build-info output.
///
/// Returns `Ok` with a (possibly empty) list when the contract is present — an
/// empty list means the compiler emitted no immutables, which is still *exact*,
/// not a failure. Errors only when the contract is absent from the build-info.
pub fn immutable_refs(
    build_info: &Value,
    source_name: &str,
    contract_name: &str,
) -> Result<Vec<Reference>, Error> {
    let contract = build_info
        .get("output")
        .and_then(|o| o.get("contracts"))
        .and_then(|c| c.get(source_name))
        .and_then(|s| s.get(contract_name))
        .ok_or_else(|| {
            Error::Artifact(format!(
                "build-info has no contract {contract_name} at {source_name}"
            ))
        })?;
    let node = contract
        .get("evm")
        .and_then(|e| e.get("deployedBytecode"))
        .and_then(|d| d.get("immutableReferences"));
    Ok(parse_immutable_refs(node))
}

/// Follow a Hardhat artifact's `<stem>.dbg.json` sibling to its build-info file.
///
/// Returns `None` when no dbg sibling exists (e.g. a lone, copied artifact). The
/// `buildInfo` field is a path relative to the dbg file's own directory.
pub fn resolve_via_dbg(artifact_path: &Path) -> Result<Option<PathBuf>, Error> {
    let dir = artifact_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = artifact_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::Artifact("artifact path has no file stem".into()))?;
    let dbg = dir.join(format!("{stem}.dbg.json"));
    if !dbg.exists() {
        return Ok(None);
    }

    let data = std::fs::read_to_string(&dbg)
        .map_err(|e| Error::Artifact(format!("read {}: {e}", dbg.display())))?;
    let json: Value = serde_json::from_str(&data)
        .map_err(|e| Error::Artifact(format!("parse {}: {e}", dbg.display())))?;

    if let Some(fmt) = json.get("_format").and_then(|v| v.as_str()) {
        if fmt != "hh-sol-dbg-1" {
            return Err(Error::Artifact(format!(
                "{}: unsupported dbg format {fmt}",
                dbg.display()
            )));
        }
    }

    let rel = json
        .get("buildInfo")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Artifact(format!("{}: missing buildInfo field", dbg.display())))?;

    Ok(Some(dir.join(rel)))
}
