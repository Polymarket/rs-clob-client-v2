use std::fmt;

use alloy::primitives::{B256, Signature, U256};
use bon::Builder;
use rust_decimal_macros::dec;
use serde::ser::{Error as _, SerializeStruct as _};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_repr::Serialize_repr;
use serde_with::{DisplayFromStr, serde_as};
use strum_macros::Display;

use crate::Result;
use crate::auth::ApiKey;
use crate::clob::order_builder::LOT_SIZE_SCALE;
use crate::clob::utilities::USDC_DECIMALS;
use crate::error::Error;
use crate::types::Decimal;

pub mod request;
pub mod response;

// Re-export RFQ types for convenient access
#[cfg(feature = "rfq")]
pub use request::{
    AcceptRfqQuoteRequest, ApproveRfqOrderRequest, CancelRfqQuoteRequest, CancelRfqRequestRequest,
    CreateRfqQuoteRequest, CreateRfqRequestRequest, RfqQuotesRequest, RfqRequestsRequest,
};
#[cfg(feature = "rfq")]
pub use response::{
    AcceptRfqQuoteResponse, ApproveRfqOrderResponse, CreateRfqQuoteResponse,
    CreateRfqRequestResponse, RfqQuote, RfqRequest,
};

#[non_exhaustive]
#[derive(
    Clone, Debug, Display, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
pub enum OrderType {
    /// Good 'til Cancelled; If not fully filled, the order rests on the book until it is explicitly
    /// cancelled.
    #[serde(alias = "gtc")]
    GTC,
    /// Fill or Kill; Order is attempted to be filled, in full, immediately. If it cannot be fully
    /// filled, the entire order is cancelled.
    #[default]
    #[serde(alias = "fok")]
    FOK,
    /// Good 'til Date; If not fully filled, the order rests on the book until the specified date.
    #[serde(alias = "gtd")]
    GTD,
    /// Fill and Kill; Order is attempted to be filled, however much is possible, immediately. If
    /// the order cannot be fully filled, the remaining quantity is cancelled.
    #[serde(alias = "fak")]
    FAK,
    /// Unknown order type from the API (captures the raw value for debugging).
    #[serde(untagged)]
    Unknown(String),
}

#[non_exhaustive]
#[derive(
    Clone, Copy, Debug, Display, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(rename_all = "UPPERCASE")]
#[strum(serialize_all = "UPPERCASE")]
#[repr(u8)]
pub enum Side {
    #[serde(alias = "buy")]
    Buy = 0,
    #[serde(alias = "sell")]
    Sell = 1,
    #[serde(other)]
    Unknown = 255,
}

impl TryFrom<u8> for Side {
    type Error = Error;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(Side::Buy),
            1 => Ok(Side::Sell),
            other => Err(Error::validation(format!(
                "Unable to create Side from {other}"
            ))),
        }
    }
}

/// Time interval for price history queries.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Display, Eq, PartialEq, Serialize, Deserialize)]
pub enum Interval {
    /// 1 minute
    #[serde(rename = "1m")]
    #[strum(serialize = "1m")]
    OneMinute,
    /// 1 hour
    #[serde(rename = "1h")]
    #[strum(serialize = "1h")]
    OneHour,
    /// 6 hours
    #[serde(rename = "6h")]
    #[strum(serialize = "6h")]
    SixHours,
    /// 1 day
    #[serde(rename = "1d")]
    #[strum(serialize = "1d")]
    OneDay,
    /// 1 week
    #[serde(rename = "1w")]
    #[strum(serialize = "1w")]
    OneWeek,
    /// Maximum available history
    #[serde(rename = "max")]
    #[strum(serialize = "max")]
    Max,
}

/// Time range specification for price history queries.
///
/// The CLOB API requires either an interval or explicit start/end timestamps.
/// This enum enforces that requirement at compile time.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(untagged)]
pub enum TimeRange {
    /// Use a predefined interval (e.g., last day, last week).
    Interval {
        /// The time interval.
        interval: Interval,
    },
    /// Use explicit start and end timestamps.
    #[serde(rename_all = "camelCase")]
    Range {
        /// Start timestamp (Unix seconds).
        start_ts: i64,
        /// End timestamp (Unix seconds).
        end_ts: i64,
    },
}

