use crate::edge::RaydiumCpEdgeIdentifier;
use anchor_lang::{AccountDeserialize, Id, InstructionData, ToAccountMetas};
use anchor_spl::associated_token::get_associated_token_address;
use raydium_cp_swap::program::RaydiumCpSwap;
use raydium_cp_swap::states::PoolState;
use raydium_cp_swap::AUTH_SEED;
use router_lib::dex::{AccountProviderView, SwapInstruction};
use solana_program::instruction::Instruction;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;

pub fn build_swap_ix(
    id: &RaydiumCpEdgeIdentifier,
    chain_data: &AccountProviderView,
    wallet_pk: &Pubkey,
    in_amount: u64,
    out_amount: u64,
    max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    let pool_account = chain_data.account(&id.pool)?;
    let pool = PoolState::try_deserialize(&mut pool_account.account.data())?;

    let amount = in_amount;
    let other_amount_threshold =
        ((out_amount as f64 * (10_000f64 - max_slippage_bps as f64)) / 10_000f64).floor() as u64;

    let (input_token_mint, output_token_mint) = if id.is_a_to_b {
        (pool.token_0_mint, pool.token_1_mint)
    } else {
        (pool.token_1_mint, pool.token_0_mint)
    };
    let (input_token_program, output_token_program) = if id.is_a_to_b {
        (pool.token_0_program, pool.token_1_program)
    } else {
        (pool.token_1_program, pool.token_0_program)
    };
    let (input_vault, output_vault) = if id.is_a_to_b {
        (pool.token_0_vault, pool.token_1_vault)
    } else {
        (pool.token_1_vault, pool.token_0_vault)
    };

    let (input_token_account, output_token_account) = (
        get_associated_token_address(wallet_pk, &input_token_mint),
        get_associated_token_address(wallet_pk, &output_token_mint),
    );

    let instruction = raydium_cp_swap::instruction::SwapBaseInput {
        amount_in: amount,
        minimum_amount_out: other_amount_threshold,
    };
    let (authority, __bump) =
        Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &RaydiumCpSwap::id());

    let accounts = raydium_cp_swap::accounts::Swap {
        payer: *wallet_pk,
        authority,
        amm_config: pool.amm_config,
        pool_state: id.pool,
        input_token_account,
        output_token_account,
        input_vault,
        output_vault,
        input_token_program,
        output_token_program,
        input_token_mint,
        output_token_mint,
        observation_state: pool.observation_key,
    };

    let result = SwapInstruction {
        instruction: Instruction {
            program_id: RaydiumCpSwap::id(),
            accounts: accounts.to_account_metas(None),
            data: instruction.data(),
        },
        out_pubkey: output_token_account,
        out_mint: output_token_mint,
        in_amount_offset: 8,
        cu_estimate: Some(40_000),
    };

    Ok(result)
}
