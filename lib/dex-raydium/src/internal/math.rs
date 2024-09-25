//! Defines PreciseNumber, a U256 wrapper with float-like operations
#![allow(clippy::assign_op_pattern)]
#![allow(clippy::ptr_offset_with_cast)]
#![allow(clippy::manual_range_contains)]
#![allow(unknown_lints)]
#![allow(clippy::reversed_empty_ranges)]
use crate::internal::error::AmmError;
use crate::internal::state::AmmInfo;
use num_traits::CheckedDiv;
use std::{cmp::Eq, convert::TryInto};
use uint::construct_uint;

construct_uint! {
    pub struct U128(2);
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u64)]
pub enum SwapDirection {
    /// Input token pc, output token coin
    PC2Coin = 1u64,
    /// Input token coin, output token pc
    Coin2PC = 2u64,
}

/// The direction to round.  Used for pool token to trading token conversions to
/// avoid losing value on any deposit or withdrawal.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoundDirection {
    /// Floor the value, ie. 1.9 => 1, 1.1 => 1, 1.5 => 1
    Floor,
    /// Ceiling the value, ie. 1.9 => 2, 1.1 => 2, 1.5 => 2
    Ceiling,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Calculator {}

impl Calculator {
    pub fn to_u128(val: u64) -> Result<u128, AmmError> {
        // cannot fail
        Ok(val.into())
    }

    pub fn to_u64(val: u128) -> Result<u64, AmmError> {
        val.try_into().map_err(|_| AmmError::ConversionFailure)
    }

    // out: 0, 1, 2, 3, 5, 8, 13, 21, 34, 55
    pub fn fibonacci(order_num: u64) -> Vec<u64> {
        let mut fb = Vec::new();
        for i in 0..order_num {
            if i == 0 {
                fb.push(0u64);
            } else if i == 1 {
                fb.push(1u64);
            } else if i == 2 {
                fb.push(2u64);
            } else {
                let ret = fb[(i - 1u64) as usize] + fb[(i - 2u64) as usize];
                fb.push(ret);
            };
        }
        fb
    }

    pub fn normalize_decimal(
        val: u64,
        native_decimal: u64,
        sys_decimal_value: u64,
    ) -> anyhow::Result<u64> {
        // e.g., amm.sys_decimal_value is 10**6, native_decimal is 10**9, price is 1.23, this function will convert (1.23*10**9) -> (1.23*10**6)
        //let ret:u64 = val.checked_mul(amm.sys_decimal_value).unwrap().checked_div((10 as u64).pow(native_decimal.into())).unwrap();
        let ret_mut = (U128::from(val))
            .checked_mul(sys_decimal_value.into())
            .ok_or(anyhow::format_err!("mul error"))?;
        Ok(Self::to_u64(
            ret_mut
                .checked_div(
                    U128::from(10)
                        .checked_pow(native_decimal.into())
                        .ok_or(anyhow::format_err!("pow error"))?,
                )
                .ok_or(anyhow::format_err!("div error"))?
                .as_u128(),
        )?)
    }

    pub fn restore_decimal(
        val: U128,
        native_decimal: u64,
        sys_decimal_value: u64,
    ) -> anyhow::Result<U128> {
        // e.g., amm.sys_decimal_value is 10**6, native_decimal is 10**9, price is 1.23, this function will convert (1.23*10**6) -> (1.23*10**9)
        // let ret:u64 = val.checked_mul((10 as u64).pow(native_decimal.into())).unwrap().checked_div(amm.sys_decimal_value).unwrap();
        let ret_mut = val
            .checked_mul(
                U128::from(10)
                    .checked_pow(native_decimal.into())
                    .ok_or(anyhow::format_err!("pow error"))?,
            )
            .ok_or(anyhow::format_err!("mul error"))?;
        Ok(ret_mut
            .checked_div(sys_decimal_value.into())
            .ok_or(anyhow::format_err!("div error"))?)
    }