impl TimeRange {
    /// Create a time range from a predefined interval.
    #[must_use]
    pub const fn from_interval(interval: Interval) -> Self {
        Self::Interval { interval }
    }

    /// Create a time range from explicit timestamps.
    #[must_use]
    pub const fn from_range(start_ts: i64, end_ts: i64) -> Self {
        Self::Range { start_ts, end_ts }
    }
}

impl From<Interval> for TimeRange {
    fn from(interval: Interval) -> Self {
        Self::from_interval(interval)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum AmountInner {
    Usdc(Decimal),
    Shares(Decimal),
}

impl AmountInner {
    pub fn as_inner(&self) -> Decimal {
        match self {
            AmountInner::Usdc(d) | AmountInner::Shares(d) => *d,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Amount(pub(crate) AmountInner);

impl Amount {
    pub fn usdc(value: Decimal) -> Result<Amount> {
        let normalized = value.normalize();
        if normalized.scale() > USDC_DECIMALS {
            return Err(Error::validation(format!(
                "Unable to build Amount with {} decimal points, must be <= {USDC_DECIMALS}",
                normalized.scale()
            )));
        }

        Ok(Amount(AmountInner::Usdc(normalized)))
    }

    pub fn shares(value: Decimal) -> Result<Amount> {
        let normalized = value.normalize();
        if normalized.scale() > LOT_SIZE_SCALE {
            return Err(Error::validation(format!(
                "Unable to build Amount with {} decimal points, must be <= {LOT_SIZE_SCALE}",
                normalized.scale()
            )));
        }

        Ok(Amount(AmountInner::Shares(normalized)))
    }

    #[must_use]
    pub fn as_inner(&self) -> Decimal {
        self.0.as_inner()
    }

    #[must_use]
    pub fn is_usdc(&self) -> bool {
        matches!(self.0, AmountInner::Usdc(_))
    }

    #[must_use]
    pub fn is_shares(&self) -> bool {
        matches!(self.0, AmountInner::Shares(_))
    }
}

#[non_exhaustive]
#[derive(
    Clone,
    Copy,
    Display,
    Debug,
    Default,
    Eq,
    Ord,
    PartialEq,
    PartialOrd,
    Serialize_repr,
    Deserialize,
)]
#[repr(u8)]
pub enum SignatureType {
    #[default]
    Eoa = 0,
    Proxy = 1,
    GnosisSafe = 2,
    /// EIP-1271 smart contract wallet signatures (V2 orders only)
    Poly1271 = 3,
}

/// RFQ state filter for queries.
#[cfg(feature = "rfq")]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RfqState {
    /// Active requests/quotes
    #[default]
    Active,
    /// Inactive requests/quotes
    Inactive,
}

/// Sort field for RFQ queries.
#[cfg(feature = "rfq")]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RfqSortBy {
    /// Sort by price
    Price,
    /// Sort by expiry
    Expiry,
    /// Sort by size
    Size,
    /// Sort by creation time (default)
    #[default]
    Created,
}

/// Sort direction for RFQ queries.
#[cfg(feature = "rfq")]
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RfqSortDir {
    /// Ascending order (default)
    #[default]
    Asc,
    /// Descending order
    Desc,
}

#[non_exhaustive]
#[derive(Clone, Debug, Display, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[strum(serialize_all = "UPPERCASE")]
pub enum OrderStatusType {
    #[serde(alias = "live")]
    Live,
    #[serde(alias = "matched")]
    Matched,
    #[serde(alias = "canceled")]
    Canceled,
    #[serde(alias = "delayed")]
    Delayed,
    #[serde(alias = "unmatched")]
    Unmatched,
    /// Unknown order status type from the API (captures the raw value for debugging).
    #[serde(untagged)]
    Unknown(String),
}

#[non_exhaustive]
#[derive(Clone, Debug, Display, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[strum(serialize_all = "UPPERCASE")]
pub enum TradeStatusType {
    #[serde(alias = "matched")]
    Matched,
    #[serde(alias = "mined")]
    Mined,
    #[serde(alias = "confirmed")]
    Confirmed,
    #[serde(alias = "retrying")]
    Retrying,
    #[serde(alias = "failed")]
    Failed,
    /// Unknown trade status type from the API (captures the raw value for debugging).
    #[serde(untagged)]
    Unknown(String),
}

#[non_exhaustive]
#[derive(
    Clone, Debug, Default, Display, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(rename_all = "UPPERCASE")]
