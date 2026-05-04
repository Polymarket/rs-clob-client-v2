//! Fee reservation utilities for buy orders.

use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::Result;
use crate::error::Error;

/// Validates that `fee_slippage` is either 0 or a value in [1, 100].
///
/// Values like 0.5, negatives, or values above 100 are rejected because they produce
/// nonsensical fee padding (sub-1% fractions are likely user error, negatives would
/// reduce the reserve).
pub fn validate_fee_slippage(fee_slippage: Decimal) -> Result<()> {
    if fee_slippage.is_sign_negative()
        || fee_slippage > dec!(100)
        || (fee_slippage > Decimal::ZERO && fee_slippage < Decimal::ONE)
    {
        return Err(Error::validation(
            "fee_slippage must be 0 or a percentage between 1 and 100",
        ));
    }
    Ok(())
}

/// Conservatively adjusts a BUY USDC `amount` so that `amount + fees ≤ user_usdc_balance`.
///
/// Returns `amount` unchanged when the balance comfortably covers the total cost.
/// Otherwise returns `balance − estimated_fees`, where fees are computed on
/// `min(amount, balance)` so that an oversized `amount` does not inflate the reserve.
///
/// `fee_slippage` pads only the platform fee component (not the builder fee) to guard
/// against on-chain price movement between signing and matching.
///
/// # Errors
///
/// Returns an error if `fee_slippage` is invalid (see [`validate_fee_slippage`]).
#[expect(
    clippy::module_name_repetitions,
    reason = "name mirrors the Python reference implementation"
)]
pub fn adjust_buy_amount_for_fees(
    amount: Decimal,
    price: Decimal,
    user_usdc_balance: Decimal,
    fee_rate: Decimal,
    fee_exponent: Decimal,
    builder_taker_fee_rate: Decimal,
    fee_slippage: Decimal,
) -> Result<Decimal> {
    validate_fee_slippage(fee_slippage)?;

    let base = price * (Decimal::ONE - price);
    let base_f64: f64 = base.try_into().unwrap_or(0.0);
    let exp_f64: f64 = fee_exponent.try_into().unwrap_or(0.0);
    let platform_fee_rate =
        fee_rate * Decimal::try_from(base_f64.powf(exp_f64)).unwrap_or(Decimal::ZERO);

    let effective_platform_fee_rate = platform_fee_rate * (Decimal::ONE + fee_slippage / dec!(100));

    let fee_base_amount = amount.min(user_usdc_balance);
    let platform_fee = (fee_base_amount / price) * effective_platform_fee_rate;
    let builder_fee = fee_base_amount * builder_taker_fee_rate;
    let total_cost = amount + platform_fee + builder_fee;

    if user_usdc_balance <= total_cost {
        Ok((user_usdc_balance - platform_fee - builder_fee).max(Decimal::ZERO))
    } else {
        Ok(amount)
    }
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;

    fn close_to(actual: Decimal, expected: Decimal, tol: Decimal) {
        let diff = (actual - expected).abs();
        assert!(
            diff <= tol,
            "|{actual} − {expected}| = {diff} exceeds tolerance {tol}"
        );
    }

    // ── validate_fee_slippage ──────────────────────────────────────────────────

    #[test]
    fn validate_fee_slippage_accepts_zero() {
        validate_fee_slippage(Decimal::ZERO).unwrap();
    }

    #[test]
    fn validate_fee_slippage_accepts_max() {
        validate_fee_slippage(dec!(100)).unwrap();
    }

    #[test]
    fn validate_fee_slippage_accepts_fractional_between_1_and_100() {
        validate_fee_slippage(dec!(12.5)).unwrap();
        validate_fee_slippage(dec!(99.9)).unwrap();
    }

    #[test]
    fn validate_fee_slippage_accepts_exactly_1() {
        validate_fee_slippage(Decimal::ONE).unwrap();
    }

    #[test]
    fn validate_fee_slippage_rejects_fraction_below_1() {
        validate_fee_slippage(dec!(0.5)).unwrap_err();
    }

    #[test]
    fn validate_fee_slippage_rejects_above_100() {
        validate_fee_slippage(dec!(101)).unwrap_err();
    }

    #[test]
    fn validate_fee_slippage_rejects_negative() {
        validate_fee_slippage(dec!(-1)).unwrap_err();
    }

    // ── adjust_buy_amount_for_fees — no adjustment paths ──────────────────────

    #[test]
    fn no_adjustment_zero_fees() {
        let result = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(50),
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        )
        .unwrap();
        assert_eq!(result, dec!(50));
    }

    #[test]
    fn no_adjustment_when_balance_exceeds_total_cost() {
        let result = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(1000),
            dec!(0.25),
            dec!(2),
            Decimal::ZERO,
            Decimal::ZERO,
        )
        .unwrap();
        assert_eq!(result, dec!(50));
    }

    #[test]
    fn balance_exactly_equal_to_amount_plus_reserved_fee_returns_amount() {
        // balance = amount + platform_fee(amount) = 50 + 1.5625 = 51.5625
        // total_cost == balance → triggers adjust branch but result == amount
        let result = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(51.5625),
            dec!(0.25),
            dec!(2),
            Decimal::ZERO,
            Decimal::ZERO,
        )
        .unwrap();
        assert_eq!(result, dec!(50));
    }

    // ── adjust_buy_amount_for_fees — platform fee only ────────────────────────
    //
    // rate=0.25, exp=2, price=0.5 → platform_fee_rate = 0.25*(0.5*0.5)^2 = 0.015625

    #[test]
    fn platform_fee_only_reserves_original_fee() {
        // platform_fee = (50/0.5)*0.015625 = 1.5625 → adjusted = 48.4375
        let adj = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(50),
            dec!(0.25),
            dec!(2),
            Decimal::ZERO,
            Decimal::ZERO,
        )
        .unwrap();
        close_to(adj, dec!(48.4375), dec!(0.000001));
        let adjusted_fee = (adj / dec!(0.5)) * dec!(0.015625);
        close_to(adjusted_fee, dec!(1.513671875), dec!(0.000001));
        close_to(adj + adjusted_fee, dec!(49.951171875), dec!(0.000001));
    }

    // ── adjust_buy_amount_for_fees — builder fee only ─────────────────────────

    #[test]
    fn builder_fee_only_reserves_original_fee() {
        // builder_fee = 50*0.01 = 0.5 → adjusted = 49.5
        let adj = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(50),
            Decimal::ZERO,
            Decimal::ZERO,
            dec!(0.01),
            Decimal::ZERO,
        )
        .unwrap();
        close_to(adj, dec!(49.5), dec!(0.000001));
        let adjusted_fee = adj * dec!(0.01);
        close_to(adjusted_fee, dec!(0.495), dec!(0.000001));
        close_to(adj + adjusted_fee, dec!(49.995), dec!(0.000001));
    }

    // ── adjust_buy_amount_for_fees — platform + builder ───────────────────────

    #[test]
    fn platform_and_builder_fee_reserves_original_fees() {
        // platform_fee = 1.5625, builder_fee = 0.5 → adjusted = 47.9375
        let adj = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(50),
            dec!(0.25),
            dec!(2),
            dec!(0.01),
            Decimal::ZERO,
        )
        .unwrap();
        close_to(adj, dec!(47.9375), dec!(0.000001));
        let adjusted_platform_fee = (adj / dec!(0.5)) * dec!(0.015625);
        let adjusted_builder_fee = adj * dec!(0.01);
        close_to(adjusted_platform_fee, dec!(1.498046875), dec!(0.000001));
        close_to(adjusted_builder_fee, dec!(0.479375), dec!(0.000001));
        close_to(
            adj + adjusted_platform_fee + adjusted_builder_fee,
            dec!(49.914921875),
            dec!(0.000001),
        );
    }

    #[test]
    fn price_0_3_platform_and_builder_reserves_original_fees() {
        // rate=0.25, exp=2, price=0.3 → platform_fee_rate = 0.25*(0.3*0.7)^2 ≈ 0.011025
        // amount=balance=30, builder=0.02
        // platform_fee ≈ (30/0.3)*0.011025 = 1.1025, builder_fee = 0.6 → adjusted ≈ 28.2975
        let adj = adjust_buy_amount_for_fees(
            dec!(30),
            dec!(0.3),
            dec!(30),
            dec!(0.25),
            dec!(2),
            dec!(0.02),
            Decimal::ZERO,
        )
        .unwrap();
        close_to(adj, dec!(28.2975), dec!(0.000001));
        let adjusted_platform_fee = (adj / dec!(0.3)) * dec!(0.011025);
        let adjusted_builder_fee = adj * dec!(0.02);
        close_to(adjusted_platform_fee, dec!(1.039933125), dec!(0.000001));
        close_to(adjusted_builder_fee, dec!(0.56595), dec!(0.000001));
        close_to(
            adj + adjusted_platform_fee + adjusted_builder_fee,
            dec!(29.903383125),
            dec!(0.000001),
        );
    }

    // ── fee_base capping: amount > balance ────────────────────────────────────

    #[test]
    fn fee_base_amount_capped_at_balance() {
        // amount=100 > balance=1 → fee_base=1
        // rate=0.072, exp=1, price=0.3 → platform_fee_rate = 0.072*0.21 = 0.01512
        // reserved_fee = (1/0.3)*0.01512 = 0.0504 → adjusted = 0.9496
        let adj = adjust_buy_amount_for_fees(
            dec!(100),
            dec!(0.3),
            dec!(1),
            dec!(0.072),
            Decimal::ONE,
            Decimal::ZERO,
            Decimal::ZERO,
        )
        .unwrap();
        assert_eq!(adj, dec!(0.9496));
        let adjusted_fee = (adj / dec!(0.3)) * dec!(0.01512);
        close_to(adjusted_fee, dec!(0.04785984), dec!(0.000001));
        close_to(adj + adjusted_fee, dec!(0.99745984), dec!(0.000001));
    }

    // ── fee_slippage ──────────────────────────────────────────────────────────

    #[test]
    fn pads_only_platform_fee_by_percentage() {
        // fee_slippage=20 → effective_rate = 0.015625*1.2 = 0.01875
        // platform_fee = (50/0.5)*0.01875 = 1.875, builder_fee = 0.5 → adjusted = 47.625
        let adj = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(50),
            dec!(0.25),
            dec!(2),
            dec!(0.01),
            dec!(20),
        )
        .unwrap();
        close_to(adj, dec!(47.625), dec!(0.000001));
        let adjusted_platform_fee = (adj / dec!(0.5)) * dec!(0.015625) * dec!(1.2);
        let adjusted_builder_fee = adj * dec!(0.01);
        close_to(adjusted_platform_fee, dec!(1.7859375), dec!(0.000001));
        close_to(adjusted_builder_fee, dec!(0.47625), dec!(0.000001));
        close_to(
            adj + adjusted_platform_fee + adjusted_builder_fee,
            dec!(49.8871875),
            dec!(0.000001),
        );
    }

    #[test]
    fn adjusts_when_balance_covers_unpadded_but_not_padded_fees() {
        // balance = 50 + 1.5625 + (1.875 - 1.5625)/2 = 51.71875
        // padded platform_fee = 1.875 > (balance - amount) so adjustment fires
        let adj = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(51.71875),
            dec!(0.25),
            dec!(2),
            Decimal::ZERO,
            dec!(20),
        )
        .unwrap();
        assert_eq!(adj, dec!(49.84375));
    }

    #[test]
    fn accepts_float_percentages_between_1_and_100() {
        // fee_slippage=1.5 → effective_rate = 0.015625*1.015 = 0.015859375
        // platform_fee = (50/0.5)*0.015859375 = 1.5859375 → adjusted = 48.4140625
        let adj = adjust_buy_amount_for_fees(
            dec!(50),
            dec!(0.5),
            dec!(50),
            dec!(0.25),
            dec!(2),
            Decimal::ZERO,
            dec!(1.5),
        )
        .unwrap();
        assert_eq!(adj, dec!(48.4140625));
        let adjusted_platform_fee = (adj / dec!(0.5)) * dec!(0.015625) * dec!(1.015);
        close_to(adjusted_platform_fee, dec!(1.535633544922), dec!(0.000001));
        close_to(
            adj + adjusted_platform_fee,
            dec!(49.949696044922),
            dec!(0.000001),
        );
    }

    // ── Production V2 fee tiers (amount=balance=100, no builder fee) ──────────
    //
    // Columns: (price, adjusted, platform_fee_rate, adjusted_fee, final)
    // platform_fee_rate = fee_rate * price * (1-price)  (exp=1)

    #[test]
    fn production_sports_v2_fee_slippage_0() {
        // rate=0.03, exp=1
        for (price, expected_adj, pfr, expected_adj_fee, expected_final) in [
            (
                dec!(0.5),
                dec!(98.5),
                dec!(0.0075),
                dec!(1.4775),
                dec!(99.9775),
            ),
            (
                dec!(0.3),
                dec!(97.9),
                dec!(0.0063),
                dec!(2.0559),
                dec!(99.9559),
            ),
            (
                dec!(0.7),
                dec!(99.1),
                dec!(0.0063),
                dec!(0.8919),
                dec!(99.9919),
            ),
        ] {
            let adj = adjust_buy_amount_for_fees(
                dec!(100),
                price,
                dec!(100),
                dec!(0.03),
                Decimal::ONE,
                Decimal::ZERO,
                Decimal::ZERO,
            )
            .unwrap();
            assert_eq!(adj, expected_adj);
            let adjusted_fee = (adj / price) * pfr;
            close_to(adjusted_fee, expected_adj_fee, dec!(0.0001));
            close_to(adj + adjusted_fee, expected_final, dec!(0.0001));
        }
    }

    #[test]
    fn production_politics_fee_slippage_0() {
        // rate=0.04, exp=1
        for (price, expected_adj, pfr, expected_adj_fee, expected_final) in [
            (dec!(0.5), dec!(98.0), dec!(0.01), dec!(1.96), dec!(99.96)),
            (
                dec!(0.3),
                dec!(97.2),
                dec!(0.0084),
                dec!(2.7216),
                dec!(99.9216),
            ),
            (
                dec!(0.7),
                dec!(98.8),
                dec!(0.0084),
                dec!(1.1856),
                dec!(99.9856),
            ),
        ] {
            let adj = adjust_buy_amount_for_fees(
                dec!(100),
                price,
                dec!(100),
                dec!(0.04),
                Decimal::ONE,
                Decimal::ZERO,
                Decimal::ZERO,
            )
            .unwrap();
            assert_eq!(adj, expected_adj);
            let adjusted_fee = (adj / price) * pfr;
            close_to(adjusted_fee, expected_adj_fee, dec!(0.0001));
            close_to(adj + adjusted_fee, expected_final, dec!(0.0001));
        }
    }

    #[test]
    fn production_culture_fee_slippage_0() {
        // rate=0.05, exp=1
        for (price, expected_adj, pfr, expected_adj_fee, expected_final) in [
            (
                dec!(0.5),
                dec!(97.5),
                dec!(0.0125),
                dec!(2.4375),
                dec!(99.9375),
            ),
            (
                dec!(0.3),
                dec!(96.5),
                dec!(0.0105),
                dec!(3.3775),
                dec!(99.8775),
            ),
            (
                dec!(0.7),
                dec!(98.5),
                dec!(0.0105),
                dec!(1.4775),
                dec!(99.9775),
            ),
        ] {
            let adj = adjust_buy_amount_for_fees(
                dec!(100),
                price,
                dec!(100),
                dec!(0.05),
                Decimal::ONE,
                Decimal::ZERO,
                Decimal::ZERO,
            )
            .unwrap();
            assert_eq!(adj, expected_adj);
            let adjusted_fee = (adj / price) * pfr;
            close_to(adjusted_fee, expected_adj_fee, dec!(0.0001));
            close_to(adj + adjusted_fee, expected_final, dec!(0.0001));
        }
    }

    #[test]
    fn production_crypto_v2_fee_slippage_0() {
        // rate=0.072, exp=1
        for (price, expected_adj, pfr, expected_adj_fee, expected_final) in [
            (
                dec!(0.5),
                dec!(96.4),
                dec!(0.018),
                dec!(3.4704),
                dec!(99.8704),
            ),
            (
                dec!(0.3),
                dec!(94.96),
                dec!(0.01512),
                dec!(4.785984),
                dec!(99.745984),
            ),
            (
                dec!(0.7),
                dec!(97.84),
                dec!(0.01512),
                dec!(2.113344),
                dec!(99.953344),
            ),
        ] {
            let adj = adjust_buy_amount_for_fees(
                dec!(100),
                price,
                dec!(100),
                dec!(0.072),
                Decimal::ONE,
                Decimal::ZERO,
                Decimal::ZERO,
            )
            .unwrap();
            assert_eq!(adj, expected_adj);
            let adjusted_fee = (adj / price) * pfr;
            close_to(adjusted_fee, expected_adj_fee, dec!(0.0001));
            close_to(adj + adjusted_fee, expected_final, dec!(0.0001));
        }
    }
}
