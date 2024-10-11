use core::convert::TryFrom;
use core::convert::TryInto;
pub use decimal::*;

use anchor_lang::prelude::*;

use crate::utils::{TrackableError, TrackableResult};
use crate::{err, function, location};

pub const PRICE_LIQUIDITY_DENOMINATOR: u128 = 1__0000_0000__0000_0000__00u128;

#[decimal(24)]
#[zero_copy]
#[derive(
    Default, std::fmt::Debug, PartialEq, Eq, PartialOrd, Ord, AnchorSerialize, AnchorDeserialize,
)]
pub struct Price {
    pub v: u128,
}

#[decimal(6)]
#[account(zero_copy)]
#[derive(
    Default, std::fmt::Debug, PartialEq, Eq, PartialOrd, Ord, AnchorSerialize, AnchorDeserialize,
)]
pub struct Liquidity {
    pub v: u128,
}

#[decimal(24)]
#[account(zero_copy)]
#[derive(
    Default, std::fmt::Debug, PartialEq, Eq, PartialOrd, Ord, AnchorSerialize, AnchorDeserialize,
)]
pub struct FeeGrowth {
    pub v: u128,
}

#[decimal(12)]
#[account(zero_copy)]
#[derive(
    Default, std::fmt::Debug, PartialEq, Eq, PartialOrd, Ord, AnchorSerialize, AnchorDeserialize,
)]
pub struct FixedPoint {
    pub v: u128,
}

// legacy not serializable may implement later
#[decimal(0)]
#[derive(Default, std::fmt::Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct TokenAmount(pub u64);

impl FeeGrowth {
    pub fn unchecked_add(self, other: FeeGrowth) -> FeeGrowth {
        FeeGrowth::new(self.get() + other.get())
    }

    pub fn unchecked_sub(self, other: FeeGrowth) -> FeeGrowth {
        FeeGrowth::new(self.get() - other.get())
    }

    pub fn from_fee(liquidity: Liquidity, fee: TokenAmount) -> Self {
        FeeGrowth::new(
            U256::from(fee.get())
                .checked_mul(FeeGrowth::one())
                .unwrap()
                .checked_mul(Liquidity::one())
                .unwrap()
                .checked_div(liquidity.here())
                .unwrap()
                .try_into()
                .unwrap(),
        )
    }

    pub fn to_fee(self, liquidity: Liquidity) -> FixedPoint {
        FixedPoint::new(
            U256::try_from(self.get())
                .unwrap()
                .checked_mul(liquidity.here())
                .unwrap()
                .checked_div(U256::from(10).pow(U256::from(
                    FeeGrowth::scale() + Liquidity::scale() - FixedPoint::scale(),
                )))
                .unwrap()
                .try_into()
                .unwrap_or_else(|_| panic!("value too big to parse in `FeeGrowth::to_fee`")),
        )
    }
}

impl FixedPoint {
    pub fn unchecked_add(self, other: FixedPoint) -> FixedPoint {
        FixedPoint::new(self.get() + other.get())
    }

    pub fn unchecked_sub(self, other: FixedPoint) -> FixedPoint {
        FixedPoint::new(self.get() - other.get())
    }
}

impl Price {
    pub fn big_div_values_to_token(nominator: U256, denominator: U256) -> Option<TokenAmount> {
        let token_amount = nominator
            .checked_mul(Self::one::<U256>())?
            .checked_div(denominator)?
            .checked_div(Self::one::<U256>())?
            .try_into()
            .ok()?;
        Some(TokenAmount::new(token_amount))
    }

    pub fn big_div_values_to_token_up(nominator: U256, denominator: U256) -> Option<TokenAmount> {
        let token_amount = nominator
            .checked_mul(Self::one::<U256>())?
            .checked_add(denominator - 1)?
            .checked_div(denominator)?
            .checked_add(Self::almost_one::<U256>())?
            .checked_div(Self::one::<U256>())?
            .try_into()
            .ok()?;

        Some(TokenAmount::new(token_amount))
    }

    pub fn big_div_values_up(nominator: U256, denominator: U256) -> Price {
        Price::new({
            nominator
                .checked_mul(Self::one::<U256>())
                .unwrap()
                .checked_add(denominator.checked_sub(U256::from(1u32)).unwrap())
                .unwrap()
                .checked_div(denominator)
                .unwrap()
                .try_into()
                .unwrap()
        })
    }

    pub fn checked_big_div_values_up(nominator: U256, denominator: U256) -> TrackableResult<Price> {
        Ok(Price::new(
            nominator
                .checked_mul(Self::one::<U256>())
                .ok_or_else(|| err!(TrackableError::MUL))?
                .checked_add(
                    denominator
                        .checked_sub(U256::from(1u32))
                        .ok_or_else(|| err!(TrackableError::SUB))?,
                )
                .ok_or_else(|| err!(TrackableError::ADD))?
                .checked_div(denominator)
                .ok_or_else(|| err!(TrackableError::DIV))?
                .try_into()
                .map_err(|_| err!(TrackableError::cast::<Self>().as_str()))?,
        ))
    }
}

