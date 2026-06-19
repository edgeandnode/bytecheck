//! Hardhat artifact parsing. Runtime code is a hex string at `deployedBytecode`,
//! library placeholders at `deployedLinkReferences`. Hardhat build artifacts do
//! NOT carry `immutableReferences` (see §6) — those offsets must come from
//! build-info or the address-shaped heuristic at compare time.

use super::{decode_hex, parse_link_refs, Artifact, ArtifactFormat};
use crate::Error;
use serde_json::Value;

pub fn parse(json: &Value, stem: &str) -> Result<Artifact, Error> {
    let object = json
        .get("deployedBytecode")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Artifact("hardhat: missing deployedBytecode string".into()))?;
    let deployed_bytecode = decode_hex(object)?;

    let contract = json
        .get("contractName")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| stem.to_string());

    // `sourceName` (e.g. `contracts/Foo.sol`) is the key needed to look the
    // contract up inside build-info output.
    let source_name = json
        .get("sourceName")
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let link_refs = parse_link_refs(json.get("deployedLinkReferences"));

    Ok(Artifact {
        contract,
        format: ArtifactFormat::Hardhat,
        source_name,
        deployed_bytecode,
        immutable_refs: Vec::new(),
        link_refs,
    })
}
