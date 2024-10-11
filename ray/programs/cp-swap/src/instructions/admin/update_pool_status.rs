use crate::states::*;
use anchor_lang::prelude::*;
use crate::error::ErrorCode;

#[derive(Accounts)]
pub struct UpdatePoolStatus<'info> {
    #[account(
        address = pool_state.load()?.pool_creator @ ErrorCode::InvalidOwner
    )]
    pub authority: Signer<'info>,

    #[account(mut)]
    pub pool_state: AccountLoader<'info, PoolState>,
}

pub fn update_pool_status(ctx: Context<UpdatePoolStatus>, status: u8) -> Result<()> {
    require_gte!(255, status);
    let mut pool_state = ctx.accounts.pool_state.load_mut()?;
    pool_state.set_status(status);
    pool_state.recent_epoch = Clock::get()?.epoch;
    Ok(())
}
