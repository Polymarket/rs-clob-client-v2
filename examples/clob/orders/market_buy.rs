#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! Place a FOK market buy. `amount` is in USDC and must match against resting asks.

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::clob::types::{Amount, OrderType, Side};
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::{Decimal, U256};
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host =
        std::env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob-v2.polymarket.com".into());
    let token_id = U256::from_str(&std::env::var("TOKEN_ID")?)?;
    let signer =
        LocalSigner::from_str(&std::env::var(PRIVATE_KEY_VAR)?)?.with_chain_id(Some(POLYGON));

    let client = Client::new(&host, Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    let resp = client
        .market_order()
        .token_id(token_id)
        .side(Side::Buy)
        .amount(Amount::usdc(Decimal::from_str("100")?)?)
        .order_type(OrderType::FOK)
        .build_sign_and_post(&signer)
        .await?;

    println!("order_id={} status={}", resp.order_id, resp.status);
    Ok(())
}
