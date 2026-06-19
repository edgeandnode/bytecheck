# bytecheck — Architecture

> A Rust CLI that verifies the EVM runtime bytecode deployed on-chain matches
> the bytecode compiled from a given commit of *any* smart-contract repository —
> correctly accounting for **proxies**, **immutables**, **linked libraries**,
> and **compiler metadata**.

## 1. Problem statement

`bytecheck` answers one question with confidence, for any project on any
EVM chain:

> *Does the contract source at commit `X` in the repository match the contract
> deployed at address `Y` on chain `Z` (at block `B`)?*

The naive recipe — build the artifact (`forge build` / `hardhat compile`), pull
`deployedBytecode`, fetch `eth_getCode <addr>`, then `diff` — produces **false
negatives** in practice, because:

1. **Proxies.** Upgradeable contracts are deployed behind a proxy.
   `eth_getCode` on the proxy returns the delegatecall stub, not the logic
   contract. The implementation address must be resolved from a storage slot
   first.
2. **Immutables.** `immutable` variables are baked into runtime bytecode at
   construction. The locally compiled artifact has **zeroed placeholders** at
   those byte offsets; the on-chain code has the real values. A raw `diff`
   flags these and reports a mismatch on a contract that genuinely matches.
3. **Metadata hash.** solc appends a CBOR metadata trailer (compiler version +
   source hash). It can differ on functionally identical code.
4. **Linked libraries.** External library references are `__$...$__`
   placeholders locally and real addresses on-chain.

`bytecheck` encodes the *correct* comparison — resolve proxy → normalize both
sides (mask immutables / libraries, split off metadata) → compare → and
optionally **label** the masked regions against an address book so a human can
confirm the injected values are the addresses they expect.

### Why normalization matters — a worked example

Consider a typical upgradeable contract that holds a handful of `immutable`
collaborator addresses and sits behind an EIP-1967 proxy. Verifying it by hand:

1. `eth_getCode(proxy)` returns the proxy stub — useless for the comparison, so
   first read the EIP-1967 implementation slot to get the logic address.
2. `eth_getCode(impl)` and the local `deployedBytecode` are the **same length**
   but differ in several 20-byte regions.
3. Each differing region is **zero locally** and an **address on-chain** — i.e.
   the contract's immutables, written at deployment. Mask those exact byte
   ranges and the two are **byte-for-byte identical, including the metadata
   hash** — proving the deployed code was built from this source with these
   compiler settings.

`bytecheck` performs exactly these steps automatically and reports each masked
region (optionally labeled), so the "explained" differences are auditable rather
than hand-waved.

## 2. Goals / non-goals

### Goals
- Single statically-linked binary; **zero runtime dependencies** (no node, jq,
  foundry, or python required to run a check).
- Verify a **single contract** (`verify`).
- Auto-detect **Hardhat** and **Foundry** artifact formats.
- Resolve proxies: EIP-1967, EIP-1822/UUPS, OZ beacon, and arbitrary
  **custom storage slots** (`--proxy-slot`).
- Normalize **immutables** and **library links** before comparison; classify
  each masked region and optionally label it via an address book.
- Pinnable to a historical **block height**.
- Machine-readable output: **JSON** and **SARIF** (for CI gating), plus a
  sectioned human-readable report.

### Non-goals
- Recompiling source. `bytecheck` consumes **build artifacts**; producing them
  (running `forge`/`hardhat`) is the caller's responsibility (and is what pins
  the commit). The CLI may *shell out* to a builder as a convenience, but the
  comparison engine only ever sees artifacts + on-chain code.
- Source-level verification / decompilation. We compare bytecode, not ASTs.
- Submitting verifications to Etherscan/Sourcify (possible future export).

## 3. High-level design

