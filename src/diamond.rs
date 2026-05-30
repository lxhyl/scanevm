//! EIP-2535 Diamond facet enumeration.
//!
//! A diamond has no single implementation — it routes each function selector to
//! one of many *facets*. We resolve the current facet set four ways, in order:
//!   1. `facets()` DiamondLoupe call (facet address + its selectors).
//!   2. `facetAddresses()` DiamondLoupe call (addresses only).
//!   3. Replay of standard EIP-2535 `DiamondCut` event logs (loupe-less diamonds).
//!   4. Replay of `SelectorToFacetSet` event logs — Pendle-style custom selector
//!      routers (OZ `Proxy` + a `selectorToFacet` mapping) that expose neither
//!      the loupe nor `DiamondCut`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::client::EtherscanClient;
use crate::error::Result;

/// `facets()`
const SEL_FACETS: &str = "0x7a0ed627";
/// `facetAddresses()`
const SEL_FACET_ADDRESSES: &str = "0x52ef6b2c";
/// topic0 = keccak256("DiamondCut((address,uint8,bytes4[])[],address,bytes)")
const DIAMOND_CUT_TOPIC: &str =
    "0x8faa70878671ccd212d20771b795c50af8fd3ff6cf27f4bde57e5d4de0aeb673";
/// topic0 = keccak256("SelectorToFacetSet(bytes4,address)") — Pendle-style
/// custom routers (OZ `Proxy` + `selectorToFacet` mapping) emit this instead of
/// the standard `DiamondCut`. Both args are indexed (selector, facet in topics).
const SELECTOR_TO_FACET_TOPIC: &str =
    "0x0038aaccfeca40ea50135fcc37980765ee22033e320eaf575624561bdbbe9300";

/// Etherscan `getLogs` returns at most this many records per call.
const GETLOGS_CAP: usize = 1000;

#[derive(Debug, Clone)]
pub struct Facet {
    pub address: String,
    /// 4-byte selectors routed to this facet (may be empty if unknown).
    pub selectors: Vec<String>,
}

/// How the facet set was obtained, for display / honesty about completeness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Loupe,
    LoupeAddressesOnly,
    DiamondCutLogs,
    SelectorEventLogs,
}

pub struct Facets {
    pub facets: Vec<Facet>,
    pub source: Source,
    /// True if the result may be incomplete (e.g. getLogs hit its cap).
    pub truncated: bool,
}

/// Try every mechanism in turn. Returns `None` if the address is not a diamond
/// / no facets could be enumerated.
pub async fn resolve_facets(client: &EtherscanClient, address: &str) -> Result<Option<Facets>> {
    // 1. facets() — gives addresses and their selectors.
    if let Some(data) = eth_call(client, address, SEL_FACETS).await {
        if let Some(facets) = decode_facets(&data) {
            if !facets.is_empty() {
                return Ok(Some(Facets {
                    facets,
                    source: Source::Loupe,
                    truncated: false,
                }));
            }
        }
    }
    // 2. facetAddresses() — addresses only.
    if let Some(data) = eth_call(client, address, SEL_FACET_ADDRESSES).await {
        if let Some(addrs) = decode_address_array(&data) {
            if !addrs.is_empty() {
                return Ok(Some(Facets {
                    facets: addrs
                        .into_iter()
                        .map(|address| Facet {
                            address,
                            selectors: vec![],
                        })
                        .collect(),
                    source: Source::LoupeAddressesOnly,
                    truncated: false,
                }));
            }
        }
    }
    // 3. Standard DiamondCut event-log replay (loupe-less EIP-2535 diamonds).
    if let Some(f) = facets_from_diamond_cut(client, address).await {
        return Ok(Some(f));
    }
    // 4. Pendle-style SelectorToFacetSet event replay (custom selector routers).
    Ok(facets_from_selector_events(client, address).await)
}

