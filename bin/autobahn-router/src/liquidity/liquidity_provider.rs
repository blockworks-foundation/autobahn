use crate::token_cache::TokenCache;
use ordered_float::{FloatCore, Pow};
use router_lib::price_feeds::price_cache::PriceCache;
use solana_program::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub struct Liquidity {
    pub liquidity_by_pool: HashMap<Pubkey, u64>,
}

pub struct LiquidityProvider {
    liquidity_by_mint: HashMap<Pubkey, Liquidity>,
    token_cache: TokenCache,
    price_cache: PriceCache,
}

pub type LiquidityProviderArcRw = Arc<RwLock<LiquidityProvider>>;

impl LiquidityProvider {
    pub fn new(token_cache: TokenCache, price_cache: PriceCache) -> LiquidityProvider {
        LiquidityProvider {
            liquidity_by_mint: Default::default(),
            token_cache,
            price_cache,
        }
    }

    pub fn set_liquidity(&mut self, mint: Pubkey, pool: Pubkey, liquidity: u64) {
        if let Some(cache) = self.liquidity_by_mint.get_mut(&mint) {
            cache.liquidity_by_pool.insert(pool, liquidity);
        } else {
            self.liquidity_by_mint.insert(
                mint,
                Liquidity {
                    liquidity_by_pool: HashMap::from([(pool, liquidity)]),
                },
            );
        }
    }

    pub fn get_total_liquidity_native(&self, mint: Pubkey) -> u64 {
        if let Some(cache) = self.liquidity_by_mint.get(&mint) {
            let mut sum = 0u64;
            for amount in cache.liquidity_by_pool.iter().map(|x| x.1) {
                sum = sum.saturating_add(*amount);
            }
            sum
        } else {
            0
        }
    }

    pub fn get_total_liquidity_in_dollars(&self, mint: Pubkey) -> anyhow::Result<f64> {
        let liquidity_native = self.get_total_liquidity_native(mint);
        let price = self
            .price_cache
            .price_ui(mint)
            .ok_or(anyhow::format_err!("no price"))?;
        let decimal = self.token_cache.token(mint).map(|x| x.decimals)?;

        let liquidity_dollars = (liquidity_native as f64 / 10.0.pow(decimal as f64)) * price;

        Ok(liquidity_dollars)
    }
}
