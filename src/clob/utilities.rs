//! Client-side utility functions for orderbook analysis, fee calculation, and price validation.

use std::fmt::Write as _;

use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use sha1::Digest as _;

use super::types::response::{OrderBookSummaryResponse, OrderSummary};
use super::types::{Amount, AmountInner, OrderType, Side, TickSize};
use crate::Result;
use crate::error::Error;

/// Number of decimal places in a USDC amount on-chain. Exposed so utility callers can
/// use the same truncation semantics.
pub const USDC_DECIMALS: u32 = 6;

/// Walks orderbook levels best-to-worst, accumulating via `accumulate`, and returns
/// the cutoff price where cumulative ≥ `target`.
///
/// The CLOB wire format delivers levels worst-first (asks descending, bids ascending),
/// so iterating with `.rev()` produces the natural matching order. This means
/// `levels[0]` is the worst price in the slice.
///
/// If no level satisfies the target:
/// - Returns `None` for [`OrderType::FOK`]
/// - Returns the worst price in the slice (`levels[0]`) for other order types, so a
///   market-order caller has a safe upper/lower bound for its limit price.
///
/// Returns `None` for empty `levels`.
pub(crate) fn walk_levels<F: Fn(&OrderSummary) -> Decimal>(
    levels: &[OrderSummary],
    target: Decimal,
    accumulate: F,
    order_type: &OrderType,
) -> Option<Decimal> {
    if levels.is_empty() {
        return None;
    }

    let mut total = Decimal::ZERO;
    for level in levels.iter().rev() {
        total += accumulate(level);
        if total >= target {
            return Some(level.price);
        }
    }

    if *order_type == OrderType::FOK {
        return None;
    }

    Some(levels[0].price)
}

/// Walks the orderbook to calculate the effective fill price for a given [`Amount`].
///
/// The unit of `amount` (USDC vs shares) determines which side of the book is walked
/// and how liquidity is accumulated:
///
/// | Side | Amount  | Walks | Accumulates      |
/// |------|---------|-------|------------------|
/// | Buy  | Usdc    | asks  | `size * price`   |
/// | Buy  | Shares  | asks  | `size`           |
/// | Sell | Shares  | bids  | `size`           |
/// | Sell | Usdc    | — invalid, returns a validation error     |
///
/// # Errors
/// - `Side::Sell` paired with an `Amount::usdc(_)` (SELL orders must size in shares).
/// - `Side::Unknown`.
/// - `OrderType::FOK` with insufficient liquidity at any level.
///
/// For non-FOK order types with insufficient liquidity, returns the worst price in the
/// walked side of the book (a safe upper/lower bound for a market-order limit price).
pub fn calculate_market_price(
    orderbook: &OrderBookSummaryResponse,
    side: Side,
    amount: Amount,
    order_type: &OrderType,
) -> Result<Decimal> {
    let (levels, acc): (&[OrderSummary], fn(&OrderSummary) -> Decimal) = match (side, amount.0) {
        (Side::Buy, AmountInner::Usdc(_)) => (&orderbook.asks, |l| l.size * l.price),
        (Side::Buy, AmountInner::Shares(_)) => (&orderbook.asks, |l| l.size),
        (Side::Sell, AmountInner::Shares(_)) => (&orderbook.bids, |l| l.size),
        (Side::Sell, AmountInner::Usdc(_)) => {
            return Err(Error::validation(
                "SELL orders must specify their amount in shares, not USDC",
            ));
        }
        (Side::Unknown, _) => return Err(Error::validation(format!("Invalid side: {side}"))),
    };

    walk_levels(levels, amount.as_inner(), acc, order_type).ok_or_else(|| {
        Error::validation(format!(
            "Insufficient liquidity to fill {} on {side:?}",
            amount.as_inner()
        ))
    })
}

