//! Proxy detection: what code runs behind an address (port of the scan core's
//! detector, corejs/src/proxy.ts).
//!
//! Detection order, over the chain reads in [`Reader`]:
//!   0. EIP-7702 delegation designator (EOA code `0xef0100‖addr`)
//!   1. EIP-1167 minimal proxy (bytecode pattern, vanity variants included)
//!   2. EIP-1967 implementation slot (transparent / UUPS told apart by the
//!      admin slot and `proxiableUUID()`)
//!   3. EIP-1967 beacon slot, then `implementation()` / `childImplementation()`
//!      on the beacon
//!   4. EIP-1822 PROXIABLE slot
//!   5. ZeppelinOS legacy slot
//!   6. Safe proxy: `masterCopy` at slot 0 when the bytecode carries it
//!   7. Diamonds and selector routers, as a facet set (see [`crate::diamond`]):
//!      the loupe when the code can DELEGATECALL, event replay when the code
//!      looks like a small selector router
//!   8. `implementation()` on the contract itself, low confidence
//!
//! scanevm adds one step the core doesn't have: when the chain can't be read
//! or nothing matched, a provider's proxy hint (Etherscan's `Implementation`,
//! Sourcify's / Blockscout's proxy resolution) is used, low confidence.
//!
//! Addresses are lowercase inside; everything reported is checksummed.

use serde::Serialize;

use crate::chains::{ChainInfo, Provider};
use crate::diamond::{self, Facets};
use crate::error::{Result, ScanevmError};
use crate::fetch::Fetcher;
use crate::rpc::Reader;
use crate::util::{checksum, now_rfc3339, to_hex};

const EIP1967_IMPL_SLOT: &str =
    "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
const EIP1967_BEACON_SLOT: &str =
    "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50";
const EIP1967_ADMIN_SLOT: &str =
    "0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103";
/// keccak256("PROXIABLE") (EIP-1822 / UUPS)
const EIP1822_PROXIABLE_SLOT: &str =
    "0xc5f16f0fcc639fa48a6947836d9850f504798523bf8c9a3a87d5876cf622bcf7";
/// keccak256("org.zeppelinos.proxy.implementation") (legacy, pre-1967)
const ZEPPELINOS_IMPL_SLOT: &str =
    "0x7050c9e0f4ca769c69bd3a8ef740bc37934f8e2c036e5a723fd8ee048ed3f8c3";
const ZERO_SLOT: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";

const SEL_IMPLEMENTATION: [u8; 4] = [0x5c, 0x60, 0xda, 0x1b]; // implementation()
const SEL_CHILD_IMPLEMENTATION: [u8; 4] = [0xda, 0x52, 0x57, 0x16]; // childImplementation()
const SEL_PROXIABLE_UUID: [u8; 4] = [0x52, 0xd1, 0x90, 0x2d]; // proxiableUUID()
const SEL_MASTER_COPY: [u8; 4] = [0xa6, 0x19, 0x48, 0x6e]; // masterCopy()
const DELEGATECALL: u8 = 0xf4;

