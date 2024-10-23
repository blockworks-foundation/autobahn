use crate::internal::swap::InvariantSwapResult;
use decimal::*;
use invariant_types::{
    decimals::{Price, TokenAmount},
    log::get_tick_at_sqrt_price,
    math::{
        compute_swap_step, cross_tick_no_fee_growth_update, get_closer_limit,
        get_max_tick, get_min_tick, is_enough_amount_to_push_price,
    },
    structs::{Pool, Tick, TickmapView, TICKS_BACK_COUNT, TICK_CROSSES_PER_IX},
};
use solana_program::pubkey::Pubkey;
use std::any::Any;

use router_lib::dex::{DexEdge, DexEdgeIdentifier};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct InvariantEdgeIdentifier {
    pub pool: Pubkey,
    pub token_x: Pubkey,
    pub token_y: Pubkey,
    pub x_to_y: bool,
}

impl DexEdgeIdentifier for InvariantEdgeIdentifier {
    fn key(&self) -> Pubkey {
        self.pool
    }

    fn desc(&self) -> String {
        format!("Invariant_{}", self.pool)
    }

    fn input_mint(&self) -> Pubkey {
        if self.x_to_y {
            self.token_x
        } else {
            self.token_y
        }
    }

    fn output_mint(&self) -> Pubkey {
        if self.x_to_y {
            self.token_y
        } else {
            self.token_x
        }
    }

    fn accounts_needed(&self) -> usize {
        10 // total accounts without ticks
         - 2 // user output ATA + user wallet address
          + TICK_CROSSES_PER_IX + TICKS_BACK_COUNT
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Default, Debug)]
pub struct InvariantEdge {
    // TODO: use pubkeys or indexes
    pub ticks: Vec<Tick>,
    pub pool: Pool,
    // TODO: possibly remove and use raw data with bytemuck?
    pub tickmap: TickmapView,
}

#[derive(Debug, Default)]
pub struct InvariantSimulationParams {
    pub x_to_y: bool,
    pub in_amount: u64,
    pub sqrt_price_limit: Price,
    pub by_amount_in: bool,
}