#[strum(serialize_all = "UPPERCASE")]
pub enum AssetType {
    #[default]
    Collateral,
    Conditional,
    /// Unknown asset type from the API (captures the raw value for debugging).
    #[serde(untagged)]
    Unknown(String),
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum TraderSide {
    Taker,
    Maker,
    /// Unknown trader side from the API (captures the raw value for debugging).
    #[serde(untagged)]
    Unknown(String),
}

/// Represents the maximum number of decimal places for an order's price field
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub enum TickSize {
    Tenth,
    Hundredth,
    Thousandth,
    TenThousandth,
}

impl fmt::Display for TickSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            TickSize::Tenth => "Tenth",
            TickSize::Hundredth => "Hundredth",
            TickSize::Thousandth => "Thousandth",
            TickSize::TenThousandth => "TenThousandth",
        };

        write!(f, "{name}({})", self.as_decimal())
    }
}

impl TickSize {
    #[must_use]
    pub fn as_decimal(&self) -> Decimal {
        match self {
            TickSize::Tenth => dec!(0.1),
            TickSize::Hundredth => dec!(0.01),
            TickSize::Thousandth => dec!(0.001),
            TickSize::TenThousandth => dec!(0.0001),
        }
    }
}

impl From<TickSize> for Decimal {
    fn from(tick_size: TickSize) -> Self {
        tick_size.as_decimal()
    }
}

impl TryFrom<Decimal> for TickSize {
    type Error = Error;

    fn try_from(value: Decimal) -> std::result::Result<Self, Self::Error> {
        match value {
            v if v == dec!(0.1) => Ok(TickSize::Tenth),
            v if v == dec!(0.01) => Ok(TickSize::Hundredth),
            v if v == dec!(0.001) => Ok(TickSize::Thousandth),
            v if v == dec!(0.0001) => Ok(TickSize::TenThousandth),
            other => Err(Error::validation(format!(
                "Unknown tick size: {other}. Expected one of: 0.1, 0.01, 0.001, 0.0001"
            ))),
        }
    }
}

impl PartialEq for TickSize {
    fn eq(&self, other: &Self) -> bool {
        self.as_decimal() == other.as_decimal()
    }
}

impl<'de> Deserialize<'de> for TickSize {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let dec = <Decimal as Deserialize>::deserialize(deserializer)?;
        TickSize::try_from(dec).map_err(de::Error::custom)
    }
}

// CLOB expects salt as a JSON number. U256 as an integer will not fit as a JSON number. Since
// we generated the salt as a u64 originally (see `salt_generator`), we can be very confident that
// we can invert the conversion to U256 and return a u64 when serializing.
fn ser_salt<S: Serializer>(value: &U256, serializer: S) -> std::result::Result<S::Ok, S::Error> {
    let v: u64 = value
        .try_into()
        .map_err(|e| S::Error::custom(format!("salt does not fit into u64: {e}")))?;
    serializer.serialize_u64(v)
}

// Each version is defined inside its own module so that `sol!` emits the
// Solidity type name `Order` for both — that is what the on-chain CTF Exchange
// contracts hash into their EIP-712 typehashes. Renaming the Rust struct would
// change the typehash and invalidate every signature.
mod v1 {
    use alloy::core::sol;

    use super::{DisplayFromStr, Serialize, ser_salt, serde_as};

