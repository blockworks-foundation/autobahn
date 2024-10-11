use crate::{err, from_result, function, location, ok_or_mark_trace, trace};
use std::{cell::RefMut, convert::TryInto};

use anchor_lang::*;

use crate::{
    decimals::*,
    errors::InvariantErrorCode,
    structs::{get_search_limit, Pool, Tick, Tickmap, MAX_TICK, TICK_LIMIT},
    utils::{TrackableError, TrackableResult},
};

#[derive(PartialEq, Debug)]
pub struct SwapResult {
    pub next_price_sqrt: Price,
    pub amount_in: TokenAmount,
    pub amount_out: TokenAmount,
    pub fee_amount: TokenAmount,
}

// converts ticks to price with reduced precision
pub fn calculate_price_sqrt(tick_index: i32) -> Price {
    // checking if tick be converted to price (overflows if more)
    let tick = tick_index.abs();
    assert!(tick <= MAX_TICK, "tick over bounds");

    let mut price = FixedPoint::from_integer(1);

    if tick & 0x1 != 0 {
        price *= FixedPoint::new(1000049998750);
    }
    if tick & 0x2 != 0 {
        price *= FixedPoint::new(1000100000000);
    }
    if tick & 0x4 != 0 {
        price *= FixedPoint::new(1000200010000);
    }
    if tick & 0x8 != 0 {
        price *= FixedPoint::new(1000400060004);
    }
    if tick & 0x10 != 0 {
        price *= FixedPoint::new(1000800280056);
    }
    if tick & 0x20 != 0 {
        price *= FixedPoint::new(1001601200560);
    }
    if tick & 0x40 != 0 {
        price *= FixedPoint::new(1003204964963);
    }
    if tick & 0x80 != 0 {
        price *= FixedPoint::new(1006420201726);
    }
    if tick & 0x100 != 0 {
        price *= FixedPoint::new(1012881622442);
    }
    if tick & 0x200 != 0 {
        price *= FixedPoint::new(1025929181080);
    }
    if tick & 0x400 != 0 {
        price *= FixedPoint::new(1052530684591);
    }
    if tick & 0x800 != 0 {
        price *= FixedPoint::new(1107820842005);
    }
    if tick & 0x1000 != 0 {
        price *= FixedPoint::new(1227267017980);
    }
    if tick & 0x2000 != 0 {
        price *= FixedPoint::new(1506184333421);
    }
    if tick & 0x4000 != 0 {
        price *= FixedPoint::new(2268591246242);
    }
    if tick & 0x8000 != 0 {
        price *= FixedPoint::new(5146506242525);
    }
    if tick & 0x0001_0000 != 0 {
        price *= FixedPoint::new(26486526504348);
    }
    if tick & 0x0002_0000 != 0 {
        price *= FixedPoint::new(701536086265529);
    }

    // Parsing to the Price type by the end by convention (should always have 12 zeros at the end)
    if tick_index >= 0 {
        Price::from_decimal(price)
    } else {
        Price::from_decimal(FixedPoint::from_integer(1).big_div(price))
    }
}

// Finds closes initialized tick in direction of trade
// and compares its price to the price limit of the trade
pub fn get_closer_limit(
    sqrt_price_limit: Price,
    x_to_y: bool,
    current_tick: i32, // tick already scaled by tick_spacing
    tick_spacing: u16,
    tickmap: &Tickmap,
) -> Result<(Price, Option<(i32, bool)>)> {
    // find initalized tick (None also for virtual tick limiated by search scope)
    let closes_tick_index = if x_to_y {
        tickmap.prev_initialized(current_tick, tick_spacing)
    } else {
        tickmap.next_initialized(current_tick, tick_spacing)
    };

    match closes_tick_index {
        Some(index) => {
            let price = calculate_price_sqrt(index);
            // trunk-ignore(clippy/if_same_then_else)
            if x_to_y && price > sqrt_price_limit {
                Ok((price, Some((index, true))))
            } else if !x_to_y && price < sqrt_price_limit {
                Ok((price, Some((index, true))))
            } else {
                Ok((sqrt_price_limit, None))
            }
        }
        None => {
            let index = get_search_limit(current_tick, tick_spacing, !x_to_y);
            let price = calculate_price_sqrt(index);

            require!(current_tick != index, InvariantErrorCode::LimitReached);

            // trunk-ignore(clippy/if_same_then_else)
            if x_to_y && price > sqrt_price_limit {
                Ok((price, Some((index, false))))
            } else if !x_to_y && price < sqrt_price_limit {
                Ok((price, Some((index, false))))
            } else {
                Ok((sqrt_price_limit, None))
            }
        }
    }
}

pub fn compute_swap_step(
    current_price_sqrt: Price,
    target_price_sqrt: Price,
    liquidity: Liquidity, // pool.liquidity
    amount: TokenAmount,  // reaming_amount (input or output depending on by_amount_in)
    by_amount_in: bool,
    fee: FixedPoint, // pool.fee
) -> TrackableResult<SwapResult> {
    if liquidity.is_zero() {
        return Ok(SwapResult {
            next_price_sqrt: target_price_sqrt,
            amount_in: TokenAmount(0),
            amount_out: TokenAmount(0),
            fee_amount: TokenAmount(0),
        });
    }

    let x_to_y = current_price_sqrt >= target_price_sqrt;

    let next_price_sqrt;
    let mut amount_in = TokenAmount(0);
    let mut amount_out = TokenAmount(0);

    if by_amount_in {
        // take fee in input_amount
        // U256(2^64) * U256(1e12) - no overflow in intermediate operations
        // no overflow in token_amount result
        let amount_after_fee = amount.big_mul(
            FixedPoint::from_integer(1u8)
                .checked_sub(fee)
                .map_err(|_| err!("sub underflow"))?,
        );

        amount_in = if x_to_y {
            get_delta_x(target_price_sqrt, current_price_sqrt, liquidity, true)
        } else {
            get_delta_y(current_price_sqrt, target_price_sqrt, liquidity, true)
        }
        .unwrap_or(TokenAmount(u64::MAX));

        // if target price was hit it will be the next price
        if amount_after_fee >= amount_in {
            next_price_sqrt = target_price_sqrt
        } else {
            // DOMAIN:
            // liquidity = U128::MAX
            // amount_after_fee = U64::MAX
            // current_price_sqrt = entire price space
            next_price_sqrt = ok_or_mark_trace!(get_next_sqrt_price_from_input(
                current_price_sqrt,
                liquidity,
                amount_after_fee,
                x_to_y,
            ))?;
        };
    } else {
        amount_out = if x_to_y {
            get_delta_y(target_price_sqrt, current_price_sqrt, liquidity, false)
        } else {
            get_delta_x(current_price_sqrt, target_price_sqrt, liquidity, false)
        }
        .unwrap_or(TokenAmount(u64::MAX));

        if amount >= amount_out {
            next_price_sqrt = target_price_sqrt
        } else {
            next_price_sqrt = ok_or_mark_trace!(get_next_sqrt_price_from_output(
                current_price_sqrt,
                liquidity,
                amount,
                x_to_y
            ))?;
        }
    }

    let not_max = target_price_sqrt != next_price_sqrt;

    if x_to_y {
        if not_max || !by_amount_in {
            amount_in = get_delta_x(next_price_sqrt, current_price_sqrt, liquidity, true)
                .ok_or_else(|| err!("get_delta_x overflow"))?;
        };
        if not_max || by_amount_in {
            amount_out = get_delta_y(next_price_sqrt, current_price_sqrt, liquidity, false)
                .ok_or_else(|| err!("get_delta_y overflow"))?;
        }
    } else {
        if not_max || !by_amount_in {
            amount_in = get_delta_y(current_price_sqrt, next_price_sqrt, liquidity, true)
                .ok_or_else(|| err!("get_delta_y overflow"))?;
        };
        if not_max || by_amount_in {
            amount_out = get_delta_x(current_price_sqrt, next_price_sqrt, liquidity, false)
                .ok_or_else(|| err!("get_delta_x overflow"))?;
        };
    }

    // Amount out can not exceed amount
    if !by_amount_in && amount_out > amount {
        amount_out = amount;
    }

    let fee_amount = if by_amount_in && next_price_sqrt != target_price_sqrt {
        // no possible to overflow in intermediate operations
        // edge case occurs when the next_price is target_price (minimal distance to target)
        amount
            .checked_sub(amount_in)
            .map_err(|_| err!("sub underflow"))?
    } else {
        // no possible to overflow in intermediate operations
        // edge case when amount_in is maximum and fee is maximum
        amount_in.big_mul_up(fee)
    };

    Ok(SwapResult {
        next_price_sqrt,
        amount_in,
        amount_out,
        fee_amount,
    })
}

