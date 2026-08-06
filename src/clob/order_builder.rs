use std::marker::PhantomData;
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::{B256, U256};
use alloy::signers::Signer;
use chrono::{DateTime, Utc};
use rand::RngExt as _;
use rust_decimal::prelude::ToPrimitive as _;

use crate::Result;
use crate::auth::Kind as AuthKind;
use crate::auth::state::Authenticated;
use crate::clob::Client;
use crate::clob::types::request::OrderBookSummaryRequest;
use crate::clob::types::response::PostOrderResponse;
use crate::clob::types::{
    Amount, AmountInner, OrderPayload, OrderType, OrderV1, OrderV2, Side, SignableOrder,
    SignatureType,
};
use crate::clob::utilities::USDC_DECIMALS;
use crate::error::Error;
use crate::types::{Address, Decimal};

/// Maximum number of decimal places for `size`
pub(crate) const LOT_SIZE_SCALE: u32 = 2;

/// Placeholder type for compile-time checks on limit order builders
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct Limit;

/// Placeholder type for compile-time checks on market order builders
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct Market;

/// Used to create an order iteratively and ensure validity with respect to its order kind.
#[derive(Clone, Debug)]
pub struct OrderBuilder<OrderKind, K: AuthKind> {
    pub(crate) client: Client<Authenticated<K>>,
    pub(crate) signer: Address,
    pub(crate) signature_type: SignatureType,
    pub(crate) salt_generator: fn() -> u64,
    pub(crate) token_id: Option<U256>,
    pub(crate) price: Option<Decimal>,
    pub(crate) size: Option<Decimal>,
    pub(crate) amount: Option<Amount>,
    pub(crate) side: Option<Side>,
    pub(crate) expiration: Option<DateTime<Utc>>,
    pub(crate) order_type: Option<OrderType>,
    pub(crate) post_only: Option<bool>,
    pub(crate) funder: Option<Address>,
    pub(crate) metadata: Option<B256>,
    pub(crate) builder_code: Option<B256>,
    pub(crate) defer_exec: Option<bool>,
    pub(crate) user_usdc_balance: Option<Decimal>,
    /// V1-only: explicit taker address. Defaults to the zero address (public order).
    pub(crate) taker: Option<Address>,
    /// V1-only: on-chain cancel nonce. Defaults to 0.
    pub(crate) nonce: Option<u64>,
    /// V1-only: caller-specified fee rate in bps. Must match the market rate when both are set.
    pub(crate) fee_rate_bps: Option<u32>,
    pub(crate) _kind: PhantomData<OrderKind>,
}

impl<OrderKind, K: AuthKind> OrderBuilder<OrderKind, K> {
    /// Sets the `token_id` for this builder. This is a required field.
    #[must_use]
    pub fn token_id(mut self, token_id: U256) -> Self {
        self.token_id = Some(token_id);
        self
    }

    /// Sets the [`Side`] for this builder. This is a required field.
    #[must_use]
    pub fn side(mut self, side: Side) -> Self {
        self.side = Some(side);
        self
    }

    #[must_use]
    pub fn expiration(mut self, expiration: DateTime<Utc>) -> Self {
        self.expiration = Some(expiration);
        self
    }

    #[must_use]
    pub fn order_type(mut self, order_type: OrderType) -> Self {
        self.order_type = Some(order_type);
        self
    }

    /// Sets the `postOnly` flag for this builder.
    #[must_use]
    pub fn post_only(mut self, post_only: bool) -> Self {
        self.post_only = Some(post_only);
        self
    }

    /// Sets the metadata field (bytes32). Defaults to zero.
    #[must_use]
    pub fn metadata(mut self, metadata: B256) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Sets the builder code for fee attribution (bytes32). Defaults to zero.
    #[must_use]
    pub fn builder_code(mut self, builder_code: B256) -> Self {
        self.builder_code = Some(builder_code);
        self
    }

    /// Sets the `deferExec` flag.
    #[must_use]
    pub fn defer_exec(mut self, defer_exec: bool) -> Self {
        self.defer_exec = Some(defer_exec);
        self
    }