/// Reconstruct the facet set by replaying standard EIP-2535 `DiamondCut` events.
async fn facets_from_diamond_cut(client: &EtherscanClient, address: &str) -> Option<Facets> {
    let (mut logs, truncated) = fetch_all_logs(client, address, DIAMOND_CUT_TOPIC).await;
    if logs.is_empty() {
        return None;
    }
    logs.sort_by_key(|l| (hex_to_u64(&l.block_number), hex_to_u64(&l.log_index)));

    // selector -> facet address (last writer wins; Remove deletes).
    let mut routes: BTreeMap<String, String> = BTreeMap::new();
    for log in &logs {
        let Some(bytes) = from_hex(&log.data) else {
            continue;
        };
        let Some(cuts) = decode_diamond_cut(&bytes) else {
            continue;
        };
        for (facet, action, selectors) in cuts {
            for sel in selectors {
                match action {
                    0 | 1 => {
                        routes.insert(sel, facet.clone());
                    }
                    2 => {
                        routes.remove(&sel);
                    }
                    _ => {}
                }
            }
        }
    }
    group_routes(routes, Source::DiamondCutLogs, truncated)
}

/// Reconstruct the facet set for Pendle-style routers that emit
/// `SelectorToFacetSet(bytes4 indexed selector, address indexed facet)`.
async fn facets_from_selector_events(client: &EtherscanClient, address: &str) -> Option<Facets> {
    let (mut logs, truncated) = fetch_all_logs(client, address, SELECTOR_TO_FACET_TOPIC).await;
    if logs.is_empty() {
        return None;
    }
    logs.sort_by_key(|l| (hex_to_u64(&l.block_number), hex_to_u64(&l.log_index)));

    let mut routes: BTreeMap<String, String> = BTreeMap::new();
    for log in &logs {
        // Both args are indexed: topics = [topic0, selector, facet].
        if log.topics.len() < 3 {
            continue;
        }
        // bytes4 selector is left-aligned: "0x" + first 8 hex chars.
        let Some(sel) = log.topics[1].get(..10) else {
            continue;
        };
        // address is right-aligned: last 40 hex chars of the 32-byte topic.
        let facet_topic = &log.topics[2];
        let Some(facet) = facet_topic.get(facet_topic.len().saturating_sub(40)..) else {
            continue;
        };
        if facet.bytes().all(|b| b == b'0') {
            routes.remove(sel);
        } else {
            routes.insert(sel.to_string(), format!("0x{}", facet.to_ascii_lowercase()));
        }
    }
    group_routes(routes, Source::SelectorEventLogs, truncated)
}

/// Group a selector→facet map into per-facet facets, or `None` if empty.
fn group_routes(
    routes: BTreeMap<String, String>,
    source: Source,
    truncated: bool,
) -> Option<Facets> {
    if routes.is_empty() {
        return None;
    }
    let mut by_facet: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (sel, facet) in routes {
        by_facet.entry(facet).or_default().push(sel);
    }
    let facets = by_facet
        .into_iter()
        .map(|(address, selectors)| Facet { address, selectors })
        .collect();
    Some(Facets {
        facets,
        source,
        truncated,
    })
}

/// Fetch all logs for `address` matching `topic0`, walking getLogs pages.
async fn fetch_all_logs(
    client: &EtherscanClient,
    address: &str,
    topic0: &str,
) -> (Vec<LogEntry>, bool) {
    // Walk pages until a short one. Bounded so a pathological contract can't loop forever.
    const MAX_PAGES: u32 = 20;
    let mut all: Vec<LogEntry> = Vec::new();
    let mut truncated = false;
    for page in 1..=MAX_PAGES {
        let page_str = page.to_string();
        let offset_str = GETLOGS_CAP.to_string();
        let logs: Vec<LogEntry> = client
            .call(&[
                ("module", "logs"),
                ("action", "getLogs"),
                ("address", address),
                ("fromBlock", "0"),
                ("toBlock", "latest"),
                ("topic0", topic0),
                ("page", &page_str),
                ("offset", &offset_str),
            ])
            .await
            .unwrap_or_default();
        let n = logs.len();
        all.extend(logs);
        if n < GETLOGS_CAP {
            break;
        }
        if page == MAX_PAGES {
            truncated = true;
        }
    }
    (all, truncated)
}

