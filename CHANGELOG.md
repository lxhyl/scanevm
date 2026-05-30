# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