/// Generates a server-compatible SHA1 hash of an orderbook snapshot.
///
/// Constructs a compact JSON payload with a specific key order
/// (`market`, `asset_id`, `timestamp`, `hash=""`, `bids`, `asks`,
/// `min_order_size`, `tick_size`, `neg_risk`, `last_trade_price`)
/// and returns the SHA1 hex digest.
///
/// **Note**: [`OrderBookSummaryResponse::hash()`] uses SHA-256 on `serde_json::to_string`
/// and produces different results. This function is for server-compatible verification.
#[must_use]
pub fn orderbook_summary_hash(orderbook: &OrderBookSummaryResponse) -> String {
    // Build JSON manually — serde_json::json! uses BTreeMap which sorts keys alphabetically,
    // but the server expects a specific non-alphabetical key order.
    let mut json = String::with_capacity(512);

    json.push('{');
    let _ = write!(json, "\"market\":\"{}\"", orderbook.market);

    let asset_id_json = serde_json::to_string(&orderbook.asset_id).unwrap_or_default();
    let _ = write!(json, ",\"asset_id\":{asset_id_json}");
    let _ = write!(
        json,
        ",\"timestamp\":\"{}\"",
        orderbook.timestamp.timestamp_millis()
    );
    json.push_str(",\"hash\":\"\"");

    json.push_str(",\"bids\":[");
    for (i, o) in orderbook.bids.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        let _ = write!(
            json,
            "{{\"price\":\"{}\",\"size\":\"{}\"}}",
            o.price, o.size
        );
    }
    json.push(']');

    json.push_str(",\"asks\":[");
    for (i, o) in orderbook.asks.iter().enumerate() {
        if i > 0 {
            json.push(',');
        }
        let _ = write!(
            json,
            "{{\"price\":\"{}\",\"size\":\"{}\"}}",
            o.price, o.size
        );
    }
    json.push(']');

    let _ = write!(json, ",\"min_order_size\":\"{}\"", orderbook.min_order_size);
    let _ = write!(
        json,
        ",\"tick_size\":\"{}\"",
        Decimal::from(orderbook.tick_size)
    );
    let _ = write!(json, ",\"neg_risk\":{}", orderbook.neg_risk);
    let last = orderbook.last_trade_price.unwrap_or(Decimal::ZERO);
    let _ = write!(json, ",\"last_trade_price\":\"{last}\"");
    json.push('}');

    let mut hasher = sha1::Sha1::new();
    hasher.update(json.as_bytes());
    let result = hasher.finalize();

    format!("{result:x}")
}

/// Adjusts a market-buy USDC amount to account for platform and builder taker fees.
///
/// Delegates to [`super::fees::adjust_buy_amount_for_fees`] with `fee_slippage = 0`,
/// then truncates to [`USDC_DECIMALS`] (matching the on-chain USDC scale).
/// Returned amount is ready to pass to
/// [`Amount::usdc`](super::types::Amount::usdc).
///
/// # Errors
/// - `user_usdc_balance` is below the minimum to cover one USDC-unit of fees; the adjusted
///   amount would truncate to zero, which would submit a zero-value order the backend
///   rejects with an opaque error. Callers should top up the balance and retry.
pub fn adjust_market_buy_amount(
    amount: Decimal,
    user_usdc_balance: Decimal,
    price: Decimal,
    fee_rate: Decimal,
    fee_exponent: Decimal,
    builder_taker_fee_rate: Decimal,
) -> Result<Decimal> {
    let raw = super::fees::adjust_buy_amount_for_fees(
        amount,
        price,
        user_usdc_balance,
        fee_rate,
        fee_exponent,
        builder_taker_fee_rate,
        Decimal::ZERO,
    )?;

    let adjusted = raw.trunc_with_scale(USDC_DECIMALS);
    if adjusted.is_zero() {
        return Err(Error::validation(format!(
            "user_usdc_balance {user_usdc_balance} too small to cover fees at price {price}; \
             fee-adjusted amount truncated to zero"
        )));
    }
    Ok(adjusted)
}

