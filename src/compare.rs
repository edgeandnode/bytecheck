//! Compare two `NormalizedCode` sides into a `Comparison` (the verdict + stats),
//! applying the `Outcome` decision table from `ARCHITECTURE.md` §5. Pure and
//! deterministic; identity/inputs and rendering live outside this module.

use crate::cli::Mode;
use crate::label::LabeledRegion;
use crate::normalize::{NormalizedCode, RegionKind};
use alloy::primitives::Bytes;
use serde::Serialize;
use std::collections::BTreeMap;

/// A contiguous byte range that differs and is NOT explained by masking.
#[derive(Debug, Clone, Serialize)]
pub struct DiffRange {
    pub offset: usize,
    pub length: usize,
}

/// A masked region paired across both sides.
#[derive(Debug, Clone, Serialize)]
pub struct MaskedRegion {
    pub offset: usize,
    pub length: usize,
    pub kind: RegionKind,
    pub identifier: Option<String>,
    pub local_value: Bytes,
    pub chain_value: Bytes,
}

/// The verdict for a single contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Match,
    MatchWithMetadataDiff,
    Mismatch,
    Error,
}

/// The result of comparing two normalized sides.
#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub outcome: Outcome,
    pub length_match: bool,
    pub local_len: usize,
    pub chain_len: usize,
    pub metadata_match: bool,
    /// Masked regions that legitimately differ (immutables/libs), each labeled.
    pub accounted_diffs: Vec<LabeledRegion>,
    /// Byte ranges that differ and are NOT explained by masking → real concern.
    pub unexplained_diffs: Vec<DiffRange>,
    /// An immutable injected an address not in the supplied address book.
    pub suspicious: bool,
}

/// Compare a local artifact's normalized code against the on-chain side.
pub fn compare(local: &NormalizedCode, chain: &NormalizedCode, mode: Mode) -> Comparison {
    let length_match = local.canonical.len() == chain.canonical.len();

    // Pair masked regions by offset; both sides were masked with the same plan.
    let chain_by_offset: BTreeMap<usize, &crate::normalize::RawRegion> =
        chain.masked.iter().map(|r| (r.offset, r)).collect();
    let accounted_diffs: Vec<LabeledRegion> = local
        .masked
        .iter()
        .map(|lr| {
            let chain_value = chain_by_offset
                .get(&lr.offset)
                .map(|c| Bytes::copy_from_slice(&c.bytes))
                .unwrap_or_default();
            LabeledRegion::unlabeled(MaskedRegion {
                offset: lr.offset,
                length: lr.length,
                kind: lr.kind,
                identifier: lr.identifier.clone(),
                local_value: Bytes::copy_from_slice(&lr.bytes),
                chain_value,
            })
        })
        .collect();

    let unexplained_diffs = diff_ranges(&local.canonical, &chain.canonical);
    let metadata_match = local.metadata == chain.metadata;

    let outcome = if !length_match || !unexplained_diffs.is_empty() {
        Outcome::Mismatch
    } else {
        match mode {
            Mode::Loose => Outcome::Match,
            Mode::Standard if metadata_match => Outcome::Match,
            Mode::Standard => Outcome::MatchWithMetadataDiff,
            Mode::Strict if metadata_match => Outcome::Match,
            Mode::Strict => Outcome::Mismatch,
        }
    };

    Comparison {
        outcome,
        length_match,
        local_len: local.canonical.len(),
        chain_len: chain.canonical.len(),
        metadata_match,
        accounted_diffs,
        unexplained_diffs,
        suspicious: false,
    }
}

/// Contiguous ranges where the two byte slices differ. Over the common prefix
/// length; a trailing length mismatch is reported as one final range.
fn diff_ranges(a: &[u8], b: &[u8]) -> Vec<DiffRange> {
    let mut ranges = Vec::new();
    let common = a.len().min(b.len());
    let mut i = 0;
    while i < common {
        if a[i] != b[i] {
            let start = i;
            while i < common && a[i] != b[i] {
                i += 1;
            }
            ranges.push(DiffRange {
                offset: start,
                length: i - start,
            });
        } else {
            i += 1;
        }
    }
    if a.len() != b.len() {
        ranges.push(DiffRange {
            offset: common,
            length: a.len().abs_diff(b.len()),
        });
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::normalize;

    #[test]
    fn exact_match() {
        let local = normalize(&[1, 2, 3], &[]);
        let chain = normalize(&[1, 2, 3], &[]);
        let c = compare(&local, &chain, Mode::Standard);
        assert_eq!(c.outcome, Outcome::Match);
        assert!(c.unexplained_diffs.is_empty());
    }

    #[test]
    fn unexplained_byte_is_mismatch() {
        let local = normalize(&[1, 2, 3], &[]);
        let chain = normalize(&[1, 9, 3], &[]);
        let c = compare(&local, &chain, Mode::Standard);
        assert_eq!(c.outcome, Outcome::Mismatch);
        assert_eq!(c.unexplained_diffs.len(), 1);
    }
}
