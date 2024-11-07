pub mod dynamic_fee;
pub mod static_fees;

pub use dynamic_fee::*;
pub use static_fees::*;

pub const FEE_RATE_DENOMINATOR_VALUE: u64 = 1_000_000;

pub fn ceil_div(token_amount: u128, fee_numerator: u128, fee_denominator: u128) -> Option<u128> {
    token_amount
        .checked_mul(fee_numerator)?
        .checked_add(fee_denominator)?
        .checked_sub(1)?
        .checked_div(fee_denominator)
}

pub fn floor_div(token_amount: u128, fee_numerator: u128, fee_denominator: u128) -> Option<u128> {
    token_amount
        .checked_mul(fee_numerator)?
        .checked_div(fee_denominator)
}
