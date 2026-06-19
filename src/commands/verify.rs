//! `verify` — the single-contract pipeline: load → resolve immutables →
//! resolve proxy → fetch → normalize → compare → label → assemble report →
//! render → exit code.

use crate::artifact::{self, build_info, Artifact, ArtifactFormat, ImmutableSource};
use crate::chain::client::{parse_block, ChainClient};
use crate::chain::proxy::{self, ProxyKind};
use crate::cli::{FailOn, VerifyArgs};
use crate::compare::{self, Comparison, Outcome};
use crate::label::{self, AddressBook};
use crate::metadata::{self, Metadata};
use crate::normalize::{self, MaskPlan, RegionKind};
use crate::report::{Config, OutcomeReport, Report};
use crate::{report, Error};
use alloy::primitives::Address;
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub async fn run(args: VerifyArgs) -> Result<i32, Error> {
    let (mut artifact, artifact_path) = load_artifact(&args)?;

    // Resolve exact immutable offsets (local-only): Foundry has them inline;
    // Hardhat resolves via `--build-info` or the `.dbg.json` chain.
    let mut imm_source =
        resolve_immutables(&mut artifact, &artifact_path, args.build_info.as_deref())?;

    let address = Address::from_str(&args.address)
        .map_err(|e| Error::Usage(format!("invalid --address {}: {e}", args.address)))?;
    let block = parse_block(&args.block)?;
    let client = ChainClient::connect(&args.rpc).await?;

    // Resolve the implementation address.
    let (impl_address, proxy_kind) = match &args.impl_address {
        Some(raw) => (
            Address::from_str(raw)
                .map_err(|e| Error::Usage(format!("invalid --impl-address {raw}: {e}")))?,
            ProxyKind::None,
        ),
        None => {
            proxy::resolve(
                &client,
                address,
                args.resolve_proxy,
                args.proxy_slot.as_deref(),
                block,
            )
            .await?
        }
    };

    // Fetch on-chain runtime code.
    let chain_code = client.get_code(impl_address, block).await?;
    if chain_code.is_empty() {
        let config = build_config(
            &args,
            &artifact,
            address,
            impl_address,
            proxy_kind,
            imm_source,
            Metadata::default(),
            Metadata::default(),
        );
        let report = Report {
            config,
            outcome: error_outcome(
                artifact.deployed_bytecode.len(),
                format!("no code at {impl_address} at the requested block"),
            ),
        };
        report::render(&report, args.format)?;
        return Ok(3);
    }

    // Build & validate the masking plan.
    let allow_heuristic = imm_source == ImmutableSource::Unresolved && args.infer_immutables;
    let (plan, heuristic_used) = build_plan(&artifact, &chain_code, allow_heuristic);
    if heuristic_used {
        imm_source = ImmutableSource::Heuristic;
    }
    let (plan, rejected) = normalize::validate_plan(&artifact.deployed_bytecode, &plan);

    // Normalize both sides and decode their metadata trailers.
    let local = normalize::normalize(&artifact.deployed_bytecode, &plan);
    let chain = normalize::normalize(&chain_code, &plan);
    let metadata_local = local
        .metadata
        .as_deref()
        .map(metadata::parse)
        .unwrap_or_default();
    let metadata_chain = chain
        .metadata
        .as_deref()
        .map(metadata::parse)
        .unwrap_or_default();

    // Compare and label.
    let mut comparison = compare::compare(&local, &chain, args.mode);
    let book = match &args.address_book {
        Some(path) => Some(AddressBook::load(path)?),
        None => None,
    };
    label::apply(&mut comparison, book.as_ref(), args.mode);

    // Assemble the report.
    let note = compose_note(imm_source, &rejected);
    let exit = exit_code(&comparison, args.fail_on);
    let config = build_config(
        &args,
        &artifact,
        address,
        impl_address,
        proxy_kind,
        imm_source,
        metadata_local.clone(),
        metadata_chain.clone(),
    );
    let report = Report {
        config,
        outcome: build_outcome(comparison, &metadata_local, &metadata_chain, note, exit),
    };

    report::render(&report, args.format)?;
    Ok(exit)
}

/// Assemble the config section from the run's inputs and resolved facts.
#[allow(clippy::too_many_arguments)]
fn build_config(
    args: &VerifyArgs,
    artifact: &Artifact,
    address: Address,
    impl_address: Address,
    proxy_kind: ProxyKind,
    immutables: ImmutableSource,
    metadata_local: Metadata,
    metadata_chain: Metadata,
) -> Config {
    Config {
        contract: artifact.contract.clone(),
        artifact: artifact_label(args),
        format: artifact.format,
        address,
        impl_address,
        proxy_kind,
        rpc: rpc_host(&args.rpc),
        block: args.block.clone(),
        mode: args.mode,
        immutables,
        metadata_local,
        metadata_chain,
    }
}

/// How the artifact was specified, for display: a path or `name:<Contract>`.
fn artifact_label(args: &VerifyArgs) -> String {
    match (&args.artifact, &args.name) {
        (Some(path), _) => path.display().to_string(),
        (None, Some(name)) => format!("name:{name}"),
        _ => "<unspecified>".to_string(),
    }
}

/// Assemble the outcome section, computing the human metadata field diff.
fn build_outcome(
    c: Comparison,
    metadata_local: &Metadata,
    metadata_chain: &Metadata,
    note: Option<String>,
    exit_code: i32,
) -> OutcomeReport {
    let metadata_diff = if c.metadata_match {
        Vec::new()
    } else {
        metadata_local.diff_fields(metadata_chain)
    };
    OutcomeReport {
        result: c.outcome,
        length_match: c.length_match,
        local_len: c.local_len,
        chain_len: c.chain_len,
        metadata_match: c.metadata_match,
        metadata_diff,
        accounted_diffs: c.accounted_diffs,
        unexplained_diffs: c.unexplained_diffs,
        suspicious: c.suspicious,
        note,
        exit_code,
    }
}

