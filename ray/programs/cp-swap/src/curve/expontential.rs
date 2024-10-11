use std::fmt;
use anchor_lang::prelude::*;
use anchor_lang::AnchorDeserialize;
pub const DEFAULT_TOKEN_RESERVES: u128 = 1000000000000000;
pub const DEFAULT_VIRTUAL_SOL_RESERVE: u128 = 30000000000;
pub const DEFUALT_VIRTUAL_TOKEN_RESERVE: u128 = 793100000000000;
pub const DEFUALT_INITIAL_VIRTUAL_TOKEN_RESERVE: u128 = 1073000000000000;
pub const DEFAULT_FEE_BASIS_POINTS: u128 = 50;

#[derive(Debug)]
pub struct BuyResult {
    pub token_amount: u64,
    pub sol_amount: u64,
}

#[derive(Debug)]
pub struct SellResult {
    pub token_amount: u64,
    pub sol_amount: u64,
}

#[derive(Debug, Clone, Copy, AnchorDeserialize, AnchorSerialize, Default)]
pub struct AMM {
    pub virtual_sol_reserves: u128,
    pub virtual_token_reserves: u128,
    pub real_sol_reserves: u128,
    pub real_token_reserves: u128,
    pub initial_virtual_token_reserves: u128,
}

impl AMM {
    pub fn new(
        virtual_sol_reserves: u128,
        virtual_token_reserves: u128,
        real_sol_reserves: u128,
        real_token_reserves: u128,
        initial_virtual_token_reserves: u128,
    ) -> Self {
        AMM {
            virtual_sol_reserves,
            virtual_token_reserves,
            real_sol_reserves,
            real_token_reserves,
            initial_virtual_token_reserves,
        }
    }
    pub fn get_buy_price(&self, tokens: u128) -> Option<u128> {
        if tokens == 0 || tokens > self.virtual_token_reserves {
            return None;
        }

        let product_of_reserves = self.virtual_sol_reserves.checked_mul(self.virtual_token_reserves)?;
        let new_virtual_token_reserves = self.virtual_token_reserves.checked_sub(tokens)?;
        let new_virtual_sol_reserves = product_of_reserves.checked_div(new_virtual_token_reserves)?.checked_add(1)?;
        let amount_needed = new_virtual_sol_reserves.checked_sub(self.virtual_sol_reserves)?;

        Some(amount_needed)
    }

    pub fn apply_buy(&mut self, token_amount: u128) -> Option<BuyResult> {
        let final_token_amount = token_amount.min(self.real_token_reserves);
        let sol_amount = self.get_buy_price(final_token_amount)?;

        self.virtual_token_reserves = self.virtual_token_reserves.checked_sub(final_token_amount)?;
        self.real_token_reserves = self.real_token_reserves.checked_sub(final_token_amount)?;

        self.virtual_sol_reserves = self.virtual_sol_reserves.checked_add(sol_amount)?;
        self.real_sol_reserves = self.real_sol_reserves.checked_add(sol_amount)?;

        Some(BuyResult {
            token_amount: final_token_amount as u64,
            sol_amount: sol_amount as u64,
        })
    }

    pub fn apply_sell(&mut self, token_amount: u128) -> Option<SellResult> {
        let sol_amount = self.get_sell_price(token_amount)?;
      
        self.virtual_token_reserves = self.virtual_token_reserves.checked_add(token_amount)?;
        self.real_token_reserves = self.real_token_reserves.checked_add(token_amount)?;

        self.virtual_sol_reserves = self.virtual_sol_reserves.checked_sub(sol_amount)?;
        self.real_sol_reserves = self.real_sol_reserves.checked_sub(sol_amount)?;

        Some(SellResult {
            token_amount: token_amount as u64,
            sol_amount: sol_amount as u64,
        })
    }

    pub fn get_sell_price(&self, tokens: u128) -> Option<u128> {
        if tokens == 0 || tokens > self.virtual_token_reserves {
            return None;
        }

        let scaling_factor = self.initial_virtual_token_reserves;

        let scaled_tokens = tokens.checked_mul(scaling_factor)?;
        let token_sell_proportion = scaled_tokens.checked_div(self.virtual_token_reserves)?;
        let sol_received = (self.virtual_sol_reserves.checked_mul(token_sell_proportion)?).checked_div(scaling_factor)?;

        Some(sol_received.min(self.real_sol_reserves))
    }
}