```
                 ┌────────────────────────────────────────────────┐
                 │                   bytecheck                      │
                 │                                                  │
  CLI args ─────▶│  cli ──▶ commands::verify                        │
  (clap)         │            │                                     │
                 │            ▼                                     │
                 │   ┌──────────────┐   ┌──────────────┐           │
   artifact ────▶│   │ artifact     │   │ chain        │◀── RPC     │
   (.json)       │   │  loader      │   │  client      │   (alloy)  │
                 │   └──────┬───────┘   └──────┬───────┘           │
                 │          │                  │                   │
                 │          │   ┌──────────────▼─────────┐         │
                 │          │   │ proxy resolver          │         │
                 │          │   │ (slot → impl address)   │         │
                 │          │   └──────────────┬─────────┘         │
                 │          ▼                  ▼                   │
                 │       ┌──────────────────────────────┐         │
                 │       │ normalizer                     │         │
                 │       │  • mask immutables             │         │
                 │       │  • mask library links          │         │
                 │       │  • split/strip metadata        │         │
                 │       └──────────────┬─────────────────┘        │
                 │                      ▼                          │
                 │              ┌───────────────┐                  │
                 │              │ comparator     │                  │
                 │              │  → Comparison  │                  │
                 │              └──────┬─────────┘                  │
                 │                     ▼                            │
                 │   ┌─────────────────────────────────┐  optional │
   address ─────▶│   │ labeler (address book lookup)    │           │
   book          │   └──────────────┬──────────────────┘           │
                 │                  ▼                               │
                 │        report::{Text,Json,Sarif}                 │
                 └────────────────────────────────────────────────┘
                                     │
                                     ▼
                              stdout / file / exit code
```

The flow is a straight pipeline: **load both sides → resolve proxy →
normalize → compare → classify/label → render**. Every stage is pure and
independently testable except the two I/O edges (artifact loader, chain
client).

## 4. Commands (CLI surface)

```
bytecheck verify [OPTIONS]
  --artifact <PATH>        Path to a Hardhat or Foundry artifact JSON
  --name <NAME>            …or resolve artifact by contract name (+ --artifacts-dir)
  --artifacts-dir <DIR>    Search root for --name (default: ./build, ./out, ./artifacts)
  --address <ADDR>         On-chain address (proxy OR implementation)
  --rpc <URL>              JSON-RPC endpoint
  --block <N|tag>          Block height (default: latest)
  --resolve-proxy <auto|eip1967|uups|eip1822|beacon|none>   (default: auto)
                           (uups → EIP-1967 slot; eip1822 → legacy PROXIABLE slot)
  --proxy-slot <SLOT>      Read the implementation address from a custom slot
  --impl-address <ADDR>    Skip resolution; use this implementation directly
  --build-info <PATH>      solc build-info JSON for exact immutable offsets
                           (overrides .dbg.json discovery; works for either tool)
  --infer-immutables       Allow heuristic immutable inference when offsets can't
                           be resolved exactly (unsound; off by default)
  --mode <strict|standard|loose>              (default: standard)
  --address-book <PATH>    JSON to label masked immutable/library addresses
  --format <text|json|sarif>                  (default: text)
  --fail-on <mismatch|suspicious|never>       Exit-code policy (default: mismatch)
```

### `verify` — one contract
Resolve (if needed) → normalize → compare → assemble a `Report` → render.

## 5. Core data model

```rust
/// A canonical, comparable view of runtime bytecode plus the regions
/// that were normalized away.
struct NormalizedCode {
    /// Runtime bytecode with masked regions zeroed to a canonical form.
    canonical: Vec<u8>,
    /// Regions that were masked, with their original (raw) bytes.
    masked: Vec<MaskedRegion>,
    /// CBOR metadata trailer, split off (if present).
    metadata: Option<Vec<u8>>,
}

struct MaskedRegion {
    offset: usize,           // byte offset into runtime code
    length: usize,           // bytes (20 for an address-shaped immutable)
    kind: RegionKind,        // Immutable | LibraryRef
    local_value: Vec<u8>,    // typically zeroes for immutables locally
    chain_value: Vec<u8>,    // value observed on-chain
}

enum RegionKind { Immutable, LibraryRef }

/// Provenance decoded from the solc CBOR metadata trailer (always split off, so
/// it can be reported in every mode; whether a diff is *fatal* is a mode choice).
struct Metadata {
    present: bool,
    solc: Option<String>,        // "0.8.19"
    hash_kind: Option<String>,   // "ipfs" | "bzzr0" | "bzzr1"
    hash: Option<String>,        // source-metadata hash, hex
    experimental: Option<bool>,
}

/// The pure verdict produced by `compare` (no identity/inputs, no rendering).
struct Comparison {
    outcome: Outcome,            // Match | MatchWithMetadataDiff | Mismatch | Error
    length_match: bool,
    local_len: usize,
    chain_len: usize,
    metadata_match: bool,
    accounted_diffs: Vec<LabeledRegion>,  // masked & legitimately different
    unexplained_diffs: Vec<DiffRange>,    // differ, NOT explained → real concern
    suspicious: bool,            // masked immutable injected an unknown address
}

struct LabeledRegion {
    region: MaskedRegion,
    label: Option<String>,       // human name from the address book, if any
    found_in_book: bool,         // false → suspicious: unknown address baked in
}
```