// delta x = (L * delta_sqrt_price) / (lower_sqrt_price * higher_sqrt_price)
pub fn get_delta_x(
    sqrt_price_a: Price,
    sqrt_price_b: Price,
    liquidity: Liquidity,
    up: bool,
) -> Option<TokenAmount> {
    let delta_price = if sqrt_price_a > sqrt_price_b {
        sqrt_price_a - sqrt_price_b
    } else {
        sqrt_price_b - sqrt_price_a
    };

    let nominator = delta_price.big_mul_to_value(liquidity);
    match up {
        true => Price::big_div_values_to_token_up(
            nominator,
            sqrt_price_a.big_mul_to_value(sqrt_price_b),
        ),
        false => Price::big_div_values_to_token(
            nominator,
            sqrt_price_a.big_mul_to_value_up(sqrt_price_b),
        ),
    }
}

// delta y = L * delta_sqrt_price
pub fn get_delta_y(
    sqrt_price_a: Price,
    sqrt_price_b: Price,
    liquidity: Liquidity,
    up: bool,
) -> Option<TokenAmount> {
    let delta_price = if sqrt_price_a > sqrt_price_b {
        sqrt_price_a - sqrt_price_b
    } else {
        sqrt_price_b - sqrt_price_a
    };

    match match up {
        true => delta_price
            .big_mul_to_value_up(liquidity)
            .checked_add(Price::almost_one())
            .unwrap()
            .checked_div(Price::one())
            .unwrap()
            .try_into(),
        false => delta_price
            .big_mul_to_value(liquidity)
            .checked_div(Price::one())
            .unwrap()
            .try_into(),
    } {
        Ok(x) => Some(TokenAmount(x)),
        Err(_) => None,
    }
}

fn get_next_sqrt_price_from_input(
    price_sqrt: Price,
    liquidity: Liquidity,
    amount: TokenAmount,
    x_to_y: bool,
) -> TrackableResult<Price> {
    if liquidity.is_zero() {
        return Err(err!("getting next price from input with zero liquidity"));
    }
    if price_sqrt.is_zero() {
        return Err(err!("getting next price from input with zero price"));
    }
    // DOMAIN:
    // price_sqrt <sqrt_price_at_min_tick, sqrt_price_at_max_tick>
    // pool.liquidity <1, u128::MAX>
    // amount <1, u64::MAX>

    let result = if x_to_y {
        // checked
        get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, true)
    } else {
        // checked
        get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, true)
    };
    ok_or_mark_trace!(result)
}

fn get_next_sqrt_price_from_output(
    price_sqrt: Price,
    liquidity: Liquidity,
    amount: TokenAmount,
    x_to_y: bool,
) -> TrackableResult<Price> {
    // DOMAIN:
    // price_sqrt <sqrt_price_at_min_tick, sqrt_price_at_max_tick>
    // pool.liquidity <1, u128::MAX>
    // amount <1, u64::MAX>

    if liquidity.is_zero() {
        return Err(err!("getting next price from output with zero liquidity"));
    }
    if price_sqrt.is_zero() {
        return Err(err!("getting next price from output with zero price"));
    }

    let result = if x_to_y {
        get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, false)
    } else {
        get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, false)
    };
    ok_or_mark_trace!(result)
}

// L * price / (L +- amount * price)
fn get_next_sqrt_price_x_up(
    price_sqrt: Price,
    liquidity: Liquidity,
    amount: TokenAmount,
    add: bool,
) -> TrackableResult<Price> {
    // DOMAIN:
    // In case add always true
    // pool.liquidity = U128::MAX
    // amount = U64::MAX
    // price_sqrt = entire price space

    if amount.is_zero() {
        return Ok(price_sqrt);
    };

    // PRICE_LIQUIDITY_DENOMINATOR = 10 ^ (24 - 6)
    // max_big_liquidity -> ceil(log2(2^128 * 10^18)) = 188
    // no possibility of overflow here
    let big_liquidity = liquidity
        .here::<U256>()
        .checked_mul(U256::from(PRICE_LIQUIDITY_DENOMINATOR)) // extends liquidity precision (operation on U256, so there is no dividing by denominator)
        .ok_or_else(|| err!("mul overflow"))?;

    // max(price * amount)
    // ceil(log2(max_price * 2^64))= 160
    // U256::from(max_price) * U256::from(2^64) / U256::(1)
    // so not possible to overflow here
    let denominator = from_result!(match add {
        // max_denominator = L + amount * price [maximize all parameters]
        // max_denominator 2^128 + 2^64 * 2^96 = 2^161 <- no possible to overflow
        true => big_liquidity.checked_add(price_sqrt.big_mul_to_value(amount)),
        false => big_liquidity.checked_sub(price_sqrt.big_mul_to_value(amount)),
    }
    .ok_or_else(|| "big_liquidity -/+ price_sqrt * amount"))?; // never should be triggered

    // max_nominator = (U256::from(max_price) * U256::from(max_liquidity) + 10^6) / 10^6
    // max_nominator = (2^96 * 2^128 + 10^6) / 10^6
    // ceil(log2(2^96 * 2^128 + 10^6)) = 225
    // ceil(log2((2^96 * 2^128 + 10^6)/10^6)) = 205
    // ceil(lg2(max_nominator)) = 205
    // no possibility of overflowing in the result or in intermediate calculations

    // result = div_up(nominator, denominator) -> so maximizing nominator while minimizing denominator
    // max_results = (max_nominator * Price::one + min_denominator) / min_denominator
    // (2^205 * 10^24 + 1) / 1 = 2^285 <- possible to overflow in result

    // maximize nominator -> (max_nominator * Price::one + max_denominator)
    // 2^205 * 10^24 + 2^161 = 2^285 <- possible to overflow in intermediate operations
    ok_or_mark_trace!(Price::checked_big_div_values_up(
        price_sqrt.big_mul_to_value_up(liquidity),
        denominator
    ))
}

// price +- (amount / L)
fn get_next_sqrt_price_y_down(
    price_sqrt: Price,
    liquidity: Liquidity,
    amount: TokenAmount,
    add: bool,
) -> TrackableResult<Price> {
    // DOMAIN:
    // price_sqrt <sqrt_price_at_min_tick, sqrt_price_at_max_tick>
    // pool.liquidity <1, u128::MAX> (zero liquidity not possible)
    // amount <1, u64::MAX>

    // quotient= amount / L
    // PRICE_LIQUIDITY_DENOMINATOR = 10 ^ (24 - 6)

    if add {
        // Price::from_scale(amount, TokenAmount::scale())
        // max_nominator = max_amount * 10^24 => 2^144 so possible to overflow here

        // max_denominator = max_liquidity
        // max_denominator = U256(u128::MAX) * U256(10^18)
        // max_denominator = U256(2^128 * 10^18) ~ 2^188 so no possible to overflow

        // quotient - max quotient nominator
        // quotient_max_nominator = U256(max_nominator) * U256(10^24)
        // quotient_max_nominator = 2^128 * 10^24 ~ 2^208 so no possible to overflow in intermediate operations

        // max_quotient = max_nominator / min_denominator
        // max_quotient = 2^128 * 10^24 / 10^18 ~ 2^148 so possible to overflow in max_quote
        let quotient = from_result!(Price::checked_from_decimal(amount)
            .map_err(|err| err!(&err))? // TODO: add util macro to map str -> TrackableError
            .checked_big_div_by_number(
                U256::from(liquidity.get())
                    .checked_mul(U256::from(PRICE_LIQUIDITY_DENOMINATOR))
                    .ok_or_else(|| err!("mul overflow"))?,
            ))?;
        // max_quotient = 2^128
        // price_sqrt = 2^96
        // possible to overflow in result
        from_result!(price_sqrt.checked_add(quotient))
    } else {
        // Price::from_scale - same as case above
        let quotient = from_result!(Price::checked_from_decimal(amount)
            .map_err(|err| err!(&err))? // TODO: add util macro to map str -> TrackableError
            .checked_big_div_by_number_up(
                U256::from(liquidity.get())
                    .checked_mul(U256::from(PRICE_LIQUIDITY_DENOMINATOR))
                    .ok_or_else(|| err!("mul overflow"))?,
            ))?;
        from_result!(price_sqrt.checked_sub(quotient))
    }
}

pub fn is_enough_amount_to_push_price(
    amount: TokenAmount,
    current_price_sqrt: Price,
    liquidity: Liquidity,
    fee: FixedPoint,
    by_amount_in: bool,
    x_to_y: bool,
) -> TrackableResult<bool> {
    if liquidity.is_zero() {
        return Ok(true);
    }

    let next_price_sqrt = ok_or_mark_trace!(if by_amount_in {
        let amount_after_fee = amount.big_mul(
            FixedPoint::from_integer(1)
                .checked_sub(fee)
                .map_err(|_| err!("sub underflow"))?,
        );
        get_next_sqrt_price_from_input(current_price_sqrt, liquidity, amount_after_fee, x_to_y)
    } else {
        get_next_sqrt_price_from_output(current_price_sqrt, liquidity, amount, x_to_y)
    })?;

    Ok(current_price_sqrt.ne(&next_price_sqrt))
}