    sol! {
        /// EIP-712 order struct for the legacy Polymarket CTF Exchange V1.
        ///
        /// `expiration` is part of the signed struct. Field order mirrors the
        /// on-chain contract's type hash and must not change.
        #[non_exhaustive]
        #[serde_as]
        #[derive(Serialize, Debug, Default, PartialEq)]
        struct Order {
            #[serde(serialize_with = "ser_salt")]
            uint256 salt;
            address maker;
            address signer;
            address taker;
            #[serde_as(as = "DisplayFromStr")]
            uint256 tokenId;
            #[serde_as(as = "DisplayFromStr")]
            uint256 makerAmount;
            #[serde_as(as = "DisplayFromStr")]
            uint256 takerAmount;
            #[serde_as(as = "DisplayFromStr")]
            uint256 expiration;
            #[serde_as(as = "DisplayFromStr")]
            uint256 nonce;
            #[serde_as(as = "DisplayFromStr")]
            uint256 feeRateBps;
            uint8   side;
            uint8   signatureType;
        }
    }
}

mod v2 {
    use alloy::core::sol;

    use super::{DisplayFromStr, Serialize, ser_salt, serde_as};

    sol! {
        /// EIP-712 order struct for the Polymarket CTF Exchange V2.
        ///
        /// `expiration` is NOT part of the signed struct; it travels on the outer JSON payload.
        #[non_exhaustive]
        #[serde_as]
        #[derive(Serialize, Debug, Default, PartialEq)]
        struct Order {
            #[serde(serialize_with = "ser_salt")]
            uint256 salt;
            address maker;
            address signer;
            #[serde_as(as = "DisplayFromStr")]
            uint256 tokenId;
            #[serde_as(as = "DisplayFromStr")]
            uint256 makerAmount;
            #[serde_as(as = "DisplayFromStr")]
            uint256 takerAmount;
            uint8   side;
            uint8   signatureType;
            #[serde_as(as = "DisplayFromStr")]
            uint256 timestamp;
            bytes32 metadata;
            bytes32 builder;
        }
    }
}

pub use v1::Order as OrderV1;
pub use v2::Order as OrderV2;

/// Deprecated alias preserved for callers that predate the V1/V2 split. Resolves to [`OrderV2`].
pub type Order = OrderV2;

/// V2 order payload: the signed struct plus the out-of-struct `expiration`.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrderPayloadV2 {
    pub order: OrderV2,
    pub expiration: U256,
}

/// V1 order payload. `expiration` lives inside the signed struct.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OrderPayloadV1 {
    pub order: OrderV1,
}

/// The order payload, version-tagged.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum OrderPayload {
    V1(OrderPayloadV1),
    V2(OrderPayloadV2),
}

impl Default for OrderPayload {
    fn default() -> Self {
        OrderPayload::V2(OrderPayloadV2::default())
    }
}

impl OrderPayload {
    /// Construct a V2 payload. Preserved for callers written against the V2-only API.
    #[must_use]
    pub fn new(order: OrderV2, expiration: U256) -> Self {
        OrderPayload::V2(OrderPayloadV2 { order, expiration })
    }

    /// Construct a V1 payload.
    #[must_use]
    pub fn new_v1(order: OrderV1) -> Self {
        OrderPayload::V1(OrderPayloadV1 { order })
    }

    /// The protocol version this payload targets (1 or 2).
    #[must_use]
    pub fn version(&self) -> u32 {
        match self {
            OrderPayload::V1(_) => 1,
            OrderPayload::V2(_) => 2,
        }
    }

    /// Returns the V2 order reference, or `None` for V1 payloads.
    #[must_use]
    pub fn as_v2(&self) -> Option<&OrderV2> {
        match self {
            OrderPayload::V2(p) => Some(&p.order),
            OrderPayload::V1(_) => None,
        }
    }

    /// Returns the V1 order reference, or `None` for V2 payloads.
    #[must_use]
    pub fn as_v1(&self) -> Option<&OrderV1> {
        match self {
            OrderPayload::V1(p) => Some(&p.order),
            OrderPayload::V2(_) => None,
        }
    }
}

