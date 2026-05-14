#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! Place a resting GTC limit buy from a deployed deposit wallet.

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::clob::types::{OrderType, Side, SignatureType};
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::{Address, Decimal, U256};
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host =
        std::env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob-v2.polymarket.com".into());
    let token_id = U256::from_str(&std::env::var("TOKEN_ID")?)?;
    let deposit_wallet = Address::from_str(&std::env::var("DEPOSIT_WALLET")?)?;
    let price =
        Decimal::from_str(&std::env::var("ORDER_PRICE").unwrap_or_else(|_| "0.4".to_owned()))?;
    let size =
        Decimal::from_str(&std::env::var("ORDER_SIZE").unwrap_or_else(|_| "100".to_owned()))?;
    let signer =
        LocalSigner::from_str(&std::env::var(PRIVATE_KEY_VAR)?)?.with_chain_id(Some(POLYGON));

    let client = Client::new(&host, Config::default())?
        .authentication_builder(&signer)
        .funder(deposit_wallet)
        .signature_type(SignatureType::Poly1271)
        .authenticate()
        .await?;

    let resp = client
        .limit_order()
        .token_id(token_id)
        .side(Side::Buy)
        .price(price)
        .size(size)
        .order_type(OrderType::GTC)
        .build_sign_and_post(&signer)
        .await?;

    println!("order_id={} status={}", resp.order_id, resp.status);
    Ok(())
}