pub fn cross_tick(tick: &mut RefMut<Tick>, pool: &mut Pool) -> Result<()> {
    tick.fee_growth_outside_x = pool
        .fee_growth_global_x
        .unchecked_sub(tick.fee_growth_outside_x);
    tick.fee_growth_outside_y = pool
        .fee_growth_global_y
        .unchecked_sub(tick.fee_growth_outside_y);

    // When going to higher tick net_liquidity should be added and for going lower subtracted
    let new_liquidity = if (pool.current_tick_index >= tick.index) ^ tick.sign {
        pool.liquidity.checked_add(tick.liquidity_change)
    } else {
        pool.liquidity.checked_sub(tick.liquidity_change)
    };

    pool.liquidity = new_liquidity.map_err(|_| InvariantErrorCode::InvalidPoolLiquidity)?;
    Ok(())
}

pub fn get_max_tick(tick_spacing: u16) -> TrackableResult<i32> {
    let limit_by_space = TICK_LIMIT
        .checked_sub(1)
        .ok_or_else(|| err!("sub underflow"))?
        .checked_mul(tick_spacing.into())
        .ok_or_else(|| err!("mul overflow"))?;
    Ok(limit_by_space.min(MAX_TICK))
}

pub fn get_min_tick(tick_spacing: u16) -> TrackableResult<i32> {
    let limit_by_space = (-TICK_LIMIT)
        .checked_add(1)
        .ok_or_else(|| err!("add overflow"))?
        .checked_mul(tick_spacing.into())
        .ok_or_else(|| err!("mul overflow"))?;
    Ok(limit_by_space.max(-MAX_TICK))
}

pub fn get_max_sqrt_price(tick_spacing: u16) -> TrackableResult<Price> {
    let max_tick = get_max_tick(tick_spacing);
    Ok(calculate_price_sqrt(max_tick?))
}

