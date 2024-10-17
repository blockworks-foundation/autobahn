use crate::debug_tools;
use crate::prelude::*;
use crate::token_cache::TokenCache;
use ordered_float::Pow;
use router_lib::dex::{
    AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, Quote, SwapInstruction,
};
use router_lib::price_feeds::price_cache::PriceCache;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::min;
use std::fmt::Formatter;
use std::time::Duration;

#[derive(Clone, Debug, Default, serde_derive::Serialize, serde_derive::Deserialize)]
pub struct EdgeState {
    /// List of (input, price, ln-price) pairs, sorted by input asc
    // TODO: it may be much better to store this centrally, so it's cheap to take a snapshot
    pub cached_prices: Vec<(u64, f64, f64)>,
    is_valid: bool,
    pub last_update: u64,
    pub last_update_slot: u64,

    /// How many time did we cool down this edge ?
    pub cooldown_event: u64,
    /// When will the edge become available again ?
    pub cooldown_until: Option<u64>,
}

pub struct Edge {
    pub input_mint: Pubkey,
    pub output_mint: Pubkey,
    pub dex: Arc<dyn DexInterface>,
    pub id: Arc<dyn DexEdgeIdentifier>,

    /// Number of accounts required to traverse this edge, not including
    /// the source token account, signer, token program, ata program, system program
    // TODO: This should maybe just be a Vec<Pubkey>, so multiple same-type edges need fewer?
    // and to help with selecting address lookup tables? but then it depends on what tick-arrays
    // are needed (so on the particular quote() result)
    pub accounts_needed: usize,

    pub state: RwLock<EdgeState>,
    // TODO: address lookup table, deboosted
}

impl std::fmt::Debug for Edge {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} => {} ({})",
            debug_tools::name(&self.input_mint),
            debug_tools::name(&self.output_mint),
            self.dex.name()
        )
    }
}

impl Serialize for Edge {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        todo!()
    }
}

impl<'de> Deserialize<'de> for Edge {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        todo!()
    }
}

impl Edge {
    pub fn key(&self) -> Pubkey {
        self.id.key()
    }

    pub fn unique_id(&self) -> (Pubkey, Pubkey) {
        (self.id.key(), self.id.input_mint())
    }

    pub fn desc(&self) -> String {
        self.id.desc()
    }

    pub fn kind(&self) -> String {
        self.dex.name()
    }

    pub fn build_swap_ix(
        &self,
        chain_data: &AccountProviderView,
        wallet_pk: &Pubkey,
        amount_in: u64,
        out_amount: u64,
        max_slippage_bps: i32,
    ) -> anyhow::Result<SwapInstruction> {
        self.dex.build_swap_ix(
            &self.id,
            chain_data,
            wallet_pk,
            amount_in,
            out_amount,
            max_slippage_bps,
        )
    }
    pub fn prepare(&self, chain_data: &AccountProviderView) -> anyhow::Result<Arc<dyn DexEdge>> {
        let edge = self.dex.load(&self.id, chain_data)?;
        Ok(edge)
    }

