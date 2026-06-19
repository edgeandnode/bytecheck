//! Address-book loading and labeling of masked regions. An address book is a
//! flat `{ "name": "0xaddr" }` JSON map; we invert it to address → name.

use crate::cli::Mode;
use crate::compare::{Comparison, MaskedRegion};
use crate::normalize::RegionKind;
use crate::Error;
use alloy::primitives::Address;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

/// A masked region annotated against the address book.
#[derive(Debug, Clone, Serialize)]
pub struct LabeledRegion {
    pub region: MaskedRegion,
    /// Human name from the address book, if the injected value was found.
    pub label: Option<String>,
    /// `false` for an address-shaped value not present in the book → suspicious.
    pub found_in_book: bool,
}

impl LabeledRegion {
    /// Wrap a region prior to labeling.
    pub fn unlabeled(region: MaskedRegion) -> Self {
        LabeledRegion {
            region,
            label: None,
            found_in_book: false,
        }
    }
}

/// address → human name.
#[derive(Debug, Default)]
pub struct AddressBook {
    map: HashMap<Address, String>,
}

impl AddressBook {
    pub fn load(path: &Path) -> Result<Self, Error> {
        let data = std::fs::read_to_string(path)
            .map_err(|e| Error::Usage(format!("read address book {}: {e}", path.display())))?;
        let raw: HashMap<String, String> = serde_json::from_str(&data)
            .map_err(|e| Error::Usage(format!("parse address book {}: {e}", path.display())))?;
        let mut map = HashMap::with_capacity(raw.len());
        for (name, addr) in raw {
            let addr = Address::from_str(addr.trim())
                .map_err(|e| Error::Usage(format!("address book entry {name}={addr}: {e}")))?;
            map.insert(addr, name);
        }
        Ok(AddressBook { map })
    }

    pub fn lookup(&self, addr: &Address) -> Option<&String> {
        self.map.get(addr)
    }
}

/// Label each accounted region against the book and set the `suspicious` flag.
///
/// In `standard` mode an address-shaped immutable whose injected value is not in
/// the book raises `suspicious` (only when a book is actually supplied — §5).
pub fn apply(comparison: &mut Comparison, book: Option<&AddressBook>, mode: Mode) {
    let mut suspicious = false;
    for lr in &mut comparison.accounted_diffs {
        // Only 20-byte, address-shaped values are book-checkable.
        if lr.region.chain_value.len() != 20 {
            continue;
        }
        // No book → nothing to confirm against; not flagged.
        let Some(book) = book else { continue };
        let addr = Address::from_slice(&lr.region.chain_value);
        match book.lookup(&addr) {
            Some(name) => {
                lr.label = Some(name.clone());
                lr.found_in_book = true;
            }
            None => {
                lr.found_in_book = false;
                if mode == Mode::Standard && lr.region.kind == RegionKind::Immutable {
                    suspicious = true;
                }
            }
        }
    }
    comparison.suspicious = suspicious;
}