    /// V1-only: sets the order's `taker` address. Defaults to the zero address (public order).
    /// Ignored when the server is running V2.
    #[must_use]
    pub fn taker(mut self, taker: Address) -> Self {
        self.taker = Some(taker);
        self
    }

    /// V1-only: sets the on-chain cancellation `nonce`. Defaults to 0.
    /// Ignored when the server is running V2.
    #[must_use]
    pub fn nonce(mut self, nonce: u64) -> Self {
        self.nonce = Some(nonce);
        self
    }

    /// V1-only: sets the maker `feeRateBps`. When set, must equal the market's fee rate
    /// (see `/fee-rate`) or `build()` rejects the order. Ignored when the server is running V2.
    #[must_use]
    pub fn fee_rate_bps(mut self, fee_rate_bps: u32) -> Self {
        self.fee_rate_bps = Some(fee_rate_bps);
        self
    }

    /// Assembles the [`OrderPayload`] for the server's current protocol version.
    ///
    /// The caller supplies values common to both versions; V1/V2-specific fields
    /// (`taker`/`nonce`/`feeRateBps` vs `timestamp`/`metadata`/`builder`) are resolved
    /// here from [`OrderBuilder`] state.
    async fn build_payload(
        &self,
        token_id: U256,
        side: Side,
        maker_amount: u128,
        taker_amount: u128,
        salt: u64,
        expiration: U256,
    ) -> Result<OrderPayload> {
        let version = self.client.resolve_version(false).await?;
        let maker = self.funder.unwrap_or(self.signer);
        let signer = if matches!(self.signature_type, SignatureType::Poly1271) {
            self.funder.ok_or_else(|| {
                Error::validation(
                    "A deposit wallet funder address is required with a Poly1271 signature type",
                )
            })?
        } else {
            self.signer
        };

        match version {
            1 => {
                if matches!(self.signature_type, SignatureType::Poly1271) {
                    return Err(Error::validation(
                        "signature type POLY_1271 is not supported for V1 orders",
                    ));
                }
                let fee_rate_bps = self
                    .client
                    .resolve_fee_rate_bps(token_id, self.fee_rate_bps)
                    .await?;
                Ok(OrderPayload::new_v1(OrderV1 {
                    salt: U256::from(salt),
                    maker,
                    signer: self.signer,
                    taker: self.taker.unwrap_or(Address::ZERO),
                    tokenId: token_id,
                    makerAmount: U256::from(maker_amount),
                    takerAmount: U256::from(taker_amount),
                    expiration,
                    nonce: U256::from(self.nonce.unwrap_or(0)),
                    feeRateBps: U256::from(fee_rate_bps),
                    side: side as u8,
                    signatureType: self.signature_type as u8,
                }))
            }
            2 => {
                let timestamp_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("time went backwards")
                    .as_millis();
                Ok(OrderPayload::new(
                    OrderV2 {
                        salt: U256::from(salt),
                        maker,
                        signer,
                        tokenId: token_id,
                        makerAmount: U256::from(maker_amount),
                        takerAmount: U256::from(taker_amount),
                        side: side as u8,
                        signatureType: self.signature_type as u8,
                        timestamp: U256::from(timestamp_ms),
                        metadata: self.metadata.unwrap_or(B256::ZERO),
                        builder: self.builder_code.unwrap_or(B256::ZERO),
                    },
                    expiration,
                ))
            }
            other => Err(Error::validation(format!(
                "unsupported CLOB protocol version: {other}"
            ))),
        }
    }
}

impl<K: AuthKind> OrderBuilder<Limit, K> {
    /// Sets the price for this limit builder. This is a required field.
    #[must_use]
    pub fn price(mut self, price: Decimal) -> Self {
        self.price = Some(price);
        self
    }

    /// Sets the size for this limit builder. This is a required field.
    #[must_use]
    pub fn size(mut self, size: Decimal) -> Self {
        self.size = Some(size);
        self
    }

