//! Proxy resolution: standard slot constants plus pluggable strategies.
//!
//! `auto` probes the EIP-1967 implementation slot, then the EIP-1967 beacon
//! slot (following through the beacon's `implementation()`), then a caller-
//! supplied custom slot, and finally treats the address as a direct
//! implementation if every probe is empty.

use crate::chain::client::ChainClient;
use crate::cli::ResolveProxy;
use crate::Error;
use alloy::eips::BlockId;
use alloy::primitives::{b256, Address, B256, U256};
use serde::Serialize;

/// `bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)`
pub const EIP1967_IMPL_SLOT: B256 =
    b256!("360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bdc");

/// `bytes32(uint256(keccak256("eip1967.proxy.beacon")) - 1)`
pub const EIP1967_BEACON_SLOT: B256 =
    b256!("a3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50");

/// `keccak256("PROXIABLE")` — the *legacy* EIP-1822 implementation slot.
///
/// Note: modern UUPS (OpenZeppelin `UUPSUpgradeable`) does NOT use this slot —
/// it stores the implementation in the EIP-1967 slot above, same as a
/// transparent proxy. This constant is only for original EIP-1822 proxies.
pub const EIP1822_PROXIABLE_SLOT: B256 =
    b256!("c5f16f0fcc639fa48a6947836d9850f504798523bf8c9a3a87d5876cf622bcf7");

/// `IBeacon.implementation()` selector.
const IMPLEMENTATION_SELECTOR: [u8; 4] = [0x5c, 0x60, 0xda, 0x1b];

/// Which storage mechanism the implementation address was reached through.
///
/// UUPS is intentionally absent: modern UUPS resolves through the `Eip1967`
/// slot, and legacy UUPS through `Eip1822` — it is not a distinct slot, so
/// reporting it as one would be misleading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProxyKind {
    None,
    Eip1967,
    Eip1822,
    Beacon,
    CustomSlot,
}

/// Resolve `address` to an implementation address per the requested strategy.
pub async fn resolve(
    client: &ChainClient,
    address: Address,
    strategy: ResolveProxy,
    custom_slot: Option<&str>,
    block: BlockId,
) -> Result<(Address, ProxyKind), Error> {
    match strategy {
        ResolveProxy::None => Ok((address, ProxyKind::None)),
        ResolveProxy::Eip1967 => require(
            read_slot(client, address, EIP1967_IMPL_SLOT, block).await?,
            ProxyKind::Eip1967,
        ),
        // Modern UUPS stores the implementation in the EIP-1967 slot — only the
        // upgrade logic lives in the implementation, not a separate slot.
        ResolveProxy::Uups => require(
            read_slot(client, address, EIP1967_IMPL_SLOT, block).await?,
            ProxyKind::Eip1967,
        ),
        ResolveProxy::Eip1822 => require(
            read_slot(client, address, EIP1822_PROXIABLE_SLOT, block).await?,
            ProxyKind::Eip1822,
        ),
        ResolveProxy::Beacon => {
            let beacon = read_slot(client, address, EIP1967_BEACON_SLOT, block)
                .await?
                .ok_or_else(|| Error::Chain("beacon slot is empty".into()))?;
            Ok((beacon_impl(client, beacon, block).await?, ProxyKind::Beacon))
        }
        ResolveProxy::Auto => resolve_auto(client, address, custom_slot, block).await,
    }
}

async fn resolve_auto(
    client: &ChainClient,
    address: Address,
    custom_slot: Option<&str>,
    block: BlockId,
) -> Result<(Address, ProxyKind), Error> {
    if let Some(impl_addr) = read_slot(client, address, EIP1967_IMPL_SLOT, block).await? {
        return Ok((impl_addr, ProxyKind::Eip1967));
    }
    if let Some(beacon) = read_slot(client, address, EIP1967_BEACON_SLOT, block).await? {
        return Ok((beacon_impl(client, beacon, block).await?, ProxyKind::Beacon));
    }
    // Legacy EIP-1822 UUPS (rare); modern UUPS was already caught by the 1967 probe.
    if let Some(impl_addr) = read_slot(client, address, EIP1822_PROXIABLE_SLOT, block).await? {
        return Ok((impl_addr, ProxyKind::Eip1822));
    }
    if let Some(slot) = custom_slot {
        let slot = parse_slot(slot)?;
        if let Some(impl_addr) = read_slot_u256(client, address, slot, block).await? {
            return Ok((impl_addr, ProxyKind::CustomSlot));
        }
    }
    // Nothing resolved — treat the queried address as the implementation itself.
    Ok((address, ProxyKind::None))
}

/// Read a storage slot and interpret its low 20 bytes as an address; `None` if
/// the slot is zero.
async fn read_slot(
    client: &ChainClient,
    address: Address,
    slot: B256,
    block: BlockId,
) -> Result<Option<Address>, Error> {
    read_slot_u256(client, address, slot.into(), block).await
}

async fn read_slot_u256(
    client: &ChainClient,
    address: Address,
    slot: U256,
    block: BlockId,
) -> Result<Option<Address>, Error> {
    let word: B256 = client.get_storage(address, slot, block).await?.into();
    let addr = Address::from_word(word);
    Ok((!addr.is_zero()).then_some(addr))
}

/// Follow a beacon's `implementation()` view function.
async fn beacon_impl(
    client: &ChainClient,
    beacon: Address,
    block: BlockId,
) -> Result<Address, Error> {
    let ret = client.call(beacon, &IMPLEMENTATION_SELECTOR, block).await?;
    if ret.len() < 32 {
        return Err(Error::Chain(format!(
            "beacon {beacon} implementation() returned {} bytes",
            ret.len()
        )));
    }
    Ok(Address::from_slice(&ret[12..32]))
}

fn require(resolved: Option<Address>, kind: ProxyKind) -> Result<(Address, ProxyKind), Error> {
    resolved
        .map(|a| (a, kind))
        .ok_or_else(|| Error::Chain(format!("{kind:?} slot is empty — not a proxy of this kind")))
}

/// Parse a custom slot expressed as `0x`-hex or decimal into a `U256` key.
fn parse_slot(s: &str) -> Result<U256, Error> {
    let parsed = if let Some(hex) = s.strip_prefix("0x") {
        U256::from_str_radix(hex, 16)
    } else {
        U256::from_str_radix(s, 10)
    };
    parsed.map_err(|e| Error::Usage(format!("invalid --proxy-slot {s}: {e}")))
}
