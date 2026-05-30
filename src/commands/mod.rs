pub mod balance;
pub mod block;
pub mod chains;
pub mod config;
pub mod contract;
pub mod gas;
pub mod token;
pub mod transfers;
pub mod tx;
pub mod txlist;

use clap::ValueEnum;

/// Sort order for list queries — validated by clap (shown as possible values).
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub fn as_str(self) -> &'static str {
        match self {
            SortOrder::Asc => "asc",
            SortOrder::Desc => "desc",
        }
    }
}

/// clap value parser: accept a 20-byte hex address (`0x` + 40 hex chars).
/// Rejects malformed input locally instead of sending a doomed request.
pub fn parse_address(s: &str) -> Result<String, String> {
    let t = s.trim();
    let valid = t.len() == 42
        && (t.starts_with("0x") || t.starts_with("0X"))
        && t[2..].bytes().all(|b| b.is_ascii_hexdigit());
    if valid {
        Ok(t.to_string())
    } else {
        Err(format!(
            "invalid address '{s}' (expected 0x followed by 40 hex chars)"
        ))
    }
}

/// clap value parser: accept a 32-byte hex transaction hash (`0x` + 64 hex chars).
pub fn parse_tx_hash(s: &str) -> Result<String, String> {
    let t = s.trim();
    let valid = t.len() == 66
        && (t.starts_with("0x") || t.starts_with("0X"))
        && t[2..].bytes().all(|b| b.is_ascii_hexdigit());
    if valid {
        Ok(t.to_string())
    } else {
        Err(format!(
            "invalid transaction hash '{s}' (expected 0x followed by 64 hex chars)"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_validation() {
        assert!(parse_address("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").is_ok());
        assert!(parse_address("0x0000000000000000000000000000000000000000").is_ok());
        assert!(parse_address("A0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48").is_err()); // no 0x
        assert!(parse_address("0x1234").is_err()); // too short
        assert!(parse_address("0xZZ...").is_err()); // non-hex
    }

    #[test]
    fn tx_hash_validation() {
        assert!(parse_tx_hash(&format!("0x{}", "a".repeat(64))).is_ok());
        assert!(parse_tx_hash("0xabc").is_err());
        assert!(parse_tx_hash(&format!("0x{}", "a".repeat(40))).is_err()); // address length
    }

    #[test]
    fn sort_order_str() {
        assert_eq!(SortOrder::Asc.as_str(), "asc");
        assert_eq!(SortOrder::Desc.as_str(), "desc");
    }
}