    /// Validates and transforms this limit builder into a [`SignableOrder`]
    ///
    /// # Panics
    ///
    /// Panics if the system clock is before the Unix epoch.
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip(self), err(level = "warn"))
    )]
    pub async fn build(self) -> Result<SignableOrder> {
        let Some(token_id) = self.token_id else {
            return Err(Error::validation(
                "Unable to build Order due to missing token ID",
            ));
        };

        let Some(side) = self.side else {
            return Err(Error::validation(
                "Unable to build Order due to missing token side",
            ));
        };

        let Some(price) = self.price else {
            return Err(Error::validation(
                "Unable to build Order due to missing price",
            ));
        };

        if price.is_sign_negative() {
            return Err(Error::validation(format!(
                "Unable to build Order due to negative price {price}"
            )));
        }

        let minimum_tick_size = self
            .client
            .tick_size(token_id)
            .await?
            .minimum_tick_size
            .as_decimal();

        let decimals = minimum_tick_size.scale();

        if price.scale() > minimum_tick_size.scale() {
            return Err(Error::validation(format!(
                "Unable to build Order: Price {price} has {} decimal places. Minimum tick size \
                {minimum_tick_size} has {} decimal places. Price decimal places <= minimum tick size decimal places",
                price.scale(),
                minimum_tick_size.scale()
            )));
        }

        if price < minimum_tick_size || price > Decimal::ONE - minimum_tick_size {
            return Err(Error::validation(format!(
                "Price {price} is too small or too large for the minimum tick size {minimum_tick_size}"
            )));
        }

        if !price_aligned_to_tick_size(price, minimum_tick_size) {
            return Err(Error::validation(format!(
                "Price {price} is not aligned to the minimum tick size {minimum_tick_size}"
            )));
        }

        let Some(size) = self.size else {
            return Err(Error::validation(
                "Unable to build Order due to missing size",
            ));
        };

        if size.scale() > LOT_SIZE_SCALE {
            return Err(Error::validation(format!(
                "Unable to build Order: Size {size} has {} decimal places. Maximum lot size is {LOT_SIZE_SCALE}",
                size.scale()
            )));
        }

        if size.is_zero() || size.is_sign_negative() {
            return Err(Error::validation(format!(
                "Unable to build Order due to negative size {size}"
            )));
        }

        let expiration = self.expiration.unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
        let order_type = self.order_type.clone().unwrap_or(OrderType::GTC);
        let post_only = Some(self.post_only.unwrap_or(false));

        if !matches!(order_type, OrderType::GTD) && expiration > DateTime::<Utc>::UNIX_EPOCH {
            return Err(Error::validation(
                "Only GTD orders may have a non-zero expiration",
            ));
        }

        if post_only == Some(true) && !matches!(order_type, OrderType::GTC | OrderType::GTD) {
            return Err(Error::validation(
                "postOnly is only supported for GTC and GTD orders",
            ));
        }

        let (taker_amount, maker_amount) = match side {
            Side::Buy => (
                size,
                (size * price).trunc_with_scale(decimals + LOT_SIZE_SCALE),
            ),
            Side::Sell => (
                (size * price).trunc_with_scale(decimals + LOT_SIZE_SCALE),
                size,
            ),
            side => return Err(Error::validation(format!("Invalid side: {side}"))),
        };

        let salt = to_ieee_754_int((self.salt_generator)());
        let expiration_u256 = U256::from(expiration.timestamp().to_u64().ok_or(
            Error::validation(format!(
                "Unable to represent expiration {expiration} as a u64"
            )),
        )?);

        let payload = self
            .build_payload(
                token_id,
                side,
                to_fixed_u128(maker_amount),
                to_fixed_u128(taker_amount),
                salt,
                expiration_u256,
            )
            .await?;

        #[cfg(feature = "tracing")]
        tracing::debug!(token_id = %token_id, side = ?side, price = %price, size = %size, "limit order built");

        Ok(SignableOrder {
            payload,
            order_type,
            post_only,
            defer_exec: self.defer_exec,
        })
    }

    /// Convenience: builds, signs, and posts this limit order in a single call.
    ///
    /// If the server rejects the order due to a version mismatch, this automatically retries
    /// once with the updated version — rebuilding and re-signing the order from scratch.
    ///
    /// # Errors
    ///
    /// Returns an error if any of the build, sign, or post steps fails.
    pub async fn build_sign_and_post<S: Signer>(self, signer: &S) -> Result<PostOrderResponse> {
        let client = self.client.clone();
        let before_version = client.resolve_version(false).await.unwrap_or(0);
        let retry = self.clone();
        let order = self.build().await?;
        let signed = client.sign(signer, order).await?;
        let result = client.post_order(signed).await;
        if let Err(err) = &result
            && let Some(status) = err.downcast_ref::<crate::error::Status>()
            && status
                .message
                .contains(crate::clob::client::ORDER_VERSION_MISMATCH_ERROR)
        {
            let after_version = client.resolve_version(false).await.unwrap_or(0);
            if after_version != before_version {
                let order = retry.build().await?;
                let signed = client.sign(signer, order).await?;
                return client.post_order(signed).await;
            }
        }
        result
    }
}

