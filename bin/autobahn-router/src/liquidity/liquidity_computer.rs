use crate::edge::Edge;
use router_lib::dex::AccountProviderView;
use std::sync::Arc;

pub fn compute_liquidity(
    edge: &Arc<Edge>,
    chain_data: &AccountProviderView,
) -> anyhow::Result<u64> {
    let loaded = edge.prepare(&chain_data)?;

    let first_in_amount = edge
        .state
        .read()
        .unwrap()
        .cached_prices
        .first()
        .map(|x| x.0);
    let Some(first_in_amount) = first_in_amount else {
        anyhow::bail!("Too early to compute liquidity");
    };

    let mut iter_counter = 0;
    let mut has_failed = false;
    let mut last_successful_in_amount = first_in_amount;
    let mut next_in_amount = first_in_amount;
    let mut last_successful_out_amount = 0;
    let acceptable_price_impact = 0.3;

    loop {
        if next_in_amount == 0 || iter_counter > 50 {
            break;
        }
        iter_counter = iter_counter + 1;

        let quote = edge.quote(&loaded, &chain_data, next_in_amount);
        let expected_output = (2.0 - acceptable_price_impact) * last_successful_out_amount as f64;

        let out_amount = quote.map(|x| x.out_amount).unwrap_or(0);

        if (out_amount as f64) < expected_output {
            if has_failed {
                break;
            }
            has_failed = true;
            next_in_amount = next_in_amount
                .saturating_add(last_successful_in_amount)
                .saturating_div(2);
            continue;
        };

        last_successful_in_amount = next_in_amount;
        last_successful_out_amount = out_amount;
        next_in_amount = next_in_amount.saturating_mul(2);
    }

    Ok(last_successful_out_amount)
}
