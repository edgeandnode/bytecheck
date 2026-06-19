//! Human-readable sectioned report: inputs → derived → outcome.

use super::{Config, OutcomeReport, Report};
use crate::chain::proxy::ProxyKind;
use crate::compare::Outcome;
use crate::metadata::Metadata;
use crate::normalize::RegionKind;
use crate::Error;
use owo_colors::OwoColorize;

pub fn render(r: &Report) -> Result<(), Error> {
    println!("{} {}", "bytecheck ·".dimmed(), r.config.contract.bold());

    let c = &r.config;
    section("Config");
    row("artifact", &c.artifact);
    row("address", &c.address.to_string());
    row("proxy", &proxy_line(c));
    row("format", &format!("{:?}", c.format).to_lowercase());
    row("block", &c.block);
    row("rpc", &c.rpc);
    row("mode", &format!("{:?}", c.mode).to_lowercase());
    row("immutables", c.immutables.label());
    row(
        "compiler",
        &compiler_line(&c.metadata_local, &c.metadata_chain),
    );
    row(
        "source hash",
        &hash_line(&c.metadata_local, &c.metadata_chain),
    );

    section("Outcome");
    row("result", &result_line(&r.outcome));
    row("bytecode", &bytecode_line(&r.outcome));
    row("metadata", &metadata_line(&r.outcome));
    row("accounted", &accounted_line(&r.outcome));
    row("unexplained", &unexplained_line(&r.outcome));
    row("exit", &r.outcome.exit_code.to_string());

    // Detail lines.
    for lr in &r.outcome.accounted_diffs {
        let label = lr.label.clone().unwrap_or_else(|| {
            if lr.found_in_book {
                "(unnamed)".into()
            } else {
                "(not in address book)".into()
            }
        });
        println!(
            "      · {:?} @{}..+{}  chain=0x{}  {}",
            lr.region.kind,
            lr.region.offset,
            lr.region.length,
            alloy::hex::encode(&lr.region.chain_value),
            label.dimmed()
        );
    }
    for d in &r.outcome.unexplained_diffs {
        println!(
            "      {} unexplained @{}..+{}",
            "!".red(),
            d.offset,
            d.length
        );
    }
    if let Some(note) = &r.outcome.note {
        println!("\n{} {note}", "note:".yellow());
    }
    Ok(())
}

fn section(title: &str) {
    println!("\n  {}", title.bold().underline());
}

fn row(key: &str, value: &str) {
    println!("    {:<13}{}", key.dimmed(), value);
}

fn proxy_line(c: &Config) -> String {
    match c.proxy_kind {
        ProxyKind::None => "none (direct)".to_string(),
        kind => format!(
            "{} → {}",
            format!("{kind:?}").to_lowercase(),
            c.impl_address
        ),
    }
}

fn compiler_line(local: &Metadata, chain: &Metadata) -> String {
    let l = local.solc.clone().unwrap_or_else(|| "unknown".into());
    if local.solc == chain.solc {
        format!("solc {l}")
    } else {
        let c = chain.solc.clone().unwrap_or_else(|| "unknown".into());
        format!("solc {} (local) {} solc {} (chain)", l, "≠".red(), c)
    }
}

fn hash_line(local: &Metadata, chain: &Metadata) -> String {
    let same = local.hash == chain.hash && local.hash_kind == chain.hash_kind;
    if same {
        match (&local.hash_kind, &local.hash) {
            (Some(kind), Some(hash)) => format!("{kind} {}", short_hex(hash)),
            _ => "none".to_string(),
        }
    } else {
        format!(
            "{} (local) {} {} (chain)",
            fmt_hash(local),
            "≠".red(),
            fmt_hash(chain)
        )
    }
}

fn fmt_hash(m: &Metadata) -> String {
    match (&m.hash_kind, &m.hash) {
        (Some(kind), Some(hash)) => format!("{kind} {}", short_hex(hash)),
        _ => "none".to_string(),
    }
}

fn result_line(o: &OutcomeReport) -> String {
    match o.result {
        Outcome::Match => "MATCH".green().bold().to_string(),
        Outcome::MatchWithMetadataDiff => "MATCH (metadata differs)".yellow().bold().to_string(),
        Outcome::Mismatch => "MISMATCH".red().bold().to_string(),
        Outcome::Error => "ERROR".red().bold().to_string(),
    }
}

fn bytecode_line(o: &OutcomeReport) -> String {
    if o.length_match {
        format!("equal · {} bytes", o.local_len)
    } else {
        format!("differ · {} local / {} chain", o.local_len, o.chain_len)
    }
}

fn metadata_line(o: &OutcomeReport) -> String {
    if o.metadata_match {
        "equal".to_string()
    } else if o.metadata_diff.is_empty() {
        "differs".to_string()
    } else {
        format!("differs · {}", o.metadata_diff.join(", "))
    }
}

fn accounted_line(o: &OutcomeReport) -> String {
    let imm = o
        .accounted_diffs
        .iter()
        .filter(|r| r.region.kind == RegionKind::Immutable)
        .count();
    let lib = o
        .accounted_diffs
        .iter()
        .filter(|r| r.region.kind == RegionKind::LibraryRef)
        .count();
    if o.accounted_diffs.is_empty() {
        "none".to_string()
    } else {
        format!(
            "{} region(s) ({imm} immutable, {lib} library)",
            o.accounted_diffs.len()
        )
    }
}

fn unexplained_line(o: &OutcomeReport) -> String {
    if o.unexplained_diffs.is_empty() {
        "none".to_string()
    } else {
        let bytes: usize = o.unexplained_diffs.iter().map(|d| d.length).sum();
        format!("{} range(s), {bytes} bytes", o.unexplained_diffs.len())
            .red()
            .to_string()
    }
}

/// Shorten a long hex hash for display: `1220ab…cd34`.
fn short_hex(hex: &str) -> String {
    if hex.len() <= 14 {
        hex.to_string()
    } else {
        format!("{}…{}", &hex[..10], &hex[hex.len() - 4..])
    }
}