The renderable **`Report`** wraps the verdict in two sections that tell the
verification story — assembled by `commands::verify`, not the engine:

```rust
struct Report { config: Config, outcome: OutcomeReport }

struct Config {     // what we ran against + what we resolved/extracted
    contract: String,
    artifact: String,            // path or "name:<Contract>"
    format: ArtifactFormat,
    address: Address,            // the address queried
    impl_address: Address,       // resolved implementation (== address if direct)
    proxy_kind: ProxyKind,       // None | Eip1967 | Eip1822 | Beacon | CustomSlot
                                 // (UUPS resolves through Eip1967, so it is not a
                                 //  distinct kind — see §6)
    rpc: String,                 // host only — credentials/path stripped
    block: String, mode: Mode,
    immutables: ImmutableSource, // inline | build-info-via-dbg | override | heuristic | unresolved
    metadata_local: Metadata, metadata_chain: Metadata,
}
struct OutcomeReport {  // the verdict + stats (flattens Comparison)
    result: Outcome, length_match: bool, local_len: usize, chain_len: usize,
    metadata_match: bool, metadata_diff: Vec<String>,   // which fields diverged
    accounted_diffs: Vec<LabeledRegion>, unexplained_diffs: Vec<DiffRange>,
    suspicious: bool, note: Option<String>, exit_code: i32,
}
```

JSON/SARIF serialize this structure directly; the text renderer prints the two
sections in order. The metadata trailer is decoded and shown (compiler + source
hash) on **every** outcome — including a match — because it is the provenance the
tool exists to surface; a metadata-only diff reports *which* field changed
(compiler vs source hash), not just that something did.

`Outcome` decision table:

| Condition                                                     | standard | strict | loose |
|---------------------------------------------------------------|----------|--------|-------|
| Canonical equal, metadata equal                               | Match    | Match  | Match |
| Canonical equal, metadata differs                             | MatchWithMetadataDiff | **Mismatch** | Match |
| Canonical equal but a masked immutable not in address book    | Match (⚠ suspicious) | Mismatch | Match |
| Any **unexplained** byte difference                           | Mismatch | Mismatch | Mismatch |
| Length differs                                                | Mismatch | Mismatch | Mismatch |

- **strict** — fail on *anything*, including metadata (forensic mode).
- **standard** — immutables/libraries masked; metadata diff is reported but not
  fatal; an unknown injected address raises a `suspicious` flag (only actionable
  when an `--address-book` is supplied).
- **loose** — also ignore metadata entirely; only unexplained code bytes fail.

## 6. Module breakdown

```
bytecheck/
├── Cargo.toml
├── ARCHITECTURE.md                ← this file
├── README.md
└── src/
    ├── main.rs                    # thin: parse args, dispatch, set exit code
    ├── cli.rs                     # clap definitions (Args/Subcommands)
    ├── artifact/
    │   ├── mod.rs                 # ArtifactFormat detection + unified Artifact
    │   ├── hardhat.rs             # `deployedBytecode` (string), sourceName, links
    │   ├── foundry.rs             # `deployedBytecode.object`, immutableReferences
    │   └── build_info.rs          # solc Standard-JSON: exact immutables, .dbg.json follow
    ├── chain/
    │   ├── client.rs              # alloy provider: get_code, get_storage_at
    │   └── proxy.rs               # slot constants + resolution strategies
    ├── normalize.rs               # mask immutables/libs, split metadata, validate plan
    ├── metadata.rs                # decode the solc CBOR trailer (minimal reader)
    ├── compare.rs                 # NormalizedCode × NormalizedCode → Comparison
    ├── label.rs                   # address-book load + lookup
    ├── report/
    │   ├── mod.rs                 # Report model: Config / OutcomeReport
    │   ├── text.rs                # sectioned, colorized human report
    │   ├── json.rs                # serde_json serialization of Report
    │   └── sarif.rs               # SARIF 2.1.0 for CI code-scanning
    └── commands/
        └── verify.rs
```

### Notable design points

