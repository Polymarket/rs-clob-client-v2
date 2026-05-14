#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! Fetch the orderbook summary for a token, then compute its hash.

use std::str::FromStr as _;

use polymarket_client_sdk_v2::clob::types::request::OrderBookSummaryRequest;
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::U256;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host =
        std::env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob-v2.polymarket.com".into());
    let token_id = U256::from_str(&std::env::var("TOKEN_ID")?)?;

    let client = Client::new(&host, Config::default())?;
    let book = client
        .order_book(
            &OrderBookSummaryRequest::builder()
                .token_id(token_id)
                .build(),
        )
        .await?;

    println!("{book:?}");
    println!("hash: {}", client.order_book_hash(&book)?);
    Ok(())
}
