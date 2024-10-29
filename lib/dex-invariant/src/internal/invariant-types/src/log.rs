use crate::{decimals::*, math::calculate_price_sqrt};

const LOG2_SCALE: u8 = 32;
const LOG2_DOUBLE_SCALE: u8 = 64;
const LOG2_ONE: u128 = 1 << LOG2_SCALE;
const LOG2_HALF: u64 = (LOG2_ONE >> 1) as u64;
const LOG2_TWO: u128 = LOG2_ONE << 1;
const LOG2_DOUBLE_ONE: u128 = 1 << LOG2_DOUBLE_SCALE;
const LOG2_SQRT_10001: u64 = 309801;
const LOG2_NEGATIVE_MAX_LOSE: u64 = 300000; // max accuracy in <-MAX_TICK, 0> domain
const LOG2_MIN_BINARY_POSITION: i32 = 15; // accuracy = 2^(-15)
const LOG2_ACCURACY: u64 = 1u64 << (31 - LOG2_MIN_BINARY_POSITION);
const PRICE_DENOMINATOR: u128 = 1_000000_000000_000000_000000;

fn price_to_x32(decimal: Price) -> u64 {
    decimal
        .v
        .checked_mul(LOG2_ONE)
        .unwrap()
        .checked_div(PRICE_DENOMINATOR)
        .unwrap() as u64
}

fn align_tick_to_spacing(accurate_tick: i32, tick_spacing: i32) -> i32 {
    match accurate_tick > 0 {
        true => accurate_tick - (accurate_tick % tick_spacing),
        false => accurate_tick - (accurate_tick.rem_euclid(tick_spacing)),
    }
}

fn log2_floor_x32(mut sqrt_price_x32: u64) -> u64 {
    let mut msb = 0;

    if sqrt_price_x32 >= 1u64 << 32 {
        sqrt_price_x32 >>= 32;
        msb |= 32;
    };
    if sqrt_price_x32 >= 1u64 << 16 {
        sqrt_price_x32 >>= 16;
        msb |= 16;
    };
    if sqrt_price_x32 >= 1u64 << 8 {
        sqrt_price_x32 >>= 8;
        msb |= 8;
    };
    if sqrt_price_x32 >= 1u64 << 4 {
        sqrt_price_x32 >>= 4;
        msb |= 4;
    };
    if sqrt_price_x32 >= 1u64 << 2 {
        sqrt_price_x32 >>= 2;
        msb |= 2;
    };
    if sqrt_price_x32 >= 1u64 << 1 {
        msb |= 1;
    };

    msb
}

fn log2_iterative_approximation_x32(mut sqrt_price_x32: u64) -> (bool, u64) {
    let mut sign = true;
    // log2(x) = -log2(1/x), when x < 1
    if (sqrt_price_x32 as u128) < LOG2_ONE {
        sign = false;
        sqrt_price_x32 = (LOG2_DOUBLE_ONE / (sqrt_price_x32 as u128 + 1)) as u64
    }
    let log2_floor = log2_floor_x32(sqrt_price_x32 >> LOG2_SCALE);
    let mut result = log2_floor << LOG2_SCALE;
    let mut y: u128 = (sqrt_price_x32 as u128) >> log2_floor;

    if y == LOG2_ONE {
        return (sign, result);
    };
    let mut delta: u64 = LOG2_HALF;
    while delta > LOG2_ACCURACY {
        y = y * y / LOG2_ONE;
        if y >= LOG2_TWO {
            result |= delta;
            y >>= 1;
        }
        delta >>= 1;
    }
    (sign, result)
}