- **Immutable offsets — the crux.** We always want the compiler's *exact*
  offset → length map; we only ever mask ranges we can name. Resolution order:
  1. **Foundry:** artifacts carry `immutableReferences` inline → use directly.
  2. **Hardhat (canonical path):** the per-contract artifact omits
     `immutableReferences`, but its sibling `<name>.dbg.json` carries a
     `buildInfo` pointer (relative to the dbg file) to the solc Standard-JSON
     **build-info**, which contains `immutableReferences` for *every* contract.
     We read `sourceName` + `contractName` from the artifact, follow the dbg
     pointer, and index `output.contracts[sourceName][contractName].evm.
     deployedBytecode.immutableReferences`. This is automatic and exact — no
     flags, given an intact `artifacts/` tree. (build-info is just solc output,
     so `--build-info <file>` is an explicit override that also works for
     Foundry.) An empty map means the contract genuinely has no immutables —
     still exact, not a failure.
  3. **Last resort (opt-in, `--infer-immutables`):** when neither the dbg
     sibling nor a build-info file is available (e.g. a lone, copied artifact),
     infer masked regions by diffing against on-chain code, accepting only
     **20/32-byte, zero-locally / non-zero-on-chain** runs. This is *unsound* —
     it can mask a real difference — so it is **off by default** and always
     disclosed in the report. Without it, unresolved immutables simply surface
     as unexplained diffs with a note pointing at `--build-info`.
- **Proxy resolution** lives behind a `ProxyResolver` trait so EIP-1967,
  EIP-1822, beacon, custom-slot, and a `none` passthrough are pluggable. `auto`
  probes the EIP-1967 implementation slot (`0x360894…382bdc`), then the beacon
  slot, then the legacy EIP-1822 `PROXIABLE` slot, then (if `--proxy-slot` is
  given) that slot, and finally treats the address as a direct implementation if
  all are empty.
- **UUPS is not a distinct slot.** Modern UUPS (OpenZeppelin `UUPSUpgradeable`)
  stores the implementation in the **EIP-1967** slot — only the upgrade *logic*
  lives in the implementation. So `--resolve-proxy uups` reads the EIP-1967 slot
  and is effectively a friendlier alias for `eip1967`; the original
  `keccak256("PROXIABLE")` slot is exposed separately as `--resolve-proxy
  eip1822` for the rare legacy proxies that still use it. Resolution is
  reported by the slot actually read (`Eip1967` / `Eip1822`), never as "UUPS",
  since the two are indistinguishable from storage alone.
- **Library masking** uses the artifact's `(deployed)linkReferences`
  offset/length map — no heuristics needed. Unlinked artifacts embed
  `__$<34 hex>$__` placeholders (not valid hex), so the loader replaces each
  40-char (20-byte) placeholder with zeros before decoding; those slots then
  read blank locally, matching how a linked address looks once masked.
- **Masking is validated before it's trusted.** A declared region is only masked
  if it is **blank (zero) in the local artifact** — that is what makes it a
  placeholder rather than real code. A non-zero region is left unmasked and
  surfaces as an unexplained diff (with a note), so a bad offset map can never
  silently hide a genuine difference. Conservative by construction: it never
  widens what counts as "explained".
- **Deterministic core.** No wall-clock or randomness in `normalize`/`compare`;
  timestamps are stamped by the reporter at the edge, keeping the engine
  golden-testable.

## 7. Technology stack

| Concern            | Crate                      | Why |
|--------------------|----------------------------|-----|
| EVM RPC + types    | `alloy` (provider, primitives) | Modern, maintained successor to ethers-rs; `eth_getCode`, `eth_getStorageAt`, `Address`/`Bytes`/`B256`. |
| Async runtime      | `tokio`                    | Required by the alloy provider for RPC calls. |
| CLI parsing        | `clap` (derive)            | Ergonomic subcommands, help, env fallbacks. |
| (De)serialization  | `serde`, `serde_json`      | Artifact + address-book parsing, JSON output. |
| Hex                | `alloy-primitives`/`hex`   | Bytecode encode/decode. |
| Errors             | `anyhow` (bin) + `thiserror` (lib) | Contextual errors at the edge, typed errors in the engine. |
| Color/table output | `comfy-table`, `owo-colors`| Human report. |
| Tests              | built-in + `insta`         | Golden snapshots for normalize/compare/report. |

The comparison engine (`artifact`, `normalize`, `compare`, `label`, `report`)
has **no network dependency** and is a candidate to expose as a `bytecheck-core`
library crate for embedding (e.g. a future MCP server or CI plugin).

## 8. Project layout

