//! clap definitions for the CLI surface described in `ARCHITECTURE.md` §4.

use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "bytecheck",
    version,
    about = "Verify on-chain bytecode matches a compiled artifact",
    long_about = None
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Verify a single contract: resolve proxy → normalize → compare → report.
    Verify(VerifyArgs),
}

#[derive(Parser, Debug)]
pub struct VerifyArgs {
    /// Path to a Hardhat or Foundry artifact JSON.
    #[arg(long, conflicts_with = "name")]
    pub artifact: Option<PathBuf>,

    /// Resolve the artifact by contract name (searches `--artifacts-dir`).
    #[arg(long)]
    pub name: Option<String>,

    /// Search roots for `--name` (default: ./build, ./out, ./artifacts).
    #[arg(long = "artifacts-dir")]
    pub artifacts_dir: Vec<PathBuf>,

    /// On-chain address (proxy OR implementation).
    #[arg(long)]
    pub address: String,

    /// JSON-RPC endpoint.
    #[arg(long, env = "BYTECHECK_RPC")]
    pub rpc: String,

    /// Block height or tag (default: latest).
    #[arg(long, default_value = "latest")]
    pub block: String,

    /// Proxy resolution strategy. `uups` reads the EIP-1967 slot (modern UUPS);
    /// `eip1822` reads the legacy PROXIABLE slot.
    #[arg(long = "resolve-proxy", value_enum, default_value_t = ResolveProxy::Auto)]
    pub resolve_proxy: ResolveProxy,

    /// Read the implementation address from a custom storage slot (hex).
    #[arg(long = "proxy-slot")]
    pub proxy_slot: Option<String>,

    /// Skip resolution; use this implementation address directly.
    #[arg(long = "impl-address", conflicts_with_all = ["resolve_proxy", "proxy_slot"])]
    pub impl_address: Option<String>,

    /// Explicit solc build-info JSON for exact immutable offsets. Overrides the
    /// automatic `.dbg.json` discovery; works for either toolchain.
    #[arg(long = "build-info")]
    pub build_info: Option<PathBuf>,

    /// Allow heuristic immutable inference when exact offsets can't be resolved.
    /// Unsound (can mask real diffs); off by default.
    #[arg(long = "infer-immutables")]
    pub infer_immutables: bool,

    /// Comparison strictness.
    #[arg(long, value_enum, default_value_t = Mode::Standard)]
    pub mode: Mode,

    /// JSON address book to label masked immutable/library addresses.
    #[arg(long = "address-book")]
    pub address_book: Option<PathBuf>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Exit-code policy.
    #[arg(long = "fail-on", value_enum, default_value_t = FailOn::Mismatch)]
    pub fail_on: FailOn,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveProxy {
    Auto,
    Eip1967,
    /// Modern UUPS — resolves via the EIP-1967 implementation slot.
    Uups,
    /// Legacy EIP-1822 — resolves via the `keccak256("PROXIABLE")` slot.
    Eip1822,
    Beacon,
    None,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Strict,
    Standard,
    Loose,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Sarif,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailOn {
    Mismatch,
    Suspicious,
    Never,
}
