use crate::utils::math::CheckedCeilDiv;
use crate::GammaError;
use anchor_lang::prelude::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConstantProductCurve;

impl ConstantProductCurve {
    pub fn swap_base_input_without_fees(
        source_amount_to_be_swapped: u128,
        swap_source_amount: u128,
        swap_destination_amount: u128,
    ) -> Result<u128> {
        let numerator = source_amount_to_be_swapped
            .checked_mul(swap_destination_amount)
            .ok_or(GammaError::MathOverflow)?;
        let denominator = swap_source_amount
            .checked_add(source_amount_to_be_swapped)
            .ok_or(GammaError::MathOverflow)?;
        let destination_amount_swapped = numerator
            .checked_div(denominator)
            .ok_or(GammaError::MathOverflow)?;
        Ok(destination_amount_swapped)
    }

    pub fn swap_base_output_without_fees(
        destination_amount_to_be_swapped: u128,
        swap_source_amount: u128,
        swap_destination_amount: u128,
    ) -> Result<u128> {
        let numerator = swap_source_amount
            .checked_mul(destination_amount_to_be_swapped)
            .ok_or(GammaError::MathOverflow)?;
        let denominator = swap_destination_amount
            .checked_sub(destination_amount_to_be_swapped)
            .ok_or(GammaError::MathOverflow)?;
        let (source_amount_swapped, _) = numerator
            .checked_ceil_div(denominator)
            .ok_or(GammaError::MathOverflow)?;
        Ok(source_amount_swapped)
    }
}
