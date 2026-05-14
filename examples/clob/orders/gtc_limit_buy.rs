#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! Place a resting GTC limit buy. Sits on the book until filled or cancelled.

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::clob::types::{OrderType, Side};
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
        .limit_order()
        .token_id(token_id)
        .side(Side::Buy)
        .price(Decimal::from_str("0.4")?)
        .size(Decimal::from_str("100")?)
        .order_type(OrderType::GTC)
        .build_sign_and_post(&signer)
        .await?;

    println!("order_id={} status={}", resp.order_id, resp.status);
    Ok(())
}