    pub fn quote(
        &self,
        prepared_quote: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote> {
        self.dex
            .quote(&self.id, &prepared_quote, chain_data, in_amount)
    }

    pub fn supports_exact_out(&self) -> bool {
        self.dex.supports_exact_out(&self.id)
    }

    pub fn quote_exact_out(
        &self,
        prepared_quote: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        out_amount: u64,
    ) -> anyhow::Result<Quote> {
        self.dex
            .quote_exact_out(&self.id, &prepared_quote, chain_data, out_amount)
    }

    pub fn update_internal(
        &self,
        chain_data: &AccountProviderView,
        decimals: u8,
        price: f64,
        path_warming_amounts: &Vec<u64>,
    ) {
        let multiplier = 10u64.pow(decimals as u32) as f64;
        let amounts = path_warming_amounts
            .iter()
            .map(|amount| {
                let quantity_ui = *amount as f64 / price;
                let quantity_native = quantity_ui * multiplier;
                quantity_native.ceil() as u64
            })
            .collect_vec();

        debug!(input_mint = %self.input_mint, pool = %self.key(), multiplier = multiplier, price = price, amounts = amounts.iter().join(";"), "price_data");

        let overflow = amounts.iter().any(|x| *x == u64::MAX);
        if overflow {
            if self.state.read().unwrap().is_valid {
                debug!("amount error, disabling edge {}", self.desc());
            }

            let mut state = self.state.write().unwrap();
            state.last_update = millis_since_epoch();
            state.last_update_slot = chain_data.newest_processed_slot();
            state.cached_prices.clear();
            state.is_valid = false;
            return;
        }

        let prepared_quote = self.prepare(chain_data);

        // do calculation for in amounts
        let quote_results_in = amounts
            .iter()
            .map(|&amount| match &prepared_quote {
                Ok(p) => (amount, self.quote(&p, chain_data, amount)),
                Err(e) => (
                    amount,
                    anyhow::Result::<Quote>::Err(anyhow::format_err!("{}", e)),
                ),
            })
            .collect_vec();

        if let Some((_, err)) = quote_results_in.iter().find(|v| v.1.is_err()) {
            if self.state.read().unwrap().is_valid {
                warn!("quote error, disabling edge: {} {err:?}", self.desc());
            } else {
                debug!("quote error: {} {err:?}", self.desc());
            }
        }

        let mut state = self.state.write().unwrap();
        state.last_update = millis_since_epoch();
        state.last_update_slot = chain_data.newest_processed_slot();
        state.cached_prices.clear();
        state.is_valid = true;

        if let Some(timestamp) = state.cooldown_until {
            if timestamp < state.last_update {
                state.cooldown_until = None;
            }
        };

        let mut has_at_least_one_non_zero = false;
        for quote_result in quote_results_in {
            if let (in_amount, Ok(quote)) = quote_result {
                // quote.in_amount may be different from in_amount if edge refuse to swap enough
                // then we want to have "actual price" for expected in_amount and not for quote.in_amount
                let price = quote.out_amount as f64 / in_amount as f64;
                if price.is_nan() {
                    state.is_valid = false;
                    continue;
                }
                if price > 0.0000001 {
                    has_at_least_one_non_zero = true;
                }
                // TODO: output == 0?!
                state.cached_prices.push((in_amount, price, f64::ln(price)));
            } else {
                // TODO: should a single quote failure really invalidate the whole edge?
                state.is_valid = false;
            };
        }

        if !has_at_least_one_non_zero {
            state.is_valid = false;
        }
    }

    pub fn update(
        &self,
        chain_data: &AccountProviderView,
        token_cache: &TokenCache,
        price_cache: &PriceCache,
        path_warming_amounts: &Vec<u64>,
    ) {
        trace!(edge = self.desc(), "updating");

        let Ok(decimals) = token_cache.token(self.input_mint).map(|x| x.decimals) else {
            let mut state = self.state.write().unwrap();
            trace!("no decimals for {}", self.input_mint);
            state.is_valid = false;
            return;
        };
        let Some(price) = price_cache.price_ui(self.input_mint) else {
            let mut state = self.state.write().unwrap();
            state.is_valid = false;
            trace!("no price for {}", self.input_mint);
            return;
        };

        self.update_internal(chain_data, decimals, price, path_warming_amounts);
    }
}

impl EdgeState {
    /// Returns the price (in native/native) and ln(price) most applicable for the in amount
    /// Returns None if invalid
    pub fn cached_price_for(&self, in_amount: u64) -> Option<(f64, f64)> {
        if !self.is_valid() || self.cached_prices.is_empty() {
            return None;
        }

        let cached_price = self
            .cached_prices
            .iter()
            .find(|(cached_in_amount, _, _)| *cached_in_amount >= in_amount)
            .unwrap_or(&self.cached_prices.last().unwrap());
        Some((cached_price.1, cached_price.2))
    }

    pub fn cached_price_exact_out_for(&self, out_amount: u64) -> Option<(f64, f64)> {
        if !self.is_valid() {
            return None;
        }

        let out_amount_f = out_amount as f64;
        let cached_price = self
            .cached_prices
            .iter()
            .find(|(cached_in_amount, p, _)| (*cached_in_amount as f64) * p >= out_amount_f)
            .unwrap_or(&self.cached_prices.last().unwrap());

        // inverse price for exact out
        let price = 1.0 / cached_price.1;
        Some((price, f64::ln(price)))
    }

    pub fn is_valid(&self) -> bool {
        if !self.is_valid {
            return false;
        }

        if self.cooldown_until.is_some() {
            // Do not check time here !
            // We will reset "cooldown until" on first account update coming after cooldown
            // So if this is not reset yet, it means that we didn't change anything
            // No reason to be working again
            return false;
        }

        true
    }

    pub fn reset_cooldown(&mut self) {
        self.cooldown_event += 0;
        self.cooldown_until = None;
    }

    pub fn add_cooldown(&mut self, duration: &Duration) {
        self.cooldown_event += 1;

        let counter = min(self.cooldown_event, 5) as f64;
        let exp_factor = 1.2.pow(counter);
        let factor = (counter * exp_factor).round() as u64;
        let until = millis_since_epoch() + (duration.as_millis() as u64 * factor);

        self.cooldown_until = match self.cooldown_until {
            None => Some(until),
            Some(current) => Some(current.max(until)),
        };
    }
}