/// Validates that a price is within the valid range `[tick_size, 1 - tick_size]`.
#[must_use]
pub fn price_valid(price: Decimal, tick_size: TickSize) -> bool {
    let ts = Decimal::from(tick_size);
    price >= ts && price <= dec!(1) - ts
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use rust_decimal_macros::dec;

    use super::*;
    use crate::types::{B256, U256};

    fn make_orderbook(
        bids: Vec<OrderSummary>,
        asks: Vec<OrderSummary>,
    ) -> OrderBookSummaryResponse {
        OrderBookSummaryResponse::builder()
            .market(B256::ZERO)
            .asset_id(U256::ZERO)
            .timestamp(Utc::now())
            .bids(bids)
            .asks(asks)
            .min_order_size(dec!(0.01))
            .neg_risk(false)
            .tick_size(TickSize::Hundredth)
            .build()
    }

    fn order(price: Decimal, size: Decimal) -> OrderSummary {
        OrderSummary::builder().price(price).size(size).build()
    }

    #[test]
    fn calculate_market_price_buy_usdc_sufficient_liquidity() {
        // Asks are delivered worst-first on the wire, so the walk proceeds 0.50 → 0.51.
        let ob = make_orderbook(
            vec![],
            vec![
                order(dec!(0.52), dec!(100)),
                order(dec!(0.51), dec!(100)),
                order(dec!(0.50), dec!(100)),
            ],
        );
        // 0.50*100 = 50, 0.51*100 = 51 → 101 ≥ 80
        let amt = Amount::usdc(dec!(80)).unwrap();
        assert_eq!(
            calculate_market_price(&ob, Side::Buy, amt, &OrderType::FOK).unwrap(),
            dec!(0.51),
        );
    }

    #[test]
    fn calculate_market_price_buy_shares_sufficient_liquidity() {
        let ob = make_orderbook(
            vec![],
            vec![
                order(dec!(0.52), dec!(100)),
                order(dec!(0.51), dec!(100)),
                order(dec!(0.50), dec!(100)),
            ],
        );
        // 100, then 200 ≥ 150 → 0.51
        let amt = Amount::shares(dec!(150)).unwrap();
        assert_eq!(
            calculate_market_price(&ob, Side::Buy, amt, &OrderType::FOK).unwrap(),
            dec!(0.51),
        );
    }

    #[test]
    fn calculate_market_price_buy_insufficient_fok() {
        let ob = make_orderbook(vec![], vec![order(dec!(0.50), dec!(10))]);
        let amt = Amount::usdc(dec!(100)).unwrap();
        calculate_market_price(&ob, Side::Buy, amt, &OrderType::FOK).unwrap_err();
    }

    #[test]
    fn calculate_market_price_buy_insufficient_fak() {
        // Asks worst-first → 0.60 is levels[0]. FAK with insufficient liquidity
        // falls back to that worst price so the caller gets a safe upper bound.
        let ob = make_orderbook(
            vec![],
            vec![order(dec!(0.60), dec!(5)), order(dec!(0.50), dec!(10))],
        );
        let amt = Amount::usdc(dec!(1000)).unwrap();
        assert_eq!(
            calculate_market_price(&ob, Side::Buy, amt, &OrderType::FAK).unwrap(),
            dec!(0.60),
        );
    }

    #[test]
    fn calculate_market_price_sell_shares() {
        // Bids are delivered worst-first on the wire, so the walk proceeds 0.50 → 0.49.
        let ob = make_orderbook(
            vec![
                order(dec!(0.48), dec!(100)),
                order(dec!(0.49), dec!(100)),
                order(dec!(0.50), dec!(100)),
            ],
            vec![],
        );
        // 100, then 200 ≥ 150 → 0.49
        let amt = Amount::shares(dec!(150)).unwrap();
        assert_eq!(
            calculate_market_price(&ob, Side::Sell, amt, &OrderType::FOK).unwrap(),
            dec!(0.49),
        );
    }

    #[test]
    fn calculate_market_price_sell_usdc_is_rejected() {
        let ob = make_orderbook(
            vec![order(dec!(0.49), dec!(100))],
            vec![order(dec!(0.51), dec!(100))],
        );
        let amt = Amount::usdc(dec!(10)).unwrap();
        calculate_market_price(&ob, Side::Sell, amt, &OrderType::FOK).unwrap_err();
    }

    #[test]
    fn calculate_market_price_empty_orderbook() {
        let ob = make_orderbook(vec![], vec![]);
        let amt = Amount::usdc(dec!(100)).unwrap();
        calculate_market_price(&ob, Side::Buy, amt, &OrderType::FOK).unwrap_err();
    }

    #[test]
    fn calculate_market_price_unknown_side_errors() {
        let ob = make_orderbook(
            vec![order(dec!(0.49), dec!(100))],
            vec![order(dec!(0.51), dec!(100))],
        );
        let amt = Amount::usdc(dec!(10)).unwrap();
        calculate_market_price(&ob, Side::Unknown, amt, &OrderType::FOK).unwrap_err();
    }

    #[test]
    fn price_valid_within_bounds() {
        assert!(price_valid(dec!(0.5), TickSize::Hundredth));
        assert!(price_valid(dec!(0.01), TickSize::Hundredth));
        assert!(price_valid(dec!(0.99), TickSize::Hundredth));
    }

    #[test]
    fn price_valid_at_boundaries() {
        assert!(price_valid(dec!(0.1), TickSize::Tenth));
        assert!(price_valid(dec!(0.9), TickSize::Tenth));
    }

    #[test]
    fn price_valid_out_of_bounds() {
        assert!(!price_valid(dec!(0.0), TickSize::Hundredth));
        assert!(!price_valid(dec!(1.0), TickSize::Hundredth));
        assert!(!price_valid(dec!(0.005), TickSize::Hundredth));
        assert!(!price_valid(dec!(0.995), TickSize::Hundredth));
    }

    #[test]
    fn price_valid_all_tick_sizes() {
        assert!(price_valid(dec!(0.5), TickSize::Tenth));
        assert!(price_valid(dec!(0.5), TickSize::Hundredth));
        assert!(price_valid(dec!(0.5), TickSize::HalfCent));
        assert!(price_valid(dec!(0.5), TickSize::QuarterCent));
        assert!(price_valid(dec!(0.5), TickSize::Thousandth));
        assert!(price_valid(dec!(0.5), TickSize::TenThousandth));
    }

    #[test]
    fn orderbook_hash_deterministic() {
        let ts = DateTime::from_timestamp_millis(1_700_000_000_000).expect("valid ts");
        let ob = OrderBookSummaryResponse::builder()
            .market(B256::ZERO)
            .asset_id(U256::ZERO)
            .timestamp(ts)
            .bids(vec![order(dec!(0.49), dec!(50))])
            .asks(vec![order(dec!(0.51), dec!(25))])
            .min_order_size(dec!(0.01))
            .neg_risk(false)
            .tick_size(TickSize::Hundredth)
            .build();

        let hash = orderbook_summary_hash(&ob);
        assert_eq!(hash.len(), 40);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash, orderbook_summary_hash(&ob));
    }

    #[test]
    fn orderbook_hash_differs_for_different_inputs() {
        let ts = DateTime::from_timestamp_millis(1_700_000_000_000).expect("valid ts");
        let ob1 = OrderBookSummaryResponse::builder()
            .market(B256::ZERO)
            .asset_id(U256::from(1_u64))
            .timestamp(ts)
            .min_order_size(dec!(0.01))
            .neg_risk(false)
            .tick_size(TickSize::Hundredth)
            .build();

        let ob2 = OrderBookSummaryResponse::builder()
            .market(B256::ZERO)
            .asset_id(U256::from(2_u64))
            .timestamp(ts)
            .min_order_size(dec!(0.01))
            .neg_risk(false)
            .tick_size(TickSize::Hundredth)
            .build();

        assert_ne!(orderbook_summary_hash(&ob1), orderbook_summary_hash(&ob2));
    }

    #[test]
    fn adjust_market_buy_no_adjustment_when_balance_sufficient() {
        let result = adjust_market_buy_amount(
            dec!(100),
            dec!(1000),
            dec!(0.5),
            dec!(0.02),
            dec!(1),
            dec!(0),
        )
        .unwrap();
        assert_eq!(result, dec!(100));
    }

    #[test]
    fn adjust_market_buy_adjusts_when_balance_insufficient() {
        let result = adjust_market_buy_amount(
            dec!(100),
            dec!(100),
            dec!(0.5),
            dec!(0.02),
            dec!(1),
            dec!(0),
        )
        .unwrap();
        assert!(result < dec!(100));
        assert!(result > dec!(0));
    }

    #[test]
    fn adjust_market_buy_with_builder_fee() {
        let result = adjust_market_buy_amount(
            dec!(100),
            dec!(100),
            dec!(0.5),
            dec!(0),
            dec!(1),
            dec!(0.005),
        )
        .unwrap();
        // Invariant: adjusted = balance - builder_fee(balance) = 100 - 0.5 = 99.5
        assert_eq!(result, dec!(99.5));
    }

    #[test]
    fn adjust_market_buy_errors_when_balance_truncates_to_zero() {
        // user_usdc_balance smaller than 1e-6 after fee-divisor → truncates to zero.
        let err = adjust_market_buy_amount(
            dec!(100),       // wanted amount
            dec!(0.0000001), // balance well below 1 USDC-micro
            dec!(0.5),
            dec!(0.02),
            dec!(1),
            dec!(0.005),
        )
        .unwrap_err();
        assert!(err.to_string().contains("truncated to zero"));
    }

    // Fee calculation tests ported from TS `feeCalculations.test.ts`.

    /// `platform_fee = (amount / price) × rate × (price × (1 − price))^exponent`.
    fn calc_platform_fee(amount: Decimal, price: Decimal, rate: Decimal, exponent: u32) -> Decimal {
        let base = price * (Decimal::ONE - price);
        let base_f64 = f64::try_from(base).unwrap_or(0.0);
        let rate_factor = rate
            * Decimal::try_from(base_f64.powi(i32::try_from(exponent).unwrap_or(0)))
                .unwrap_or(Decimal::ZERO);
        (amount / price) * rate_factor
    }

    /// `builder_fee = amount × rate` (flat percentage on notional).
    fn calc_builder_fee(amount: Decimal, rate: Decimal) -> Decimal {
        amount * rate
    }

    fn close_to(actual: Decimal, expected: Decimal, tol: Decimal) {
        let diff = (actual - expected).abs();
        assert!(
            diff <= tol,
            "|{actual} − {expected}| = {diff} exceeds tolerance {tol}"
        );
    }

    // Platform fee at representative prices (rate=0.25, exp=2, C=100 contracts).

    #[test]
    fn platform_fee_0_25_exp_2_at_midprice() {
        // price=0.5 → 1.5625
        close_to(
            calc_platform_fee(dec!(100) * dec!(0.5), dec!(0.5), dec!(0.25), 2),
            dec!(1.5625),
            dec!(0.000001),
        );
    }

    #[test]
    fn platform_fee_0_25_exp_2_symmetric_prices() {
        // (0.3, 0.7), (0.1, 0.9), (0.05, 0.95), (0.01, 0.99) must all pair up.
        let cases = [
            (dec!(0.3), dec!(0.7), dec!(1.1025)),
            (dec!(0.1), dec!(0.9), dec!(0.2025)),
            (dec!(0.05), dec!(0.95), dec!(0.05640625)),
            (dec!(0.01), dec!(0.99), dec!(0.00245025)),
        ];
        for (p_low, p_high, expected) in cases {
            close_to(
                calc_platform_fee(dec!(100) * p_low, p_low, dec!(0.25), 2),
                expected,
                dec!(0.000001),
            );
            close_to(
                calc_platform_fee(dec!(100) * p_high, p_high, dec!(0.25), 2),
                expected,
                dec!(0.000001),
            );
        }
    }

    #[test]
    fn platform_fee_0_25_exp_2_fractional_contracts() {
        // price=0.5, C=125.5 → 1.9609375
        close_to(
            calc_platform_fee(dec!(125.5) * dec!(0.5), dec!(0.5), dec!(0.25), 2),
            dec!(1.9609375),
            dec!(0.000001),
        );
    }

    // Builder fee (flat %).

    #[test]
    fn builder_fee_1_pct() {
        // 1% on 100 contracts at 50c → 0.5
        close_to(
            calc_builder_fee(dec!(100) * dec!(0.5), dec!(0.01)),
            dec!(0.5),
            dec!(0.000001),
        );
    }

    #[test]
    fn builder_fee_5_pct() {
        // 5% on 200 contracts at 75c → 7.5
        close_to(
            calc_builder_fee(dec!(200) * dec!(0.75), dec!(0.05)),
            dec!(7.5),
            dec!(0.000001),
        );
    }

    // Combined platform + builder fee.

    #[test]
    fn combined_platform_and_builder_fee() {
        let amount_usd = dec!(100) * dec!(0.5);
        let platform = calc_platform_fee(amount_usd, dec!(0.5), dec!(0.25), 2);
        let builder = calc_builder_fee(amount_usd, dec!(0.01));
        close_to(platform, dec!(1.5625), dec!(0.000001));
        close_to(builder, dec!(0.5), dec!(0.000001));
        close_to(platform + builder, dec!(2.0625), dec!(0.000001));
    }

    // `adjust_market_buy_amount` boundary behaviour.

    #[test]
    fn adjust_buy_balance_strictly_greater_returns_amount_unchanged() {
        let amount = dec!(50);
        let price = dec!(0.5);
        let fee = calc_platform_fee(amount, price, dec!(0.25), 2);
        let balance = amount + fee + dec!(1); // comfortably above total cost
        let result =
            adjust_market_buy_amount(amount, balance, price, dec!(0.25), dec!(2), dec!(0)).unwrap();
        assert_eq!(result, amount);
    }

    #[test]
    fn adjust_buy_balance_equal_to_total_cost_returns_amount() {
        // At `balance == totalCost` the `<=` check fires; adjusted = balance − fee(balance).
        // When balance = amount + fee(amount), adjusted = amount + fee(amount) − fee(amount) = amount.
        let amount = dec!(50);
        let price = dec!(0.5);
        let fee = calc_platform_fee(amount, price, dec!(0.25), 2);
        let total_cost = amount + fee;
        let result =
            adjust_market_buy_amount(amount, total_cost, price, dec!(0.25), dec!(2), dec!(0))
                .unwrap();
        close_to(result, amount, dec!(0.000001));
    }

    #[test]
    fn adjust_buy_platform_only_exact_value() {
        // balance = amount = 50, price = 0.5, rate = 0.25, exp = 2.
        // platform_fee = (50/0.5)*0.015625 = 1.5625 → adjusted = 48.4375
        let amount = dec!(50);
        let price = dec!(0.5);
        let adjusted =
            adjust_market_buy_amount(amount, amount, price, dec!(0.25), dec!(2), dec!(0)).unwrap();
        close_to(adjusted, dec!(48.4375), dec!(0.000001));
        assert!(adjusted < amount);
        // Fee computed on adjusted is less than fee on balance (conservative).
        let fee_on_adjusted = calc_platform_fee(adjusted, price, dec!(0.25), 2);
        close_to(
            adjusted + fee_on_adjusted,
            dec!(49.951171875),
            dec!(0.000001),
        );
    }

    #[test]
    fn adjust_buy_builder_only_exact_value() {
        // adjusted = 50 − 50*0.01 = 49.5
        let amount = dec!(50);
        let price = dec!(0.5);
        let builder_rate = dec!(0.01);
        let adjusted =
            adjust_market_buy_amount(amount, amount, price, dec!(0), dec!(0), builder_rate)
                .unwrap();
        close_to(adjusted, dec!(49.5), dec!(0.000001));
    }

    #[test]
    fn adjust_buy_combined_platform_and_builder_exact_value() {
        // adjusted = 50 − 1.5625 − 0.5 = 47.9375
        let amount = dec!(50);
        let price = dec!(0.5);
        let builder_rate = dec!(0.01);
        let adjusted =
            adjust_market_buy_amount(amount, amount, price, dec!(0.25), dec!(2), builder_rate)
                .unwrap();
        close_to(adjusted, dec!(47.9375), dec!(0.000001));
        assert!(adjusted < amount);
    }

    #[test]
    fn adjust_buy_at_price_0_3() {
        // Verify adjusted < amount and adjusted > 0 for a non-midpoint price.
        let amount = dec!(30);
        let price = dec!(0.3);
        let builder_rate = dec!(0.02);
        let adjusted =
            adjust_market_buy_amount(amount, amount, price, dec!(0.25), dec!(2), builder_rate)
                .unwrap();
        assert!(adjusted < amount);
        assert!(adjusted > dec!(0));
    }

    // Production V2 fee tiers (all exp=1):
    //   sports          rate=0.03
    //   politics family rate=0.04  (politics, tech, finance_prices, mentions)
    //   culture family  rate=0.05  (culture, weather, general, economics)
    //   crypto          rate=0.072

    #[test]
    fn production_fee_sports_v2() {
        close_to(
            calc_platform_fee(dec!(100), dec!(0.5), dec!(0.03), 1),
            dec!(1.5),
            dec!(0.000001),
        );
        close_to(
            calc_platform_fee(dec!(100), dec!(0.3), dec!(0.03), 1),
            dec!(2.1),
            dec!(0.000001),
        );
        close_to(
            calc_platform_fee(dec!(100), dec!(0.7), dec!(0.03), 1),
            dec!(0.9),
            dec!(0.000001),
        );
    }

    #[test]
    fn production_fee_politics_family() {
        // rate=0.04, exp=1 — politics, tech, finance_prices, mentions
        close_to(
            calc_platform_fee(dec!(100), dec!(0.5), dec!(0.04), 1),
            dec!(2.0),
            dec!(0.000001),
        );
        close_to(
            calc_platform_fee(dec!(100), dec!(0.3), dec!(0.04), 1),
            dec!(2.8),
            dec!(0.000001),
        );
        close_to(
            calc_platform_fee(dec!(100), dec!(0.7), dec!(0.04), 1),
            dec!(1.2),
            dec!(0.000001),
        );
    }

    #[test]
    fn production_fee_culture_family() {
        // rate=0.05, exp=1 — culture, weather, general, economics
        close_to(
            calc_platform_fee(dec!(100), dec!(0.5), dec!(0.05), 1),
            dec!(2.5),
            dec!(0.000001),
        );
        close_to(
            calc_platform_fee(dec!(100), dec!(0.3), dec!(0.05), 1),
            dec!(3.5),
            dec!(0.000001),
        );
        close_to(
            calc_platform_fee(dec!(100), dec!(0.7), dec!(0.05), 1),
            dec!(1.5),
            dec!(0.000001),
        );
    }

    #[test]
    fn production_fee_crypto_v2() {
        // rate=0.072, exp=1
        close_to(
            calc_platform_fee(dec!(100), dec!(0.5), dec!(0.072), 1),
            dec!(3.6),
            dec!(0.000001),
        );
        close_to(
            calc_platform_fee(dec!(100), dec!(0.3), dec!(0.072), 1),
            dec!(5.04),
            dec!(0.000001),
        );
        close_to(
            calc_platform_fee(dec!(100), dec!(0.7), dec!(0.072), 1),
            dec!(2.16),
            dec!(0.000001),
        );
    }

    #[test]
    fn production_adjust_buy_exact_values_across_all_tiers() {
        // For every production tier at prices {0.3, 0.5, 0.7}, verify the exact adjusted
        // value when balance == amount (budget fully consumed, no builder fee).
        let amount = dec!(100);
        // (tier_name, rate, exponent, price, expected_adjusted)
        let cases: &[(&str, Decimal, u32, Decimal, Decimal)] = &[
            ("sports_v2 p=0.5", dec!(0.03), 1, dec!(0.5), dec!(98.5)),
            ("sports_v2 p=0.3", dec!(0.03), 1, dec!(0.3), dec!(97.9)),
            ("sports_v2 p=0.7", dec!(0.03), 1, dec!(0.7), dec!(99.1)),
            ("politics p=0.5", dec!(0.04), 1, dec!(0.5), dec!(98.0)),
            ("politics p=0.3", dec!(0.04), 1, dec!(0.3), dec!(97.2)),
            ("politics p=0.7", dec!(0.04), 1, dec!(0.7), dec!(98.8)),
            ("culture p=0.5", dec!(0.05), 1, dec!(0.5), dec!(97.5)),
            ("culture p=0.3", dec!(0.05), 1, dec!(0.3), dec!(96.5)),
            ("culture p=0.7", dec!(0.05), 1, dec!(0.7), dec!(98.5)),
            ("crypto_v2 p=0.5", dec!(0.072), 1, dec!(0.5), dec!(96.4)),
            ("crypto_v2 p=0.3", dec!(0.072), 1, dec!(0.3), dec!(94.96)),
            ("crypto_v2 p=0.7", dec!(0.072), 1, dec!(0.7), dec!(97.84)),
        ];
        for (name, rate, exponent, price, expected) in cases {
            let adjusted = adjust_market_buy_amount(
                amount,
                amount,
                *price,
                *rate,
                Decimal::from(*exponent),
                dec!(0),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            close_to(adjusted, *expected, dec!(0.0001));
        }
    }
}
