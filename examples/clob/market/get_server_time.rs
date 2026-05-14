#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! Fetch the CLOB server's current Unix timestamp.

use polymarket_client_sdk_v2::clob::{Client, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host =
        std::env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob-v2.polymarket.com".into());

    let client = Client::new(&host, Config::default())?;
    println!("{}", client.server_time().await?);
    Ok(())
}
