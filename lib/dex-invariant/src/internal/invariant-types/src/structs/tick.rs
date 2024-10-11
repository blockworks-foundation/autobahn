use crate::{decimals::*, size};
use anchor_lang::prelude::*;

#[account(zero_copy)]
#[repr(packed)]
#[derive(PartialEq, Default, Debug, AnchorDeserialize)]
pub struct Tick {
    pub pool: Pubkey,
    pub index: i32,
    pub sign: bool, // true means positive
    pub liquidity_change: Liquidity,
    pub liquidity_gross: Liquidity,
    pub sqrt_price: Price,
    pub fee_growth_outside_x: FeeGrowth,
    pub fee_growth_outside_y: FeeGrowth,
    pub seconds_per_liquidity_outside: FixedPoint,
    pub seconds_outside: u64,
    pub bump: u8,
}
size!(Tick);
