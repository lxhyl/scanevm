use serde::{Deserialize, Deserializer, Serialize};

fn empty_string_as_none<'de, D>(d: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = Option::<String>::deserialize(d)?;
    Ok(s.filter(|v| !v.is_empty()))
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Transaction {
    pub hash: String,
    #[serde(rename = "blockNumber")]
    pub block_number: String,
    #[serde(rename = "timeStamp")]
    pub timestamp: String,
    pub from: String,
    pub to: String,
    pub value: String,
    pub gas: String,
    #[serde(rename = "gasPrice")]
    pub gas_price: String,
    #[serde(rename = "gasUsed")]
    pub gas_used: String,
    #[serde(rename = "isError")]
    pub is_error: String,
    #[serde(rename = "functionName", deserialize_with = "empty_string_as_none", default)]
    pub function_name: Option<String>,
    #[serde(rename = "txreceipt_status", deserialize_with = "empty_string_as_none", default)]
    pub receipt_status: Option<String>,
    #[serde(rename = "contractAddress", deserialize_with = "empty_string_as_none", default)]
    pub contract_address: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TokenTransfer {
    pub hash: String,
    #[serde(rename = "blockNumber")]
    pub block_number: String,
    #[serde(rename = "timeStamp")]
    pub timestamp: String,
    pub from: String,
    pub to: String,
    pub value: String,
    #[serde(rename = "tokenName")]
    pub token_name: String,
    #[serde(rename = "tokenSymbol")]
    pub token_symbol: String,
    #[serde(rename = "tokenDecimal")]
    pub token_decimal: String,
    #[serde(rename = "contractAddress")]
    pub contract_address: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NftTransfer {
    pub hash: String,
    #[serde(rename = "blockNumber")]
    pub block_number: String,
    #[serde(rename = "timeStamp")]
    pub timestamp: String,
    pub from: String,
    pub to: String,
    #[serde(rename = "tokenID")]
    pub token_id: String,
    #[serde(rename = "tokenName")]
    pub token_name: String,
    #[serde(rename = "tokenSymbol")]
    pub token_symbol: String,
    #[serde(rename = "contractAddress")]
    pub contract_address: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ContractSource {
    #[serde(rename = "SourceCode")]
    pub source_code: String,
    #[serde(rename = "ABI")]
    pub abi: String,
    #[serde(rename = "ContractName")]
    pub contract_name: String,
    #[serde(rename = "CompilerVersion")]
    pub compiler_version: String,
    #[serde(rename = "OptimizationUsed")]
    pub optimization_used: String,
    #[serde(rename = "Runs")]
    pub runs: String,
    #[serde(rename = "LicenseType")]
    pub license_type: String,
    #[serde(rename = "Proxy")]
    pub proxy: String,
    #[serde(rename = "Implementation", deserialize_with = "empty_string_as_none", default)]
    pub implementation: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GasOracle {
    #[serde(rename = "LastBlock")]
    pub last_block: String,
    #[serde(rename = "SafeGasPrice")]
    pub safe_gas_price: String,
    #[serde(rename = "ProposeGasPrice")]
    pub propose_gas_price: String,
    #[serde(rename = "FastGasPrice")]
    pub fast_gas_price: String,
    #[serde(rename = "suggestBaseFee")]
    pub suggest_base_fee: String,
    #[serde(rename = "gasUsedRatio")]
    pub gas_used_ratio: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct TokenInfo {
    #[serde(rename = "contractAddress")]
    pub contract_address: String,
    #[serde(rename = "tokenName")]
    pub token_name: String,
    pub symbol: String,
    pub divisor: String,
    #[serde(rename = "tokenType")]
    pub token_type: String,
    #[serde(rename = "totalSupply")]
    pub total_supply: String,
    #[serde(rename = "blueCheckmark")]
    pub blue_checkmark: String,
    pub description: String,
    pub website: String,
    #[serde(rename = "holderCount")]
    pub holder_count: String,
}
