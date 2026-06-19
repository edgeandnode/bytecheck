# bytecheck

A Rust CLI that verifies the EVM runtime bytecode deployed on-chain matches the
bytecode compiled from a given commit of *any* smart-contract repository —
correctly accounting for **proxies**, **immutables**, **linked libraries**, and
**compiler metadata**.

See [`ARCHITECTURE.md`](./ARCHITECTURE.md) for the full design.

## Status

Pre-release. Implemented:

- Artifact loading — Foundry (inline `immutableReferences`) and Hardhat (exact
  immutables via the `.dbg.json` → build-info chain, with an explicit
  `--build-info` override).
- Proxy resolution — EIP-1967, UUPS (EIP-1967 slot), legacy EIP-1822, OZ beacon,
  and custom storage slots.
- Immutable & library masking, with a safety check that refuses to mask any
  region that isn't blank in the local artifact.
- solc CBOR metadata decoding (compiler version + source hash), reported on every
  outcome.
- `text` / `json` / `sarif` reporting and a CI exit-code contract.

Not yet validated end-to-end against a live chain; integration/golden fixtures
are still to come.

## Build

```sh
cargo build --release
```

## Usage

```sh
bytecheck verify \
  --artifact ./out/MyContract.sol/MyContract.json \
  --address 0xYourProxyOrImpl \
  --rpc https://eth.example/rpc \
  --mode standard
```

Resolve by contract name instead of an explicit path:

```sh
bytecheck verify --name MyContract --artifacts-dir ./out \
  --address 0x… --rpc "$BYTECHECK_RPC"
```

| Flag | Purpose |
|------|---------|
| `--artifact <PATH>` / `--name <NAME>` | Select the artifact by path, or by contract name under `--artifacts-dir`. |
| `--address <ADDR>` | On-chain address (proxy or implementation). |
| `--rpc <URL>` | JSON-RPC endpoint (or `BYTECHECK_RPC`). |
| `--block <N\|tag>` | Pin to a block height/tag (default `latest`). |
| `--resolve-proxy <auto\|eip1967\|uups\|eip1822\|beacon\|none>` | Proxy resolution strategy (default `auto`). |
| `--proxy-slot <SLOT>` | Read the implementation from a custom storage slot. |
| `--impl-address <ADDR>` | Skip resolution entirely. |
| `--build-info <PATH>` | solc build-info JSON for exact immutable offsets (overrides `.dbg.json` discovery). |
| `--infer-immutables` | Allow the unsound address-shaped heuristic when offsets can't be resolved (off by default). |
| `--mode <strict\|standard\|loose>` | Comparison strictness. |
| `--address-book <PATH>` | Label masked immutable/library addresses. |
| `--format <text\|json\|sarif>` | Output format. |
| `--fail-on <mismatch\|suspicious\|never>` | Exit-code policy. |

### Modes

- `strict` — fail on anything, including a metadata-only difference (forensic).
- `standard` — mask immutables/libraries; a metadata difference is reported but
  not fatal; an unknown injected address raises a `suspicious` flag.
- `loose` — also ignore metadata entirely; only unexplained code bytes fail.

## Output

The text report is organized into **Config** (what we ran against and resolved)
and **Outcome** (the verdict and stats):

```
bytecheck · MyToken

  Config
    artifact     ./out/MyToken.sol/MyToken.json
    address      0x1f9840a85d5aF5bf1D1762F925BDADdC4201F984
    proxy        eip1967 → 0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2
    format       foundry
    block        latest
    rpc          eth-mainnet.example
    mode         standard
    immutables   exact (inline)
    compiler     solc 0.8.19
    source hash  ipfs 1220a1b2c3…d7e8

  Outcome
    result       MATCH
    bytecode     equal · 12345 bytes
    metadata     equal
    accounted    1 region(s) (1 immutable, 0 library)
    unexplained  none
    exit         0
```

`--format json` emits the same structure as `{ "config": …, "outcome": … }`;
`--format sarif` emits SARIF 2.1.0 for CI code-scanning.

## Exit codes

| Code | Meaning |
|------|---------|
| `0`  | Satisfied the `--fail-on` policy. |
| `1`  | Mismatch (or suspicious, under `--fail-on suspicious`). |
| `2`  | Usage / configuration error. |
| `3`  | Operational error (RPC unreachable, no code at address). |

## License

MIT OR Apache-2.0.
