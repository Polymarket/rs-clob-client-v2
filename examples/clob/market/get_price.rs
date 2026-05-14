#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! Fetch the best BUY and SELL prices for a token.

use std::str::FromStr as _;

use polymarket_client_sdk_v2::clob::types::Side;
use polymarket_client_sdk_v2::clob::types::request::PriceRequest;
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::U256;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host =
        std::env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob-v2.polymarket.com".into());
    let token_id = U256::from_str(&std::env::var("TOKEN_ID")?)?;

    let client = Client::new(&host, Config::default())?;

    for side in [Side::Buy, Side::Sell] {
        let resp = client
            .price(
                &PriceRequest::builder()
                    .token_id(token_id)
                    .side(side)
                    .build(),
            )
            .await?;
        println!("{side} {resp:?}");
    }

    Ok(())
}