    // convert srm pc_lot_size -> internal pc_lot_size
    pub fn convert_in_pc_lot_size(
        pc_decimals: u8,
        coin_decimals: u8,
        pc_lot_size: u64,
        coin_lot_size: u64,
        sys_decimal_value: u64,
    ) -> anyhow::Result<u64> {
        Ok(Self::to_u64(
            U128::from(pc_lot_size)
                .checked_mul(sys_decimal_value.into())
                .ok_or(anyhow::format_err!("mul error"))?
                .checked_mul(
                    U128::from(10)
                        .checked_pow(coin_decimals.into())
                        .ok_or(anyhow::format_err!("pow error"))?,
                )
                .ok_or(anyhow::format_err!("mul error"))?
                .checked_div(
                    U128::from(coin_lot_size)
                        .checked_mul(
                            U128::from(10)
                                .checked_pow(pc_decimals.into())
                                .ok_or(anyhow::format_err!("pow error"))?,
                        )
                        .ok_or(anyhow::format_err!("mul error"))?,
                )
                .ok_or(anyhow::format_err!("div error"))?
                .as_u128(),
        )?)
    }

    pub fn calc_total_without_take_pnl_no_orderbook(
        pc_amount: u64,
        coin_amount: u64,
        amm: &AmmInfo,
    ) -> anyhow::Result<(u64, u64)> {
        let total_pc_without_take_pnl = pc_amount
            .checked_sub(amm.state_data.need_take_pnl_pc)
            .ok_or(anyhow::format_err!("sub error"))?;
        let total_coin_without_take_pnl = coin_amount
            .checked_sub(amm.state_data.need_take_pnl_coin)
            .ok_or(anyhow::format_err!("sub error"))?;
        Ok((total_pc_without_take_pnl, total_coin_without_take_pnl))
    }

    pub fn swap_token_amount_base_in(
        amount_in: U128,
        total_pc_without_take_pnl: U128,
        total_coin_without_take_pnl: U128,
        swap_direction: SwapDirection,
    ) -> anyhow::Result<U128> {
        match swap_direction {
            SwapDirection::Coin2PC => {
                // (x + delta_x) * (y + delta_y) = x * y
                // (coin + amount_in) * (pc - amount_out) = coin * pc
                // => amount_out = pc - coin * pc / (coin + amount_in)
                // => amount_out = ((pc * coin + pc * amount_in) - coin * pc) / (coin + amount_in)
                // => amount_out =  pc * amount_in / (coin + amount_in)
                let denominator = total_coin_without_take_pnl
                    .checked_add(amount_in)
                    .ok_or(anyhow::format_err!("add error"))?;
                Ok(total_pc_without_take_pnl
                    .checked_mul(amount_in)
                    .ok_or(anyhow::format_err!("mul error"))?
                    .checked_div(denominator)
                    .ok_or(anyhow::format_err!("div error"))?)
            }
            SwapDirection::PC2Coin => {
                // (x + delta_x) * (y + delta_y) = x * y
                // (pc + amount_in) * (coin - amount_out) = coin * pc
                // => amount_out = coin - coin * pc / (pc + amount_in)
                // => amount_out = (coin * pc + coin * amount_in - coin * pc) / (pc + amount_in)
                // => amount_out = coin * amount_in / (pc + amount_in)
                let denominator = total_pc_without_take_pnl
                    .checked_add(amount_in)
                    .ok_or(anyhow::format_err!("sub error"))?;
                Ok(total_coin_without_take_pnl
                    .checked_mul(amount_in)
                    .ok_or(anyhow::format_err!("mul error"))?
                    .checked_div(denominator)
                    .ok_or(anyhow::format_err!("div error"))?)
            }
        }
    }

    pub fn swap_token_amount_base_out(
        amount_out: U128,
        total_pc_without_take_pnl: U128,
        total_coin_without_take_pnl: U128,
        swap_direction: SwapDirection,
    ) -> anyhow::Result<U128> {
        match swap_direction {
            SwapDirection::Coin2PC => {
                // (x + delta_x) * (y + delta_y) = x * y
                // (coin + amount_in) * (pc - amount_out) = coin * pc
                // => amount_in = coin * pc / (pc - amount_out) - coin
                // => amount_in = (coin * pc - pc * coin + amount_out * coin) / (pc - amount_out)
                // => amount_in = (amount_out * coin) / (pc - amount_out)
                let denominator = total_pc_without_take_pnl
                    .checked_sub(amount_out)
                    .ok_or(anyhow::format_err!("sub error"))?;
                Ok(total_coin_without_take_pnl
                    .checked_mul(amount_out)
                    .ok_or(anyhow::format_err!("mul error"))?
                    .checked_ceil_div(denominator)
                    .ok_or(anyhow::format_err!("ceil div error"))?
                    .0)
            }
            SwapDirection::PC2Coin => {
                // (x + delta_x) * (y + delta_y) = x * y
                // (pc + amount_in) * (coin - amount_out) = coin * pc
                // => amount_out = coin - coin * pc / (pc + amount_in)
                // => amount_out = (coin * pc + coin * amount_in - coin * pc) / (pc + amount_in)
                // => amount_out = coin * amount_in / (pc + amount_in)

                // => amount_in = coin * pc / (coin - amount_out) - pc
                // => amount_in = (coin * pc - pc * coin + pc * amount_out) / (coin - amount_out)
                // => amount_in = (pc * amount_out) / (coin - amount_out)
                let denominator = total_coin_without_take_pnl
                    .checked_sub(amount_out)
                    .ok_or(anyhow::format_err!("sub error"))?;
                Ok(total_pc_without_take_pnl
                    .checked_mul(amount_out)
                    .ok_or(anyhow::format_err!("mul error"))?
                    .checked_ceil_div(denominator)
                    .ok_or(anyhow::format_err!("ceil div error"))?
                    .0)
            }
        }
    }
}