#[derive(Debug, Deserialize, Default)]
struct LogEntry {
    #[serde(default)]
    data: String,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(rename = "blockNumber", default)]
    block_number: String,
    #[serde(rename = "logIndex", default)]
    log_index: String,
}

async fn eth_call(client: &EtherscanClient, to: &str, data: &str) -> Option<Vec<u8>> {
    let raw: String = client
        .proxy(
            "eth_call",
            &format!(r#"{{"to":"{to}","data":"{data}","tag":"latest"}}"#),
        )
        .await
        .ok()?;
    from_hex(&raw)
}

// ---- minimal ABI decoding (no external crate) ----

/// Read the 32-byte word at byte offset `off`.
fn word(data: &[u8], off: usize) -> Option<&[u8]> {
    data.get(off..off + 32)
}

/// Read a 32-byte word as a usize (only the low bytes matter for our sizes).
fn word_usize(data: &[u8], off: usize) -> Option<usize> {
    let w = word(data, off)?;
    let mut n: u64 = 0;
    // Use the low 8 bytes; offsets/lengths never exceed that here.
    for b in &w[24..32] {
        n = (n << 8) | u64::from(*b);
    }
    Some(n as usize)
}

/// Read the address (low 20 bytes) from the 32-byte word at `off`.
fn word_address(data: &[u8], off: usize) -> Option<String> {
    let w = word(data, off)?;
    Some(format!("0x{}", to_hex(&w[12..32])))
}

/// Decode an ABI `address[]` return value.
fn decode_address_array(data: &[u8]) -> Option<Vec<String>> {
    let off = word_usize(data, 0)?;
    let len = word_usize(data, off)?;
    let base = off + 32;
    (0..len)
        .map(|i| word_address(data, base + i * 32))
        .collect()
}

/// Decode an ABI `(address, bytes4[])[]` return value (the `facets()` shape).
fn decode_facets(data: &[u8]) -> Option<Vec<Facet>> {
    let arr = word_usize(data, 0)?;
    let n = word_usize(data, arr)?;
    let elems_base = arr + 32;
    let mut facets = Vec::with_capacity(n);
    for i in 0..n {
        let elem = elems_base + word_usize(data, elems_base + i * 32)?;
        let address = word_address(data, elem)?;
        let sel_arr = elem + word_usize(data, elem + 32)?;
        let selectors = decode_selectors(data, sel_arr)?;
        facets.push(Facet { address, selectors });
    }
    Some(facets)
}

/// Decode a `DiamondCut` event `data`: `(FacetCut[], address, bytes)` where
/// `FacetCut = (address facetAddress, uint8 action, bytes4[] functionSelectors)`.
/// Returns `(facetAddress, action, selectors)` per cut.
fn decode_diamond_cut(data: &[u8]) -> Option<Vec<(String, u8, Vec<String>)>> {
    let arr = word_usize(data, 0)?;
    let n = word_usize(data, arr)?;
    let elems_base = arr + 32;
    let mut cuts = Vec::with_capacity(n);
    for i in 0..n {
        let elem = elems_base + word_usize(data, elems_base + i * 32)?;
        let address = word_address(data, elem)?;
        let action = word(data, elem + 32)?[31];
        let sel_arr = elem + word_usize(data, elem + 64)?;
        let selectors = decode_selectors(data, sel_arr)?;
        cuts.push((address, action, selectors));
    }
    Some(cuts)
}

/// Decode a `bytes4[]` array starting at byte offset `at` (the length word).
fn decode_selectors(data: &[u8], at: usize) -> Option<Vec<String>> {
    let len = word_usize(data, at)?;
    let base = at + 32;
    (0..len)
        .map(|j| {
            // bytes4 is left-aligned: the 4 bytes are the high-order bytes.
            word(data, base + j * 32).map(|w| format!("0x{}", to_hex(&w[0..4])))
        })
        .collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        out.push((hex_nibble(bytes[i])? << 4) | hex_nibble(bytes[i + 1])?);
        i += 2;
    }
    Some(out)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_to_u64(s: &str) -> u64 {
    let s = s.trim().strip_prefix("0x").unwrap_or(s.trim());
    u64::from_str_radix(s, 16).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // facetAddresses() returning [0xaaaa..., 0xbbbb...]
    #[test]
    fn decodes_address_array() {
        let data = from_hex(&address_array_hex(&[
            "0x000000000000000000000000000000000000aaaa",
            "0x000000000000000000000000000000000000bbbb",
        ]))
        .unwrap();
        let got = decode_address_array(&data).unwrap();
        assert_eq!(
            got,
            vec![
                "0x000000000000000000000000000000000000aaaa".to_string(),
                "0x000000000000000000000000000000000000bbbb".to_string()
            ]
        );
    }

    #[test]
    fn decodes_facets_single() {
        // facets() = [ (0x...aaaa, [0x12345678]) ]
        let data = from_hex(&facets_hex_one()).unwrap();
        let facets = decode_facets(&data).unwrap();
        assert_eq!(facets.len(), 1);
        assert_eq!(
            facets[0].address,
            "0x000000000000000000000000000000000000aaaa"
        );
        assert_eq!(facets[0].selectors, vec!["0x12345678".to_string()]);
    }

    #[test]
    fn selector_is_high_order_bytes() {
        // a 32-byte word with 0x12345678 left-aligned
        let mut word = vec![0u8; 32];
        word[0] = 0x12;
        word[1] = 0x34;
        word[2] = 0x56;
        word[3] = 0x78;
        // length 1 then the word
        let mut data = vec![0u8; 32];
        data[31] = 1;
        data.extend_from_slice(&word);
        let sels = decode_selectors(&data, 0).unwrap();
        assert_eq!(sels, vec!["0x12345678".to_string()]);
    }

    // ---- helpers to build ABI fixtures ----
    fn pad_word(hex_no_prefix: &str) -> String {
        let mut s = String::from(hex_no_prefix);
        while s.len() < 64 {
            s.insert(0, '0');
        }
        s
    }
    fn addr_word(addr: &str) -> String {
        pad_word(addr.trim_start_matches("0x"))
    }
    fn num_word(n: u64) -> String {
        pad_word(&format!("{n:x}"))
    }
    fn address_array_hex(addrs: &[&str]) -> String {
        let mut s = String::from("0x");
        s.push_str(&num_word(0x20)); // offset to array
        s.push_str(&num_word(addrs.len() as u64));
        for a in addrs {
            s.push_str(&addr_word(a));
        }
        s
    }
    fn facets_hex_one() -> String {
        // outer offset 0x20; array len 1; one element-offset 0x20 (rel to elems_base);
        // element: address word, selectors-offset 0x40 (rel to elem), then bytes4[] = len 1 + selector word
        let mut s = String::from("0x");
        s.push_str(&num_word(0x20)); // [0] offset to array
                                     // array body starts at 0x20:
        s.push_str(&num_word(1)); // len = 1
        s.push_str(&num_word(0x20)); // element[0] offset, relative to elems_base (right after len)
                                     // element starts here (elems_base + 0x20):
        s.push_str(&addr_word("0x000000000000000000000000000000000000aaaa")); // facetAddress
        s.push_str(&num_word(0x40)); // offset to selectors, relative to elem start
                                     // selectors array (elem + 0x40):
        s.push_str(&num_word(1)); // selectors len = 1
        let mut sel = String::from("12345678");
        while sel.len() < 64 {
            sel.push('0'); // left-aligned (high-order)
        }
        s.push_str(&sel);
        s
    }
}
