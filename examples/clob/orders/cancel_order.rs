#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! Cancel a single order by ID.

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host =
        std::env::var("CLOB_API_URL").unwrap_or_else(|_| "https://clob-v2.polymarket.com".into());
    let order_id = std::env::var("ORDER_ID")?;
    let signer =
        LocalSigner::from_str(&std::env::var(PRIVATE_KEY_VAR)?)?.with_chain_id(Some(POLYGON));

    let client = Client::new(&host, Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    let resp = client.cancel_order(&order_id).await?;
    if resp.canceled.iter().any(|id| id == &order_id) {
        println!("canceled: {order_id}");
    } else if let Some(reason) = resp.not_canceled.get(&order_id) {
        println!("not canceled ({order_id}): {reason}");
    } else {
        println!("no-op for {order_id}");
    }
    Ok(())
}