`bytecheck` is a self-contained Cargo crate with no dependency on any particular
host repository — it can live in its own repository, be installed via
`cargo install bytecheck`, or be vendored into a monorepo. When vendored into a
non-Cargo monorepo, keep it isolated from that ecosystem's workspace tooling
(e.g. exclude it from a JS `pnpm-workspace.yaml`, since it has no `package.json`)
and build it in a dedicated `cargo build --release` CI job.

## 9. Concurrency & performance

- `verify` is a single RPC round-trip pair (`eth_getCode`, plus 1–2
  `eth_getStorageAt` for proxy resolution). Sub-second.
- Comparison is CPU-cheap (byte-slice equality), so wall-clock is dominated by
  RPC latency, not the engine.

## 10. Exit codes (CI contract)

| Code | Meaning |
|------|---------|
| `0`  | The contract satisfied the `--fail-on` policy (e.g. matched). |
| `1`  | A **mismatch** (or suspicious, if `--fail-on suspicious`). |
| `2`  | Usage / configuration error (bad args, unreadable artifact). |
| `3`  | Operational error (RPC unreachable, address has no code). |

SARIF output pairs with `--fail-on never` to *report* without failing the job,
letting a code-scanning dashboard own the gating.

## 11. Edge cases & limitations

- **Hardhat immutables.** Resolved exactly via the `.dbg.json` → build-info
  chain whenever the `artifacts/` tree is intact (§6) — the normal case. A
  **lone, copied `Foo.json`** (no dbg sibling, no build-info) cannot be resolved
  exactly: by default its immutables surface as unexplained diffs with a note to
  pass `--build-info`; `--infer-immutables` opts into the unsound heuristic,
  which additionally cannot recognize *non-address* immutables (e.g. a `uint`).
- **Vyper / non-solc metadata.** Metadata splitting assumes solc's CBOR trailer;
  unknown trailers are left in place and only matter in `strict` mode.
- **`SELFDESTRUCT`ed or not-yet-deployed at `--block`.** `eth_getCode` returns
  empty → `Outcome::Error` with a clear message.
- **Proxies that store the implementation outside a standard/known slot.** Use
  `--proxy-slot` to point at the slot, or `--impl-address` to bypass resolution
  entirely.
- **Constructor-only code differences** are irrelevant: we compare *runtime*
  (`deployedBytecode` / `eth_getCode`), never creation bytecode.

## 12. Testing strategy

- **Golden tests (`insta`)** for `normalize` and `compare` over committed
  fixture pairs (local artifact + recorded on-chain hex), covering: an exact
  match, an immutable-only diff, a library-link diff, a metadata-only diff, and
  a genuine mismatch.
- **Proxy resolver unit tests** with mocked storage-slot responses for each
  proxy kind.
- **Format-detection tests** over real Hardhat and Foundry artifact samples.
- **Immutable-resolution tests**: a Hardhat artifact + sibling `.dbg.json` +
  build-info fixture (exact offsets), an explicit `--build-info` override, and a
  lone artifact that falls back to unexplained-diffs / opt-in heuristic.
- **CLI integration tests** (`assert_cmd`) asserting exit codes per `--fail-on`
  policy.
- On-chain calls are mocked in CI; an opt-in `--features live-rpc` test tier can
  run against a public RPC for smoke testing.

## 13. Roadmap

1. **MVP:** `verify` for a single contract, EIP-1967 proxy resolution,
   Foundry + Hardhat artifacts (exact immutables via inline refs or the
   `.dbg.json` → build-info chain), immutable + library masking, text + JSON
   output.
2. **Address-book labeling** (arbitrary `name → address` JSON) + SARIF output.
3. **More proxy kinds** (UUPS, beacon, custom slot).
4. **`bytecheck-core`** library extraction + optional MCP server mode.
5. **Export** to Sourcify/Etherscan verification formats.

## 14. Prior art

- **bytematch** (cognis-digital) — Python, metadata strip + keccak compare,
  SARIF/JSON, MCP mode. *Lacks* proxy resolution, immutable masking, RPC-by-
  address fetch, and address labeling — and is under a commercial-use-restricted
  license (COCL 1.0). `bytecheck` borrows its CI-output ideas (SARIF, strict
  mode) while solving the proxy/immutable problem and shipping a permissive
  single binary.
- **Sourcify / Etherscan verification** — source-based, centralized submission;
  `bytecheck` is the local, self-hosted, bytecode-only complement.