impl SignableOrder {
    /// Returns the V2 order struct.
    ///
    /// # Panics
    ///
    /// Panics if this is a V1 order. Callers that may encounter either version should
    /// inspect [`SignableOrder::payload`] directly.
    #[must_use]
    pub fn order(&self) -> &OrderV2 {
        &self.v2().order
    }

    /// Returns the V2 payload.
    ///
    /// # Panics
    ///
    /// Panics if this is a V1 order.
    #[must_use]
    pub fn v2(&self) -> &OrderPayloadV2 {
        match &self.payload {
            OrderPayload::V2(p) => p,
            OrderPayload::V1(_) => panic!("SignableOrder is V1; match on .payload directly"),
        }
    }
}

impl SignedOrder {
    /// Returns the V2 order struct.
    ///
    /// # Panics
    ///
    /// Panics if this is a V1 order. Callers that may encounter either version should
    /// inspect [`SignedOrder::payload`] directly.
    #[must_use]
    pub fn order(&self) -> &OrderV2 {
        &self.v2().order
    }

    /// Returns the V2 payload.
    ///
    /// # Panics
    ///
    /// Panics if this is a V1 order.
    #[must_use]
    pub fn v2(&self) -> &OrderPayloadV2 {
        match &self.payload {
            OrderPayload::V2(p) => p,
            OrderPayload::V1(_) => panic!("SignedOrder is V1; match on .payload directly"),
        }
    }
}

#[non_exhaustive]
#[derive(Clone, Debug, Default, Builder, PartialEq)]
pub struct SignableOrder {
    pub payload: OrderPayload,
    pub order_type: OrderType,
    pub post_only: Option<bool>,
    pub defer_exec: Option<bool>,
}

#[non_exhaustive]
#[derive(Debug, Builder, PartialEq)]
pub struct SignedOrder {
    pub payload: OrderPayload,
    #[builder(into)]
    pub signature: OrderSignature,
    pub order_type: OrderType,
    pub owner: ApiKey,
    pub post_only: Option<bool>,
    pub defer_exec: Option<bool>,
}

/// Signature material attached to a signed order.
///
/// Most orders carry a normal 65-byte ECDSA signature. Deposit wallet orders
/// using [`SignatureType::Poly1271`] carry the longer wrapped signature that the
/// deposit wallet validates through EIP-1271.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum OrderSignature {
    Ecdsa(Signature),
    Wrapped(String),
}

impl OrderSignature {
    /// Returns the ECDSA `r` value for a normal order signature.
    ///
    /// # Panics
    ///
    /// Panics when called on a wrapped deposit wallet signature.
    #[must_use]
    pub fn r(&self) -> U256 {
        match self {
            OrderSignature::Ecdsa(sig) => sig.r(),
            OrderSignature::Wrapped(_) => {
                panic!("wrapped deposit wallet signatures do not expose an ECDSA r value")
            }
        }
    }

    /// Returns the ECDSA `s` value for a normal order signature.
    ///
    /// # Panics
    ///
    /// Panics when called on a wrapped deposit wallet signature.
    #[must_use]
    pub fn s(&self) -> U256 {
        match self {
            OrderSignature::Ecdsa(sig) => sig.s(),
            OrderSignature::Wrapped(_) => {
                panic!("wrapped deposit wallet signatures do not expose an ECDSA s value")
            }
        }
    }
}

impl From<Signature> for OrderSignature {
    fn from(signature: Signature) -> Self {
        OrderSignature::Ecdsa(signature)
    }
}

impl From<String> for OrderSignature {
    fn from(signature: String) -> Self {
        OrderSignature::Wrapped(signature)
    }
}

impl From<&str> for OrderSignature {
    fn from(signature: &str) -> Self {
        OrderSignature::Wrapped(signature.to_owned())
    }
}

impl fmt::Display for OrderSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrderSignature::Ecdsa(sig) => write!(f, "{sig}"),
            OrderSignature::Wrapped(sig) => f.write_str(sig),
        }
    }
}