pub fn get_min_sqrt_price(tick_spacing: u16) -> TrackableResult<Price> {
    let min_tick = get_min_tick(tick_spacing);
    Ok(calculate_price_sqrt(min_tick?))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use decimal::{BetweenDecimals, BigOps, Decimal, Factories};

    use crate::{
        decimals::{FixedPoint, Liquidity, Price, TokenAmount},
        math::{
            compute_swap_step, cross_tick, get_delta_x, get_delta_y, get_max_sqrt_price,
            get_max_tick, get_min_sqrt_price, get_min_tick, get_next_sqrt_price_from_input,
            get_next_sqrt_price_from_output, get_next_sqrt_price_x_up, get_next_sqrt_price_y_down,
            SwapResult,
        },
        structs::{Pool, Tick, MAX_TICK},
        utils::TrackableError,
        MAX_SQRT_PRICE, MIN_SQRT_PRICE,
    };

    use super::{calculate_price_sqrt, is_enough_amount_to_push_price, FeeGrowth};

    #[test]
    fn test_compute_swap_step() {
        // VALIDATE BASE SAMPLES
        // one token by amount in
        {
            let price = Price::from_integer(1);
            let target = Price::new(1004987562112089027021926);
            let liquidity = Liquidity::from_integer(2000);
            let amount = TokenAmount(1);
            let fee = FixedPoint::from_scale(6, 4);

            let result = compute_swap_step(price, target, liquidity, amount, true, fee).unwrap();

            let expected_result = SwapResult {
                next_price_sqrt: price,
                amount_in: TokenAmount(0),
                amount_out: TokenAmount(0),
                fee_amount: TokenAmount(1),
            };
            assert_eq!(result, expected_result)
        }
        // amount out capped at target price
        {
            let price = Price::from_integer(1);
            let target = Price::new(1004987562112089027021926);
            let liquidity = Liquidity::from_integer(2000);
            let amount = TokenAmount(20);
            let fee = FixedPoint::from_scale(6, 4);

            let result_in = compute_swap_step(price, target, liquidity, amount, true, fee).unwrap();
            let result_out =
                compute_swap_step(price, target, liquidity, amount, false, fee).unwrap();

            let expected_result = SwapResult {
                next_price_sqrt: target,
                amount_in: TokenAmount(10),
                amount_out: TokenAmount(9),
                fee_amount: TokenAmount(1),
            };
            assert_eq!(result_in, expected_result);
            assert_eq!(result_out, expected_result);
        }
        // amount in not capped
        {
            let price = Price::from_scale(101, 2);
            let target = Price::from_integer(10);
            let liquidity = Liquidity::from_integer(300000000);
            let amount = TokenAmount(1000000);
            let fee = FixedPoint::from_scale(6, 4);

            let result = compute_swap_step(price, target, liquidity, amount, true, fee).unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: Price::new(1013331333333_333333333333),
                amount_in: TokenAmount(999400),
                amount_out: TokenAmount(976487), // ((1.013331333333 - 1.01) * 300000000) / (1.013331333333 * 1.01)
                fee_amount: TokenAmount(600),
            };
            assert_eq!(result, expected_result)
        }
        // amount out not capped
        {
            let price = Price::from_integer(101);
            let target = Price::from_integer(100);
            let liquidity = Liquidity::from_integer(5000000000000u128);
            let amount = TokenAmount(2000000);
            let fee = FixedPoint::from_scale(6, 4);

            let result = compute_swap_step(price, target, liquidity, amount, false, fee).unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: Price::new(100999999600000_000000000000),
                amount_in: TokenAmount(197), // (5000000000000 * (101 - 100.9999996)) /  (101 * 100.9999996)
                amount_out: amount,
                fee_amount: TokenAmount(1),
            };
            assert_eq!(result, expected_result)
        }
        // empty swap step when price is at tick
        {
            let current_price_sqrt = Price::new(999500149965_000000000000);
            let target_price_sqrt = Price::new(999500149965_000000000000);

            let liquidity = Liquidity::new(20006000_000000);
            let amount = TokenAmount(1_000_000);
            let by_amount_in = true;
            let fee = FixedPoint::from_scale(6, 4); // 0.0006 -> 0.06%

            let result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                amount,
                by_amount_in,
                fee,
            )
            .unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: current_price_sqrt,
                amount_in: TokenAmount(0),
                amount_out: TokenAmount(0),
                fee_amount: TokenAmount(0),
            };
            assert_eq!(result, expected_result)
        }
        // empty swap step by amount out when price is at tick
        {
            let current_price_sqrt = Price::new(999500149965_000000000000);
            let target_price_sqrt = Price::from_integer(1);
            let liquidity = Liquidity::new(u128::MAX / 1_000000);
            let amount = TokenAmount(1);
            let by_amount_in = false;
            let fee = FixedPoint::from_scale(6, 4); // 0.0006 -> 0.06%

            let result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                amount,
                by_amount_in,
                fee,
            )
            .unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: Price::new(999500149965_000000000001),
                amount_in: TokenAmount(341),
                amount_out: TokenAmount(1),
                fee_amount: TokenAmount(1),
            };
            assert_eq!(result, expected_result)
        }
        // if liquidity is high, small amount in should not push price
        {
            let current_price_sqrt = Price::from_scale(999500149965u128, 12);
            let target_price_sqrt = Price::from_scale(1999500149965u128, 12);
            let liquidity = Liquidity::from_integer(100_000000000000_000000000000u128);
            let amount = TokenAmount(10);
            let by_amount_in = true;
            let fee = FixedPoint::from_scale(6, 4); // 0.0006 -> 0.06%

            let result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                amount,
                by_amount_in,
                fee,
            )
            .unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: current_price_sqrt,
                amount_in: TokenAmount(0),
                amount_out: TokenAmount(0),
                fee_amount: TokenAmount(10),
            };
            assert_eq!(result, expected_result)
        }
        // amount_in > u64 for swap to target price and when liquidity > 2^64
        {
            let current_price_sqrt = Price::from_integer(1);
            let target_price_sqrt = Price::from_scale(100005, 5); // 1.00005
            let liquidity = Liquidity::from_integer(368944000000_000000000000u128);
            let amount = TokenAmount(1);
            let by_amount_in = true;
            let fee = FixedPoint::from_scale(6, 4); // 0.0006 -> 0.06%

            let result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                amount,
                by_amount_in,
                fee,
            )
            .unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: current_price_sqrt,
                amount_in: TokenAmount(0),
                amount_out: TokenAmount(0),
                fee_amount: TokenAmount(1),
            };
            assert_eq!(result, expected_result)
        }
        // amount_out > u64 for swap to target price and when liquidity > 2^64
        {
            let current_price_sqrt = Price::from_integer(1);
            let target_price_sqrt = Price::from_scale(100005, 5); // 1.00005
            let liquidity = Liquidity::from_integer(368944000000_000000000000u128);
            let amount = TokenAmount(1);
            let by_amount_in = false;
            let fee = FixedPoint::from_scale(6, 4); // 0.0006 -> 0.06%

            let result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                amount,
                by_amount_in,
                fee,
            )
            .unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: Price::new(1_000000000000_000000000003),
                amount_in: TokenAmount(2),
                amount_out: TokenAmount(1),
                fee_amount: TokenAmount(1),
            };
            assert_eq!(result, expected_result)
        }
        // liquidity is zero and by amount_in should skip to target price
        {
            let current_price_sqrt = Price::from_integer(1);
            let target_price_sqrt = Price::from_scale(100005, 5); // 1.00005
            let liquidity = Liquidity::new(0);
            let amount = TokenAmount(100000);
            let by_amount_in = true;
            let fee = FixedPoint::from_scale(6, 4); // 0.0006 -> 0.06%

            let result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                amount,
                by_amount_in,
                fee,
            )
            .unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: target_price_sqrt,
                amount_in: TokenAmount(0),
                amount_out: TokenAmount(0),
                fee_amount: TokenAmount(0),
            };
            assert_eq!(result, expected_result)
        }
        // liquidity is zero and by amount_out should skip to target price
        {
            let current_price_sqrt = Price::from_integer(1);
            let target_price_sqrt = Price::from_scale(100005, 5); // 1.00005
            let liquidity = Liquidity::new(0);
            let amount = TokenAmount(100000);
            let by_amount_in = false;
            let fee = FixedPoint::from_scale(6, 4); // 0.0006 -> 0.06%

            let result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                amount,
                by_amount_in,
                fee,
            )
            .unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: target_price_sqrt,
                amount_in: TokenAmount(0),
                amount_out: TokenAmount(0),
                fee_amount: TokenAmount(0),
            };
            assert_eq!(result, expected_result)
        }
        // normal swap step but fee is set to 0
        {
            let current_price_sqrt = Price::from_scale(99995, 5); // 0.99995
            let target_price_sqrt = Price::from_integer(1);
            let liquidity = Liquidity::from_integer(50000000);
            let amount = TokenAmount(1000);
            let by_amount_in = true;
            let fee = FixedPoint::new(0);

            let result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                amount,
                by_amount_in,
                fee,
            )
            .unwrap();
            let expected_result = SwapResult {
                next_price_sqrt: Price::from_scale(99997, 5),
                amount_in: TokenAmount(1000),
                amount_out: TokenAmount(1000),
                fee_amount: TokenAmount(0),
            };
            assert_eq!(result, expected_result)
        }
        // by_amount_out and x_to_y edge cases
        {
            let target_price_sqrt = calculate_price_sqrt(-10);
            let current_price_sqrt = target_price_sqrt + Price::from_integer(1);
            let liquidity = Liquidity::from_integer(340282366920938463463374607u128);
            let one_token = TokenAmount(1);
            let tokens_with_same_output = TokenAmount(85);
            let zero_token = TokenAmount(0);
            let by_amount_in = false;
            let max_fee = FixedPoint::from_scale(9, 1);
            let min_fee = FixedPoint::from_integer(0);

            let one_token_result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                one_token,
                by_amount_in,
                max_fee,
            )
            .unwrap();
            let tokens_with_same_output_result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                tokens_with_same_output,
                by_amount_in,
                max_fee,
            )
            .unwrap();
            let zero_token_result = compute_swap_step(
                current_price_sqrt,
                target_price_sqrt,
                liquidity,
                zero_token,
                by_amount_in,
                min_fee,
            )
            .unwrap();
            /*
                86x -> [1, 85]y
                rounding due to price accuracy
                it does not matter if you want 1 or 85 y tokens, will take you the same input amount
            */
            let expected_one_token_result = SwapResult {
                next_price_sqrt: current_price_sqrt - Price::new(1),
                amount_in: TokenAmount(86),
                amount_out: TokenAmount(1),
                fee_amount: TokenAmount(78),
            };
            let expected_tokens_with_same_output_result = SwapResult {
                next_price_sqrt: current_price_sqrt - Price::new(1),
                amount_in: TokenAmount(86),
                amount_out: TokenAmount(85),
                fee_amount: TokenAmount(78),
            };
            let expected_zero_token_result = SwapResult {
                next_price_sqrt: current_price_sqrt,
                amount_in: TokenAmount(0),
                amount_out: TokenAmount(0),
                fee_amount: TokenAmount(0),
            };
            assert_eq!(one_token_result, expected_one_token_result);
            assert_eq!(
                tokens_with_same_output_result,
                expected_tokens_with_same_output_result
            );
            assert_eq!(zero_token_result, expected_zero_token_result);
        }

        // VALIDATE DOMAIN
        let one_price_sqrt = Price::from_integer(1);
        let two_price_sqrt = Price::from_integer(2);
        let max_price_sqrt = calculate_price_sqrt(MAX_TICK);
        let min_price_sqrt = calculate_price_sqrt(-MAX_TICK);
        let one_liquidity = Liquidity::from_integer(1);
        let max_liquidity = Liquidity::max_instance();
        let max_amount = TokenAmount::max_instance();
        let max_amount_not_reached_target_price = TokenAmount(TokenAmount::max_value() - 1);
        let max_fee = FixedPoint::from_integer(1);
        let min_fee = FixedPoint::new(0);

        // 100% fee | max_amount
        {
            let result = compute_swap_step(
                one_price_sqrt,
                two_price_sqrt,
                one_liquidity,
                max_amount,
                true,
                max_fee,
            )
            .unwrap();
            assert_eq!(
                result,
                SwapResult {
                    next_price_sqrt: Price::from_integer(1),
                    amount_in: TokenAmount(0),
                    amount_out: TokenAmount(0),
                    fee_amount: max_amount,
                }
            )
        }
        // 0% fee | max_amount | max_liquidity | price slice
        {
            let (_, cause, stack) = compute_swap_step(
                one_price_sqrt,
                two_price_sqrt,
                max_liquidity,
                max_amount,
                true,
                min_fee,
            )
            .unwrap_err()
            .get();

            assert_eq!(cause, "get_delta_x overflow");
            assert_eq!(stack.len(), 1);
        }
        // by_amount_in == true || close to target_price but not reached
        {
            let big_liquidity = Liquidity::from_integer(100_000_000_000_000u128);
            let amount_pushing_price_to_target = TokenAmount(100000000000000);

            let result = compute_swap_step(
                one_price_sqrt,
                two_price_sqrt,
                big_liquidity,
                amount_pushing_price_to_target - TokenAmount(1),
                true,
                min_fee,
            )
            .unwrap();
            assert_eq!(
                result,
                SwapResult {
                    next_price_sqrt: Price::new(1999999999999990000000000),
                    amount_in: TokenAmount(99999999999999),
                    amount_out: TokenAmount(49999999999999),
                    fee_amount: TokenAmount(0)
                }
            )
        }
        // maximize fee_amount || close to target_price but not reached
        {
            let non_fee_input = TokenAmount(340282367);
            let result = compute_swap_step(
                one_price_sqrt,
                two_price_sqrt,
                max_liquidity,
                TokenAmount::max_instance(),
                true,
                max_fee - FixedPoint::new(19),
            )
            .unwrap();
            assert_eq!(
                result,
                SwapResult {
                    next_price_sqrt: one_price_sqrt + Price::new(1),
                    amount_in: non_fee_input,
                    amount_out: non_fee_input - TokenAmount(1),
                    fee_amount: TokenAmount::max_instance() - non_fee_input,
                }
            )
        }
        // get_next_sqrt_price_from_input -> get_next_sqrt_price_x_up
        {
            // by_amount_in == true
            // x_to_y == true => current_price_sqrt >= target_price_sqrt == true

            // validate both: trace and panic possibilities
            let (_, cause, stack) = compute_swap_step(
                max_price_sqrt,
                min_price_sqrt,
                max_liquidity,
                max_amount_not_reached_target_price,
                true,
                min_fee,
            )
            .unwrap_err()
            .get();

            assert_eq!(cause, "multiplication overflow");
            assert_eq!(stack.len(), 4);
        }
        // get_next_sqrt_price_from_input -> get_next_sqrt_price_y_down
        {
            // by_amount_in == true
            // x_to_y == false => current_price_sqrt >= target_price_sqrt == false

            // 1. scale - maximize amount_after_fee => (max_amount, min_fee) && not reached target
            {
                let (_, cause, stack) = compute_swap_step(
                    min_price_sqrt,
                    max_price_sqrt,
                    max_liquidity,
                    max_amount_not_reached_target_price,
                    true,
                    min_fee,
                )
                .unwrap_err()
                .get();

                assert_eq!(cause, "checked_from_scale: (multiplier * base) overflow");
                assert_eq!(stack.len(), 3);
            }
            // 2. checked_big_div - no possible to trigger from compute_swap_step
            {
                let min_overflow_token_amount = TokenAmount::new(340282366920939);
                let result = compute_swap_step(
                    min_price_sqrt,
                    max_price_sqrt,
                    one_liquidity - Liquidity::new(1),
                    min_overflow_token_amount - TokenAmount(1),
                    true,
                    min_fee,
                )
                .unwrap();
                assert_eq!(
                    result,
                    SwapResult {
                        next_price_sqrt: max_price_sqrt,
                        amount_in: TokenAmount(65536),
                        amount_out: TokenAmount(65535),
                        fee_amount: TokenAmount(0),
                    }
                )
            }
        }
        // get_next_sqrt_price_from_output -> get_next_sqrt_price_x_up
        {
            // by_amount_in == false
            // x_to_y == false => current_price_sqrt >= target_price_sqrt == false
            // TRY TO UNWRAP IN SUBTRACTION

            // min price different at maximum amount
            {
                let min_diff = 232_826_265_438_719_159_684u128;
                let (_, cause, stack) = compute_swap_step(
                    max_price_sqrt - Price::new(min_diff),
                    max_price_sqrt,
                    max_liquidity,
                    TokenAmount(TokenAmount::max_value() - 1),
                    false,
                    min_fee,
                )
                .unwrap_err()
                .get();
                assert_eq!(cause, "multiplication overflow");
                assert_eq!(stack.len(), 4);
            }
            // min price different at maximum amount
            {
                let result = compute_swap_step(
                    min_price_sqrt,
                    max_price_sqrt,
                    Liquidity::from_integer(281_477_613_507_675u128),
                    TokenAmount(TokenAmount::max_value() - 1),
                    false,
                    min_fee,
                )
                .unwrap();

                assert_eq!(
                    result,
                    SwapResult {
                        next_price_sqrt: Price::new(65535263695369929348256523309),
                        amount_in: TokenAmount(18446709621273854098),
                        amount_out: TokenAmount(18446744073709551613),
                        fee_amount: TokenAmount(0)
                    }
                );
            }
            // min token change
            {
                let result = compute_swap_step(
                    max_price_sqrt - Price::from_integer(1),
                    max_price_sqrt,
                    Liquidity::from_integer(100_000_000_00u128),
                    TokenAmount(1),
                    false,
                    min_fee,
                )
                .unwrap();

                assert_eq!(
                    result,
                    SwapResult {
                        next_price_sqrt: Price::new(65534813412874974599766965330u128),
                        amount_in: TokenAmount(4294783624),
                        amount_out: TokenAmount(1),
                        fee_amount: TokenAmount(0),
                    }
                );
            }
            //Fee above 1 && by_amount_in == true
            {
                let (_, cause, _) = compute_swap_step(
                    max_price_sqrt - Price::from_integer(1),
                    max_price_sqrt,
                    Liquidity::from_integer(100_000_000_00u128),
                    TokenAmount(1),
                    true,
                    FixedPoint::from_integer(1) + FixedPoint::new(1),
                )
                .unwrap_err()
                .get();

                assert_eq!(cause, "sub underflow");
            }
            //max fee that fits within u64 && by_amount_in == false
            {
                let result = compute_swap_step(
                    max_price_sqrt - Price::from_integer(1),
                    max_price_sqrt,
                    Liquidity::from_integer(100_000_000_00u128),
                    TokenAmount::new(u64::MAX),
                    false,
                    FixedPoint::from_integer(i32::MAX / 2),
                )
                .unwrap();

                assert_eq!(
                    result,
                    SwapResult {
                        next_price_sqrt: Price::new(65535383934512647000000000000),
                        amount_in: TokenAmount(10000000000),
                        amount_out: TokenAmount(2),
                        fee_amount: TokenAmount(10737418230000000000)
                    }
                );
            }
        }
    }

    #[test]
    fn test_get_next_sqrt_price_y_down() {
        // VALIDATE BASE SAMPLES
        {
            let price_sqrt = Price::from_integer(1);
            let liquidity = Liquidity::from_integer(1);
            let amount = TokenAmount(1);

            let result = get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, true).unwrap();

            assert_eq!(result, Price::from_integer(2));
        }
        {
            let price_sqrt = Price::from_integer(1);
            let liquidity = Liquidity::from_integer(2);
            let amount = TokenAmount(3);

            let result = get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, true).unwrap();

            assert_eq!(result, Price::from_scale(25, 1));
        }
        {
            let price_sqrt = Price::from_integer(2);
            let liquidity = Liquidity::from_integer(3);
            let amount = TokenAmount(5);

            let result = get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, true).unwrap();

            assert_eq!(
                result,
                Price::from_integer(11).big_div(Price::from_integer(3))
            );
        }
        {
            let price_sqrt = Price::from_integer(24234);
            let liquidity = Liquidity::from_integer(3000);
            let amount = TokenAmount(5000);

            let result = get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, true).unwrap();

            assert_eq!(
                result,
                Price::from_integer(72707).big_div(Price::from_integer(3))
            );
        }
        // bool = false
        {
            let price_sqrt = Price::from_integer(1);
            let liquidity = Liquidity::from_integer(2);
            let amount = TokenAmount(1);

            let result = get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, false).unwrap();

            assert_eq!(result, Price::from_scale(5, 1));
        }
        {
            let price_sqrt = Price::from_integer(100_000);
            let liquidity = Liquidity::from_integer(500_000_000);
            let amount = TokenAmount(4_000);

            let result = get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, false).unwrap();
            assert_eq!(result, Price::new(99999999992000000_000000000000));
        }
        {
            let price_sqrt = Price::from_integer(3);
            let liquidity = Liquidity::from_integer(222);
            let amount = TokenAmount(37);

            let result = get_next_sqrt_price_y_down(price_sqrt, liquidity, amount, false).unwrap();

            // expected 2.833333333333
            // real     2.999999999999833...
            assert_eq!(result, Price::new(2833333333333_333333333333));
        }

        // VALIDATE DOMAIN
        let max_amount = TokenAmount::max_instance();
        let min_price = Price::new(1);
        let sample_liquidity = Liquidity::new(1);
        let min_overflow_token_amount = TokenAmount::new(340282366920939);
        let max_price = calculate_price_sqrt(MAX_TICK);
        let one_liquidity: Liquidity = Liquidity::from_integer(1);
        let max_liquidity = Liquidity::max_instance();
        // max_liquidity
        {
            let result = get_next_sqrt_price_y_down(
                max_price,
                max_liquidity,
                min_overflow_token_amount - TokenAmount(1),
                false,
            )
            .unwrap();
            assert_eq!(result, Price::new(65535383934512646999999000000));
        }
        // extension TokenAmount to Price decimal overflow
        {
            {
                let result =
                    get_next_sqrt_price_y_down(min_price, sample_liquidity, max_amount, true)
                        .unwrap_err();
                let (_, cause, stack) = result.get();
                assert_eq!(cause, "checked_from_scale: (multiplier * base) overflow");
                assert_eq!(stack.len(), 1);
            }
            {
                let result =
                    get_next_sqrt_price_y_down(min_price, sample_liquidity, max_amount, false)
                        .unwrap_err();
                let (_, cause, stack) = result.get();
                assert_eq!(cause, "checked_from_scale: (multiplier * base) overflow");
                assert_eq!(stack.len(), 1);
            }
        }
        // quotient overflow
        {
            {
                {
                    let result = get_next_sqrt_price_y_down(
                        min_price,
                        one_liquidity - Liquidity::new(1),
                        min_overflow_token_amount - TokenAmount(1),
                        true,
                    )
                    .unwrap_err();
                    let (_, cause, stack) = result.get();
                    assert_eq!(cause, "checked_big_div_by_number: can't convert to result");
                    assert_eq!(stack.len(), 1);
                }
                {
                    let result = get_next_sqrt_price_y_down(
                        min_price,
                        one_liquidity - Liquidity::new(1),
                        min_overflow_token_amount - TokenAmount(1),
                        false,
                    )
                    .unwrap_err();
                    let (_, cause, stack) = result.get();
                    assert_eq!(
                        cause,
                        "checked_big_div_by_number_up: can't convert to result"
                    );
                    assert_eq!(stack.len(), 1);
                }
            }
            {
                let result = get_next_sqrt_price_y_down(
                    min_price,
                    one_liquidity,
                    min_overflow_token_amount - TokenAmount(1),
                    true,
                )
                .unwrap();
                assert_eq!(result, Price::new(340282366920938000000000000000000000001));
            }
        }
        // overflow in price difference
        {
            {
                let result = get_next_sqrt_price_y_down(
                    max_price,
                    one_liquidity,
                    min_overflow_token_amount - TokenAmount(1),
                    true,
                )
                .unwrap_err();
                let (_, cause, stack) = result.get();
                assert_eq!(cause, "checked_add: (self + rhs) additional overflow");
                assert_eq!(stack.len(), 1);
            }
            {
                let result = get_next_sqrt_price_y_down(
                    min_price,
                    one_liquidity,
                    min_overflow_token_amount - TokenAmount(1),
                    false,
                )
                .unwrap_err();
                let (_, cause, stack) = result.get();
                assert_eq!(cause, "checked_sub: (self - rhs) subtraction underflow");
                assert_eq!(stack.len(), 1);
            }
        }
    }

    #[test]
    fn test_get_delta_x() {
        // validate base samples
        // zero at zero liquidity
        {
            let result = get_delta_x(
                Price::from_integer(1u8),
                Price::from_integer(1u8),
                Liquidity::new(0),
                false,
            )
            .unwrap();
            assert_eq!(result, TokenAmount(0));
        }
        // equal at equal liquidity
        {
            let result = get_delta_x(
                Price::from_integer(1u8),
                Price::from_integer(2u8),
                Liquidity::from_integer(2u8),
                false,
            )
            .unwrap();
            assert_eq!(result, TokenAmount(1));
        }
        // complex
        {
            let sqrt_price_a = Price::new(234__878_324_943_782_000000000000);
            let sqrt_price_b = Price::new(87__854_456_421_658_000000000000);
            let liquidity = Liquidity::new(983_983__249_092);

            let result_down = get_delta_x(sqrt_price_a, sqrt_price_b, liquidity, false).unwrap();
            let result_up = get_delta_x(sqrt_price_a, sqrt_price_b, liquidity, true).unwrap();

            // 7010.8199533068819376891841727789301497024557314488455622925765280
            assert_eq!(result_down, TokenAmount(7010));
            assert_eq!(result_up, TokenAmount(7011));
        }
        // big
        {
            let sqrt_price_a = Price::from_integer(1u8);
            let sqrt_price_b = Price::from_scale(5u8, 1);
            let liquidity = Liquidity::from_integer(2u128.pow(64) - 1);

            let result_down = get_delta_x(sqrt_price_a, sqrt_price_b, liquidity, false).unwrap();
            let result_up = get_delta_x(sqrt_price_a, sqrt_price_b, liquidity, true).unwrap();

            assert_eq!(result_down, TokenAmount::from_decimal(liquidity));
            assert_eq!(result_up, TokenAmount::from_decimal(liquidity));
        }
        // overflow
        {
            let sqrt_price_a = Price::from_integer(1u8);
            let sqrt_price_b = Price::from_scale(5u8, 1);
            let liquidity = Liquidity::from_integer(2u128.pow(64));

            let result_down = get_delta_x(sqrt_price_a, sqrt_price_b, liquidity, false);
            let result_up = get_delta_x(sqrt_price_a, sqrt_price_b, liquidity, true);

            assert!(result_down.is_none());
            assert!(result_up.is_none());
        }
        // huge liquidity
        {
            let sqrt_price_a = Price::from_integer(1u8);
            let sqrt_price_b = Price::new(Price::one()) + Price::new(1000000);
            let liquidity = Liquidity::from_integer(2u128.pow(80));

            let result_down = get_delta_x(sqrt_price_a, sqrt_price_b, liquidity, false);
            let result_up = get_delta_x(sqrt_price_a, sqrt_price_b, liquidity, true);

            assert!(result_down.is_some());
            assert!(result_up.is_some());
        }

        let max_sqrt_price = calculate_price_sqrt(MAX_TICK);
        let min_sqrt_price = calculate_price_sqrt(-MAX_TICK);
        let almost_max_sqrt_price = calculate_price_sqrt(MAX_TICK - 1);
        let almost_min_sqrt_price = calculate_price_sqrt(-MAX_TICK + 1);

        // DOMAIN:
        let max_liquidity = Liquidity::new(u128::MAX);
        let min_liquidity = Liquidity::new(1);

        // maximize numerator for overflow of TokenAmount -> maximize delta_price and liquidity
        {
            {
                let result = get_delta_x(max_sqrt_price, min_sqrt_price, max_liquidity, true);
                assert_eq!(None, result);
            }
            {
                let result = get_delta_x(max_sqrt_price, min_sqrt_price, max_liquidity, false);
                assert_eq!(None, result);
            }
        }
        // maximize denominator for overflow of TokenAmount -> maximize prices product
        {
            {
                let result: Option<TokenAmount> =
                    get_delta_x(max_sqrt_price, almost_max_sqrt_price, max_liquidity, true);
                assert_eq!(None, result);
            }
            {
                let result =
                    get_delta_x(max_sqrt_price, almost_max_sqrt_price, max_liquidity, false);
                assert_eq!(None, result);
            }
        }
        // maximize denominator without overflow of TokenAmount -> maximize prices product
        {
            {
                let result: Option<TokenAmount> =
                    get_delta_x(max_sqrt_price, almost_max_sqrt_price, min_liquidity, true);
                assert_eq!(Some(TokenAmount(1)), result);
            }
            {
                let result =
                    get_delta_x(max_sqrt_price, almost_max_sqrt_price, min_liquidity, false);
                assert_eq!(Some(TokenAmount(0)), result);
            }
        }
        // minimize denominator on maximize liquidity for overflow of TokenAmount
        {
            {
                let result: Option<TokenAmount> =
                    get_delta_x(min_sqrt_price, almost_min_sqrt_price, max_liquidity, true);
                assert_eq!(None, result);
            }
            {
                let result =
                    get_delta_x(min_sqrt_price, almost_min_sqrt_price, max_liquidity, false);
                assert_eq!(None, result);
            }
        }
        // minimize denominator on minimize liquidity which fits into TokenAmount
        {
            {
                let result: Option<TokenAmount> =
                    get_delta_x(min_sqrt_price, almost_min_sqrt_price, min_liquidity, true);
                assert_eq!(Some(TokenAmount(1)), result);
            }
            {
                let result =
                    get_delta_x(min_sqrt_price, almost_min_sqrt_price, min_liquidity, false);
                assert_eq!(Some(TokenAmount(0)), result);
            }
        }
        // maximize denominator with maximum liquidity which fit into TokenAmount
        {
            let liquidity = Liquidity::new(max_liquidity.v >> 46);
            {
                let result: Option<TokenAmount> =
                    get_delta_x(min_sqrt_price, almost_min_sqrt_price, liquidity, true);
                assert_eq!(Some(TokenAmount(15845800777794838947)), result);
            }
            {
                let result = get_delta_x(min_sqrt_price, almost_min_sqrt_price, liquidity, false);
                assert_eq!(Some(TokenAmount(15845800777794838946)), result);
            }
        }
    }

    #[test]
    fn test_get_delta_y() {
        // base samples
        // zero at zero liquidity
        {
            let result = get_delta_y(
                Price::from_integer(1),
                Price::from_integer(1),
                Liquidity::new(0),
                false,
            )
            .unwrap();
            assert_eq!(result, TokenAmount(0));
        }
        // equal at equal liquidity
        {
            let result = get_delta_y(
                Price::from_integer(1),
                Price::from_integer(2),
                Liquidity::from_integer(2),
                false,
            )
            .unwrap();
            assert_eq!(result, TokenAmount(2));
        }
        // big numbers
        {
            let sqrt_price_a = Price::new(234__878_324_943_782_000000000000);
            let sqrt_price_b = Price::new(87__854_456_421_658_000000000000);
            let liquidity = Liquidity::new(983_983__249_092);

            let result_down = get_delta_y(sqrt_price_a, sqrt_price_b, liquidity, false).unwrap();
            let result_up = get_delta_y(sqrt_price_a, sqrt_price_b, liquidity, true).unwrap();

            // 144669023.842474597804911408
            assert_eq!(result_down, TokenAmount(144669023));
            assert_eq!(result_up, TokenAmount(144669024));
        }
        // big
        {
            let sqrt_price_a = Price::from_integer(1u8);
            let sqrt_price_b = Price::from_integer(2u8);
            let liquidity = Liquidity::from_integer(2u128.pow(64) - 1);

            let result_down = get_delta_y(sqrt_price_a, sqrt_price_b, liquidity, false).unwrap();
            let result_up = get_delta_y(sqrt_price_a, sqrt_price_b, liquidity, true).unwrap();

            assert_eq!(result_down, TokenAmount::from_decimal(liquidity));
            assert_eq!(result_up, TokenAmount::from_decimal(liquidity));
        }
        // overflow
        {
            let sqrt_price_a = Price::from_integer(1u8);
            let sqrt_price_b = Price::from_integer(2u8);
            let liquidity = Liquidity::from_integer(2u128.pow(64));

            let result_down = get_delta_y(sqrt_price_a, sqrt_price_b, liquidity, false);
            let result_up = get_delta_y(sqrt_price_a, sqrt_price_b, liquidity, true);

            assert!(result_down.is_none());
            assert!(result_up.is_none());
        }
        // huge liquidity
        {
            let sqrt_price_a = Price::from_integer(1u8);
            let sqrt_price_b = Price::new(Price::one()) + Price::new(1000000);
            let liquidity = Liquidity::from_integer(2u128.pow(80));

            let result_down = get_delta_y(sqrt_price_a, sqrt_price_b, liquidity, false);
            let result_up = get_delta_y(sqrt_price_a, sqrt_price_b, liquidity, true);

            assert!(result_down.is_some());
            assert!(result_up.is_some());
        }

        // DOMAIN
        let max_sqrt_price = calculate_price_sqrt(MAX_TICK);
        let min_sqrt_price = calculate_price_sqrt(-MAX_TICK);
        let max_liquidity = Liquidity::new(u128::MAX);
        // maximize delta_price and liquidity
        {
            {
                let result = get_delta_y(max_sqrt_price, min_sqrt_price, max_liquidity, true);
                assert!(result.is_none());
            }
            {
                let result = get_delta_y(max_sqrt_price, min_sqrt_price, max_liquidity, false);
                assert!(result.is_none());
            }
        }
    }

    #[test]
    fn test_get_next_sqrt_price_x_up() {
        // basic samples
        // Add
        {
            let price_sqrt = Price::from_integer(1);
            let liquidity = Liquidity::from_integer(1);
            let amount = TokenAmount(1);

            let result = get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, true);

            assert_eq!(result.unwrap(), Price::from_scale(5, 1));
        }
        {
            let price_sqrt = Price::from_integer(1);
            let liquidity = Liquidity::from_integer(2);
            let amount = TokenAmount(3);

            let result = get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, true);

            assert_eq!(result.unwrap(), Price::from_scale(4, 1));
        }
        {
            let price_sqrt = Price::from_integer(2);
            let liquidity = Liquidity::from_integer(3);
            let amount = TokenAmount(5);

            let result = get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, true);

            assert_eq!(
                result.unwrap(),
                Price::new(461538461538461538461539) // rounded up Decimal::from_integer(6).div(Decimal::from_integer(13))
            );
        }
        {
            let price_sqrt = Price::from_integer(24234);
            let liquidity = Liquidity::from_integer(3000);
            let amount = TokenAmount(5000);

            let result = get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, true);

            assert_eq!(
                result.unwrap(),
                Price::new(599985145205615112277488) // rounded up Decimal::from_integer(24234).div(Decimal::from_integer(40391))
            );
        }
        // Subtract
        {
            let price_sqrt = Price::from_integer(1);
            let liquidity = Liquidity::from_integer(2);
            let amount = TokenAmount(1);

            let result = get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, false);

            assert_eq!(result.unwrap(), Price::from_integer(2));
        }
        {
            let price_sqrt = Price::from_integer(100_000);
            let liquidity = Liquidity::from_integer(500_000_000);
            let amount = TokenAmount(4_000);

            let result = get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, false);

            assert_eq!(result.unwrap(), Price::from_integer(500_000));
        }
        {
            let price_sqrt = Price::new(3_333333333333333333333333);
            let liquidity = Liquidity::new(222_222222);
            let amount = TokenAmount(37);

            // expected 7.490636713462104974072145
            // real     7.4906367134621049740721443...
            let result = get_next_sqrt_price_x_up(price_sqrt, liquidity, amount, false);

            assert_eq!(result.unwrap(), Price::new(7490636713462104974072145));
        }

        // DOMAIN:
        let max_liquidity = Liquidity::new(u128::MAX);
        let min_liquidity = Liquidity::new(1);
        let max_price_sqrt = calculate_price_sqrt(MAX_TICK);
        let max_amount = TokenAmount(u64::MAX);
        {
            let result = get_next_sqrt_price_x_up(max_price_sqrt, max_liquidity, max_amount, true)
                .unwrap_err();

            let (_, cause, stack) = result.get();
            assert_eq!(stack.len(), 2);
            assert_eq!(cause, TrackableError::MUL);
        }
        // subtraction underflow (not possible from upper-level function)
        {
            let (_, cause, stack) = get_next_sqrt_price_x_up(
                max_price_sqrt,
                min_liquidity,
                TokenAmount(u64::MAX),
                false,
            )
            .unwrap_err()
            .get();

            assert_eq!(cause, "big_liquidity -/+ price_sqrt * amount");
            assert_eq!(stack.len(), 1);
        }
        // max_liquidity
        {
            let result = get_next_sqrt_price_y_down(
                Price::from_integer(1),
                max_liquidity,
                TokenAmount(10000),
                false,
            )
            .unwrap();
            assert_eq!(result, Price::new(999999999999999999999999));
        }
    }

    #[test]
    fn test_get_next_sqrt_price_from_input() {
        {
            let (_, cause, _) = get_next_sqrt_price_from_input(
                Price::from_integer(1),
                Liquidity::from_integer(0),
                TokenAmount::from_integer(1),
                false,
            )
            .unwrap_err()
            .get();

            assert_eq!(cause, "getting next price from input with zero liquidity")
        }
        {
            let (_, cause, _) = get_next_sqrt_price_from_input(
                Price::from_integer(0),
                Liquidity::from_integer(1),
                TokenAmount::from_integer(1),
                false,
            )
            .unwrap_err()
            .get();

            assert_eq!(cause, "getting next price from input with zero price")
        }
    }

    #[test]
    fn test_get_next_sqrt_price_from_output() {
        {
            let (_, cause, _) = get_next_sqrt_price_from_output(
                Price::from_integer(1),
                Liquidity::from_integer(0),
                TokenAmount::from_integer(1),
                false,
            )
            .unwrap_err()
            .get();

            assert_eq!(cause, "getting next price from output with zero liquidity")
        }
        {
            let (_, cause, _) = get_next_sqrt_price_from_output(
                Price::from_integer(0),
                Liquidity::from_integer(1),
                TokenAmount::from_integer(1),
                false,
            )
            .unwrap_err()
            .get();

            assert_eq!(cause, "getting next price from output with zero price")
        }
    }
    #[test]
    fn test_is_enough_amount_to_push_price() {
        // Validate traceable error
        let min_liquidity = Liquidity::new(1);
        let max_price_sqrt = calculate_price_sqrt(MAX_TICK);
        let min_fee = FixedPoint::from_integer(0);
        {
            let (_, cause, stack) = is_enough_amount_to_push_price(
                TokenAmount(u64::MAX),
                max_price_sqrt,
                min_liquidity,
                min_fee,
                false,
                false,
            )
            .unwrap_err()
            .get();

            assert_eq!(cause, "big_liquidity -/+ price_sqrt * amount");
            assert_eq!(stack.len(), 3);
        }
        let fee_over_one = FixedPoint::from_integer(1) + FixedPoint::new(1);

        let (_, cause, _) = is_enough_amount_to_push_price(
            TokenAmount::new(1000),
            calculate_price_sqrt(10),
            Liquidity::new(1),
            fee_over_one,
            true,
            false,
        )
        .unwrap_err()
        .get();

        assert_eq!(cause, "sub underflow")
    }

    #[test]
    fn test_price_limitation() {
        {
            let global_max_price = calculate_price_sqrt(MAX_TICK);
            assert_eq!(global_max_price, Price::new(MAX_SQRT_PRICE)); // ceil(log2(this)) = 96
            let global_min_price = calculate_price_sqrt(-MAX_TICK);
            assert_eq!(global_min_price, Price::new(MIN_SQRT_PRICE)); // ceil(log2(this)) = 64
        }
        {
            let max_price = get_max_sqrt_price(1).unwrap();
            let max_tick: i32 = get_max_tick(1).unwrap();
            assert_eq!(max_price, Price::new(9189293893553000000000000));
            assert_eq!(
                calculate_price_sqrt(max_tick),
                Price::new(9189293893553000000000000)
            );

            let max_price = get_max_sqrt_price(2).unwrap();
            let max_tick: i32 = get_max_tick(2).unwrap();
            assert_eq!(max_price, Price::new(84443122262186000000000000));
            assert_eq!(
                calculate_price_sqrt(max_tick),
                Price::new(84443122262186000000000000)
            );

            let max_price = get_max_sqrt_price(5).unwrap();
            let max_tick: i32 = get_max_tick(5).unwrap();
            assert_eq!(max_price, Price::new(65525554855399275000000000000));
            assert_eq!(
                calculate_price_sqrt(max_tick),
                Price::new(65525554855399275000000000000)
            );

            let max_price = get_max_sqrt_price(10).unwrap();
            let max_tick: i32 = get_max_tick(10).unwrap();
            assert_eq!(max_price, Price::new(65535383934512647000000000000));
            assert_eq!(
                calculate_price_sqrt(max_tick),
                Price::new(65535383934512647000000000000)
            );

            let max_price = get_max_sqrt_price(100).unwrap();
            let max_tick: i32 = get_max_tick(100).unwrap();
            assert_eq!(max_price, Price::new(65535383934512647000000000000));
            assert_eq!(
                calculate_price_sqrt(max_tick),
                Price::new(65535383934512647000000000000)
            );
        }
        {
            let min_price = get_min_sqrt_price(1).unwrap();
            let min_tick: i32 = get_min_tick(1).unwrap();
            assert_eq!(min_price, Price::new(108822289458000000000000));
            assert_eq!(
                calculate_price_sqrt(min_tick),
                Price::new(108822289458000000000000)
            );

            let min_price = get_min_sqrt_price(2).unwrap();
            let min_tick: i32 = get_min_tick(2).unwrap();
            assert_eq!(min_price, Price::new(11842290682000000000000));
            assert_eq!(
                calculate_price_sqrt(min_tick),
                Price::new(11842290682000000000000)
            );

            let min_price = get_min_sqrt_price(5).unwrap();
            let min_tick: i32 = get_min_tick(5).unwrap();
            assert_eq!(min_price, Price::new(15261221000000000000));
            assert_eq!(
                calculate_price_sqrt(min_tick),
                Price::new(15261221000000000000)
            );

            let min_price = get_min_sqrt_price(10).unwrap();
            let min_tick: i32 = get_min_tick(10).unwrap();
            assert_eq!(min_price, Price::new(15258932000000000000));
            assert_eq!(
                calculate_price_sqrt(min_tick),
                Price::new(15258932000000000000)
            );

            let min_price = get_min_sqrt_price(100).unwrap();
            let min_tick: i32 = get_min_tick(100).unwrap();
            assert_eq!(min_price, Price::new(15258932000000000000));
            assert_eq!(
                calculate_price_sqrt(min_tick),
                Price::new(15258932000000000000)
            );

            get_min_tick(u16::MAX).unwrap_err();
            get_max_tick(u16::MAX).unwrap_err();
            get_max_sqrt_price(u16::MAX).unwrap_err();
            get_min_sqrt_price(u16::MAX).unwrap_err();
        }
    }

    #[test]
    fn test_cross_tick() {
        // add liquidity to pool
        {
            let mut pool = Pool {
                fee_growth_global_x: FeeGrowth::new(45),
                fee_growth_global_y: FeeGrowth::new(35),
                liquidity: Liquidity::from_integer(4),
                current_tick_index: 7,
                ..Default::default()
            };
            let tick = Tick {
                fee_growth_outside_x: FeeGrowth::new(30),
                fee_growth_outside_y: FeeGrowth::new(25),
                index: 3,
                liquidity_change: Liquidity::from_integer(1),
                ..Default::default()
            };
            let result_pool = Pool {
                fee_growth_global_x: FeeGrowth::new(45),
                fee_growth_global_y: FeeGrowth::new(35),
                liquidity: Liquidity::from_integer(5),
                current_tick_index: 7,
                ..Default::default()
            };
            let result_tick = Tick {
                fee_growth_outside_x: FeeGrowth::new(15),
                fee_growth_outside_y: FeeGrowth::new(10),
                index: 3,
                liquidity_change: Liquidity::from_integer(1),
                ..Default::default()
            };

            let ref_tick = RefCell::new(tick);
            let mut refmut_tick = ref_tick.borrow_mut();

            cross_tick(&mut refmut_tick, &mut pool).unwrap();

            assert_eq!(*refmut_tick, result_tick);
            assert_eq!(pool, result_pool);
        }
        {
            let mut pool = Pool {
                fee_growth_global_x: FeeGrowth::new(68),
                fee_growth_global_y: FeeGrowth::new(59),
                liquidity: Liquidity::new(0),
                current_tick_index: 4,
                ..Default::default()
            };
            let tick = Tick {
                fee_growth_outside_x: FeeGrowth::new(42),
                fee_growth_outside_y: FeeGrowth::new(14),
                index: 9,
                liquidity_change: Liquidity::new(0),
                ..Default::default()
            };
            let result_pool = Pool {
                fee_growth_global_x: FeeGrowth::new(68),
                fee_growth_global_y: FeeGrowth::new(59),
                liquidity: Liquidity::new(0),
                current_tick_index: 4,
                ..Default::default()
            };
            let result_tick = Tick {
                fee_growth_outside_x: FeeGrowth::new(26),
                fee_growth_outside_y: FeeGrowth::new(45),
                index: 9,
                liquidity_change: Liquidity::from_integer(0),
                ..Default::default()
            };

            let ref_tick = RefCell::new(tick);
            let mut refmut_tick = ref_tick.borrow_mut();
            cross_tick(&mut refmut_tick, &mut pool).unwrap();
            assert_eq!(*refmut_tick, result_tick);
            assert_eq!(pool, result_pool);
        }
        // fee_growth_outside should underflow
        {
            let mut pool = Pool {
                fee_growth_global_x: FeeGrowth::new(3402),
                fee_growth_global_y: FeeGrowth::new(3401),
                liquidity: Liquidity::new(14),
                current_tick_index: 9,
                ..Default::default()
            };
            let tick = Tick {
                fee_growth_outside_x: FeeGrowth::new(26584),
                fee_growth_outside_y: FeeGrowth::new(1256588),
                index: 45,
                liquidity_change: Liquidity::new(10),
                ..Default::default()
            };
            let result_pool = Pool {
                fee_growth_global_x: FeeGrowth::new(3402),
                fee_growth_global_y: FeeGrowth::new(3401),
                liquidity: Liquidity::new(4),
                current_tick_index: 9,
                ..Default::default()
            };
            let result_tick = Tick {
                fee_growth_outside_x: FeeGrowth::new(340282366920938463463374607431768188274),
                fee_growth_outside_y: FeeGrowth::new(340282366920938463463374607431766958269),
                index: 45,
                liquidity_change: Liquidity::new(10),
                ..Default::default()
            };

            let fef_tick = RefCell::new(tick);
            let mut refmut_tick = fef_tick.borrow_mut();
            cross_tick(&mut refmut_tick, &mut pool).unwrap();
            assert_eq!(*refmut_tick, result_tick);
            assert_eq!(pool, result_pool);
        }
        // seconds_per_liquidity_outside should underflow
        {
            let mut pool = Pool {
                fee_growth_global_x: FeeGrowth::new(145),
                fee_growth_global_y: FeeGrowth::new(364),
                liquidity: Liquidity::new(14),
                current_tick_index: 9,
                ..Default::default()
            };
            let tick = Tick {
                fee_growth_outside_x: FeeGrowth::new(99),
                fee_growth_outside_y: FeeGrowth::new(256),
                index: 45,
                liquidity_change: Liquidity::new(10),
                ..Default::default()
            };
            let result_pool = Pool {
                fee_growth_global_x: FeeGrowth::new(145),
                fee_growth_global_y: FeeGrowth::new(364),
                liquidity: Liquidity::new(4),
                current_tick_index: 9,
                ..Default::default()
            };
            let result_tick = Tick {
                fee_growth_outside_x: FeeGrowth::new(46),
                fee_growth_outside_y: FeeGrowth::new(108),
                index: 45,
                liquidity_change: Liquidity::new(10),
                ..Default::default()
            };

            let fef_tick = RefCell::new(tick);
            let mut refmut_tick = fef_tick.borrow_mut();
            cross_tick(&mut refmut_tick, &mut pool).unwrap();
            assert_eq!(*refmut_tick, result_tick);
            assert_eq!(pool, result_pool);
        }
        // inconsistent state test cases
        {
            // underflow of pool.liquidity during cross_tick
            {
                let mut pool = Pool {
                    liquidity: Liquidity::from_integer(4),
                    current_tick_index: 7,
                    ..Default::default()
                };
                let tick = Tick {
                    index: 10,
                    liquidity_change: Liquidity::from_integer(5),
                    ..Default::default()
                };
                // state of pool and tick be should unchanged
                let result_pool = pool.clone();
                let result_tick = tick.clone();

                let ref_tick = RefCell::new(tick);
                let mut refmut_tick = ref_tick.borrow_mut();

                cross_tick(&mut refmut_tick, &mut pool).unwrap_err();
                assert_eq!(*refmut_tick, result_tick);
                assert_eq!(pool, result_pool);
            }
        }
    }
}
