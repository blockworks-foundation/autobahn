use crate::invariant_dex::TICK_CROSSES_PER_ROUTE_IX;
use invariant_types::{
    decimals::{CheckedOps, Decimal, Price, TokenAmount},
    log::get_tick_at_sqrt_price,
    math::{
        compute_swap_step, cross_tick, get_closer_limit, get_max_sqrt_price, get_max_tick,
        get_min_sqrt_price, get_min_tick, is_enough_amount_to_push_price,
    },
    MAX_VIRTUAL_CROSS,
};

pub struct InvariantSimulationParams {
    pub in_amount: u64,
    pub x_to_y: bool,
    pub by_amount_in: bool,
    pub sqrt_price_limit: Price,
}

#[derive(Clone, Default)]
pub struct InvariantSwapResult {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub starting_sqrt_price: Price,
    pub ending_sqrt_price: Price,
    pub used_ticks: Vec<i32>,
    pub virtual_cross_counter: u16,
    pub global_insufficient_liquidity: bool,
    pub ticks_accounts_outdated: bool,
}

impl InvariantSwapResult {
    pub fn is_not_enough_liquidity(&self) -> bool {
        // since "is_referral" is not specified in the quote parameters, we pessimistically assume that the referral is always used
        self.ticks_accounts_outdated || self.global_insufficient_liquidity
    }

    pub fn break_swap_loop_early(
        ticks_crossed: u16,
        virtual_ticks_crossed: u16,
    ) -> Result<bool, String> {
        Ok(ticks_crossed
            .checked_add(virtual_ticks_crossed)
            .ok_or_else(|| "virtual ticks crossed + ticks crossed overflow")?
            > MAX_VIRTUAL_CROSS + TICK_CROSSES_PER_ROUTE_IX as u16)
    }
}