pub fn get_tick_at_sqrt_price(sqrt_price_decimal: Price, tick_spacing: u16) -> i32 {
    let sqrt_price_x32: u64 = price_to_x32(sqrt_price_decimal);
    let (log2_sign, log2_sqrt_price) = log2_iterative_approximation_x32(sqrt_price_x32);

    let abs_floor_tick: i32 = match log2_sign {
        true => log2_sqrt_price / LOG2_SQRT_10001,
        false => (log2_sqrt_price + LOG2_NEGATIVE_MAX_LOSE) / LOG2_SQRT_10001,
    } as i32;

    let nearer_tick = match log2_sign {
        true => abs_floor_tick,
        false => -abs_floor_tick,
    };
    let farther_tick = match log2_sign {
        true => abs_floor_tick + 1,
        false => -abs_floor_tick - 1,
    };
    let farther_tick_with_spacing = align_tick_to_spacing(farther_tick, tick_spacing as i32);
    let nearer_tick_with_spacing = align_tick_to_spacing(nearer_tick, tick_spacing as i32);
    if farther_tick_with_spacing == nearer_tick_with_spacing {
        return nearer_tick_with_spacing;
    };

    let accurate_tick = match log2_sign {
        true => {
            let farther_tick_sqrt_price_decimal = calculate_price_sqrt(farther_tick);
            match sqrt_price_decimal >= farther_tick_sqrt_price_decimal {
                true => farther_tick_with_spacing,
                false => nearer_tick_with_spacing,
            }
        }
        false => {
            let nearer_tick_sqrt_price_decimal = calculate_price_sqrt(nearer_tick);
            match nearer_tick_sqrt_price_decimal <= sqrt_price_decimal {
                true => nearer_tick_with_spacing,
                false => farther_tick_with_spacing,
            }
        }
    };
    match tick_spacing > 1 {
        true => align_tick_to_spacing(accurate_tick, tick_spacing as i32),
        false => accurate_tick,
    }
}

#[cfg(test)]
mod tests {
    use crate::{math::calculate_price_sqrt, structs::MAX_TICK};

    use super::*;

    #[test]
    fn test_price_to_u64() {
        // min sqrt price -> sqrt(1.0001)^MIN_TICK
        {
            let min_sqrt_price_decimal = calculate_price_sqrt(-MAX_TICK);
            let min_sqrt_price_x32 = price_to_x32(min_sqrt_price_decimal);

            let expected_min_sqrt_price_x32 = 65536;
            assert_eq!(min_sqrt_price_x32, expected_min_sqrt_price_x32);
        }
        // max sqrt price -> sqrt(1.0001)^MAX_TICK
        {
            let max_sqrt_price_decimal = calculate_price_sqrt(MAX_TICK);
            let max_sqrt_price_x32 = price_to_x32(max_sqrt_price_decimal);

            let expected_max_sqrt_price_x32 = 281472330729535;
            assert_eq!(max_sqrt_price_x32, expected_max_sqrt_price_x32);
        }
    }

    #[test]
    fn test_log2_x32() {
        // log2 of 1
        {
            let sqrt_price_decimal = Price::from_integer(1);
            let sqrt_price_x32 = price_to_x32(sqrt_price_decimal);
            let (sign, value) = log2_iterative_approximation_x32(sqrt_price_x32);
            assert_eq!(sign, true);
            assert_eq!(value, 0);
        }
        // log2 > 0 when x > 1
        {
            let sqrt_price_decimal = Price::from_integer(879);
            let sqrt_price_x32 = price_to_x32(sqrt_price_decimal);
            let (sign, value) = log2_iterative_approximation_x32(sqrt_price_x32);
            assert_eq!(sign, true);
            assert_eq!(value, 42003464192);
        }
        // log2 < 0 when x < 1
        {
            let sqrt_price_decimal = Price::from_scale(59, 4);
            let sqrt_price_x32 = price_to_x32(sqrt_price_decimal);
            let (sign, value) = log2_iterative_approximation_x32(sqrt_price_x32);
            assert_eq!(sign, false);
            assert_eq!(value, 31804489728);
        }
        // log2 of max sqrt price
        {
            let max_sqrt_price = calculate_price_sqrt(MAX_TICK);
            let sqrt_price_x32 = price_to_x32(max_sqrt_price);
            let (sign, value) = log2_iterative_approximation_x32(sqrt_price_x32);
            assert_eq!(sign, true);
            assert_eq!(value, 68719345664);
        }
        // log2 of min sqrt price
        {
            let min_sqrt_price = calculate_price_sqrt(-MAX_TICK);
            let sqrt_price_x32 = price_to_x32(min_sqrt_price);
            let (sign, value) = log2_iterative_approximation_x32(sqrt_price_x32);
            assert_eq!(sign, false);
            assert_eq!(value, 68719345664);
        }
        // log2 of sqrt(1.0001^(-19_999)) - 1
        {
            let mut sqrt_price_decimal = calculate_price_sqrt(-19_999);
            sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
            let sqrt_price_x32 = price_to_x32(sqrt_price_decimal);
            let (sign, value) = log2_iterative_approximation_x32(sqrt_price_x32);
            assert_eq!(sign, false);
            assert_eq!(value, 6195642368);
        }
        // log2 of sqrt(1.0001^(19_999)) + 1
        {
            let mut sqrt_price_decimal = calculate_price_sqrt(19_999);
            sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
            let sqrt_price_x32 = price_to_x32(sqrt_price_decimal);
            let (sign, value) = log2_iterative_approximation_x32(sqrt_price_x32);
            assert_eq!(sign, true);
            assert_eq!(value, 6195642368);
        }
    }

