//! The presentation model and renderers.
//!
//! A [`Report`] is organized into two sections that tell the verification
//! story: **config** (what we ran against and what we resolved/extracted along
//! the way — target, proxy, format, compiler provenance) and **outcome** (the
//! verdict and stats). The comparison engine produces the raw verdict;
//! `commands::verify` assembles it into a `Report`.

pub mod json;
pub mod sarif;
pub mod text;

use crate::artifact::{ArtifactFormat, ImmutableSource};
use crate::chain::proxy::ProxyKind;
use crate::cli::{Format, Mode};
use crate::compare::{DiffRange, Outcome};
use crate::label::LabeledRegion;
use crate::metadata::Metadata;
use crate::Error;
use alloy::primitives::Address;
use serde::Serialize;

/// A full verification report, ready to render in any format.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub config: Config,
    pub outcome: OutcomeReport,
}

/// What the tool ran against, and what it resolved/extracted along the way.
#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub contract: String,
    /// Artifact path or `name:<Contract>`.
    pub artifact: String,
    pub format: ArtifactFormat,
    /// The address queried on-chain.
    pub address: Address,
    /// Resolved implementation (equals `address` when not a proxy).
    pub impl_address: Address,
    pub proxy_kind: ProxyKind,
    /// RPC endpoint host (credentials/path stripped).
    pub rpc: String,
    pub block: String,
    pub mode: Mode,
    pub immutables: ImmutableSource,
}

/// The verdict and supporting statistics.
#[derive(Debug, Clone, Serialize)]
pub struct OutcomeReport {
    pub result: Outcome,
    pub length_match: bool,
    pub local_len: usize,
    pub chain_len: usize,
    /// Compiler provenance decoded from the local artifact's metadata.
    pub metadata_local: Metadata,
    /// Compiler provenance decoded from the on-chain code's metadata.
    pub metadata_chain: Metadata,
    pub metadata_match: bool,
    /// Which metadata fields diverged (empty when matching).
    pub metadata_diff: Vec<String>,
    pub accounted_diffs: Vec<LabeledRegion>,
    pub unexplained_diffs: Vec<DiffRange>,
    pub suspicious: bool,
    pub note: Option<String>,
    /// The exit code this report will produce under the active `--fail-on`.
    pub exit_code: i32,
}

/// Render `report` in the requested format to stdout.
pub fn render(report: &Report, format: Format) -> Result<(), Error> {
    match format {
        Format::Text => text::render(report),
        Format::Json => {
            println!("{}", json::render(report)?);
            Ok(())
        }
        Format::Sarif => {
            println!("{}", sarif::render(report)?);
            Ok(())
        }
    }
}