impl PartialEq<Signature> for OrderSignature {
    fn eq(&self, other: &Signature) -> bool {
        match self {
            OrderSignature::Ecdsa(sig) => sig == other,
            OrderSignature::Wrapped(_) => false,
        }
    }
}

/// V2 `order` body with the signature folded in.
#[serde_as]
#[derive(Serialize)]
struct OrderV2WithSignature<'order> {
    #[serde(serialize_with = "ser_salt")]
    salt: &'order U256,
    maker: &'order alloy::primitives::Address,
    signer: &'order alloy::primitives::Address,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(rename = "tokenId")]
    token_id: &'order U256,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(rename = "makerAmount")]
    maker_amount: &'order U256,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(rename = "takerAmount")]
    taker_amount: &'order U256,
    side: Side,
    #[serde_as(as = "DisplayFromStr")]
    expiration: &'order U256,
    #[serde(rename = "signatureType")]
    signature_type: u8,
    #[serde_as(as = "DisplayFromStr")]
    timestamp: &'order U256,
    metadata: &'order B256,
    builder: &'order B256,
    signature: String,
}

/// V1 `order` body with the signature folded in.
#[serde_as]
#[derive(Serialize)]
struct OrderV1WithSignature<'order> {
    #[serde(serialize_with = "ser_salt")]
    salt: &'order U256,
    maker: &'order alloy::primitives::Address,
    signer: &'order alloy::primitives::Address,
    taker: &'order alloy::primitives::Address,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(rename = "tokenId")]
    token_id: &'order U256,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(rename = "makerAmount")]
    maker_amount: &'order U256,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(rename = "takerAmount")]
    taker_amount: &'order U256,
    side: Side,
    #[serde_as(as = "DisplayFromStr")]
    expiration: &'order U256,
    #[serde_as(as = "DisplayFromStr")]
    nonce: &'order U256,
    #[serde_as(as = "DisplayFromStr")]
    #[serde(rename = "feeRateBps")]
    fee_rate_bps: &'order U256,
    #[serde(rename = "signatureType")]
    signature_type: u8,
    signature: String,
}

// The CLOB expects the signature folded into the inner `order` object. Shape differs
// between V1 and V2; the outer wrapper (order / orderType / owner / postOnly / deferExec)
// is identical.
impl Serialize for SignedOrder {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut field_count = 3;
        if self.post_only.is_some() {
            field_count += 1;
        }
        if self.defer_exec.is_some() {
            field_count += 1;
        }
        let mut st = serializer.serialize_struct("SignedOrder", field_count)?;

        match &self.payload {
            OrderPayload::V2(payload) => {
                let order = &payload.order;
                let side = Side::try_from(order.side).map_err(S::Error::custom)?;
                let body = OrderV2WithSignature {
                    salt: &order.salt,
                    maker: &order.maker,
                    signer: &order.signer,
                    token_id: &order.tokenId,
                    maker_amount: &order.makerAmount,
                    taker_amount: &order.takerAmount,
                    side,
                    expiration: &payload.expiration,
                    signature_type: order.signatureType,
                    timestamp: &order.timestamp,
                    metadata: &order.metadata,
                    builder: &order.builder,
                    signature: self.signature.to_string(),
                };
                st.serialize_field("order", &body)?;
            }
            OrderPayload::V1(payload) => {
                let order = &payload.order;
                let side = Side::try_from(order.side).map_err(S::Error::custom)?;
                let body = OrderV1WithSignature {
                    salt: &order.salt,
                    maker: &order.maker,
                    signer: &order.signer,
                    taker: &order.taker,
                    token_id: &order.tokenId,
                    maker_amount: &order.makerAmount,
                    taker_amount: &order.takerAmount,
                    side,
                    expiration: &order.expiration,
                    nonce: &order.nonce,
                    fee_rate_bps: &order.feeRateBps,
                    signature_type: order.signatureType,
                    signature: self.signature.to_string(),
                };
                st.serialize_field("order", &body)?;
            }
        }