impl<K: AuthKind> OrderBuilder<Market, K> {
    /// Sets the price for this market builder. This is an optional field.
    #[must_use]
    pub fn price(mut self, price: Decimal) -> Self {
        self.price = Some(price);
        self
    }

    /// Sets the [`Amount`] for this market order. This is a required field.
    #[must_use]
    pub fn amount(mut self, amount: Amount) -> Self {
        self.amount = Some(amount);
        self
    }

    /// Sets the user's USDC balance. When set on a BUY market order, `build()` shrinks
    /// the USDC amount to cover platform + builder taker fees so the order stays within
    /// the user's balance.
    #[must_use]
    pub fn user_usdc_balance(mut self, balance: Decimal) -> Self {
        self.user_usdc_balance = Some(balance);
        self
    }

    // Attempts to calculate the market price from the top of the book for the particular token.
    // - Uses an orderbook depth search to find the cutoff price:
    //   - BUY + USDC: walk asks until notional >= USDC
    //   - BUY + Shares: walk asks until shares >= N
    //   - SELL + Shares: walk bids until shares >= N
    async fn calculate_price(&self, order_type: OrderType) -> Result<Decimal> {
        let token_id = self
            .token_id
            .expect("Token ID was already validated in `build`");
        let side = self.side.expect("Side was already validated in `build`");
        let amount = self
            .amount
            .as_ref()
            .expect("Amount was already validated in `build`");

        let book = self
            .client
            .order_book(&OrderBookSummaryRequest {
                token_id,
                side: None,
            })
            .await?;

        if !matches!(order_type, OrderType::FAK | OrderType::FOK) {
            return Err(Error::validation(
                "Cannot set an order type other than FAK/FOK for a market order",
            ));
        }

        let (levels, amount_inner) = match side {
            Side::Buy => (&book.asks, amount.0),
            Side::Sell => match amount.0 {
                a @ AmountInner::Shares(_) => (&book.bids, a),
                AmountInner::Usdc(_) => {
                    return Err(Error::validation(
                        "Sell Orders must specify their `amount`s in shares",
                    ));
                }
            },

            side => return Err(Error::validation(format!("Invalid side: {side}"))),
        };

        if levels.is_empty() {
            return Err(Error::validation(format!(
                "No opposing orders for {token_id} which means there is no market price"
            )));
        }

        let target = amount_inner.as_inner();
        let cutoff_price = match amount_inner {
            AmountInner::Usdc(_) => {
                super::utilities::walk_levels(levels, target, |l| l.size * l.price, &order_type)
            }
            AmountInner::Shares(_) => {
                super::utilities::walk_levels(levels, target, |l| l.size, &order_type)
            }
        };

        cutoff_price.ok_or_else(|| {
            Error::validation(format!(
                "Insufficient liquidity to fill order for {token_id} at {target}"
            ))
        })
    }