/// The invariant calculator.
pub struct InvariantToken {
    /// Token coin
    pub token_coin: u64,
    /// Token pc
    pub token_pc: u64,
}

/// The invariant calculator.
pub struct InvariantPool {
    /// Token input
    pub token_input: u64,
    /// Token total
    pub token_total: u64,
}

/// Perform a division that does not truncate value from either side, returning
/// the (quotient, divisor) as a tuple
///
/// When dividing integers, we are often left with a remainder, which can
/// cause information to be lost.  By checking for a remainder, adjusting
/// the quotient, and recalculating the divisor, this provides the most fair
/// calculation.
///
/// For example, 400 / 32 = 12, with a remainder cutting off 0.5 of amount.
/// If we simply ceiling the quotient to 13, then we're saying 400 / 32 = 13, which
/// also cuts off value.  To improve this result, we calculate the other way
/// around and again check for a remainder: 400 / 13 = 30, with a remainder of
/// 0.77, and we ceiling that value again.  This gives us a final calculation
/// of 400 / 31 = 13, which provides a ceiling calculation without cutting off
/// more value than needed.
///
/// This calculation fails if the divisor is larger than the dividend, to avoid
/// having a result like: 1 / 1000 = 1.
pub trait CheckedCeilDiv: Sized {
    /// Perform ceiling division
    fn checked_ceil_div(&self, rhs: Self) -> Option<(Self, Self)>;
}

impl CheckedCeilDiv for u128 {
    fn checked_ceil_div(&self, mut rhs: Self) -> Option<(Self, Self)> {
        let mut quotient = self.checked_div(&rhs)?;
        // Avoid dividing a small number by a big one and returning 1, and instead
        // fail.
        if quotient == 0 {
            // return None;
            if self.checked_mul(2u128)? >= rhs {
                return Some((1, 0));
            } else {
                return Some((0, 0));
            }
        }

        // Ceiling the destination amount if there's any remainder, which will
        // almost always be the case.
        let remainder = self.checked_rem(rhs)?;
        if remainder > 0 {
            quotient = quotient.checked_add(1)?;
            // calculate the minimum amount needed to get the dividend amount to
            // avoid truncating too much
            rhs = self.checked_div(&quotient)?;
            let remainder = self.checked_rem(quotient)?;
            if remainder > 0 {
                rhs = rhs.checked_add(1)?;
            }
        }
        Some((quotient, rhs))
    }
}

impl CheckedCeilDiv for U128 {
    fn checked_ceil_div(&self, mut rhs: Self) -> Option<(Self, Self)> {
        let mut quotient = self.checked_div(rhs)?;
        // Avoid dividing a small number by a big one and returning 1, and instead
        // fail.
        let zero = U128::from(0);
        let one = U128::from(1);
        if quotient.is_zero() {
            // return None;
            if self.checked_mul(U128::from(2))? >= rhs {
                return Some((one, zero));
            } else {
                return Some((zero, zero));
            }
        }

        // Ceiling the destination amount if there's any remainder, which will
        // almost always be the case.
        let remainder = self.checked_rem(rhs)?;
        if remainder > zero {
            quotient = quotient.checked_add(one)?;
            // calculate the minimum amount needed to get the dividend amount to
            // avoid truncating too much
            rhs = self.checked_div(quotient)?;
            let remainder = self.checked_rem(quotient)?;
            if remainder > zero {
                rhs = rhs.checked_add(one)?;
            }
        }
        Some((quotient, rhs))
    }
}
