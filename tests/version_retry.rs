#![cfg(feature = "clob")]
#![allow(
    clippy::unwrap_used,
    reason = "Do not need additional syntax for setting up tests"
)]

mod common;

use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use httpmock::{Method, MockServer};
use polymarket_client_sdk_v2::POLYGON;
use polymarket_client_sdk_v2::clob::types::response::OrderSummary;
use polymarket_client_sdk_v2::clob::types::{Amount, OrderType, Side, TickSize};
use polymarket_client_sdk_v2::types::Decimal;
use reqwest::StatusCode;
use rust_decimal_macros::dec;
use serde_json::json;

use crate::common::{
    PRIVATE_KEY, create_authenticated, ensure_requirements, ensure_version, token_1,
};

/// Reproduces rs-clob-client-v2#100: on `order_version_mismatch`, `build_sign_and_post`
/// must refresh the cached version and retry once. We seed the cache with the stale
/// version (1), then have the server report version 2 and reject every post with a
/// version-mismatch error. The retry firing means `/order` is hit twice.
#[tokio::test]
async fn limit_build_sign_and_post_retries_on_version_mismatch() -> anyhow::Result<()> {
    let server = MockServer::start();
    let client = create_authenticated(&server).await?;
    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));

    // Seed the version cache with the OLD version (1).
    let mut version_v1 = server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/version");
        then.status(StatusCode::OK)
            .json_body(json!({ "version": 1 }));
    });
    assert_eq!(client.version().await?, 1);
    version_v1.delete();

    // Server has since upgraded to version 2 (ensure_requirements mocks /version -> 2)
    // and rejects the stale-version order.
    ensure_requirements(&server, token_1(), TickSize::Tenth);

    let order_mock = server.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/order");
        then.status(StatusCode::BAD_REQUEST)
            .json_body(json!({ "error": "order_version_mismatch" }));
    });

    let result = client
        .limit_order()
        .token_id(token_1())
        .size(Decimal::ONE_HUNDRED)
        .price(dec!(0.1))
        .side(Side::Buy)
        .build_sign_and_post(&signer)
        .await;

    assert!(
        result.is_err(),
        "expected the version-mismatch error to propagate after the retry"
    );
    assert_eq!(
        order_mock.calls(),
        2,
        "version-mismatch retry did not fire: /order should be posted twice (initial + retry)"
    );

    Ok(())
}

/// Same as above, but for the market-order path (rs-clob-client-v2#100 notes both paths).
#[tokio::test]
async fn market_build_sign_and_post_retries_on_version_mismatch() -> anyhow::Result<()> {
    let server = MockServer::start();
    let client = create_authenticated(&server).await?;
    let signer = LocalSigner::from_str(PRIVATE_KEY)?.with_chain_id(Some(POLYGON));

    // Seed the version cache with the OLD version (1).
    let mut version_v1 = server.mock(|when, then| {
        when.method(Method::GET).path("/version");
        then.status(StatusCode::OK)
            .json_body(json!({ "version": 1 }));
    });
    assert_eq!(client.version().await?, 1);
    version_v1.delete();

    // Server has since upgraded to version 2.
    ensure_version(&server, 2);

    // Book / tick-size / fee-rate needed to compute the market price and build the order.
    let asks = vec![
        OrderSummary::builder()
            .price(dec!(0.5))
            .size(dec!(1000))
            .build(),
    ];
    server.mock(|when, then| {
        when.method(Method::GET)
            .path("/book")
            .query_param("token_id", token_1().to_string());
        then.status(StatusCode::OK).json_body(json!({
            "market": "0xbd31dc8a20211944f6b70f31557f1001557b59905b7738480ca09bd4532f84af",
            "asset_id": token_1(),
            "timestamp": "1000",
            "bids": [],
            "asks": asks,
            "min_order_size": "5",
            "neg_risk": false,
            "tick_size": "0.1",
        }));
    });
    server.mock(|when, then| {
        when.method(Method::GET)
            .path("/tick-size")
            .query_param("token_id", token_1().to_string());
        then.status(StatusCode::OK)
            .json_body(json!({ "minimum_tick_size": "0.1" }));
    });
    server.mock(|when, then| {
        when.method(Method::GET)
            .path("/fee-rate")
            .query_param("token_id", token_1().to_string());
        then.status(StatusCode::OK)
            .json_body(json!({ "base_fee": 0 }));
    });
    server.mock(|when, then| {
        when.method(Method::GET).path("/neg-risk");
        then.status(StatusCode::OK)
            .json_body(json!({ "neg_risk": false }));
    });

    let order_mock = server.mock(|when, then| {
        when.method(Method::POST).path("/order");
        then.status(StatusCode::BAD_REQUEST)
            .json_body(json!({ "error": "order_version_mismatch" }));
    });

    let result = client
        .market_order()
        .token_id(token_1())
        .amount(Amount::usdc(Decimal::ONE_HUNDRED)?)
        .side(Side::Buy)
        .order_type(OrderType::FOK)
        .build_sign_and_post(&signer)
        .await;

    assert!(
        result.is_err(),
        "expected the version-mismatch error to propagate after the retry"
    );
    assert_eq!(
        order_mock.calls(),
        2,
        "version-mismatch retry did not fire: /order should be posted twice (initial + retry)"
    );

    Ok(())
}
