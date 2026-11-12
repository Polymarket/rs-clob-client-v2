#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! List the authenticated account's open orders, optionally filtered by market or token.
//!
//! Set `MARKET` to a condition ID to filter by market. Filtering by `TOKEN_ID`
//! also requires `MARKET`.

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::clob::types::request::OrdersRequest;
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::{B256, U256};
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host =
        std::env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob-v2.polymarket.com".into());
    let signer =
        LocalSigner::from_str(&std::env::var(PRIVATE_KEY_VAR)?)?.with_chain_id(Some(POLYGON));

    let client = Client::new(&host, Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    let mut request = OrdersRequest::builder().build();
    if let Ok(market) = std::env::var("MARKET") {
        request.market = Some(B256::from_str(&market)?);
    }
    if let Ok(token) = std::env::var("TOKEN_ID") {
        request.asset_id = Some(U256::from_str(&token)?);
    }

    let page = client.orders(&request, None).await?;
    println!("{} open order(s)", page.data.len());
    for order in &page.data {
        println!(
            "  {} {} {} @ {}",
            order.id, order.side, order.original_size, order.price
        );
    }
    Ok(())
}
