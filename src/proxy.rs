//! Resolve the implementation address behind a proxy contract.
//!
//! Reads the standard proxy storage slots directly via `eth_getStorageAt`
//! (EIP-1967, EIP-1822/UUPS, the EIP-1967 beacon, and the legacy zeppelinos
//! slot), so it works even when Etherscan has not flagged the contract as a
//! proxy. Etherscan's own `Implementation` field is used as a last resort.
//!
//! Slot constants are `keccak256(...)`-derived and match the EIP specs.

use std::time::Duration;

use crate::client::EtherscanClient;
use crate::error::Result;
use crate::types::ContractSource;

/// keccak256("eip1967.proxy.implementation") - 1
const EIP1967_IMPL: &str = "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc";
/// keccak256("eip1967.proxy.beacon") - 1
const EIP1967_BEACON: &str = "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50";
/// keccak256("PROXIABLE")  (EIP-1822 / UUPS)
const EIP1822_PROXIABLE: &str =
    "0xc5f16f0fcc639fa48a6947836d9850f504798523bf8c9a3a87d5876cf622bcf7";
/// keccak256("org.zeppelinos.proxy.implementation")  (legacy, pre-1967)
const ZEPPELINOS_IMPL: &str = "0x7050c9e0f4ca769c69bd3a8ef740bc37934f8e2c036e5a723fd8ee048ed3f8c3";
/// `implementation()` selector — called on an EIP-1967 beacon.
const IMPLEMENTATION_SELECTOR: &str = "0x5c60da1b";

/// Implementations can be upgraded, so cache slot reads only briefly.
const SLOT_TTL: Duration = Duration::from_secs(60);

pub struct Implementation {
    pub address: String,
    /// Which mechanism revealed the implementation (for display).
    pub via: &'static str,
}

/// Try every known proxy mechanism in turn; return the first implementation
/// found, or `None` if the address does not look like a proxy.
pub async fn resolve_implementation(
    client: &EtherscanClient,
    address: &str,
) -> Result<Option<Implementation>> {
    // 1. EIP-1967 implementation slot (the common case).
    if let Some(address) = read_slot_address(client, address, EIP1967_IMPL).await {
        return Ok(Some(Implementation {
            address,
            via: "EIP-1967",
        }));
    }
    // 2. EIP-1822 (UUPS) PROXIABLE slot.
    if let Some(address) = read_slot_address(client, address, EIP1822_PROXIABLE).await {
        return Ok(Some(Implementation {
            address,
            via: "EIP-1822 (UUPS)",
        }));
    }
    // 3. EIP-1967 beacon slot -> call implementation() on the beacon.
    if let Some(beacon) = read_slot_address(client, address, EIP1967_BEACON).await {
        if let Some(address) = call_implementation(client, &beacon).await {
            return Ok(Some(Implementation {
                address,
                via: "EIP-1967 beacon",
            }));
        }
    }
    // 4. Legacy zeppelinos slot (old proxies).
    if let Some(address) = read_slot_address(client, address, ZEPPELINOS_IMPL).await {
        return Ok(Some(Implementation {
            address,
            via: "zeppelinos (legacy)",
        }));
    }
    // 5. Etherscan's own proxy metadata, as a last resort.
    if let Ok(sources) = client
        .call_cached::<Vec<ContractSource>>(
            None,
            &[
                ("module", "contract"),
                ("action", "getsourcecode"),
                ("address", address),
            ],
        )
        .await
    {
        if let Some(address) = sources.into_iter().next().and_then(|s| s.implementation) {
            return Ok(Some(Implementation {
                address,
                via: "Etherscan",
            }));
        }
    }
    Ok(None)
}

async fn read_slot_address(client: &EtherscanClient, address: &str, slot: &str) -> Option<String> {
    let raw: String = client
        .proxy_cached(
            Some(SLOT_TTL),
            "eth_getStorageAt",
            &format!(r#"{{"address":"{address}","position":"{slot}","tag":"latest"}}"#),
        )
        .await
        .ok()?;
    word_to_address(&raw)
}

async fn call_implementation(client: &EtherscanClient, beacon: &str) -> Option<String> {
    let raw: String = client
        .proxy(
            "eth_call",
            &format!(r#"{{"to":"{beacon}","data":"{IMPLEMENTATION_SELECTOR}","tag":"latest"}}"#),
        )
        .await
        .ok()?;
    word_to_address(&raw)
}

/// Extract the address (the low 20 bytes) from a 32-byte storage word, or
/// `None` if the slot is zero or the value is malformed.
fn word_to_address(word: &str) -> Option<String> {
    let hex = word.trim().trim_start_matches("0x");
    if hex.len() < 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let addr = &hex[hex.len() - 40..];
    if addr.bytes().all(|b| b == b'0') {
        return None;
    }
    Some(format!("0x{}", addr.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_address_from_padded_word() {
        let w = "0x0000000000000000000000003d0768da09ce77d25e2d998e6a7b6ed4b9116c2d";
        assert_eq!(
            word_to_address(w).as_deref(),
            Some("0x3d0768da09ce77d25e2d998e6a7b6ed4b9116c2d")
        );
    }

    #[test]
    fn zero_slot_is_none() {
        assert_eq!(
            word_to_address("0x0000000000000000000000000000000000000000000000000000000000000000"),
            None
        );
    }

    #[test]
    fn short_or_garbage_is_none() {
        assert_eq!(word_to_address("0x1234"), None);
        assert_eq!(
            word_to_address("0xnothexnothexnothexnothexnothexnothexnothex"),
            None
        );
    }
}
