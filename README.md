![Polymarket](assets/logo.png)

# Polymarket Rust Client

[![Crates.io](https://img.shields.io/crates/v/polymarket_client_sdk_v2.svg)](https://crates.io/crates/polymarket_client_sdk_v2)
[![CI](https://github.com/Polymarket/rs-clob-client-v2/actions/workflows/ci.yml/badge.svg)](https://github.com/Polymarket/rs-clob-client-v2/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/Polymarket/rs-clob-client-v2/graph/badge.svg?token=FW1BYWWFJ2)](https://codecov.io/gh/Polymarket/rs-clob-client-v2)

An ergonomic Rust client for interacting with Polymarket services, primarily the Central Limit Order Book (CLOB).
This crate provides strongly typed request builders, authenticated endpoints, `alloy` support and more.

## Table of Contents

- [Overview](#overview)
- [Getting Started](#getting-started)
- [Feature Flags](#feature-flags)
- [Re-exported Types](#re-exported-types)
- [Examples](#examples)
  - [CLOB Client](#clob-client)
  - [WebSocket Streaming](#websocket-streaming)
  - [Optional APIs](#optional-apis)
- [Additional CLOB Capabilities](#additional-clob-capabilities)
- [Setting Token Allowances](#token-allowances)
- [Minimum Supported Rust Version (MSRV)](#minimum-supported-rust-version-msrv)
- [Contributing](#contributing)
- [About Polymarket](#about-polymarket)

## Overview

- **Typed CLOB requests** (orders, trades, markets, balances, and more)
- **Dual authentication flows**
    - Normal authenticated flow
    - [Builder](https://docs.polymarket.com/developers/builders/builder-intro) authentication flow
- **Type-level state machine**
    - Prevents using authenticated endpoints before authenticating
    - Compile-time enforcement of correct transitions
- **Signer support** via `alloy::signers::Signer`
    - Including remote signers, e.g. AWS KMS
- **Zero-cost abstractions** — no dynamic dispatch in hot paths
- **Order builders** for easy construction & signing
- **Full `serde` support**
- **Async-first design** with `reqwest`


## Getting started

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
polymarket_client_sdk_v2 = "0.6"
```

or

```bash
cargo add polymarket_client_sdk_v2
```

Then run any of the examples
```bash
cargo run --example unauthenticated
```

## Feature Flags

The crate is modular with optional features for different Polymarket APIs:

| Feature      | Description                                                                                                                                    |
|--------------|------------------------------------------------------------------------------------------------------------------------------------------------|
| `clob`       | Core CLOB client for order placement, market data, and authentication                                                                          |
| `tracing`    | Structured logging via [`tracing`](https://docs.rs/tracing) for HTTP requests, auth flows, and caching                                         |
| `ws`         | WebSocket client for real-time orderbook, price, and user event streaming                                                                      |
| `rtds`       | Real-time data streams for crypto prices (Binance, Chainlink) and comments                                                                     |
| `data`       | Data API client for positions, trades, leaderboards, and analytics                                                                             |
| `gamma`      | Gamma API client for market/event discovery, search, and metadata                                                                              |
| `bridge`     | Bridge API client for cross-chain deposits (EVM, Solana, Bitcoin)                                                                              |
| `rfq`        | RFQ API (within CLOB) for submitting and querying quotes                                                                                       |
| `heartbeats` | Clob feature that automatically sends heartbeat messages to the Polymarket server, if the client disconnects all open orders will be cancelled |
| `ctf`        | CTF API client to perform split/merge/redeem on binary and neg risk markets

Enable features in your `Cargo.toml`:

```toml
[dependencies]
polymarket_client_sdk_v2 = { version = "0.6", features = ["ws", "data"] }
```

## Re-exported Types

This SDK re-exports commonly used types from external crates so you don't need to add them to your `Cargo.toml`:

### From `types` module

```rust
use polymarket_client_sdk_v2::types::{
    Address, ChainId, Signature, address,  // from alloy::primitives
    DateTime, NaiveDate, Utc,              // from chrono
    Decimal, dec,                          // from rust_decimal + rust_decimal_macros
};
```

### From `auth` module

```rust
use polymarket_client_sdk_v2::auth::{
    LocalSigner, Signer,          // from alloy::signers (LocalSigner + trait)
    Uuid, ApiKey,                 // from uuid (ApiKey = Uuid)
    SecretString, ExposeSecret,   // from secrecy
};
```

### From `error` module

```rust
use polymarket_client_sdk_v2::error::{
    StatusCode, Method,           // from reqwest (for error inspection)
};
```

This allows you to work with the SDK without managing version compatibility for these common dependencies.

## Examples

See `examples/` for the complete set. Below are hand-picked examples for common use cases.

### CLOB Client

#### Unauthenticated client (read-only)
```rust,ignore
use polymarket_client_sdk_v2::clob::Client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::default();

    let ok = client.ok().await?;
    println!("Ok: {ok}");

    Ok(())
}
```

#### Authenticated client

Set `POLYMARKET_PRIVATE_KEY` as an environment variable with your private key.

##### [EOA](https://www.binance.com/en/academy/glossary/externally-owned-account-eoa) wallets
If using MetaMask or hardware wallet, you must first set token allowances. See [Token Allowances](#token-allowances) section below.

```rust,ignore
use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};
use polymarket_client_sdk_v2::clob::{Client, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let private_key = std::env::var(PRIVATE_KEY_VAR).expect("Need a private key");
    let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));
    let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    let ok = client.ok().await?;
    println!("Ok: {ok}");

    let api_keys = client.api_keys().await?;
    println!("API keys: {api_keys:?}");

    Ok(())
}
```

##### Proxy/Safe wallets
For proxy/Safe wallets, the funder address is **automatically derived** using CREATE2 from your signer's EOA address:

```rust,ignore
let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
    .authentication_builder(&signer)
    .signature_type(SignatureType::GnosisSafe)  // Funder auto-derived via CREATE2
    .authenticate()
    .await?;
```

The SDK computes the deterministic wallet address that Polymarket deploys for your EOA. This is the same address
shown on polymarket.com when you log in with a browser wallet.

If you need to override the derived address (e.g., for advanced use cases), you can explicitly provide it:

```rust,ignore
let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
    .authentication_builder(&signer)
    .funder(address!("<your-polymarket-wallet-address>"))
    .signature_type(SignatureType::GnosisSafe)
    .authenticate()
    .await?;
```

You can also derive these addresses manually:

```rust,ignore
use polymarket_client_sdk_v2::{derive_safe_wallet, derive_proxy_wallet, POLYGON};

// For browser wallet users (GnosisSafe)
let safe_address = derive_safe_wallet(signer.address(), POLYGON);

// For Magic/email wallet users (Proxy)
let proxy_address = derive_proxy_wallet(signer.address(), POLYGON);
```

##### Funder Address
The **funder address** is the actual address that holds your funds on Polymarket. When using proxy wallets (email wallets
like Magic or browser extension wallets), the signing key differs from the address holding the funds. The SDK automatically
derives the correct funder address using CREATE2 when you specify `SignatureType::Proxy` or `SignatureType::GnosisSafe`.
You can override this with `.funder(address)` if needed.

##### Signature Types
The **signature_type** parameter tells the system how to verify your signatures:
- `signature_type=0` (default): Standard EOA (Externally Owned Account) signatures - includes MetaMask, hardware wallets,
   and any wallet where you control the private key directly
- `signature_type=1`: Email/Magic wallet signatures (delegated signing)
- `signature_type=2`: Browser wallet proxy signatures (when using a proxy contract, not direct wallet connections)
- `signature_type=3`: EIP-1271 smart contract wallet signatures (**V2 orders only**)

See [`SignatureType`](src/clob/types/mod.rs) for more information.

For deposit wallets, pass the deployed deposit wallet as the funder and use `SignatureType::Poly1271`:

```rust,ignore
let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
    .authentication_builder(&signer)
    .funder(deposit_wallet)
    .signature_type(SignatureType::Poly1271)
    .authenticate()
    .await?;
```

##### Place a market order

```rust,ignore
use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::clob::types::{Amount, OrderType, Side};
use polymarket_client_sdk_v2::types::Decimal;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let private_key = std::env::var(PRIVATE_KEY_VAR).expect("Need a private key");
    let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));
    let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    let order = client
        .market_order()
        .token_id("<token-id>")
        .amount(Amount::usdc(Decimal::ONE_HUNDRED)?)
        .side(Side::Buy)
        .order_type(OrderType::FOK)
        .build()
        .await?;
    let signed_order = client.sign(&signer, order).await?;
    let response = client.post_order(signed_order).await?;
    println!("Order response: {:?}", response);

    Ok(())
}
```

##### Place a limit order

```rust,ignore
use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::clob::types::Side;
use polymarket_client_sdk_v2::types::Decimal;
use rust_decimal_macros::dec;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let private_key = std::env::var(PRIVATE_KEY_VAR).expect("Need a private key");
    let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));
    let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    let order = client
        .limit_order()
        .token_id("<token-id>")
        .size(Decimal::ONE_HUNDRED)
        .price(dec!(0.1))
        .side(Side::Buy)
        .build()
        .await?;
    let signed_order = client.sign(&signer, order).await?;
    let response = client.post_order(signed_order).await?;
    println!("Order response: {:?}", response);

    Ok(())
}
```

##### V1 and V2 protocols

The SDK supports both V1 (legacy) and V2 exchange contracts. Protocol is **auto-detected** on
the first order build via `GET /version` and cached for the lifetime of the `Client`. You pick
the protocol by pointing the client at the corresponding host:

| Protocol | Host                              | Collateral   | EIP-712 domain version |
|----------|-----------------------------------|--------------|------------------------|
| V2       | `https://clob-v2.polymarket.com` | pUSD         | `"2"`                  |
| V1       | `https://clob.polymarket.com`     | USDC.e       | `"1"`                  |

V2 orders add `timestamp`, `metadata`, and `builder` fields. V1 orders use `taker`, `nonce`,
and `feeRateBps` instead. The order builder exposes both sets of fields — the ones that don't
apply to the detected protocol are silently ignored at build time, so you can write one
code-path that works against either server.

V2-specific builder fields (ignored when the server runs V1):

```rust,ignore
use polymarket_client_sdk_v2::types::B256;

let order = client
    .limit_order()
    .token_id("<token-id>")
    .size(Decimal::ONE_HUNDRED)
    .price(dec!(0.5))
    .side(Side::Buy)
    .metadata(B256::ZERO)           // 32 bytes of custom metadata
    .builder_code(B256::ZERO)       // builder fee attribution
    .defer_exec(false)              // defer execution
    .build()
    .await?;
```

V1-specific builder fields (ignored when the server runs V2):

```rust,ignore
use polymarket_client_sdk_v2::types::Address;

let order = client
    .limit_order()
    .token_id("<token-id>")
    .size(Decimal::ONE_HUNDRED)
    .price(dec!(0.5))
    .side(Side::Buy)
    .taker(Address::ZERO)           // explicit taker; default zero = public order
    .nonce(0)                       // on-chain cancel nonce; default 0
    .fee_rate_bps(0)                // must match the market rate when set
    .build()
    .await?;
```

#### Builder-attributed trading

In V2, builder attribution is carried on the order's `builder_code` field (and as a
query parameter on `builder_trades`). Set a default `builder_code` on the [`Config`] and
every order constructed via [`Client::limit_order`] / [`Client::market_order`] inherits
it unless overridden.

```rust,ignore
use std::str::FromStr as _;

use alloy::signers::Signer as _;
use alloy::signers::local::LocalSigner;
use polymarket_client_sdk_v2::clob::{Client, Config};
use polymarket_client_sdk_v2::clob::types::request::TradesRequest;
use polymarket_client_sdk_v2::types::B256;
use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let private_key = std::env::var(PRIVATE_KEY_VAR).expect("Need a private key");
    let builder_code = B256::from_str(&std::env::var("POLYMARKET_BUILDER_CODE")?)?;
    let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));

    let config = Config::builder().builder_code(builder_code).build();
    let client = Client::new("https://clob-v2.polymarket.com", config)?
        .authentication_builder(&signer)
        .authenticate()
        .await?;

    // Orders built below will carry the configured `builder_code`.
    let builder_trades = client
        .builder_trades(builder_code, &TradesRequest::default(), None)
        .await?;
    println!("Builder trades: {builder_trades:?}");

    Ok(())
}
```

### WebSocket Streaming

Real-time orderbook and user event streaming. Requires the `ws` feature.

```toml
polymarket_client_sdk_v2 = { version = "0.6", features = ["ws"] }
```

```rust,ignore
use std::str::FromStr as _;

use futures::StreamExt as _;
use polymarket_client_sdk_v2::clob::ws::Client;
use polymarket_client_sdk_v2::types::U256;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::default();

    // Subscribe to orderbook updates for specific assets.
    let asset_ids = vec![U256::from_str("<asset-id>")?];
    let stream = client.subscribe_orderbook(asset_ids)?;
    let mut stream = Box::pin(stream);

    while let Some(book_result) = stream.next().await {
        let book = book_result?;
        println!(
            "Orderbook update for {}: {} bids, {} asks",
            book.asset_id, book.bids.len(), book.asks.len()
        );
    }
    Ok(())
}
```

Available streams:
- `subscribe_orderbook()` - Bid/ask levels for assets
- `subscribe_prices()` - Price change events
- `subscribe_midpoints()` - Calculated midpoint prices
- `subscribe_orders()` - User order updates (authenticated)
- `subscribe_trades()` - User trade executions (authenticated)

See [`examples/clob/ws/`](examples/clob/ws/) for more WebSocket examples including authenticated user streams.

### Optional APIs

#### Data API
Trading analytics, positions, and leaderboards. Requires the `data` feature.

```rust,ignore
use polymarket_client_sdk_v2::data::Client;
use polymarket_client_sdk_v2::data::types::request::PositionsRequest;
use polymarket_client_sdk_v2::types::address;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::default();
    let user = address!("0x0000000000000000000000000000000000000000"); // Your address

    let request = PositionsRequest::builder().user(user).limit(10)?.build();
    let positions = client.positions(&request).await?;
    println!("Open positions: {:?}", positions);
    Ok(())
}
```

See [`examples/data.rs`](examples/data.rs) for trades, leaderboards, activity, and more.

#### Gamma API
Market and event discovery. Requires the `gamma` feature.

```rust,ignore
use polymarket_client_sdk_v2::gamma::Client;
use polymarket_client_sdk_v2::gamma::types::request::{EventsRequest, SearchRequest};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::default();

    // Find active events
    let request = EventsRequest::builder().active(true).limit(5).build();
    let events = client.events(&request).await?;
    println!("Found {} events", events.len());

    // Search for markets
    let search = SearchRequest::builder().q("bitcoin").build();
    let results = client.search(&search).await?;
    println!("Search results: {:?}", results);
    Ok(())
}
```

See [`examples/gamma.rs`](examples/gamma/client.rs) for tags, series, comments, and sports endpoints.

#### Bridge API
Cross-chain deposits from EVM chains, Solana, and Bitcoin. Requires the `bridge` feature.

```rust,ignore
use polymarket_client_sdk_v2::bridge::Client;
use polymarket_client_sdk_v2::bridge::types::DepositRequest;
use polymarket_client_sdk_v2::types::address;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::default();

    // Get deposit addresses for your wallet
    let request = DepositRequest::builder()
        .address(address!("0x0000000000000000000000000000000000000000")) // Your address
        .build();
    let response = client.deposit(&request).await?;

    println!("EVM: {}", response.address.evm);
    println!("Solana: {}", response.address.svm);
    println!("Bitcoin: {}", response.address.btc);
    Ok(())
}
```

See [`examples/bridge.rs`](examples/bridge.rs) for supported assets and minimum deposits.

## Additional CLOB Capabilities

Beyond basic order placement, the CLOB client supports:

- **Rewards & Earnings** - Query maker rewards, daily earnings, and reward percentages
- **Streaming Pagination** - `stream_data()` for iterating through large result sets
- **Batch Operations** - `post_orders()` and `cancel_orders()` for multiple orders at once
- **Order Scoring** - Check if orders qualify for maker rewards
- **Notifications** - Manage trading notifications
- **Balance Management** - Query and refresh balance/allowance caches
- **Geoblock Detection** - Check if trading is available in your region

See [`examples/clob/authenticated.rs`](examples/clob/authenticated.rs) for comprehensive usage.

## Token Allowances

### Do I need to set allowances?
MetaMask and EOA users must set token allowances.
If you are using a proxy or [Safe](https://help.safe.global/en/articles/40869-what-is-safe)-type wallet, then you do not.

### What are allowances?
Think of allowances as permissions. Before Polymarket can move your funds to execute trades, you need to give the
exchange contracts permission to access your USDC and conditional tokens.

### Quick Setup
You need to approve two types of tokens:
1. **USDC** (for deposits and trading)
2. **Conditional Tokens** (the outcome tokens you trade)

Each needs approval for the exchange contracts to work properly.

### Setting Allowances
Use [examples/approvals.rs](examples/approvals.rs) to approve the right contracts. Run once to approve USDC. Then change
the `TOKEN_TO_APPROVE` and run for each conditional token.

**Pro tip**: You only need to set these once per wallet. After that, you can trade freely.

## Minimum Supported Rust Version (MSRV)

**MSRV: Rust [1.88](https://releases.rs/docs/1.88.0/)**

Older versions *may* compile, but are not supported.

This project aims to maintain compatibility with a Rust version that is at least six months old.

Version updates may occur more frequently than the policy guideline states if external forces require it. For example,
a CVE in a downstream dependency requiring an MSRV bump would be considered an acceptable reason to violate the six-month
guideline.


## Contributing
We encourage contributions from the community. Check out our [contributing guidelines](.github/CONTRIBUTING.md) for
instructions on how to contribute to this SDK.


## About Polymarket
[Polymarket](https://docs.polymarket.com/polymarket-learn/get-started/what-is-polymarket) is the world’s largest prediction market, allowing you to stay informed and profit from your knowledge by
betting on future events across various topics.
Studies show prediction markets are often more accurate than pundits because they combine news, polls, and expert
opinions into a single value that represents the market’s view of an event’s odds. Our markets reflect accurate, unbiased,
and real-time probabilities for the events that matter most to you. Markets seek truth.