/// A terminal error outcome (no comparison was possible).
fn error_outcome(local_len: usize, note: String) -> OutcomeReport {
    OutcomeReport {
        result: Outcome::Error,
        length_match: false,
        local_len,
        chain_len: 0,
        metadata_match: false,
        metadata_diff: Vec::new(),
        accounted_diffs: Vec::new(),
        unexplained_diffs: Vec::new(),
        suspicious: false,
        note: Some(note),
        exit_code: 3,
    }
}

fn load_artifact(args: &VerifyArgs) -> Result<(Artifact, PathBuf), Error> {
    if let Some(path) = &args.artifact {
        Ok((artifact::load_from_path(path)?, path.clone()))
    } else if let Some(name) = &args.name {
        let dirs = if args.artifacts_dir.is_empty() {
            artifact::default_dirs()
        } else {
            args.artifacts_dir.clone()
        };
        artifact::resolve_by_name(name, &dirs)
    } else {
        Err(Error::Usage(
            "one of --artifact or --name is required".into(),
        ))
    }
}

/// Populate `artifact.immutable_refs` with exact offsets when possible, and
/// report where they came from (see §6).
fn resolve_immutables(
    artifact: &mut Artifact,
    artifact_path: &Path,
    build_info_override: Option<&Path>,
) -> Result<ImmutableSource, Error> {
    if artifact.format != ArtifactFormat::Hardhat || !artifact.immutable_refs.is_empty() {
        return Ok(ImmutableSource::ArtifactInline);
    }
    let Some(source_name) = artifact.source_name.clone() else {
        return Ok(ImmutableSource::Unresolved);
    };

    if let Some(path) = build_info_override {
        let bi = build_info::load(path)?;
        artifact.immutable_refs =
            build_info::immutable_refs(&bi, &source_name, &artifact.contract)?;
        return Ok(ImmutableSource::BuildInfoOverride);
    }

    if let Some(bi_path) = build_info::resolve_via_dbg(artifact_path)? {
        let bi = build_info::load(&bi_path)?;
        artifact.immutable_refs =
            build_info::immutable_refs(&bi, &source_name, &artifact.contract)?;
        return Ok(ImmutableSource::BuildInfoViaDbg);
    }

    Ok(ImmutableSource::Unresolved)
}

/// Build the mask plan from artifact references; append heuristic immutables
/// only when allowed. Returns whether the heuristic contributed any regions.
fn build_plan(
    artifact: &Artifact,
    chain_code: &[u8],
    allow_heuristic: bool,
) -> (Vec<MaskPlan>, bool) {
    let mut plan: Vec<MaskPlan> = Vec::new();
    for r in &artifact.immutable_refs {
        plan.push(MaskPlan {
            offset: r.offset,
            length: r.length,
            kind: RegionKind::Immutable,
            identifier: Some(r.identifier.clone()),
        });
    }
    for r in &artifact.link_refs {
        plan.push(MaskPlan {
            offset: r.offset,
            length: r.length,
            kind: RegionKind::LibraryRef,
            identifier: Some(r.identifier.clone()),
        });
    }

    let mut heuristic_used = false;
    if allow_heuristic && artifact.immutable_refs.is_empty() {
        let inferred = normalize::infer_immutables(&artifact.deployed_bytecode, chain_code);
        if !inferred.is_empty() {
            heuristic_used = true;
            plan.extend(inferred);
        }
    }
    (plan, heuristic_used)
}

/// Combine the immutable-resolution note with a warning about any masked regions
/// rejected for not being blank locally.
fn compose_note(source: ImmutableSource, rejected: &[MaskPlan]) -> Option<String> {
    let mut parts = Vec::new();
    match source {
        ImmutableSource::Heuristic => parts.push(
            "Immutable offsets were inferred heuristically (address/word-shaped) \
             because no .dbg.json or build-info was found. This can mask real \
             differences — pass --build-info for exact offsets."
                .to_string(),
        ),
        ImmutableSource::Unresolved => parts.push(
            "Could not resolve immutable offsets: no sibling .dbg.json or \
             build-info found. Any immutables will surface as unexplained diffs. \
             Pass --build-info, keep the artifacts/ tree intact, or use \
             --infer-immutables (unsound)."
                .to_string(),
        ),
        _ => {}
    }
    if !rejected.is_empty() {
        parts.push(format!(
            "{} declared masked region(s) were non-zero in the local artifact and \
             were left unmasked (masking them could hide a real difference); they \
             appear as unexplained diffs.",
            rejected.len()
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Map the outcome to an exit code under the `--fail-on` policy (§10).
fn exit_code(comparison: &Comparison, fail_on: FailOn) -> i32 {
    let mismatch = matches!(comparison.outcome, Outcome::Mismatch);
    match fail_on {
        FailOn::Never => 0,
        FailOn::Mismatch => i32::from(mismatch),
        FailOn::Suspicious => i32::from(mismatch || comparison.suspicious),
    }
}

/// Reduce an RPC URL to its host so credentials/paths never enter the report.
fn rpc_host(rpc: &str) -> String {
    let after_scheme = rpc.split("://").nth(1).unwrap_or(rpc);
    let authority = after_scheme.split('/').next().unwrap_or(after_scheme);
    // Drop any `user:pass@` credentials.
    authority
        .rsplit('@')
        .next()
        .unwrap_or(authority)
        .to_string()
}
