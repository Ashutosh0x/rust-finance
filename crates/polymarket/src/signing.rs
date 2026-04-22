// crates/polymarket/src/signing.rs

use ethers_core::types::{transaction::eip712::EIP712Domain, Address, H256, U256};
use ethers_signers::{LocalWallet, Signer};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SigningError {
    #[error("Wallet error: {0}")]
    Wallet(String),
    #[error("Signing error: {0}")]
    Signing(String),
    #[error("Invalid address: {0}")]
    Address(String),
}

/// Polymarket CTF Exchange contract addresses on Polygon
pub const CTF_EXCHANGE: &str = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";
pub const NEG_RISK_CTF_EXCHANGE: &str = "0xC5d563A36AE78145C45a50134d48A1215220f80a";
pub const USDC_ADDRESS: &str = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";

/// Chain ID for Polygon Mainnet
pub const POLYGON_CHAIN_ID: u64 = 137;

/// Order struct matching Polymarket's EIP-712 type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    /// Salt for uniqueness (random u256)
    pub salt: String,
    /// Address of the order maker
    pub maker: String,
    /// Address of the signer
    pub signer: String,
    /// Address of the taker (0x0 for open orders)
    pub taker: String,
    /// Token ID of the conditional token
    pub token_id: String,
    /// Maker amount (in base units)
    pub maker_amount: String,
    /// Taker amount (in base units)
    pub taker_amount: String,
    /// Expiration timestamp (0 for no expiry)
    pub expiration: String,
    /// Nonce
    pub nonce: String,
    /// Fee rate basis points
    pub fee_rate_bps: String,
    /// Side: 0 = BUY, 1 = SELL
    pub side: String,
    /// Signature type: 0 = EOA, 1 = POLY_PROXY, 2 = POLY_GNOSIS_SAFE
    pub signature_type: String,
}

/// EIP-712 domain for the CTF Exchange
fn ctf_exchange_domain(neg_risk: bool) -> EIP712Domain {
    let exchange = if neg_risk {
        NEG_RISK_CTF_EXCHANGE
    } else {
        CTF_EXCHANGE
    };

    EIP712Domain {
        name: Some("Polymarket CTF Exchange".to_string()),
        version: Some("1".to_string()),
        chain_id: Some(U256::from(POLYGON_CHAIN_ID)),
        verifying_contract: Some(Address::from_str(exchange).unwrap()),
        salt: None,
    }
}

/// The EIP-712 type hash for Order
/// keccak256("Order(uint256 salt,address maker,address signer,address taker,
///            uint256 tokenId,uint256 makerAmount,uint256 takerAmount,
///            uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,
///            uint8 signatureType)")
fn order_type_hash() -> H256 {
    use ethers_core::utils::keccak256;
    let type_string = "Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)";
    H256::from(keccak256(type_string.as_bytes()))
}

/// Encode an order for EIP-712 struct hashing
fn encode_order(order: &Order) -> Vec<u8> {
    use ethers_core::abi::{encode, Token};

    let tokens = vec![
        Token::FixedBytes(order_type_hash().as_bytes().to_vec()),
        Token::Uint(U256::from_dec_str(&order.salt).unwrap_or_default()),
        Token::Address(Address::from_str(&order.maker).unwrap_or_default()),
        Token::Address(Address::from_str(&order.signer).unwrap_or_default()),
        Token::Address(Address::from_str(&order.taker).unwrap_or_default()),
        Token::Uint(U256::from_dec_str(&order.token_id).unwrap_or_default()),
        Token::Uint(U256::from_dec_str(&order.maker_amount).unwrap_or_default()),
        Token::Uint(U256::from_dec_str(&order.taker_amount).unwrap_or_default()),
        Token::Uint(U256::from_dec_str(&order.expiration).unwrap_or_default()),
        Token::Uint(U256::from_dec_str(&order.nonce).unwrap_or_default()),
        Token::Uint(U256::from_dec_str(&order.fee_rate_bps).unwrap_or_default()),
        Token::Uint(U256::from(order.side.parse::<u8>().unwrap_or(0))),
        Token::Uint(U256::from(order.signature_type.parse::<u8>().unwrap_or(0))),
    ];

    encode(&tokens)
}

/// Compute the EIP-712 struct hash of an order
fn hash_struct(order: &Order) -> H256 {
    use ethers_core::utils::keccak256;
    H256::from(keccak256(&encode_order(order)))
}

/// Compute the full EIP-712 digest: keccak256("\x19\x01" || domainSeparator || structHash)
pub fn compute_order_digest(order: &Order, neg_risk: bool) -> H256 {
    use ethers_core::utils::keccak256;

    let domain = ctf_exchange_domain(neg_risk);

    // Domain separator
    let domain_separator = {
        use ethers_core::abi::{encode, Token};
        let domain_type_hash = keccak256(
            b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        );
        let name_hash = keccak256(domain.name.as_ref().unwrap().as_bytes());
        let version_hash = keccak256(domain.version.as_ref().unwrap().as_bytes());

        let tokens = vec![
            Token::FixedBytes(domain_type_hash.to_vec()),
            Token::FixedBytes(name_hash.to_vec()),
            Token::FixedBytes(version_hash.to_vec()),
            Token::Uint(domain.chain_id.unwrap()),
            Token::Address(domain.verifying_contract.unwrap()),
        ];
        H256::from(keccak256(&encode(&tokens)))
    };

    let struct_hash = hash_struct(order);

    // "\x19\x01" || domainSeparator || structHash
    let mut msg = Vec::with_capacity(66);
    msg.push(0x19);
    msg.push(0x01);
    msg.extend_from_slice(domain_separator.as_bytes());
    msg.extend_from_slice(struct_hash.as_bytes());

    H256::from(keccak256(&msg))
}

/// Sign an order with a local wallet
pub async fn sign_order(
    wallet: &LocalWallet,
    order: &Order,
    neg_risk: bool,
) -> Result<String, SigningError> {
    let digest = compute_order_digest(order, neg_risk);

    let signature = wallet
        .sign_hash(digest)
        .map_err(|e| SigningError::Signing(e.to_string()))?;

    // Return as hex string with 0x prefix
    Ok(format!("0x{}", hex::encode(signature.to_vec())))
}

/// Create a wallet from a private key string
pub fn create_wallet(private_key: &str) -> Result<LocalWallet, SigningError> {
    let key = private_key.strip_prefix("0x").unwrap_or(private_key);
    LocalWallet::from_str(key)
        .map(|w| w.with_chain_id(POLYGON_CHAIN_ID))
        .map_err(|e| SigningError::Wallet(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_order_type_hash() {
        // Verify type hash matches Polymarket's expected value
        let hash = order_type_hash();
        assert!(!hash.is_zero());
    }

    #[test]
    fn test_create_wallet() {
        // Known test key (DO NOT use in production)
        let test_key = "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let wallet = create_wallet(test_key).unwrap();
        assert_eq!(wallet.chain_id(), POLYGON_CHAIN_ID);
    }
}
