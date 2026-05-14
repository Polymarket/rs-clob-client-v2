//! Polymarket CLOB (Central Limit Order Book) API client and types.
//!
//! **Feature flag:** None (this is the core module, always available)
//!
//! This module provides the primary client for interacting with the Polymarket CLOB API,
//! which handles all trading operations including order placement, cancellation, market
//! data queries, and account management.
//!
//! # Overview
//!
//! The CLOB API is the main trading interface for Polymarket. It supports both
//! authenticated and unauthenticated operations:
//!
//! - **Unauthenticated**: Market data, pricing, orderbooks, health checks
//! - **Authenticated**: Order placement/cancellation, balances, API keys, rewards
//! - **Builder Authentication**: Special endpoints for market makers and builders
//!
//! ## Orders
//!
//! This SDK uses the V2 CTF Exchange contract. Order fields include:
//! `timestamp`, `metadata` (bytes32), `builder` (bytes32 for fee attribution).
//! EIP-712 domain version is `"2"`.
//! Supports [`Poly1271`](types::SignatureType::Poly1271) signature type for EIP-1271 smart contract wallets.
//! Supports `deferExec` on order submission to defer execution.
//!
//! ## Public Endpoints (No Authentication Required)
//!
//! | Endpoint | Description |
//! |----------|-------------|
//! | `/` | Health check - returns "OK" |
//! | `/time` | Current server timestamp |
//! | `/version` | API version (1 or 2) |
//! | `/midpoint` | Mid-market price for a token |
//! | `/midpoints` | Batch midpoint prices |
//! | `/price` | Best bid or ask price |
//! | `/prices` | Batch best prices |
//! | `/spread` | Bid-ask spread |
//! | `/spreads` | Batch spreads |
//! | `/last-trade-price` | Most recent trade price |
//! | `/last-trades-prices` | Batch last trade prices |
//! | `/prices-all` | All token prices |
//! | `/tick-size` | Minimum price increment (cached) |
//! | `/neg-risk` | `NegRisk` adapter flag (cached) |
//! | `/fee-rate-bps` | Trading fee in basis points (cached) |
//! | `/book` | Full orderbook depth |
//! | `/books` | Batch orderbooks |
//! | `/market` | Single market details |
//! | `/markets` | All markets (paginated) |
//! | `/sampling-markets` | Sampling program markets |
//! | `/simplified-markets` | Markets with reduced detail |
//! | `/sampling-simplified-markets` | Simplified sampling markets |
//! | `/prices-history` | Historical price data |
//! | `/geoblock` | Geographic restriction check |
//!
//! ## Authenticated Endpoints
//!
//! | Endpoint | Description |
//! |----------|-------------|
//! | `/order` | Place a new order |
//! | `/cancel` | Cancel an order |
//! | `/cancel-market-orders` | Cancel all orders in a market |
//! | `/cancel-all` | Cancel all orders |
//! | `/orders` | Get user's orders |
//! | `/trades` | Get user's trade history |
//! | `/balances` | Get USDC balances and allowances |
//! | `/api-keys` | List API keys |
//! | `/create-api-key` | Create new API key |
//! | `/delete-api-key` | Delete an API key |
//! | `/notifications` | Get notifications |
//! | `/mark-notifications-as-read` | Mark notifications read |
//! | `/drop-notifications` | Delete notifications |
//! | `/rewards/current` | Current rewards info |
//! | `/rewards/percentages` | Rewards percentages |
//! | `/order-scoring` | Order score for rewards |
//! | `/ban` | Check ban status |
//!
//! # Examples
//!
//! ## Unauthenticated Client
//!
//! ```rust,no_run
//! use std::str::FromStr as _;
//!
//! use polymarket_client_sdk_v2::clob::{Client, Config};
//! use polymarket_client_sdk_v2::clob::types::request::MidpointRequest;
//! use polymarket_client_sdk_v2::types::U256;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create an unauthenticated client
//! let client = Client::new("https://clob-v2.polymarket.com", Config::default())?;
//!
//! // Check API health
//! let status = client.ok().await?;
//! println!("Status: {status}");
//!
//! // Get midpoint price for a token
//! let request = MidpointRequest::builder()
//!     .token_id(U256::from_str("15871154585880608648532107628464183779895785213830018178010423617714102767076")?)
//!     .build();
//! let midpoint = client.midpoint(&request).await?;
//! println!("Midpoint: {}", midpoint.mid);
//! # Ok(())
//! # }
//! ```
//!
//! ## Authenticated Client
//!
//! ```rust,no_run
//! use std::str::FromStr as _;
//!
//! use alloy::signers::Signer;
//! use alloy::signers::local::LocalSigner;
//! use polymarket_client_sdk_v2::{POLYGON, PRIVATE_KEY_VAR};
//! use polymarket_client_sdk_v2::clob::{Client, Config};
//! use polymarket_client_sdk_v2::clob::types::{Side, SignedOrder};
//! use polymarket_client_sdk_v2::types::{dec, Decimal, U256};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create signer from private key
//! let private_key = std::env::var(PRIVATE_KEY_VAR)?;
//! let signer = LocalSigner::from_str(&private_key)?.with_chain_id(Some(POLYGON));
//!
//! let client = Client::new("https://clob-v2.polymarket.com", Config::default())?
//!     .authentication_builder(&signer)
//!     .authenticate()
//!     .await?;
//!
//! let order = client
//!     .limit_order()
//!     .token_id(U256::from_str("15871154585880608648532107628464183779895785213830018178010423617714102767076")?)
//!     .side(Side::Buy)
//!     .price(dec!(0.5))
//!     .size(Decimal::TEN)
//!     .build()
//!     .await?;
//!
//! let signed_order = client.sign(&signer, order).await?;
//! let response = client.post_order(signed_order).await?;
//! println!("Order ID: {}", response.order_id);
//! # Ok(())
//! # }
//! ```
//!
//! # Optional Features
//!
//! - **`ws`**: Enables WebSocket support for real-time orderbook and trade streams
//! - **`heartbeats`**: Enables automatic heartbeat mechanism for authenticated sessions
//! - **`tracing`**: Enables detailed request/response tracing
//! - **`rfq`**: Enables RFQ (Request for Quote) endpoints for institutional trading
//!
//! # API Base URL
//!
//! The default API endpoint is `https://clob-v2.polymarket.com`.

pub mod client;
pub mod order_builder;
pub mod types;
pub mod utilities;
#[cfg(feature = "ws")]
pub mod ws;

pub use client::{Client, Config};