#[cfg(test)]
pub mod tests {
    use crate::{math::calculate_price_sqrt, structs::MAX_TICK};

    use super::*;

    #[test]
    pub fn test_denominator() {
        assert_eq!(Price::from_integer(1).get(), 1_000000_000000_000000_000000);
        assert_eq!(Liquidity::from_integer(1).get(), 1_000000);
        assert_eq!(
            FeeGrowth::from_integer(1).get(),
            1_000000_000000_000000_000000
        );
        assert_eq!(TokenAmount::from_integer(1).get(), 1);
    }

    #[test]
    pub fn test_ops() {
        let result = TokenAmount::from_integer(1).big_mul(Price::from_integer(1));
        assert_eq!(result.get(), 1);
    }

    #[test]
    fn test_from_fee() {
        // One
        {
            let fee_growth = FeeGrowth::from_fee(Liquidity::from_integer(1), TokenAmount(1));
            assert_eq!(fee_growth, FeeGrowth::from_integer(1));
        }
        // Half
        {
            let fee_growth = FeeGrowth::from_fee(Liquidity::from_integer(2), TokenAmount(1));
            assert_eq!(fee_growth, FeeGrowth::from_scale(5, 1))
        }
        // Little
        {
            let fee_growth = FeeGrowth::from_fee(Liquidity::from_integer(u64::MAX), TokenAmount(1));
            // real    5.42101086242752217003726400434970855712890625 × 10^-20
            // expected 54210
            assert_eq!(fee_growth, FeeGrowth::new(54210))
        }
        // Fairly big
        {
            let fee_growth =
                FeeGrowth::from_fee(Liquidity::from_integer(100), TokenAmount(1_000_000));
            assert_eq!(fee_growth, FeeGrowth::from_integer(10000))
        }
    }

    #[test]
    fn test_to_fee() {
        // equal
        {
            let amount = TokenAmount(100);
            let liquidity = Liquidity::from_integer(1_000_000);

            let fee_growth = FeeGrowth::from_fee(liquidity, amount);
            let out = fee_growth.to_fee(liquidity);
            assert_eq!(out, FixedPoint::from_decimal(amount));
        }
        // greater liquidity
        {
            let amount = TokenAmount(100);
            let liquidity_before = Liquidity::from_integer(1_000_000);
            let liquidity_after = Liquidity::from_integer(10_000_000);

            let fee_growth = FeeGrowth::from_fee(liquidity_before, amount);
            let out = fee_growth.to_fee(liquidity_after);
            assert_eq!(out, FixedPoint::from_integer(1000))
        }
        // huge liquidity
        {
            let amount = TokenAmount(100_000_000__000000);
            let liquidity = Liquidity::from_integer(2u128.pow(77));

            let fee_growth = FeeGrowth::from_fee(liquidity, amount);
            // real    6.61744490042422139897126953655970282852649688720703125 × 10^-22
            // expected 661744490042422
            assert_eq!(fee_growth, FeeGrowth::new(661744490042422));

            let out = fee_growth.to_fee(liquidity);
            // real    9.9999999999999978859343891977453174784 × 10^25
            // expected 99999999999999978859343891
            assert_eq!(out, FixedPoint::new(99999999999999978859343891))
        }
        // overflowing `big_mul`
        {
            let amount = TokenAmount(600000000000000000);
            let liquidity = Liquidity::from_integer(10000000000000000000u128);

            let fee_growth = FeeGrowth::from_fee(liquidity, amount);
            // real     0.06
            // expected 0.06
            assert_eq!(fee_growth, FeeGrowth::new(60000000000000000000000));

            let out = fee_growth.to_fee(liquidity);
            // real     600000000000000000
            // expected 99999999999999978859343891
            assert_eq!(out, FixedPoint::from_integer(1) * amount)
        }
    }

    #[test]
    fn test_decimal_ops() {
        let liquidity = Liquidity::new(4_902_430_892__340393);
        let price: Price = Price::new(9833__489034_289032_430082_130832);

        // real:           4.8208000421189050674873214903955408904296976 × 10^13
        // expected price: 4_8208000421189050674873214903955408904
        // expected liq:   4_8208000421189050674

        let expected = Liquidity::new(48208000421189050674);
        assert_eq!(liquidity.big_mul(price), expected);
        assert_eq!(liquidity.big_mul_up(price), expected + Liquidity::new(1));

        let expected_price = Price::new(48208000421189050674873214903955408904);
        assert_eq!(price.big_mul(liquidity), expected_price);
        assert_eq!(price.big_mul_up(liquidity), expected_price + Price::new(1));
    }