/// The proxy kinds, serialized as the scan core names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    None,
    Eip7702,
    Eip1167,
    Eip1967,
    Eip1967Beacon,
    Eip1822,
    Zeppelinos,
    Safe,
    Diamond,
    SelectorRouter,
    ImplementationCall,
    /// scanevm only: a provider said so (the chain couldn't confirm).
    ProviderHint,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::None => "not a proxy",
            Kind::Eip7702 => "EIP-7702 delegation",
            Kind::Eip1167 => "EIP-1167 minimal proxy",
            Kind::Eip1967 => "EIP-1967",
            Kind::Eip1967Beacon => "EIP-1967 beacon",
            Kind::Eip1822 => "EIP-1822 (UUPS)",
            Kind::Zeppelinos => "zeppelinos (legacy)",
            Kind::Safe => "Safe proxy",
            Kind::Diamond => "diamond (EIP-2535)",
            Kind::SelectorRouter => "selector router",
            Kind::ImplementationCall => "implementation() call",
            Kind::ProviderHint => "provider hint",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FacetOut {
    pub address: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub selectors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Resolution {
    pub kind: Kind,
    /// transparent | uups, for EIP-1967.
    pub variant: Option<&'static str>,
    pub high_confidence: bool,
    /// Checksummed addresses.
    pub implementation: Option<String>,
    pub beacon: Option<String>,
    pub admin: Option<String>,
    pub facets: Option<Facets>,
    /// The provider behind a [`Kind::ProviderHint`].
    pub hint_from: Option<Provider>,
}

impl Resolution {
    fn high(kind: Kind) -> Self {
        Resolution {
            kind,
            variant: None,
            high_confidence: true,
            implementation: None,
            beacon: None,
            admin: None,
            facets: None,
            hint_from: None,
        }
    }

    fn with_impl(kind: Kind, implementation: &str) -> Self {
        Resolution {
            implementation: Some(checksum(implementation)),
            ..Self::high(kind)
        }
    }

    /// Several facets behind one address (a diamond or a selector router).
    pub fn is_multi_facet(&self) -> bool {
        self.facets.as_ref().is_some_and(|f| !f.facets.is_empty())
    }
}

/// Resolve what `address` points at. `fetcher` lets a provider's proxy hint
/// fill in when the chain can't be read or nothing matched.
pub async fn resolve(
    reader: &Reader<'_>,
    fetcher: Option<&Fetcher<'_>>,
    address: &str,
) -> Result<Resolution> {
    let address = address.trim().to_ascii_lowercase();
    let detected = detect(reader, &address).await;
    if matches!(&detected, Ok(r) if r.kind != Kind::None)
        || matches!(&detected, Err(ScanevmError::NotFound(_)))
    {
        return detected;
    }
    if let Some(f) = fetcher {
        if let Ok(stored) = f.fetch(&address).await.into_source() {
            let fetched = &stored.fetched;
            if let Some(h) = fetched.proxy_hint.as_ref().filter(|h| h.is_proxy) {
                if let Some(imp) = h.implementations.first() {
                    return Ok(Resolution {
                        high_confidence: false,
                        hint_from: Some(fetched.provider),
                        ..Resolution::with_impl(Kind::ProviderHint, imp)
                    });
                }
            }
        }
    }
    detected
}

/// Detection over chain reads only.
pub async fn detect(reader: &Reader<'_>, addr: &str) -> Result<Resolution> {
    let code = reader.code(addr).await?;
    if code.is_empty() {
        return Err(ScanevmError::NotFound(format!(
            "no contract code at {}",
            checksum(addr)
        )));
    }
    if let Some(d) = parse_eip7702(&code) {
        return Ok(Resolution::with_impl(Kind::Eip7702, &d));
    }
    if let Some(c) = parse_eip1167(&code) {
        return Ok(Resolution::with_impl(Kind::Eip1167, &c));
    }

    if let Some(i) = slot_addr(reader, addr, EIP1967_IMPL_SLOT).await? {
        let mut r = Resolution::with_impl(Kind::Eip1967, &i);
        if let Some(admin) = slot_addr(reader, addr, EIP1967_ADMIN_SLOT).await? {
            r.admin = Some(checksum(&admin));
            r.variant = Some("transparent");
        } else if let Ok(out) = reader.call(&i, &sel(SEL_PROXIABLE_UUID)).await {
            // UUPS implementations answer proxiableUUID() with the implementation slot.
            if out.len() >= 32 && format!("0x{}", to_hex(&out[..32])) == EIP1967_IMPL_SLOT {
                r.variant = Some("uups");
            }
        }
        return Ok(r);
    }
    if let Some(b) = slot_addr(reader, addr, EIP1967_BEACON_SLOT).await? {
        let imp = match call_addr(reader, &b, SEL_IMPLEMENTATION).await? {
            Some(i) => Some(i),
            None => call_addr(reader, &b, SEL_CHILD_IMPLEMENTATION).await?,
        };
        let mut r = Resolution::high(Kind::Eip1967Beacon);
        r.implementation = imp.as_deref().map(checksum);
        r.beacon = Some(checksum(&b));
        return Ok(r);
    }
    if let Some(p) = slot_addr(reader, addr, EIP1822_PROXIABLE_SLOT).await? {
        return Ok(Resolution::with_impl(Kind::Eip1822, &p));
    }
    if let Some(z) = slot_addr(reader, addr, ZEPPELINOS_IMPL_SLOT).await? {
        return Ok(Resolution::with_impl(Kind::Zeppelinos, &z));
    }
    // Safe proxies answer masterCopy() from slot 0; their code carries that selector.
    if code.len() < 1200 && contains(&code, &SEL_MASTER_COPY) {
        if let Some(m) = slot_addr(reader, addr, ZERO_SLOT).await? {
            if has_code(reader, &m).await? {
                return Ok(Resolution::with_impl(Kind::Safe, &m));
            }
        }
    }
    // The loupe lives in a facet, so its selectors needn't appear in this code.
    if code.contains(&DELEGATECALL) {
        if let Some(f) = diamond::loupe(reader, addr).await {
            return Ok(multi_facet(Kind::Diamond, f));
        }
    }
    if looks_like_router(&code) {
        if let Some(f) = diamond::replay(reader, addr).await? {
            let kind = match f.source {
                diamond::Source::SelectorEventLogs => Kind::SelectorRouter,
                _ => Kind::Diamond,
            };
            return Ok(multi_facet(kind, f));
        }
    }
    // Low-confidence fallback: only if the code can DELEGATECALL at all.
    if code.contains(&DELEGATECALL) && contains(&code, &SEL_IMPLEMENTATION) {
        if let Some(imp) = call_addr(reader, addr, SEL_IMPLEMENTATION).await? {
            if imp != addr && has_code(reader, &imp).await? {
                return Ok(Resolution {
                    high_confidence: false,
                    ..Resolution::with_impl(Kind::ImplementationCall, &imp)
                });
            }
        }
    }
    Ok(Resolution::high(Kind::None))
}

fn multi_facet(kind: Kind, mut f: Facets) -> Resolution {
    for facet in &mut f.facets {
        facet.address = facet.address.to_ascii_lowercase();
    }
    Resolution {
        facets: Some(f),
        ..Resolution::high(kind)
    }
}

fn sel(s: [u8; 4]) -> String {
    format!("0x{}", to_hex(&s))
}

async fn slot_addr(reader: &Reader<'_>, addr: &str, slot: &str) -> Result<Option<String>> {
    Ok(word_to_address(&reader.storage(addr, slot).await?))
}

/// `eth_call` returning one address. A revert or junk is `None`; trouble
/// reaching the chain is an error.
async fn call_addr(reader: &Reader<'_>, to: &str, selector: [u8; 4]) -> Result<Option<String>> {
    match reader.call(to, &sel(selector)).await {
        Ok(out) if out.len() >= 32 => Ok(word_to_address(&format!("0x{}", to_hex(&out[..32])))),
        Ok(_) | Err(ScanevmError::ApiError(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

async fn has_code(reader: &Reader<'_>, addr: &str) -> Result<bool> {
    Ok(!reader.code(addr).await?.is_empty())
}

/// The address in the low 20 bytes of a 32-byte word whose high 12 bytes are
/// zero; `None` for zero, packed or malformed words.
fn word_to_address(word: &str) -> Option<String> {
    let hex = word.trim().trim_start_matches("0x");
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let (high, addr) = hex.split_at(24);
    if !high.bytes().all(|b| b == b'0') || addr.bytes().all(|b| b == b'0') {
        return None;
    }
    Some(format!("0x{}", addr.to_ascii_lowercase()))
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn parse_eip7702(code: &[u8]) -> Option<String> {
    (code.len() == 23 && code.starts_with(&[0xef, 0x01, 0x00]))
        .then(|| format!("0x{}", to_hex(&code[3..])))
}

/// EIP-1167 runtime code, including vanity variants with a shorter PUSHn address.
fn parse_eip1167(code: &[u8]) -> Option<String> {
    const PREFIX: [u8; 9] = [0x36, 0x3d, 0x3d, 0x37, 0x3d, 0x3d, 0x3d, 0x36, 0x3d];
    const SUFFIX_HEAD: [u8; 9] = [0x5a, 0xf4, 0x3d, 0x82, 0x80, 0x3e, 0x90, 0x3d, 0x91];
    let rest = code.strip_prefix(&PREFIX)?;
    let push = *rest.first()?;
    if !(0x60..=0x73).contains(&push) {
        return None;
    }
    let n = (push - 0x5f) as usize;
    let body = rest.get(1..1 + n)?;
    if !rest.get(1 + n..)?.starts_with(&SUFFIX_HEAD) {
        return None;
    }
    let mut a = [0u8; 20];
    a[20 - n..].copy_from_slice(body);
    Some(format!("0x{}", to_hex(&a)))
}

/// Small code that masks the selector (`PUSH32 0xffffffff00…`) and
/// delegatecalls: a router worth replaying events for.
fn looks_like_router(code: &[u8]) -> bool {
    let mut mask = [0u8; 33];
    mask[0] = 0x7f;
    mask[1..5].copy_from_slice(&[0xff; 4]);
    code.len() < 4096 && code.contains(&DELEGATECALL) && contains(code, &mask)
}

// ---- the resolution as `--json` prints it (the scan core's resolve report) ----

#[derive(Serialize)]
pub struct ResolveJson {
    target: String,
    chain_id: u64,
    address: String,
    status: &'static str,
    kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<&'static str>,
    confidence: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    implementation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    beacon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    admin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    facets: Option<Vec<FacetOut>>,
    /// scanevm only: which provider gave a `provider-hint`.
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<Provider>,
    checked_at: String,
}

pub fn resolve_json(r: &Resolution, chain: &ChainInfo, address: &str) -> ResolveJson {
    let cs = checksum(address);
    ResolveJson {
        target: format!("{}:{cs}", chain.name),
        chain_id: chain.chain_id,
        address: cs,
        status: "checked",
        kind: r.kind,
        variant: r.variant,
        confidence: if r.high_confidence { "high" } else { "low" },
        implementation: r.implementation.clone(),
        beacon: r.beacon.clone(),
        admin: r.admin.clone(),
        facets: r.facets.as_ref().map(|f| {
            f.facets
                .iter()
                .map(|x| FacetOut {
                    address: checksum(&x.address),
                    selectors: x.selectors.clone(),
                })
                .collect()
        }),
        provider: r.hint_from,
        checked_at: now_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::from_hex;

    #[test]
    fn words_to_addresses() {
        let w = "0x0000000000000000000000003d0768da09ce77d25e2d998e6a7b6ed4b9116c2d";
        assert_eq!(
            word_to_address(w).as_deref(),
            Some("0x3d0768da09ce77d25e2d998e6a7b6ed4b9116c2d")
        );
        assert_eq!(word_to_address(ZERO_SLOT), None);
        // Packed slot (non-zero high bytes) is not an address.
        let packed = "0x0000000000000000000000013d0768da09ce77d25e2d998e6a7b6ed4b9116c2d";
        assert_eq!(word_to_address(packed), None);
        assert_eq!(word_to_address("0x1234"), None);
    }

    #[test]
    fn eip1167_minimal_proxies() {
        // Standard 45-byte clone of 0xbebebebebebebebebebebebebebebebebebebebe.
        let code = from_hex("0x363d3d373d3d3d363d73bebebebebebebebebebebebebebebebebebebebe5af43d82803e903d91602b57fd5bf3").unwrap();
        assert_eq!(
            parse_eip1167(&code).as_deref(),
            Some("0xbebebebebebebebebebebebebebebebebebebebe")
        );
        // Vanity variant: PUSH19 of a 19-byte address (leading zero byte).
        let vanity = from_hex("0x363d3d373d3d3d363d72bebebebebebebebebebebebebebebebebebebe5af43d82803e903d91602a57fd5bf3").unwrap();
        assert_eq!(
            parse_eip1167(&vanity).as_deref(),
            Some("0x00bebebebebebebebebebebebebebebebebebebe")
        );
        assert_eq!(parse_eip1167(&[0x60, 0x80]), None);
    }

    #[test]
    fn eip7702_designator() {
        let code = from_hex("0xef010063c0c19a282a1b52b07dd5a65b58948a07dae32b").unwrap();
        assert_eq!(
            parse_eip7702(&code).as_deref(),
            Some("0x63c0c19a282a1b52b07dd5a65b58948a07dae32b")
        );
        assert_eq!(parse_eip7702(&code[..22]), None);
    }

    #[test]
    fn router_shape() {
        let mut code = vec![0x60, 0x80];
        code.push(0x7f);
        code.extend([0xff; 4]);
        code.extend([0u8; 28]);
        code.push(DELEGATECALL);
        assert!(looks_like_router(&code));
        assert!(!looks_like_router(&code[..code.len() - 1]));
    }

    #[test]
    fn kinds_serialize_as_the_core_names_them() {
        let names: Vec<String> = [
            Kind::Eip1967Beacon,
            Kind::SelectorRouter,
            Kind::ImplementationCall,
            Kind::Eip7702,
        ]
        .iter()
        .map(|k| {
            serde_json::to_value(k)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
        assert_eq!(
            names,
            [
                "eip1967-beacon",
                "selector-router",
                "implementation-call",
                "eip7702"
            ]
        );
    }

    // Network test (public RPC, no key): cargo test -- --ignored
    #[tokio::test]
    #[ignore]
    async fn detects_well_known_proxies() {
        let chain = crate::chains::resolve_chain("ethereum").unwrap();
        let reader = Reader::new(chain, None, true);
        // USDC: legacy zeppelinos slot.
        let r = detect(&reader, "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
            .await
            .unwrap();
        assert_eq!(r.kind, Kind::Zeppelinos);
        // LI.FI: a diamond with a loupe.
        let r = detect(&reader, "0x1231deb6f5749ef6ce6943a275a1d3e7486f4eae")
            .await
            .unwrap();
        assert_eq!(r.kind, Kind::Diamond);
        assert!(r.is_multi_facet());
        // Multicall3: not a proxy.
        let r = detect(&reader, "0xca11bde05977b3631167028862be2a173976ca11")
            .await
            .unwrap();
        assert_eq!(r.kind, Kind::None);
    }
}