        st.serialize_field("orderType", &self.order_type)?;
        st.serialize_field("owner", &self.owner)?;
        if let Some(post_only) = self.post_only {
            st.serialize_field("postOnly", &post_only)?;
        }
        if let Some(defer_exec) = self.defer_exec {
            st.serialize_field("deferExec", &defer_exec)?;
        }

        st.end()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::to_value;

    use super::*;
    use crate::error::Validation;

    #[test]
    fn tick_size_decimals_should_succeed() {
        assert_eq!(TickSize::Tenth.as_decimal().scale(), 1);
        assert_eq!(TickSize::Hundredth.as_decimal().scale(), 2);
        assert_eq!(TickSize::Thousandth.as_decimal().scale(), 3);
        assert_eq!(TickSize::TenThousandth.as_decimal().scale(), 4);
    }

    #[test]
    fn tick_size_should_display() {
        assert_eq!(format!("{}", TickSize::Tenth), "Tenth(0.1)");
        assert_eq!(format!("{}", TickSize::Hundredth), "Hundredth(0.01)");
        assert_eq!(format!("{}", TickSize::Thousandth), "Thousandth(0.001)");
        assert_eq!(
            format!("{}", TickSize::TenThousandth),
            "TenThousandth(0.0001)"
        );
    }

    #[test]
    fn tick_from_decimal_should_succeed() {
        assert_eq!(
            TickSize::try_from(dec!(0.0001)).unwrap(),
            TickSize::TenThousandth
        );
        assert_eq!(
            TickSize::try_from(dec!(0.001)).unwrap(),
            TickSize::Thousandth
        );
        assert_eq!(TickSize::try_from(dec!(0.01)).unwrap(), TickSize::Hundredth);
        assert_eq!(TickSize::try_from(dec!(0.1)).unwrap(), TickSize::Tenth);
    }

