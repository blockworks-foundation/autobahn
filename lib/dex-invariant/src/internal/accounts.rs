use anchor_lang::prelude::*;
use anyhow::Error;
use invariant_types::{SEED, STATE_SEED};
use router_lib::dex::AccountProviderView;
use solana_sdk::account::ReadableAccount;

use super::swap::InvariantSwapResult;
use crate::{invariant_edge::InvariantEdge, InvariantDex};

#[derive(Clone)]
pub struct InvariantSwapParams<'a> {
    pub invariant_swap_result: &'a InvariantSwapResult,
    pub owner: Pubkey,
    pub source_mint: Pubkey,
    pub destination_mint: Pubkey,
    pub source_account: Pubkey,
    pub destination_account: Pubkey,
    pub referral_fee: Option<Pubkey>,
}

#[derive(Clone, Default, Debug)]
pub struct InvariantSwapAccounts {
    state: Pubkey,
    pool: Pubkey,
    tickmap: Pubkey,
    token_x: Pubkey,
    token_y: Pubkey,
    account_x: Pubkey,
    account_y: Pubkey,
    reserve_x: Pubkey,
    reserve_y: Pubkey,
    owner: Pubkey,
    program_authority: Pubkey,
    token_x_program: Pubkey,
    token_y_program: Pubkey,
    ticks_accounts: Vec<Pubkey>,
    referral_fee: Option<Pubkey>,
}

impl InvariantSwapAccounts {
    pub fn from_pubkeys(
        chain_data: &AccountProviderView,
        invariant_edge: &InvariantEdge,
        pool_pk: Pubkey,
        invariant_swap_params: &InvariantSwapParams,
    ) -> anyhow::Result<(Self, bool), Error> {
        let InvariantSwapParams {
            invariant_swap_result,
            owner,
            source_mint,
            destination_mint,
            source_account,
            destination_account,
            referral_fee,
        } = invariant_swap_params;

        let (x_to_y, account_x, account_y) = match (
            invariant_edge.pool.token_x.eq(source_mint),
            invariant_edge.pool.token_y.eq(destination_mint),
            invariant_edge.pool.token_x.eq(destination_mint),
            invariant_edge.pool.token_y.eq(source_mint),
        ) {
            (true, true, _, _) => (true, *source_account, *destination_account),
            (_, _, true, true) => (false, *destination_account, *source_account),
            _ => return Err(anyhow::Error::msg("Invalid source or destination mint")),
        };

        let ticks_accounts =
            InvariantDex::tick_indexes_to_addresses(pool_pk, &invariant_swap_result.used_ticks);

        let token_x_program = *chain_data
            .account(&invariant_edge.pool.token_x)?
            .account
            .owner();
        let token_y_program = *chain_data
            .account(&invariant_edge.pool.token_y)?
            .account
            .owner();

        let invariant_swap_accounts = Self {
            state: Self::get_state_address(crate::ID),
            pool: pool_pk,
            tickmap: invariant_edge.pool.tickmap,
            token_x: invariant_edge.pool.token_x,
            token_y: invariant_edge.pool.token_y,
            account_x,
            account_y,
            reserve_x: invariant_edge.pool.token_x_reserve,
            reserve_y: invariant_edge.pool.token_y_reserve,
            owner: *owner,
            program_authority: Self::get_program_authority(crate::ID),
            token_x_program,
            token_y_program,
            ticks_accounts,
            referral_fee: *referral_fee,
        };

        Ok((invariant_swap_accounts, x_to_y))
    }

    pub fn to_account_metas(&self) -> Vec<AccountMeta> {
        let mut account_metas: Vec<AccountMeta> = vec![
            AccountMeta::new_readonly(self.state, false),
            AccountMeta::new(self.pool, false),
            AccountMeta::new(self.tickmap, false),
            AccountMeta::new(self.token_x, false),
            AccountMeta::new(self.token_y, false),
            AccountMeta::new(self.account_x, false),
            AccountMeta::new(self.account_y, false),
            AccountMeta::new(self.reserve_x, false),
            AccountMeta::new(self.reserve_y, false),
            AccountMeta::new(self.owner, true),
            AccountMeta::new_readonly(self.program_authority, false),
            AccountMeta::new_readonly(self.token_x_program, false),
            AccountMeta::new_readonly(self.token_y_program, false),
        ];

        let ticks_metas: Vec<AccountMeta> = self
            .ticks_accounts
            .iter()
            .map(|tick_address| AccountMeta::new(*tick_address, false))
            .collect();

        account_metas.extend(ticks_metas);

        account_metas
    }

    fn get_program_authority(program_id: Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[SEED.as_bytes()], &program_id).0
    }

    fn get_state_address(program_id: Pubkey) -> Pubkey {
        Pubkey::find_program_address(&[STATE_SEED.as_bytes()], &program_id).0
    }
}