    #[test]
    fn test_get_tick_at_sqrt_price_x32() {
        // around 0 tick
        {
            // get tick at 1
            {
                let sqrt_price_decimal = Price::from_integer(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, 0);
            }
            // get tick slightly below 1
            {
                let sqrt_price_decimal = Price::from_integer(1) - Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, -1);
            }
            // get tick slightly above 1
            {
                let sqrt_price_decimal = Price::from_integer(1) + Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, 0);
            }
        }
        // around 1 tick
        {
            let sqrt_price_decimal = calculate_price_sqrt(1);
            // get tick at sqrt(1.0001)
            {
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, 1);
            }
            // get tick slightly below sqrt(1.0001)
            {
                let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, 0);
            }
            // get tick slightly above sqrt(1.0001)
            {
                let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, 1);
            }
        }
        // around -1 tick
        {
            let sqrt_price_decimal = calculate_price_sqrt(-1);
            // get tick at sqrt(1.0001^(-1))
            {
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, -1);
            }
            // get tick slightly below sqrt(1.0001^(-1))
            {
                let sqrt_price_decimal = calculate_price_sqrt(-1) - Price::new(1);

                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, -2);
            }
            // get tick slightly above sqrt(1.0001^(-1))
            {
                let sqrt_price_decimal = calculate_price_sqrt(-1) + Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, -1);
            }
        }
        // around max - 1 tick
        {
            let sqrt_price_decimal = calculate_price_sqrt(MAX_TICK - 1);
            // get tick at sqrt(1.0001^(MAX_TICK - 1))
            {
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, MAX_TICK - 1);
            }
            // get tick slightly below sqrt(1.0001^(MAX_TICK - 1))
            {
                let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, MAX_TICK - 2);
            }
            // get tick slightly above sqrt(1.0001^(MAX_TICK - 1))
            {
                let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, MAX_TICK - 1);
            }
        }
        // around min + 1 tick
        {
            let sqrt_price_decimal = calculate_price_sqrt(-(MAX_TICK - 1));
            // get tick at sqrt(1.0001^(-MAX_TICK + 1))
            {
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, -(MAX_TICK - 1));
            }
            // get tick slightly below sqrt(1.0001^(-MAX_TICK + 1))
            {
                let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, -MAX_TICK);
            }
            // get tick slightly above sqrt(1.0001^(-MAX_TICK + 1))
            {
                let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, -(MAX_TICK - 1));
            }
        }
        //get tick slightly below at max tick
        {
            let max_sqrt_price = Price::from_scale(655354, 1);
            let sqrt_price_decimal = max_sqrt_price - Price::new(1);
            let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
            assert_eq!(tick, MAX_TICK);
        }
        // around 19_999 tick
        {
            let expected_tick = 19_999;
            let sqrt_price_decimal = calculate_price_sqrt(expected_tick);
            // get tick at sqrt(1.0001^19_999)
            {
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, expected_tick);
            }
            // get tick slightly below sqrt(1.0001^19_999)
            {
                let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);

                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, expected_tick - 1);
            }
            // get tick slightly above sqrt(1.0001^19_999)
            {
                let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, expected_tick);
            }
        }
        // around -19_999 tick
        {
            let expected_tick = -19_999;
            let sqrt_price_decimal = calculate_price_sqrt(expected_tick);
            // get tick at sqrt(1.0001^(-19_999))
            {
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, expected_tick);
            }
            // get tick slightly below sqrt(1.0001^(-19_999))
            {
                // let sqrt_price_decimal = sqrt_price_decimal - Decimal::new(150);
                let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, expected_tick - 1);
            }
            // get tick slightly above sqrt(1.0001^(-19_999))
            {
                let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                assert_eq!(tick, expected_tick);
            }
        }
        //get tick slightly above at min tick
        {
            let min_sqrt_price = calculate_price_sqrt(-MAX_TICK);
            let sqrt_price_decimal = min_sqrt_price + Price::new(1);
            let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
            assert_eq!(tick, -MAX_TICK);
        }
    }

    #[test]
    fn test_align_tick_with_spacing() {
        // zero
        {
            let accurate_tick = 0;
            let tick_spacing = 3;

            let tick_with_spacing = align_tick_to_spacing(accurate_tick, tick_spacing);
            assert_eq!(tick_with_spacing, 0);
        }
        // positive
        {
            let accurate_tick = 14;
            let tick_spacing = 10;

            let tick_with_spacing = align_tick_to_spacing(accurate_tick, tick_spacing);
            assert_eq!(tick_with_spacing, 10);
        }
        // positive at tick
        {
            let accurate_tick = 20;
            let tick_spacing = 10;

            let tick_with_spacing = align_tick_to_spacing(accurate_tick, tick_spacing);
            assert_eq!(tick_with_spacing, 20);
        }
        // negative
        {
            let accurate_tick = -14;
            let tick_spacing = 10;

            let tick_with_spacing = align_tick_to_spacing(accurate_tick, tick_spacing);
            assert_eq!(tick_with_spacing, -20);
        }
        // negative at tick
        {
            let accurate_tick = -120;
            let tick_spacing = 3;

            let tick_with_spacing = align_tick_to_spacing(accurate_tick, tick_spacing);
            assert_eq!(tick_with_spacing, -120);
        }
    }

    #[test]
    fn test_all_positive_ticks() {
        for n in 0..MAX_TICK {
            {
                let expected_tick = n;
                let sqrt_price_decimal = calculate_price_sqrt(expected_tick);
                // get tick at sqrt(1.0001^(n))
                {
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                    assert_eq!(tick, expected_tick);
                }
                // get tick slightly below sqrt(1.0001^n)
                {
                    let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                    assert_eq!(tick, expected_tick - 1);
                }
                // get tick slightly above sqrt(1.0001^n)
                {
                    let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                    assert_eq!(tick, expected_tick);
                }
            }
        }
    }

    #[test]
    fn test_all_negative_ticks() {
        for n in 0..MAX_TICK {
            {
                let expected_tick = -n;
                let sqrt_price_decimal = calculate_price_sqrt(expected_tick);
                // get tick at sqrt(1.0001^(n))
                {
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                    assert_eq!(tick, expected_tick);
                }
                // get tick slightly below sqrt(1.0001^n)
                {
                    let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                    assert_eq!(tick, expected_tick - 1);
                }
                // get tick slightly above sqrt(1.0001^n)
                {
                    let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, 1);
                    assert_eq!(tick, expected_tick);
                }
            }
        }
    }

    #[test]
    fn test_all_positive_tick_spacing_greater_than_1() {
        let tick_spacing: i32 = 3;
        for n in 0..MAX_TICK {
            {
                let input_tick = n;
                let sqrt_price_decimal = calculate_price_sqrt(input_tick);
                // get tick at sqrt(1.0001^(n))
                {
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, tick_spacing as u16);
                    let expected_tick = align_tick_to_spacing(input_tick, tick_spacing);
                    assert_eq!(tick, expected_tick);
                }
                // get tick slightly below sqrt(1.0001^n)
                {
                    let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, tick_spacing as u16);
                    let expected_tick = align_tick_to_spacing(input_tick - 1, tick_spacing);
                    assert_eq!(tick, expected_tick);
                }
                // get tick slightly above sqrt(1.0001^n)
                {
                    let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, tick_spacing as u16);
                    let expected_tick = align_tick_to_spacing(input_tick, tick_spacing);
                    assert_eq!(tick, expected_tick);
                }
            }
        }
    }

    #[test]
    fn test_all_negative_tick_spacing_greater_than_1() {
        let tick_spacing: i32 = 4;
        for n in 0..MAX_TICK {
            {
                let input_tick = -n;
                let sqrt_price_decimal = calculate_price_sqrt(input_tick);
                // get tick at sqrt(1.0001^(n))
                {
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, tick_spacing as u16);
                    let expected_tick = align_tick_to_spacing(input_tick, tick_spacing);
                    assert_eq!(tick, expected_tick);
                }
                // get tick slightly below sqrt(1.0001^n)
                {
                    let sqrt_price_decimal = sqrt_price_decimal - Price::new(1);
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, tick_spacing as u16);
                    let expected_tick = align_tick_to_spacing(input_tick - 1, tick_spacing);
                    assert_eq!(tick, expected_tick);
                }
                // get tick slightly above sqrt(1.0001^n)
                {
                    let sqrt_price_decimal = sqrt_price_decimal + Price::new(1);
                    let tick = get_tick_at_sqrt_price(sqrt_price_decimal, tick_spacing as u16);
                    let expected_tick = align_tick_to_spacing(input_tick, tick_spacing);
                    assert_eq!(tick, expected_tick);
                }
            }
        }
    }
}
