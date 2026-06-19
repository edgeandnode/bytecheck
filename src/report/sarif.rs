//! Minimal SARIF 2.1.0 output for CI code-scanning (§4, §10). Pairs with
//! `--fail-on never` to report without failing the job.

use super::Report;
use crate::compare::Outcome;
use crate::Error;
use serde_json::{json, Value};

pub fn render(report: &Report) -> Result<String, Error> {
    let o = &report.outcome;
    let compiler = report
        .outcome
        .metadata_local
        .solc
        .clone()
        .unwrap_or_else(|| "unknown".into());

    let (level, text) = match o.result {
        Outcome::Match => ("note", "Bytecode matches.".to_string()),
        Outcome::MatchWithMetadataDiff => (
            "note",
            format!(
                "Bytecode matches; metadata differs ({}).",
                o.metadata_diff.join(", ")
            ),
        ),
        Outcome::Mismatch => ("error", "Bytecode mismatch.".to_string()),
        Outcome::Error => (
            "error",
            o.note
                .clone()
                .unwrap_or_else(|| "Comparison error.".to_string()),
        ),
    };

    let mut results: Vec<Value> = vec![json!({
        "ruleId": "bytecode-match",
        "level": level,
        "message": { "text": format!(
            "{text} contract={} impl={} compiler=solc {compiler}",
            report.config.contract, report.config.impl_address
        ) },
    })];

    for d in &o.unexplained_diffs {
        results.push(json!({
            "ruleId": "unexplained-diff",
            "level": "error",
            "message": { "text": format!(
                "Unexplained byte difference at offset {} (len {}).", d.offset, d.length
            ) },
        }));
    }

    if o.suspicious {
        results.push(json!({
            "ruleId": "suspicious-immutable",
            "level": "warning",
            "message": { "text": "An immutable injected an address not present in the address book." },
        }));
    }

    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "bytecheck",
                "informationUri": "https://github.com/thegraph/bytecheck",
                "rules": [
                    { "id": "bytecode-match" },
                    { "id": "unexplained-diff" },
                    { "id": "suspicious-immutable" }
                ]
            }},
            "results": results
        }]
    });

    serde_json::to_string_pretty(&doc).map_err(|e| Error::Operational(format!("sarif: {e}")))
}
