//! Canonicalize runtime bytecode: split off the solc CBOR metadata trailer and
//! zero out masked regions (immutables, library links) so that two sides can be
//! compared byte-for-byte. Deterministic and network-free (§6).

use serde::Serialize;

/// What a masked region represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegionKind {
    Immutable,
    LibraryRef,
}

/// A masking instruction: zero `length` bytes at `offset`, tagging the result
/// with `kind` and an optional human identifier.
#[derive(Debug, Clone)]
pub struct MaskPlan {
    pub offset: usize,
    pub length: usize,
    pub kind: RegionKind,
    pub identifier: Option<String>,
}

/// The raw bytes that occupied a masked region on one particular side.
#[derive(Debug, Clone)]
pub struct RawRegion {
    pub offset: usize,
    pub length: usize,
    pub kind: RegionKind,
    pub identifier: Option<String>,
    pub bytes: Vec<u8>,
}

/// A canonical, comparable view of runtime bytecode plus the regions normalized
/// away.
#[derive(Debug, Clone)]
pub struct NormalizedCode {
    /// Runtime bytecode with masked regions zeroed and (optionally) metadata removed.
    pub canonical: Vec<u8>,
    /// The masked regions, carrying the original bytes from this side.
    pub masked: Vec<RawRegion>,
    /// The CBOR metadata trailer, split off when `strip_metadata` is set.
    pub metadata: Option<Vec<u8>>,
}

/// Normalize `code` against a masking `plan`. The solc CBOR metadata trailer is
/// always split off (so it can be reported in any mode); whether a metadata
/// difference affects the verdict is decided later by `compare`, per mode.
pub fn normalize(code: &[u8], plan: &[MaskPlan]) -> NormalizedCode {
    let mut canonical = code.to_vec();
    let metadata = split_metadata(&mut canonical);

    let mut masked = Vec::with_capacity(plan.len());
    for m in plan {
        let end = m.offset.saturating_add(m.length);
        if end > canonical.len() {
            // Region falls outside this side's code (e.g. past the metadata cut);
            // skip rather than panic — compare() will surface any real diff.
            continue;
        }
        let bytes = canonical[m.offset..end].to_vec();
        canonical[m.offset..end].fill(0);
        masked.push(RawRegion {
            offset: m.offset,
            length: m.length,
            kind: m.kind,
            identifier: m.identifier.clone(),
            bytes,
        });
    }

    NormalizedCode {
        canonical,
        masked,
        metadata,
    }
}

/// Validate a mask plan against the **local** artifact bytes before trusting it.
///
/// A region is only safe to mask if it is zero in the local artifact — that is
/// precisely what makes it a placeholder (a zeroed immutable, or a library link
/// slot) rather than real code. A non-zero region is *not* a placeholder, so
/// masking it could silently hide a genuine difference; such regions are
/// returned separately as `rejected` and left unmasked by the caller, surfacing
/// as unexplained diffs instead. Conservative by construction: it never widens
/// what counts as "explained".
pub fn validate_plan(local: &[u8], plan: &[MaskPlan]) -> (Vec<MaskPlan>, Vec<MaskPlan>) {
    let mut trusted = Vec::new();
    let mut rejected = Vec::new();
    for m in plan {
        let end = m.offset.saturating_add(m.length);
        let is_blank = end <= local.len() && local[m.offset..end].iter().all(|&b| b == 0);
        if is_blank {
            trusted.push(m.clone());
        } else {
            rejected.push(m.clone());
        }
    }
    (trusted, rejected)
}

/// Split the trailing solc metadata: the last two bytes are the big-endian
/// length of the preceding CBOR blob. Returns the trailer (CBOR + length) and
/// truncates `code` to the runtime prefix. `None` if the trailer is implausible.
pub fn split_metadata(code: &mut Vec<u8>) -> Option<Vec<u8>> {
    let n = code.len();
    if n < 2 {
        return None;
    }
    let cbor_len = ((code[n - 2] as usize) << 8) | (code[n - 1] as usize);
    let total = cbor_len.checked_add(2)?;
    if total > n || cbor_len == 0 {
        return None;
    }
    Some(code.split_off(n - total))
}

/// Heuristic immutable detection for Hardhat artifacts lacking
/// `immutableReferences` (§6, §11): contiguous runs that are zero locally and
/// non-zero on-chain, accepted only at address/word widths (20 or 32 bytes).
/// Reported transparently by the caller; never silently widens "explained".
pub fn infer_immutables(local: &[u8], chain: &[u8]) -> Vec<MaskPlan> {
    let mut out = Vec::new();
    if local.len() != chain.len() {
        return out;
    }
    let mut i = 0;
    while i < local.len() {
        if local[i] == 0 && chain[i] != 0 {
            let start = i;
            while i < local.len() && local[i] == 0 && chain[i] != 0 {
                i += 1;
            }
            let len = i - start;
            if len == 20 || len == 32 {
                out.push(MaskPlan {
                    offset: start,
                    length: len,
                    kind: RegionKind::Immutable,
                    identifier: Some("inferred".to_string()),
                });
            }
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_and_records_region() {
        let code = vec![0x11, 0x22, 0x33, 0x44];
        let plan = vec![MaskPlan {
            offset: 1,
            length: 2,
            kind: RegionKind::Immutable,
            identifier: None,
        }];
        let n = normalize(&code, &plan);
        assert_eq!(n.canonical, vec![0x11, 0x00, 0x00, 0x44]);
        assert_eq!(n.masked[0].bytes, vec![0x22, 0x33]);
        assert!(n.metadata.is_none());
    }

    #[test]
    fn splits_metadata_trailer() {
        // 3 runtime bytes, then a 2-byte CBOR blob, then its length (0x0002).
        let mut code = vec![0xaa, 0xbb, 0xcc, 0xde, 0xad, 0x00, 0x02];
        let meta = split_metadata(&mut code).unwrap();
        assert_eq!(code, vec![0xaa, 0xbb, 0xcc]);
        assert_eq!(meta, vec![0xde, 0xad, 0x00, 0x02]);
    }

    #[test]
    fn validate_plan_rejects_non_blank_region() {
        // Region [1,3) is zero locally → trusted; region [4,6) is non-zero → rejected.
        let local = vec![0xaa, 0x00, 0x00, 0xbb, 0x11, 0x22, 0xcc];
        let plan = vec![
            MaskPlan {
                offset: 1,
                length: 2,
                kind: RegionKind::Immutable,
                identifier: None,
            },
            MaskPlan {
                offset: 4,
                length: 2,
                kind: RegionKind::LibraryRef,
                identifier: None,
            },
        ];
        let (trusted, rejected) = validate_plan(&local, &plan);
        assert_eq!(trusted.len(), 1);
        assert_eq!(trusted[0].offset, 1);
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].offset, 4);
    }

    #[test]
    fn infers_address_width_immutable() {
        let local = vec![0u8; 24];
        let mut chain = vec![0u8; 24];
        for b in chain.iter_mut().take(22).skip(2) {
            *b = 0xff;
        }
        let inferred = infer_immutables(&local, &chain);
        assert_eq!(inferred.len(), 1);
        assert_eq!(inferred[0].offset, 2);
        assert_eq!(inferred[0].length, 20);
    }
}