    /// Validates and transforms this market builder into a [`SignableOrder`]
    ///
    /// # Panics
    ///
    /// Panics if the system clock is before the Unix epoch.
    #[cfg_attr(
        feature = "tracing",
        tracing::instrument(skip(self), err(level = "warn"))
    )]
    pub async fn build(self) -> Result<SignableOrder> {
        let Some(token_id) = self.token_id else {
            return Err(Error::validation(
                "Unable to build Order due to missing token ID",
            ));
        };

        let Some(side) = self.side else {
            return Err(Error::validation(
                "Unable to build Order due to missing token side",
            ));
        };

        let amount = self
            .amount
            .ok_or_else(|| Error::validation("Unable to build Order due to missing amount"))?;

        let order_type = self.order_type.clone().unwrap_or(OrderType::FAK);
        let post_only = self.post_only;
        if post_only == Some(true) {
            return Err(Error::validation(
                "postOnly is only supported for limit orders",
            ));
        }

        let price = match self.price {
            Some(price) => price,
            None => self.calculate_price(order_type.clone()).await?,
        };

        let minimum_tick_size = self
            .client
            .tick_size(token_id)
            .await?
            .minimum_tick_size
            .as_decimal();

        let decimals = minimum_tick_size.scale();

        // Ensure that the market price returned internally is truncated to our tick size
        let price = price.trunc_with_scale(decimals);
        if price < minimum_tick_size || price > Decimal::ONE - minimum_tick_size {
            return Err(Error::validation(format!(
                "Price {price} is too small or too large for the minimum tick size {minimum_tick_size}"
            )));
        }

        if !price_aligned_to_tick_size(price, minimum_tick_size) {
            return Err(Error::validation(format!(
                "Price {price} is not aligned to the minimum tick size {minimum_tick_size}"
            )));
        }

        let amount = match (side, amount.0, self.user_usdc_balance) {
            (Side::Buy, AmountInner::Usdc(raw), Some(balance)) => {
                // V2 uses `/clob-markets/{id}` `fd` (rate + exponent); `/fee-rate`
                // only exposes V1 bps and would silently mis-size V2 orders.
                let fee = self.client.fee_info(token_id).await?;
                let fee_rate = fee.rate;
                let fee_exponent = Decimal::from(fee.exponent);
                let builder_taker_fee = match self.builder_code {
                    Some(code) if code != B256::ZERO => {
                        let rate = self.client.builder_fee_rate(code).await?;
                        Decimal::from(rate.builder_taker_fee_rate_bps) / Decimal::from(10_000_u32)
                    }
                    _ => Decimal::ZERO,
                };

                let adjusted = super::utilities::adjust_market_buy_amount(
                    raw,
                    balance,
                    price,
                    fee_rate,
                    fee_exponent,
                    builder_taker_fee,
                )?;
                Amount::usdc(adjusted)?
            }
            _ => amount,
        };

        let raw_amount = amount.as_inner();

        let (taker_amount, maker_amount) = match (side, amount.0) {
            (Side::Buy, AmountInner::Usdc(_)) => {
                let shares = (raw_amount / price).trunc_with_scale(decimals + LOT_SIZE_SCALE);
                (shares, raw_amount)
            }
            (Side::Buy, AmountInner::Shares(_)) => {
                let usdc = (raw_amount * price).trunc_with_scale(decimals + LOT_SIZE_SCALE);
                (raw_amount, usdc)
            }
            (Side::Sell, AmountInner::Shares(_)) => {
                let usdc = (raw_amount * price).trunc_with_scale(decimals + LOT_SIZE_SCALE);
                (usdc, raw_amount)
            }
            (Side::Sell, AmountInner::Usdc(_)) => {
                return Err(Error::validation(
                    "Sell Orders must specify their `amount`s in shares",
                ));
            }
            (side, _) => return Err(Error::validation(format!("Invalid side: {side}"))),
        };

        let salt = to_ieee_754_int((self.salt_generator)());

        let payload = self
            .build_payload(
                token_id,
                side,
                to_fixed_u128(maker_amount),
                to_fixed_u128(taker_amount),
                salt,
                U256::ZERO,
            )
            .await?;

        #[cfg(feature = "tracing")]
        tracing::debug!(token_id = %token_id, side = ?side, price = %price, amount = %amount.as_inner(), "market order built");

        Ok(SignableOrder {
            payload,
            order_type,
            post_only: None,
            defer_exec: self.defer_exec,
        })
    }

    /// Convenience: builds, signs, and posts this market order in a single call.
    ///
    /// If the server rejects the order due to a version mismatch, this automatically retries
    /// once with the updated version — rebuilding and re-signing the order from scratch.
    ///
    /// # Errors
    ///
    /// Returns an error if any of the build, sign, or post steps fails.
    pub async fn build_sign_and_post<S: Signer>(self, signer: &S) -> Result<PostOrderResponse> {
        let client = self.client.clone();
        let before_version = client.resolve_version(false).await.unwrap_or(0);
        let retry = self.clone();
        let order = self.build().await?;
        let signed = client.sign(signer, order).await?;
        let result = client.post_order(signed).await;
        if let Err(err) = &result
            && let Some(status) = err.downcast_ref::<crate::error::Status>()
            && status
                .message
                .contains(crate::clob::client::ORDER_VERSION_MISMATCH_ERROR)
        {
            let after_version = client.resolve_version(false).await.unwrap_or(0);
            if after_version != before_version {
                let order = retry.build().await?;
                let signed = client.sign(signer, order).await?;
                return client.post_order(signed).await;
            }
        }
        result
    }
}