impl fmt::Display for AMM {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "AMM {{ virtual_sol_reserves: {}, virtual_token_reserves: {}, real_sol_reserves: {}, real_token_reserves: {}, initial_virtual_token_reserves: {} }}",
            self.virtual_sol_reserves, self.virtual_token_reserves, self.real_sol_reserves, self.real_token_reserves, self.initial_virtual_token_reserves
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buy_and_sell_too_much() {
        let mut amm = AMM::new(
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFUALT_VIRTUAL_TOKEN_RESERVE,
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFAULT_TOKEN_RESERVES,
            DEFUALT_INITIAL_VIRTUAL_TOKEN_RESERVE
        );

        // Attempt to buy more tokens than available in reserves
        let buy_result = amm.apply_buy(DEFAULT_TOKEN_RESERVES * 2);
        assert!(buy_result.is_some());
        let buy_result = buy_result.unwrap();
        assert!(buy_result.token_amount <= DEFAULT_TOKEN_RESERVES as u64);
        assert!(amm.real_token_reserves <= DEFAULT_TOKEN_RESERVES);
        assert!(amm.virtual_token_reserves <= DEFUALT_VIRTUAL_TOKEN_RESERVE);
        assert!(amm.real_sol_reserves >= DEFAULT_VIRTUAL_SOL_RESERVE);
        assert!(amm.virtual_sol_reserves >= DEFAULT_VIRTUAL_SOL_RESERVE);

        // Attempt to sell more tokens than available in reserves
        let sell_result = amm.apply_sell(DEFAULT_TOKEN_RESERVES * 2);
        assert!(sell_result.is_some());
    }

    #[test]
    fn test_apply_sell() {
        let mut amm = AMM::new(
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFUALT_VIRTUAL_TOKEN_RESERVE,
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFAULT_TOKEN_RESERVES,
            DEFUALT_INITIAL_VIRTUAL_TOKEN_RESERVE
        );

        let sell_amount = 1_000_000_000_000; // 1 trillion tokens
        let result = amm.apply_sell(sell_amount);

        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.token_amount, sell_amount as u64);
        assert!(result.sol_amount > 0);
        assert!(amm.virtual_token_reserves > DEFUALT_VIRTUAL_TOKEN_RESERVE);
        assert!(amm.real_token_reserves > DEFAULT_TOKEN_RESERVES);
        assert!(amm.virtual_sol_reserves < DEFAULT_VIRTUAL_SOL_RESERVE);
        assert!(amm.real_sol_reserves < DEFAULT_VIRTUAL_SOL_RESERVE);
    }

    #[test]
    fn test_get_sell_price() {
        let amm = AMM::new(
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFUALT_VIRTUAL_TOKEN_RESERVE,
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFAULT_TOKEN_RESERVES,
            DEFUALT_INITIAL_VIRTUAL_TOKEN_RESERVE
        );

        assert_eq!(amm.get_sell_price(0), None);

        let sell_amount = DEFAULT_TOKEN_RESERVES / 100;
        let price = amm.get_sell_price(sell_amount);
        assert!(price.is_some());
        assert!(price.unwrap() > 0);

        assert!(amm.get_sell_price(DEFAULT_TOKEN_RESERVES * 2).is_some());
    }

    #[test]
    fn test_apply_buy() {
        let mut amm = AMM::new(
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFUALT_VIRTUAL_TOKEN_RESERVE,
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFAULT_TOKEN_RESERVES,
            DEFUALT_INITIAL_VIRTUAL_TOKEN_RESERVE
        );

        let purchase_amount = 1_000_000_000_000; // 1 trillion tokens

        let result = amm.apply_buy(purchase_amount);

        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.token_amount > 0);
        assert!(result.sol_amount > 0);
        assert!(amm.virtual_token_reserves < DEFUALT_VIRTUAL_TOKEN_RESERVE);
        assert!(amm.real_token_reserves < DEFAULT_TOKEN_RESERVES);
        assert!(amm.virtual_sol_reserves > DEFAULT_VIRTUAL_SOL_RESERVE);
        assert!(amm.real_sol_reserves > DEFAULT_VIRTUAL_SOL_RESERVE);
    }

    #[test]
    fn test_get_buy_price() {
        let amm = AMM::new(
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFUALT_VIRTUAL_TOKEN_RESERVE,
            DEFAULT_VIRTUAL_SOL_RESERVE,
            DEFAULT_TOKEN_RESERVES,
            DEFUALT_INITIAL_VIRTUAL_TOKEN_RESERVE
        );

        assert_eq!(amm.get_buy_price(0), None);

        let buy_amount = DEFAULT_TOKEN_RESERVES / 100;
        let price = amm.get_buy_price(buy_amount);
        assert!(price.is_some());
        assert!(price.unwrap() > 0);

        assert!(amm.get_buy_price(DEFAULT_TOKEN_RESERVES * 2).is_some());
    }
}