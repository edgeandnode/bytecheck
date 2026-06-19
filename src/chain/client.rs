//! alloy provider wrapper exposing only the three calls the tool needs:
//! `eth_getCode`, `eth_getStorageAt`, and `eth_call` (for beacon resolution).

use crate::Error;
use alloy::eips::BlockId;
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{DynProvider, Provider, ProviderBuilder};
use alloy::rpc::types::{TransactionInput, TransactionRequest};

pub struct ChainClient {
    provider: DynProvider,
}

impl ChainClient {
    /// Connect to a JSON-RPC endpoint (transport auto-detected from the URL).
    pub async fn connect(rpc: &str) -> Result<Self, Error> {
        let provider = ProviderBuilder::new()
            .connect(rpc)
            .await
            .map_err(|e| Error::Chain(format!("connect {rpc}: {e}")))?
            .erased();
        Ok(Self { provider })
    }

    /// `eth_getCode` at a pinned block.
    pub async fn get_code(&self, address: Address, block: BlockId) -> Result<Bytes, Error> {
        self.provider
            .get_code_at(address)
            .block_id(block)
            .await
            .map_err(|e| Error::Chain(format!("eth_getCode {address}: {e}")))
    }

    /// `eth_getStorageAt` at a pinned block, returned as a 256-bit word.
    pub async fn get_storage(
        &self,
        address: Address,
        slot: U256,
        block: BlockId,
    ) -> Result<U256, Error> {
        self.provider
            .get_storage_at(address, slot)
            .block_id(block)
            .await
            .map_err(|e| Error::Chain(format!("eth_getStorageAt {address}: {e}")))
    }

    /// `eth_call` with raw calldata at a pinned block.
    pub async fn call(&self, to: Address, data: &[u8], block: BlockId) -> Result<Bytes, Error> {
        let tx = TransactionRequest::default()
            .to(to)
            .input(TransactionInput::new(Bytes::copy_from_slice(data)));
        self.provider
            .call(tx)
            .block(block)
            .await
            .map_err(|e| Error::Chain(format!("eth_call {to}: {e}")))
    }
}

/// Parse a `--block` value (`latest`, a named tag, decimal, or `0x` hex).
pub fn parse_block(s: &str) -> Result<BlockId, Error> {
    match s {
        "latest" => Ok(BlockId::latest()),
        "earliest" => Ok(BlockId::earliest()),
        "pending" => Ok(BlockId::pending()),
        "safe" => Ok(BlockId::safe()),
        "finalized" => Ok(BlockId::finalized()),
        _ => {
            let n = if let Some(hex) = s.strip_prefix("0x") {
                u64::from_str_radix(hex, 16)
            } else {
                s.parse::<u64>()
            };
            n.map(BlockId::number)
                .map_err(|_| Error::Usage(format!("invalid --block: {s}")))
        }
    }
}