impl InvariantEdge {
    pub fn simulate_invariant_swap(
        &self,
        invariant_simulation_params: &InvariantSimulationParams,
    ) -> Result<InvariantSwapResult, String> {
        let InvariantSimulationParams {
            x_to_y,
            in_amount,
            sqrt_price_limit,
            by_amount_in,
        } = *invariant_simulation_params;

        let mut pool = self.pool.clone();
        let tickmap = &self.tickmap;
        let ticks = self.ticks.to_vec();
        let starting_sqrt_price = pool.sqrt_price;
        let current_tick_index = pool.current_tick_index;
        let pool = &mut pool;

        let (mut remaining_amount, mut total_amount_in, mut total_amount_out, mut total_fee_amount) = (
            TokenAmount::new(in_amount),
            TokenAmount::new(0),
            TokenAmount::new(0),
            TokenAmount::new(0),
        );
        let (
            mut used_ticks,
            mut virtual_cross_counter,
            mut global_insufficient_liquidity,
            mut ticks_accounts_outdated,
        ) = (Vec::new(), 0u16, false, false);

        let mut current_tick_array_index = 0;
        while current_tick_array_index < ticks.len() {
            let index = ticks[current_tick_array_index].index;
            let skip = if x_to_y {
                index > current_tick_index
            } else {
                index <= current_tick_index
            };
            if skip {
                current_tick_array_index += 1;
            } else {
                break;
            }
        }

        while !remaining_amount.is_zero() {
            let (swap_limit, limiting_tick) = match get_closer_limit(
                sqrt_price_limit,
                x_to_y,
                pool.current_tick_index,
                pool.tick_spacing,
                tickmap,
            ) {
                Ok((swap_limit, limiting_tick)) => (swap_limit, limiting_tick),
                Err(_) => {
                    global_insufficient_liquidity = true;
                    break;
                }
            };

            let result = compute_swap_step(
                pool.sqrt_price,
                swap_limit,
                pool.liquidity,
                remaining_amount,
                by_amount_in,
                pool.fee,
            )
            .map_err(|e| {
                let (formatted, _, _) = e.get();
                formatted
            })?;

            remaining_amount =
                remaining_amount.checked_sub(result.amount_in.checked_add(result.fee_amount)?)?;
            pool.sqrt_price = result.next_price_sqrt;
            total_amount_in = total_amount_in
                .checked_add(result.amount_in)?
                .checked_add(result.fee_amount)?;
            total_amount_out = total_amount_out.checked_add(result.amount_out)?;
            total_fee_amount = total_fee_amount.checked_add(result.fee_amount)?;

            if { pool.sqrt_price } == sqrt_price_limit && !remaining_amount.is_zero() {
                global_insufficient_liquidity = true;
                break;
            }
            let reached_tick_limit = match x_to_y {
                true => {
                    pool.current_tick_index
                        <= get_min_tick(pool.tick_spacing).map_err(|err| err.cause)?
                }
                false => {
                    pool.current_tick_index
                        >= get_max_tick(pool.tick_spacing).map_err(|err| err.cause)?
                }
            };
            if reached_tick_limit {
                global_insufficient_liquidity = true;
                break;
            }
            // crossing tick
            if result.next_price_sqrt == swap_limit && limiting_tick.is_some() {
                let (tick_index, initialized) = limiting_tick.unwrap();
                let is_enough_amount_to_cross = is_enough_amount_to_push_price(
                    remaining_amount,
                    result.next_price_sqrt,
                    pool.liquidity,
                    pool.fee,
                    by_amount_in,
                    x_to_y,
                )
                .map_err(|e| {
                    let (formatted, _, _) = e.get();
                    formatted
                })?;

                if initialized {
                    // tick to fallback to in case no tick is found
                    used_ticks.push(tick_index);
                    let default_tick = Tick {
                        index: tick_index,
                        ..Default::default()
                    };

                    // ticks should be sorted in the same order as the swap
                    let tick = &match ticks.get(current_tick_array_index) {
                        Some(tick) => {
                            if tick.index != tick_index {
                                default_tick
                            } else {
                                current_tick_array_index += 1;
                                *tick
                            }
                        }
                        None => default_tick,
                    };

                    // crossing tick
                    if !x_to_y || is_enough_amount_to_cross {
                        if cross_tick_no_fee_growth_update(tick, pool).is_err() {
                            global_insufficient_liquidity = true;
                            break;
                        }
                    } else if !remaining_amount.is_zero() {
                        total_amount_in = total_amount_in
                            .checked_add(remaining_amount)
                            .map_err(|_| "add overflow")?;
                        remaining_amount = TokenAmount(0);
                    }
                } else {
                    virtual_cross_counter =
                        virtual_cross_counter.checked_add(1).ok_or("add overflow")?;
                    if InvariantSwapResult::break_swap_loop_early(
                        used_ticks.len() as u16,
                        virtual_cross_counter,
                    )? {
                        global_insufficient_liquidity = true;
                        break;
                    }
                }

                pool.current_tick_index = if x_to_y && is_enough_amount_to_cross {
                    tick_index
                        .checked_sub(pool.tick_spacing as i32)
                        .ok_or("sub overflow")?
                } else {
                    tick_index
                };
            } else {
                if pool
                    .current_tick_index
                    .checked_rem(pool.tick_spacing.into())
                    .unwrap()
                    != 0
                {
                    return Err("Internal Invariant Error: Invalid tick".to_string());
                }
                pool.current_tick_index =
                    get_tick_at_sqrt_price(result.next_price_sqrt, pool.tick_spacing);
                virtual_cross_counter =
                    virtual_cross_counter.checked_add(1).ok_or("add overflow")?;
                if InvariantSwapResult::break_swap_loop_early(
                    used_ticks.len() as u16,
                    virtual_cross_counter,
                )? {
                    global_insufficient_liquidity = true;
                    break;
                }
            }
        }

        // TODO: split into multiple errors or move up
        if remaining_amount.0 != 0 || ticks_accounts_outdated || global_insufficient_liquidity {
            return Err("Insuffcient liquidity".into());
        }

        // TODO remove unused fields
        Ok(InvariantSwapResult {
            in_amount: total_amount_in.0,
            out_amount: total_amount_out.0,
            fee_amount: total_fee_amount.0,
            starting_sqrt_price: starting_sqrt_price,
            ending_sqrt_price: pool.sqrt_price,
            used_ticks,
            virtual_cross_counter,
            global_insufficient_liquidity,
            ticks_accounts_outdated,
        })
    }
}
impl DexEdge for InvariantEdge {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