    #[test]
    fn non_standard_decimal_to_tick_size_should_fail() {
        let result = TickSize::try_from(Decimal::ONE);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Unknown tick size: 1")
        );
    }

    #[test]
    fn amount_should_succeed() -> Result<()> {
        let usdc = Amount::usdc(Decimal::ONE_HUNDRED)?;
        assert!(usdc.is_usdc());
        assert_eq!(usdc.as_inner(), Decimal::ONE_HUNDRED);

        let shares = Amount::shares(Decimal::ONE_HUNDRED)?;
        assert!(shares.is_shares());
        assert_eq!(shares.as_inner(), Decimal::ONE_HUNDRED);

        Ok(())
    }

    #[test]
    fn improper_shares_lot_size_should_fail() {
        let Err(err) = Amount::shares(dec!(0.23400)) else {
            panic!()
        };

        let message = err.downcast_ref::<Validation>().unwrap();
        assert_eq!(
            message.reason,
            format!("Unable to build Amount with 3 decimal points, must be <= {LOT_SIZE_SCALE}")
        );
    }

    #[test]
    fn improper_usdc_decimal_size_should_fail() {
        let Err(err) = Amount::usdc(dec!(0.2340011)) else {
            panic!()
        };

        let message = err.downcast_ref::<Validation>().unwrap();
        assert_eq!(
            message.reason,
            format!("Unable to build Amount with 7 decimal points, must be <= {USDC_DECIMALS}")
        );
    }

    #[test]
    fn side_to_string_should_succeed() {
        assert_eq!(Side::Buy.to_string(), "BUY");
        assert_eq!(Side::Sell.to_string(), "SELL");
    }

    #[test]
    fn order_type_deserialize_known_variants() {
        // Test that known variants still deserialize correctly
        assert_eq!(
            serde_json::from_str::<OrderType>(r#""GTC""#).unwrap(),
            OrderType::GTC
        );
        assert_eq!(
            serde_json::from_str::<OrderType>(r#""gtc""#).unwrap(),
            OrderType::GTC
        );
        assert_eq!(
            serde_json::from_str::<OrderType>(r#""FOK""#).unwrap(),
            OrderType::FOK
        );
    }

    #[test]
    fn order_type_deserialize_unknown_variant() {
        // Test that unknown variants are captured
        let result = serde_json::from_str::<OrderType>(r#""NEW_ORDER_TYPE""#).unwrap();
        assert_eq!(result, OrderType::Unknown("NEW_ORDER_TYPE".to_owned()));
    }

    #[test]
    fn order_status_type_deserialize_known_variants() {
        assert_eq!(
            serde_json::from_str::<OrderStatusType>(r#""LIVE""#).unwrap(),
            OrderStatusType::Live
        );
        assert_eq!(
            serde_json::from_str::<OrderStatusType>(r#""live""#).unwrap(),
            OrderStatusType::Live
        );
    }

    #[test]
    fn order_status_type_deserialize_unknown_variant() {
        let result = serde_json::from_str::<OrderStatusType>(r#""NEW_STATUS""#).unwrap();
        assert_eq!(result, OrderStatusType::Unknown("NEW_STATUS".to_owned()));
    }

    #[test]
    fn order_type_display_known_variants() {
        assert_eq!(format!("{}", OrderType::GTC), "GTC");
        assert_eq!(format!("{}", OrderType::FOK), "FOK");
    }

    #[test]
    fn order_type_display_unknown_variant() {
        // strum Display will show the variant name + contents for tuple variants
        let unknown = OrderType::Unknown("NEW_TYPE".to_owned());
        let display = format!("{unknown}");
        // Just verify it displays something reasonable (contains the inner value)
        assert!(display.contains("Unknown") || display.contains("NEW_TYPE"));
    }

    #[test]
    fn signed_order_serialization_omits_post_only_when_none() {
        let signed_order = SignedOrder {
            payload: OrderPayload::default(),
            signature: Signature::new(U256::ZERO, U256::ZERO, false).into(),
            order_type: OrderType::GTC,
            owner: ApiKey::nil(),
            post_only: None,
            defer_exec: None,
        };

        let value = to_value(&signed_order).expect("serialize SignedOrder");
        let object = value
            .as_object()
            .expect("SignedOrder should serialize to an object");

        assert!(!object.contains_key("postOnly"));
        assert!(!object.contains_key("deferExec"));
    }

    #[test]
    fn signed_order_serialization_includes_fields() {
        let signed_order = SignedOrder {
            payload: OrderPayload::default(),
            signature: Signature::new(U256::ZERO, U256::ZERO, false).into(),
            order_type: OrderType::GTC,
            owner: ApiKey::nil(),
            post_only: None,
            defer_exec: Some(false),
        };

        let value = to_value(&signed_order).expect("serialize SignedOrder");
        let object = value
            .as_object()
            .expect("SignedOrder should serialize to an object");

        let order_obj = object.get("order").unwrap().as_object().unwrap();
        assert!(order_obj.contains_key("timestamp"));
        assert!(order_obj.contains_key("metadata"));
        assert!(order_obj.contains_key("builder"));
        assert!(order_obj.contains_key("expiration"));
        assert!(!order_obj.contains_key("taker"));
        assert!(!order_obj.contains_key("nonce"));
        assert!(!order_obj.contains_key("feeRateBps"));
        // deferExec should be present
        assert!(object.contains_key("deferExec"));
    }

    #[test]
    fn signed_order_serialization_uses_wrapped_signature() {
        let signed_order = SignedOrder {
            payload: OrderPayload::default(),
            signature: OrderSignature::Wrapped("0xwrapped".to_owned()),
            order_type: OrderType::GTC,
            owner: ApiKey::nil(),
            post_only: None,
            defer_exec: None,
        };

        let value = to_value(&signed_order).expect("serialize SignedOrder");
        let order_obj = value
            .as_object()
            .and_then(|object| object.get("order"))
            .and_then(serde_json::Value::as_object)
            .expect("order object");

        assert_eq!(order_obj["signature"], "0xwrapped");
    }
}
