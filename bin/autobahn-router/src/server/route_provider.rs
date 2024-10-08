use solana_program::pubkey::Pubkey;

use router_lib::model::quote_response::QuoteResponse;

use crate::debug_tools;
use crate::hot_mints::HotMintsCache;
use crate::prelude::*;
use crate::routing::Routing;
use crate::routing_types::{Route, RouteStep};
use crate::token_cache::TokenCache;
use router_config_lib::SafetyCheckConfig;
use router_lib::dex::{AccountProviderView, SwapMode};
use router_lib::price_feeds::price_cache::PriceCache;

pub trait RouteProvider {
    fn prepare_pruned_edges_and_cleanup_cache(
        &self,
        hot_mints: &HashSet<Pubkey>,
        swap_mode: SwapMode,
    );

    fn prepare_cache_for_input_mint<F>(
        &self,
        from_mint: Pubkey,
        amount_native: u64,
        max_accounts: usize,
        filter: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(&Pubkey, &Pubkey) -> bool;

    fn best_quote(
        &self,
        from_mint: Pubkey,
        to_mint: Pubkey,
        amount_native: u64,
        max_accounts: usize,
        swap_mode: SwapMode,
    ) -> anyhow::Result<Route>;

    fn try_from(&self, quote_response: &QuoteResponse) -> anyhow::Result<Route>;
}

pub struct RoutingRouteProvider {
    pub chain_data: AccountProviderView,
    pub routing: Arc<Routing>,
    pub prices: PriceCache,
    pub tokens: TokenCache,
    pub config: SafetyCheckConfig,
    pub hot_mints: Arc<RwLock<HotMintsCache>>,
}

impl RouteProvider for RoutingRouteProvider {
    fn prepare_pruned_edges_and_cleanup_cache(
        &self,
        hot_mints: &HashSet<Pubkey>,
        swap_mode: SwapMode,
    ) {
        self.routing
            .prepare_pruned_edges_and_cleanup_cache(hot_mints, swap_mode)
    }

    fn prepare_cache_for_input_mint<F>(
        &self,
        from_mint: Pubkey,
        amount_native: u64,
        max_accounts: usize,
        filter: F,
    ) -> anyhow::Result<()>
    where
        F: Fn(&Pubkey, &Pubkey) -> bool,
    {
        self.routing
            .prepare_cache_for_input_mint(&from_mint, amount_native, max_accounts, filter)
    }

    // called per request
    #[tracing::instrument(skip_all, level = "trace")]
    fn best_quote(
        &self,
        from_mint: Pubkey,
        to_mint: Pubkey,
        amount_native: u64,
        max_accounts: usize,
        swap_mode: SwapMode,
    ) -> anyhow::Result<Route> {
        let hot_mints = {
            let mut hot_mints_guard = self.hot_mints.write().unwrap();
            hot_mints_guard.add(from_mint);
            hot_mints_guard.add(to_mint);
            hot_mints_guard.get()
        };

        // ensure new hot mints are ready (edge cached_price should be available)
        self.routing
            .lazy_compute_prices(&self.chain_data, &self.tokens, &self.prices, &from_mint, &to_mint);

        let route = self.routing.find_best_route(
            &self.chain_data,
            &from_mint,
            &to_mint,
            amount_native,
            max_accounts,
            false,
            &hot_mints,
            None,
            swap_mode,
        )?;

        if !self.config.check_quote_out_amount_deviation {
            return Ok(route);
        }

        let in_token = self.tokens.token(from_mint)?;
        let out_token = self.tokens.token(to_mint)?;
        let in_multiplier = 10u64.pow(in_token.decimals as u32) as f64;
        let out_multiplier = 10u64.pow(out_token.decimals as u32) as f64;

        let in_price_ui = self.prices.price_ui(from_mint);
        let out_price_ui = self.prices.price_ui(to_mint);

        if in_price_ui.is_none() || out_price_ui.is_none() {
            error!("Refusing to quote - missing $ price, can't add safety check");
            anyhow::bail!("Refusing to quote - missing $ price, can't add safety check");
        }

        let out_amount_native = route.out_amount;

        let in_amount_usd = in_price_ui.unwrap_or(0.0) * amount_native as f64 / in_multiplier;
        let out_amount_usd =
            out_price_ui.unwrap_or(0.0) * out_amount_native as f64 / out_multiplier;

        if out_amount_usd < self.config.min_quote_out_to_in_amount_ratio * in_amount_usd {
            error!(
                from = debug_tools::name(&from_mint),
                to = debug_tools::name(&to_mint),
                in_amount_usd,
                out_amount_usd,
                amount_native,
                out_amount_native,
                in_price_ui,
                out_price_ui,
                route = route.steps.iter().map(|x| x.edge.desc()).join(" -> "),
                "Very bad route - refusing it",
            );
            anyhow::bail!(
                "Very bad route - refusing it: in_amount={}$, out_amount={}$ ({} of {} => {} of {})\r\n{}\r\nin_price={:?}, out_price={:?}",
                in_amount_usd,
                out_amount_usd,
                amount_native,
                debug_tools::name(&from_mint),
                out_amount_native,
                debug_tools::name(&to_mint),
                route.steps.iter().map(|x| x.edge.desc()).join(" -> "),
                in_price_ui,
                out_price_ui,
            );
        }

        info!(
            from = debug_tools::name(&from_mint),
            to = debug_tools::name(&to_mint),
            in_amount_usd,
            out_amount_usd,
            "Good route",
        );

        Ok(route)
    }

    fn try_from(&self, quote_response: &QuoteResponse) -> anyhow::Result<Route> {
        let input_mint = Pubkey::from_str(&quote_response.input_mint)?;
        let output_mint = Pubkey::from_str(&quote_response.output_mint)?;
        let in_amount = quote_response.in_amount.clone().unwrap().parse()?; // TODO Remove opt ? Handle exact out ?
        let out_amount = quote_response.out_amount.parse()?;
        let price_impact_pct: f64 = quote_response.price_impact_pct.parse()?;
        let price_impact_bps = (price_impact_pct * 100.0).round() as u64;
        let slot = quote_response.context_slot;

        let steps: anyhow::Result<Vec<_>> = quote_response
            .route_plan
            .clone()
            .into_iter()
            .map(|x| -> anyhow::Result<RouteStep> {
                let step = x.swap_info.unwrap(); // TODO
                Ok(RouteStep {
                    edge: self.routing.find_edge(
                        step.input_mint.parse()?,
                        step.output_mint.parse()?,
                        step.amm_key.parse()?,
                    )?,
                    in_amount: step.in_amount.parse()?,
                    out_amount: step.out_amount.parse()?,
                    fee_amount: step.fee_amount.parse()?,
                    fee_mint: step.fee_mint.parse()?,
                })
            })
            .collect();

        Ok(Route {
            input_mint,
            output_mint,
            in_amount,
            out_amount,
            price_impact_bps,
            slot,
            steps: steps?,
            accounts: None, // TODO FAS
        })
    }
}
