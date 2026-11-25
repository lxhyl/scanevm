# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Removed
- **Breaking:** the chain-data commands `balance`, `txlist`, `transfers`, `gas`,
  `token`, `tx` and `block` are gone. scanevm now does one thing: fetch on-chain
  contract code (`contract source` / `abi` / `bytecode` / `impl` / `facets`).
  Exit code 7 ("no such block / tx") is retired with them.
- **Breaking:** the chains `moonbeam` (1284), `holesky` (17000) and `scroll-sepolia`
  (534351) are gone: Etherscan v2 no longer serves them and no other provider does.

### Changed
- The chain table now records, per chain, which verified-source providers to ask and
  in what order, a Blockscout instance and Routescan network where there is one, and
  public RPC endpoints. Seven chains are new: hyperevm, sonic, mantle, berachain,
  monad, xlayer and plasma. `polygon`'s currency is `POL`.
- Any numeric chain id (decimal or `0x` hex) is accepted, not only listed chains.
- `scanevm chains` shows each chain's sources; `--json` prints
  `{"chains": [{id, name, title, aliases, testnet, explorer, sources}]}`.

## [0.2.1]

### Added
- `contract bytecode --json` and `config show --json` (the latter never prints the
  full key — only a masked preview and an `api_key_set` flag), so every query
  command now has a `--json` form.
- Exit codes are documented at the bottom of `--help` and in the README.

### Changed
- `--sort` is now a validated `asc`/`desc` choice (shown in `--help`) instead of a
  free-form string.
- Addresses and transaction hashes are validated locally before any request, so a
  typo fails fast with a clear message instead of an opaque API error.
- `--limit` is bounded to 1–10000.
- Clarified `--help` text for `contract facets` (also handles non-EIP-2535 routers
  and can download sources) and `contract source --json` (metadata only).

## [0.2.0]

### Added
- **Proxy implementation resolution** — `contract source --impl` / `contract abi --impl`
  follow a proxy to its implementation and fetch *that* contract's source/ABI.
  Detection reads EIP-1967, EIP-1822 (UUPS), beacon, and legacy storage slots
  directly (works even when the explorer hasn't flagged the contract as a proxy).
- **`contract impl`** — resolve and print the implementation address behind a proxy.
- **`contract facets`** — enumerate the facets of a diamond / multi-facet proxy via
  DiamondLoupe `facets()`/`facetAddresses()`, EIP-2535 `DiamondCut` log replay, or
  `SelectorToFacetSet` log replay (Pendle-style custom routers). `--save` downloads
  every facet's source; `--abi` merges all facets' ABIs.
- **`contract source --flatten`** — concatenate a multi-file contract into a single output.
- **Local response cache** at `~/.scanevm/cache/` with per-query TTLs (set
  `SCANEVM_NO_CACHE=1` to disable). Verified source/ABI/bytecode are cached
  permanently; balances briefly.
- **Smart networking** — request/connect timeouts, a `User-Agent`, automatic retry
  with backoff honoring `Retry-After`, and client-side throttling under the rate limit.
- Richer `contract source` metadata (EVM version, constructor arguments) and a
  clear error for unverified contracts.

### Changed
- **Distinct exit codes per failure class** (usage/config 2, network 3, rate-limited 4,
  invalid key 5, API 6, not-found 7, not-verified 8) so scripts can branch on failures.
- Number formatting uses exact integer math — fixes wrong amounts for tokens with
  more than 18 decimals and precision loss on large values.
- Tables size to content and only wrap to fit the terminal (no more compressed addresses).

### Security
- The API key is no longer leaked to stderr via HTTP error messages (the request
  URL is sanitized).
- The config file is written with `0600` permissions on Unix.

## [0.1.0]

- Initial release: `contract source`/`abi`/`bytecode`, `balance`, `txlist`,
  `transfers`, `gas`, `token`, `tx`, `block`, `chains`, `config` across many EVM chains.
