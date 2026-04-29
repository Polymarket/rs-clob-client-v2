#![allow(
    clippy::print_stdout,
    reason = "Examples print their results to stdout"
)]

//! Cancel every open order belonging to the authenticated account.

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};
use polymarket_client_sdk_v2::CLOB_HOST;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let host =
        std::env::var("CLOB_API_URL").unwrap_or_else(|_| CLOB_HOST.into());
    let signer =
        LocalSigner::from_str(&std::env::var(PRIVATE_KEY_VAR)?)?.with_chain_id(Some(POLYGON));

    let client = Client::new(&host, Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    let resp = client.cancel_all_orders().await?;
    println!(
        "canceled: {}, not canceled: {}",
        resp.canceled.len(),
        resp.not_canceled.len()
    );
    Ok(())
}
