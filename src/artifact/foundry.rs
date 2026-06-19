//! Foundry artifact parsing. Runtime code lives at `deployedBytecode.object`,
//! with `immutableReferences` and `linkReferences` siblings — so immutables are
//! available inline, no build-info needed.

use super::{decode_hex, parse_immutable_refs, parse_link_refs, Artifact, ArtifactFormat};
use crate::Error;
use serde_json::Value;

pub fn parse(json: &Value, stem: &str) -> Result<Artifact, Error> {
    let deployed = &json["deployedBytecode"];
    let object = deployed
        .get("object")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Artifact("foundry: missing deployedBytecode.object".into()))?;
    let deployed_bytecode = decode_hex(object)?;

    let contract = json
        .get("contractName")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| stem.to_string());

    let immutable_refs = parse_immutable_refs(deployed.get("immutableReferences"));
    let link_refs = parse_link_refs(deployed.get("linkReferences"));

    Ok(Artifact {
        contract,
        format: ArtifactFormat::Foundry,
        source_name: None,
        deployed_bytecode,
        immutable_refs,
        link_refs,
    })
}