/// Removes trailing zeros, truncates to [`USDC_DECIMALS`] decimal places, and quanitizes as an
/// integer.
fn to_fixed_u128(d: Decimal) -> u128 {
    d.normalize()
        .trunc_with_scale(USDC_DECIMALS)
        .mantissa()
        .to_u128()
        .expect("The `build` call in `OrderBuilder<S, OrderKind, K>` ensures that only positive values are being multiplied/divided")
}

/// `Number.MAX_SAFE_INTEGER` (2^53 − 1). The CLOB backend deserializes the order's
/// uniqueness nonce as a JSON number; values above this bound lose precision in
/// JavaScript. Not a cryptographic constant — the nonce is not security-sensitive.
const JS_SAFE_INTEGER_MAX: u64 = (1 << 53) - 1;

fn to_ieee_754_int(value: u64) -> u64 {
    value & JS_SAFE_INTEGER_MAX
}

fn price_aligned_to_tick_size(price: Decimal, tick_size: Decimal) -> bool {
    (price % tick_size).is_zero()
}

#[must_use]
#[expect(
    clippy::float_arithmetic,
    reason = "We are not concerned with precision for the seed"
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "We are not concerned with truncation for a seed"
)]
#[expect(clippy::cast_sign_loss, reason = "We only need positive integers")]
pub(crate) fn generate_seed() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards");

    let seconds = now.as_secs_f64();
    let rand = rand::rng().random::<f64>();

    (seconds * rand).round() as u64
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn to_fixed_u128_should_succeed() {
        assert_eq!(to_fixed_u128(dec!(123.456)), 123_456_000);
        assert_eq!(to_fixed_u128(dec!(123.456789)), 123_456_789);
        assert_eq!(to_fixed_u128(dec!(123.456789111111111)), 123_456_789);
        assert_eq!(to_fixed_u128(dec!(3.456789111111111)), 3_456_789);
        assert_eq!(to_fixed_u128(Decimal::ZERO), 0);
    }

    #[test]
    #[should_panic(
        expected = "The `build` call in `OrderBuilder<S, OrderKind, K>` ensures that only positive values are being multiplied/divided"
    )]
    fn to_fixed_u128_panics() {
        to_fixed_u128(dec!(-123.456));
    }

    #[test]
    fn order_salt_should_be_less_than_or_equal_to_2_to_the_53_minus_1() {
        let raw_salt = u64::MAX;
        let masked_salt = to_ieee_754_int(raw_salt);

        assert!(masked_salt < (1 << 53));
    }
}
