//! Demonstrates builder-attributed trading with the CLOB client.
//!
//! V2 authenticates builder operations with the same L2 credentials as any other user;
//! attribution is carried on the order's `builder_code` field and as a query parameter
//! on `builder_trades`.
//!
//! Run with tracing enabled:
//! ```sh
//! RUST_LOG=info,hyper_util=off,hyper=off,reqwest=off,h2=off,rustls=off cargo run --example builder_authenticated --features clob,tracing
//! ```
//!
//! Requires `POLYMARKET_PRIVATE_KEY` and `POLYMARKET_BUILDER_CODE` environment variables.

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::clob::types::request::TradesRequest;
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::types::{B256, U256};
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let private_key = std::env::var(PRIVATE_KEY_VAR).expect("Need POLYMARKET_PRIVATE_KEY");
    let builder_code = B256::from_str(
        &std::env::var("POLYMARKET_BUILDER_CODE").expect("Need POLYMARKET_BUILDER_CODE"),
    )?;
    let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));

    let config = Config::builder().builder_code(builder_code).build();
    let client = Client::new("https://clob-v2.polymarket.com", config)?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    match client.builder_api_keys().await {
        Ok(keys) => info!(endpoint = "builder_api_keys", count = keys.len()),
        Err(e) => error!(endpoint = "builder_api_keys", error = %e),
    }

    let token_id = U256::from_str(
        "15871154585880608648532107628464183779895785213830018178010423617714102767076",
    )?;
    let request = TradesRequest::builder().asset_id(token_id).build();

    match client.builder_trades(builder_code, &request, None).await {
        Ok(trades) => {
            info!(endpoint = "builder_trades", token_id = %token_id, count = trades.data.len());
        }
        Err(e) => error!(endpoint = "builder_trades", token_id = %token_id, error = %e),
    }

    Ok(())
}
