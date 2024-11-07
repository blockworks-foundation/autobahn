use invariant_types::{
    decimals::{CheckedOps, Decimal, Price, TokenAmount},
    log::get_tick_at_sqrt_price,
    math::{
        compute_swap_step, cross_tick, get_closer_limit, get_max_sqrt_price, get_max_tick,
        get_min_sqrt_price, get_min_tick, is_enough_amount_to_push_price,
    },
    structs::{TICKS_BACK_COUNT, TICK_CROSSES_PER_IX},
    MAX_VIRTUAL_CROSS,
};

pub struct InvariantSimulationParams {
    pub in_amount: u64,
    pub x_to_y: bool,
    pub by_amount_in: bool,
    pub sqrt_price_limit: Price,
}

#[derive(Clone, Default, Debug)]
pub struct InvariantSwapResult {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub starting_sqrt_price: Price,
    pub ending_sqrt_price: Price,
    pub used_ticks: Vec<i32>,
    pub global_insufficient_liquidity: bool,
}

impl InvariantSwapResult {
    pub fn break_swap_loop_early(
        ticks_used: u16,
        virtual_ticks_crossed: u16,
    ) -> Result<bool, String> {
        let break_loop = ticks_used
            .checked_add(virtual_ticks_crossed)
            .ok_or_else(|| "virtual ticks crossed + ticks crossed overflow")?
            >= TICK_CROSSES_PER_IX as u16 + MAX_VIRTUAL_CROSS
            || TICK_CROSSES_PER_IX <= ticks_used as usize;

        Ok(break_loop)
    }
}
