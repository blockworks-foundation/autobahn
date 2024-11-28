use crate::fees::{DynamicFee, FeeType};
use crate::GammaError;
use crate::ObservationState;
use crate::{curve::constant_product::ConstantProductCurve, fees::StaticFee};
use anchor_lang::prelude::*;
use std::fmt::Debug;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TradeDirection {
    ZeroForOne,
    OneForZero,
}

impl TradeDirection {
    pub fn opposite(&self) -> TradeDirection {
        match self {
            TradeDirection::ZeroForOne => TradeDirection::OneForZero,
            TradeDirection::OneForZero => TradeDirection::ZeroForOne,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct SwapResult {
    pub new_swap_source_amount: u128,
    pub new_swap_destination_amount: u128,
    pub source_amount_swapped: u128,
    pub destination_amount_swapped: u128,
    pub dynamic_fee: u128,
    pub protocol_fee: u128,
    pub fund_fee: u128,
}

/// Concrete struct to wrap around the trait object which performs calculation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CurveCalculator {}

impl CurveCalculator {
    pub fn swap_base_input(
        source_amount_to_be_swapped: u128,
        swap_source_amount: u128,
        swap_destination_amount: u128,
        trade_fee_rate: u64,
        protocol_fee_rate: u64,
        fund_fee_rate: u64,
        block_timestamp: u64,
        observation_state: &ObservationState,
        // TODO: add fee type here once that is configurable on pool level/ or we can use it from pool_state
    ) -> Result<SwapResult> {
        let dynamic_fee = DynamicFee::dynamic_fee(
            source_amount_to_be_swapped,
            block_timestamp,
            observation_state,
            FeeType::Volatility,
            trade_fee_rate,
        )?;

        let protocol_fee = StaticFee::protocol_fee(dynamic_fee, protocol_fee_rate)
            .ok_or(GammaError::InvalidFee)?;
        let fund_fee =
            StaticFee::fund_fee(dynamic_fee, fund_fee_rate).ok_or(GammaError::InvalidFee)?;

        let source_amount_after_fees = source_amount_to_be_swapped
            .checked_sub(dynamic_fee)
            .ok_or(GammaError::MathOverflow)?;
        let destination_amount_swapped = ConstantProductCurve::swap_base_input_without_fees(
            source_amount_after_fees,
            swap_source_amount,
            swap_destination_amount,
        )?;

        Ok(SwapResult {
            new_swap_source_amount: swap_source_amount
                .checked_add(source_amount_to_be_swapped)
                .ok_or(GammaError::MathOverflow)?,
            new_swap_destination_amount: swap_destination_amount
                .checked_sub(destination_amount_swapped)
                .ok_or(GammaError::MathOverflow)?,
            source_amount_swapped: source_amount_to_be_swapped,
            destination_amount_swapped,
            dynamic_fee,
            protocol_fee,
            fund_fee,
        })
    }

    /// Subtract fees and calculate how much source token will be required
    pub fn swap_base_output(
        destination_amount_to_be_swapped: u128,
        swap_source_amount: u128,
        swap_destination_amount: u128,
        trade_fee_rate: u64,
        protocol_fee_rate: u64,
        fund_fee_rate: u64,
        block_timestamp: u64,
        observation_state: &ObservationState,
    ) -> Result<SwapResult> {
        let source_amount_swapped = ConstantProductCurve::swap_base_output_without_fees(
            destination_amount_to_be_swapped,
            swap_source_amount,
            swap_destination_amount,
        )?;

        let source_amount = DynamicFee::calculate_pre_fee_amount(
            block_timestamp,
            source_amount_swapped,
            observation_state,
            FeeType::Volatility,
            trade_fee_rate,
        )?;

        let dynamic_fee = source_amount
            .checked_sub(source_amount_swapped)
            .ok_or(GammaError::MathOverflow)?;
        let protocol_fee = StaticFee::protocol_fee(dynamic_fee, protocol_fee_rate)
            .ok_or(GammaError::MathOverflow)?;
        let fund_fee =
            StaticFee::fund_fee(dynamic_fee, fund_fee_rate).ok_or(GammaError::MathOverflow)?;

        Ok(SwapResult {
            new_swap_source_amount: swap_source_amount
                .checked_add(source_amount)
                .ok_or(GammaError::MathOverflow)?,
            new_swap_destination_amount: swap_destination_amount
                .checked_sub(destination_amount_to_be_swapped)
                .ok_or(GammaError::MathOverflow)?,
            source_amount_swapped: source_amount,
            destination_amount_swapped: destination_amount_to_be_swapped,
            protocol_fee,
            fund_fee,
            dynamic_fee,
        })
    }
}