    #[test]
    fn test_big_div_values_to_token() {
        // base examples tested in up-level functions
        let max_sqrt_price = calculate_price_sqrt(MAX_TICK);
        let min_sqrt_price = calculate_price_sqrt(-MAX_TICK);
        let almost_max_sqrt_price = calculate_price_sqrt(MAX_TICK - 1);
        let almost_min_sqrt_price = calculate_price_sqrt(-MAX_TICK + 1);

        // DOMAIN:
        // max_nominator =             22300535562308408361215204585786568048575995442267771385000000000000 (< 2^224)
        // max_no_overflow_nominator = 115792089237316195423570985008687907853269984665640564               (< 2^177)
        // max_denominator =           4294671819208808709990254332190838                                   (< 2^112)
        // min_denominator =           232846648345740                                                      (< 2^48)
        let max_nominator: U256 = U256::from(max_sqrt_price.v) * U256::from(u128::MAX);
        let max_no_overflow_nominator: U256 = U256::MAX / Price::one::<U256>();
        let min_denominator: U256 = min_sqrt_price.big_mul_to_value_up(almost_min_sqrt_price);
        let max_denominator = max_sqrt_price.big_mul_to_value_up(almost_max_sqrt_price);

        // overflow due too large nominator (max nominator)
        {
            let result = Price::big_div_values_to_token(max_nominator, min_denominator);
            assert!(result.is_none())
        }
        // overflow due too large nominator (min overflow nominator)
        {
            let result =
                Price::big_div_values_to_token(max_no_overflow_nominator + 1, min_denominator);
            assert!(result.is_none())
        }
        // result not fits into u64 type (without overflow)
        {
            let result = Price::big_div_values_to_token(max_no_overflow_nominator, min_denominator);
            assert!(result.is_none())
        }
        // result fits intro u64 type (with max denominator)
        {
            let result =
                Price::big_div_values_to_token(max_no_overflow_nominator / 2, max_denominator);
            assert_eq!(result, Some(TokenAmount(13480900766318407300u64)));
        }
    }

    #[test]
    fn test_big_div_values_to_token_up() {
        // base examples tested in up-level functions
        let max_sqrt_price = calculate_price_sqrt(MAX_TICK);
        let min_sqrt_price = calculate_price_sqrt(-MAX_TICK);
        let almost_max_sqrt_price = calculate_price_sqrt(MAX_TICK - 1);
        let almost_min_sqrt_price = calculate_price_sqrt(-MAX_TICK + 1);

        // DOMAIN:
        // max_nominator =             22300535562308408361215204585786568048575995442267771385000000000000 (< 2^224)
        // max_no_overflow_nominator = 115792089237316195423570985008687907853269984665640564               (< 2^177)
        // max_denominator =           4294671819208808709990254332190838                                   (< 2^112)
        // min_denominator =           232846648345740                                                      (< 2^48)
        let max_nominator: U256 = U256::from(max_sqrt_price.v) * U256::from(u128::MAX);
        let max_no_overflow_nominator: U256 = U256::MAX / Price::one::<U256>();
        let min_denominator: U256 = min_sqrt_price.big_mul_to_value(almost_min_sqrt_price);
        let max_denominator = max_sqrt_price.big_mul_to_value(almost_max_sqrt_price);

        // overflow due too large nominator (max nominator)
        {
            let result = Price::big_div_values_to_token_up(max_nominator, min_denominator);
            assert!(result.is_none())
        }
        // overflow due too large nominator (min overflow nominator)
        {
            let result =
                Price::big_div_values_to_token_up(max_no_overflow_nominator + 1, min_denominator);
            assert!(result.is_none())
        }
        // overflow due too large denominator
        {
            let result =
                Price::big_div_values_to_token_up(max_no_overflow_nominator, max_denominator);
            assert!(result.is_none());
        }
        // result not fits into u64 type (without overflow)
        {
            let result =
                Price::big_div_values_to_token_up(max_no_overflow_nominator, min_denominator);
            assert!(result.is_none())
        }
        // result fits intro u64 type (with max denominator)
        {
            let result =
                Price::big_div_values_to_token_up(max_no_overflow_nominator / 2, max_denominator);
            assert_eq!(result, Some(TokenAmount(13480900766318407301u64)));
        }
    }

    #[test]
    fn test_price_overflow() {
        // max_sqrt_price
        {
            let max_sqrt_price = calculate_price_sqrt(MAX_TICK);

            let result = max_sqrt_price.big_mul_to_value(max_sqrt_price);
            let result_up = max_sqrt_price.big_mul_to_value_up(max_sqrt_price);
            let expected_result = U256::from(4294886547443978352291489402946609u128);

            // real:     4294841257.231131321329014894029466
            // expected: 4294886547.443978352291489402946609
            assert_eq!(result, expected_result);
            assert_eq!(result_up, expected_result);
        }
        // min_sqrt_price
        {
            let min_sqrt_price = calculate_price_sqrt(-MAX_TICK);

            let result = min_sqrt_price.big_mul_to_value(min_sqrt_price);
            let result_up = min_sqrt_price.big_mul_to_value_up(min_sqrt_price);
            let expected_result = U256::from(232835005780624u128);

            // real:     0.000000000232835005780624
            // expected: 0.000000000232835005780624
            assert_eq!(result, expected_result);
            assert_eq!(result_up, expected_result);
        }
    }
}
