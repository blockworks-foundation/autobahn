use crate::edge::GammaEdgeIdentifier;
use anchor_lang::{AccountDeserialize, Id, InstructionData, ToAccountMetas};
use anchor_spl::associated_token::get_associated_token_address;
use gamma::program::Gamma;
use gamma::PoolState;
use gamma::AUTH_SEED;
use router_lib::dex::{AccountProviderView, SwapInstruction};
use solana_program::instruction::Instruction;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;

pub fn build_swap_ix(
    id: &GammaEdgeIdentifier,
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
        (pool.token0_mint, pool.token1_mint)
    } else {
        (pool.token1_mint, pool.token0_mint)
    };
    let (input_token_program, output_token_program) = if id.is_a_to_b {
        (pool.token0_program, pool.token1_program)
    } else {
        (pool.token1_program, pool.token0_program)
    };
    let (input_vault, output_vault) = if id.is_a_to_b {
        (pool.token0_vault, pool.token1_vault)
    } else {
        (pool.token1_vault, pool.token0_vault)
    };

    let (input_token_account, output_token_account) = (
        get_associated_token_address(wallet_pk, &input_token_mint),
        get_associated_token_address(wallet_pk, &output_token_mint),
    );

    let instruction = gamma::instruction::SwapBaseInput {
        _amount_in: amount,
        _minimum_amount_out: other_amount_threshold,
    };
    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &Gamma::id());

    let accounts = gamma::accounts::SwapBaseInput {
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
            program_id: Gamma::id(),
            accounts: accounts.to_account_metas(None),
            data: instruction.data(),
        },
        out_pubkey: output_token_account,
        out_mint: output_token_mint,
        in_amount_offset: 8,
        cu_estimate: Some(80_000),
    };

    Ok(result)
}
