use super::{ceil_div, FEE_RATE_DENOMINATOR_VALUE};
use crate::GammaError;
use crate::{Observation, ObservationState};
use anchor_lang::prelude::*;

use num_traits::cast::ToPrimitive;
use num_traits::identities::Zero;

pub const OBSERVATION_NUM: usize = 100;

pub const MAX_FEE_VOLATILITY: u64 = 10000; // 1% max fee
pub const VOLATILITY_WINDOW: u64 = 3600; // 1 hour window for volatility calculation

const MAX_FEE: u64 = 100000; // 10% max fee
const VOLATILITY_FACTOR: u64 = 300_000; // Adjust based on desired sensitivity

pub enum FeeType {
    Volatility,
}

struct ObservationWithIndex {
    observation: Observation,
    index: u16,
}

pub struct DynamicFee {}

impl DynamicFee {
    pub fn dynamic_fee(
        amount: u128,
        block_timestamp: u64,
        observation_state: &ObservationState,
        fee_type: FeeType,
        base_fees: u64,
    ) -> Result<u128> {
        let dynamic_fee_rate =
            Self::calculate_dynamic_fee(block_timestamp, observation_state, fee_type, base_fees)?;

        Ok(ceil_div(
            amount,
            u128::from(dynamic_fee_rate),
            u128::from(FEE_RATE_DENOMINATOR_VALUE),
        )
        .ok_or(GammaError::MathOverflow)?)
    }

    fn calculate_dynamic_fee(
        block_timestamp: u64,
        observation_state: &ObservationState,
        fee_type: FeeType,
        base_fees: u64,
    ) -> Result<u64> {
        match fee_type {
            FeeType::Volatility => {
                Self::calculate_volatile_fee(block_timestamp, observation_state, base_fees)
            }
        }
    }

    fn calculate_volatile_fee(
        block_timestamp: u64,
        observation_state: &ObservationState,
        base_fees: u64,
    ) -> Result<u64> {
        let (min_price, max_price, twap_price) =
            Self::get_price_range(observation_state, block_timestamp, VOLATILITY_WINDOW)?;
        if min_price == 0 || max_price == 0 || twap_price == 0 || twap_price == 1 {
            return Ok(base_fees);
        }

        let log_max_price = (max_price as f64).ln();
        let log_min_price = (min_price as f64).ln();
        let log_twap_price = (twap_price as f64).ln();

        let volatility_numerator = (log_max_price - log_min_price).abs();
        let volatility_denominator = log_twap_price.abs();

        if volatility_denominator.is_zero() {
            return Ok(base_fees);
        }

        let volatility = volatility_numerator / volatility_denominator;

        let volatility_component_calculated = (VOLATILITY_FACTOR as f64 * volatility)
            .to_u64()
            .ok_or(GammaError::MathOverflow)?;

        let dynamic_fee = base_fees
            .checked_add(volatility_component_calculated)
            .ok_or(GammaError::MathOverflow)?;

        Ok(std::cmp::min(dynamic_fee, MAX_FEE))
    }

    fn get_price_range(
        observation_state: &ObservationState,
        current_time: u64,
        window: u64,
    ) -> Result<(u128, u128, u128)> {
        let mut min_price = u128::MAX;
        let mut max_price = 0u128;
        let mut descending_order_observations = observation_state
            .observations
            .iter()
            .enumerate()
            .filter(|(_, observation)| {
                observation.block_timestamp != 0
                    && observation.cumulative_token0_price_x32 != 0
                    && observation.cumulative_token1_price_x32 != 0
                    && current_time.saturating_sub(observation.block_timestamp) <= window
            })
            .map(|(index, observation)| ObservationWithIndex {
                index: index as u16,
                observation: *observation,
            })
            .collect::<Vec<_>>();

        // Sort observations by timestamp (newest first)
        descending_order_observations.sort_by(|a, b| {
            { b.observation.block_timestamp }.cmp(&{ a.observation.block_timestamp })
        });

        if descending_order_observations.len() < 2 {
            return Ok((0, 0, 0));
        }

        let newest_obs = descending_order_observations.first().unwrap();
        let oldest_obs = descending_order_observations.last().unwrap();

        let total_time_delta = newest_obs
            .observation
            .block_timestamp
            .saturating_sub(oldest_obs.observation.block_timestamp)
            as u128;

        if total_time_delta == 0 {
            return Ok((0, 0, 0));
        }

        let twap_price = newest_obs
            .observation
            .cumulative_token0_price_x32
            .checked_sub(oldest_obs.observation.cumulative_token0_price_x32)
            .ok_or(GammaError::MathOverflow)?
            .checked_div(total_time_delta)
            .ok_or(GammaError::MathOverflow)?;

        for observation_with_index in descending_order_observations {
            let last_observation_index = if observation_with_index.index == 0 {
                OBSERVATION_NUM - 1
            } else {
                observation_with_index.index as usize - 1
            };

            if observation_state.observations[last_observation_index].block_timestamp == 0 {
                continue;
            }

            if observation_state.observations[last_observation_index].block_timestamp
                > observation_with_index.observation.block_timestamp
            {
                break;
            }

            let obs = observation_state.observations[last_observation_index];
            let next_obs = observation_with_index.observation;

            let time_delta = next_obs.block_timestamp.saturating_sub(obs.block_timestamp) as u128;

            if time_delta == 0 {
                continue;
            }

            let price = next_obs
                .cumulative_token0_price_x32
                .checked_sub(obs.cumulative_token0_price_x32)
                .ok_or(GammaError::MathOverflow)?
                .checked_div(time_delta)
                .ok_or(GammaError::MathOverflow)?;

            min_price = min_price.min(price);
            max_price = max_price.max(price);
        }

        Ok((min_price, max_price, twap_price))
    }

    pub fn calculate_pre_fee_amount(
        block_timestamp: u64,
        post_fee_amount: u128,
        observation_state: &ObservationState,
        fee_type: FeeType,
        base_fees: u64,
    ) -> Result<u128> {
        let dynamic_fee_rate =
            Self::calculate_dynamic_fee(block_timestamp, observation_state, fee_type, base_fees)?;
        if dynamic_fee_rate == 0 {
            Ok(post_fee_amount)
        } else {
            let numerator = post_fee_amount
                .checked_mul(u128::from(FEE_RATE_DENOMINATOR_VALUE))
                .ok_or(GammaError::MathOverflow)?;
            let denominator = u128::from(FEE_RATE_DENOMINATOR_VALUE)
                .checked_sub(u128::from(dynamic_fee_rate))
                .ok_or(GammaError::MathOverflow)?;

            let result = numerator
                .checked_add(denominator)
                .ok_or(GammaError::MathOverflow)?
                .checked_sub(1)
                .ok_or(GammaError::MathOverflow)?
                .checked_div(denominator)
                .ok_or(GammaError::MathOverflow)?;

            Ok(result)
        }
    }
}
