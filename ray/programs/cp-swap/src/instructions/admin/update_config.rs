use crate::error::ErrorCode;
use crate::states::*;
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct UpdateAmmConfig<'info> {
    /// The amm config owner or admin
    #[account(address = crate::admin::id() @ ErrorCode::InvalidOwner)]
    pub owner: Signer<'info>,

    /// Amm config account to be changed
    #[account(mut)]
    pub amm_config: Account<'info, AmmConfig>,
}

pub fn update_amm_config(ctx: Context<UpdateAmmConfig>, param: u8, value: u64) -> Result<()> {
    let amm_config = &mut ctx.accounts.amm_config;
    let match_param = Some(param);
    match match_param {
        Some(0) => {
            let new_fund_owner = *ctx.remaining_accounts.iter().next().unwrap().key;
            set_new_fund_owner(amm_config, new_fund_owner)?;
        }
        Some(1) => amm_config.token_1_lp_rate = value,
        Some(2) => amm_config.token_0_lp_rate = value,
        Some(3) => amm_config.token_0_creator_rate = value,
        Some(4) => amm_config.token_1_creator_rate = value,
        Some(5) => amm_config.disable_create_pool = if value == 0 { false } else { true },
        _ => return err!(ErrorCode::InvalidInput),
    }
    Ok(())
}
fn set_new_fund_owner(amm_config: &mut Account<AmmConfig>, new_fund_owner: Pubkey) -> Result<()> {
    require_keys_neq!(new_fund_owner, Pubkey::default());
    #[cfg(feature = "enable-log")]
    msg!(
        "amm_config, old_fund_owner:{}, new_fund_owner:{}",
        amm_config.fund_owner.to_string(),
        new_fund_owner.key().to_string()
    );
    amm_config.fund_owner = new_fund_owner;
    Ok(())
}
