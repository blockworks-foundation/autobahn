use crate::edge::{GobblerEdgeIdentifier, Operation, Direction};
use anchor_lang::{AccountDeserialize, InstructionData, ToAccountMetas};
use anchor_spl::associated_token::get_associated_token_address;
use raydium_cp_swap::AUTH_SEED;
use raydium_cp_swap::instruction::{Deposit, Withdraw, SwapBaseInput};
use raydium_cp_swap::program::RaydiumCpSwap;
use raydium_cp_swap::states::PoolState;
use router_lib::dex::{AccountProviderView, SwapInstruction};
use solana_program::instruction::Instruction;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::ReadableAccount;
use spl_memo::id as spl_memo_id;
use spl_token::id as spl_token_id;
use spl_token_2022::id as spl_token_2022_id;

pub fn build_swap_ix(
    id: &GobblerEdgeIdentifier,
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

    match id.operation {
        Operation::Swap => {
            // Handle swap operation
            let (input_token_mint, output_token_mint) = match id.direction {
                Direction::AtoB => (pool.token_0_mint, pool.token_1_mint),
                Direction::BtoA => (pool.token_1_mint, pool.token_0_mint),
                _ => return Err(anyhow::anyhow!("Invalid direction for swap operation")),
            };

            let (input_token_program, output_token_program) = match id.direction {
                Direction::AtoB => (pool.token_0_program, pool.token_1_program),
                Direction::BtoA => (pool.token_1_program, pool.token_0_program),
                _ => return Err(anyhow::anyhow!("Invalid direction for swap operation")),
            };

            let (input_vault, output_vault) = match id.direction {
                Direction::AtoB => (pool.token_0_vault, pool.token_1_vault),
                Direction::BtoA => (pool.token_1_vault, pool.token_0_vault),
                _ => return Err(anyhow::anyhow!("Invalid direction for swap operation")),
            };

            let (input_token_account, output_token_account) = (
                get_associated_token_address(wallet_pk, &input_token_mint),
                get_associated_token_address(wallet_pk, &output_token_mint),
            );

            let instruction = SwapBaseInput {
                amount_in: amount,
                minimum_amount_out: other_amount_threshold,
            };
            let (authority, __bump) =
                Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &raydium_cp_swap::id());

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
                    program_id: raydium_cp_swap::id(),
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
        Operation::Deposit => build_deposit_ix(
            id,
            chain_data,
            wallet_pk,
            in_amount,
            max_slippage_bps,
        ),
        Operation::Withdraw => build_withdraw_ix(
            id,
            chain_data,
            wallet_pk,
            in_amount,
            max_slippage_bps,
        ),
    }
}

pub fn build_deposit_ix(
    id: &GobblerEdgeIdentifier,
    chain_data: &AccountProviderView,
    wallet_pk: &Pubkey,
    lp_amount: u64,
    max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    let pool_account = chain_data.account(&id.pool)?;
    let pool = PoolState::try_deserialize(&mut pool_account.account.data())?;

    let (token_a_mint, token_b_mint) = (pool.token_0_mint, pool.token_1_mint);
    let (vault_a, vault_b) = (pool.token_0_vault, pool.token_1_vault);

    let user_token_a_account = get_associated_token_address(wallet_pk, &token_a_mint);
    let user_token_b_account = get_associated_token_address(wallet_pk, &token_b_mint);
    let user_lp_token_account = get_associated_token_address(wallet_pk, &pool.lp_mint);

    // You may need to calculate maximum amounts based on slippage
    let instruction = Deposit {
        lp_token_amount: lp_amount,
        maximum_token_0_amount: u64::MAX, // Adjust as needed
        maximum_token_1_amount: u64::MAX, // Adjust as needed
    };

    let (authority, __bump) =
        Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &raydium_cp_swap::id());

    let accounts = raydium_cp_swap::accounts::Deposit {
        owner: *wallet_pk,
        authority,
        pool_state: id.pool,
        owner_lp_token: user_lp_token_account,
        token_0_account: user_token_a_account,
        token_1_account: user_token_b_account,
        token_0_vault: vault_a,
        token_1_vault: vault_b,
        token_program: spl_token_id(),
        token_program_2022: spl_token_2022_id(),
        vault_0_mint: token_a_mint,
        vault_1_mint: token_b_mint,
        lp_mint: pool.lp_mint,
    };

    let deposit_instruction = Instruction {
        program_id: raydium_cp_swap::id(),
        accounts: accounts.to_account_metas(None),
        data: instruction.data(),
    };

    let result = SwapInstruction {
        instruction: deposit_instruction,
        out_pubkey: user_lp_token_account,
        out_mint: pool.lp_mint,
        in_amount_offset: 8,
        cu_estimate: Some(40_000),
    };

    Ok(result)
}

pub fn build_withdraw_ix(
    id: &GobblerEdgeIdentifier,
    chain_data: &AccountProviderView,
    wallet_pk: &Pubkey,
    lp_amount: u64,
    max_slippage_bps: i32,
) -> anyhow::Result<SwapInstruction> {
    let pool_account = chain_data.account(&id.pool)?;
    let pool = PoolState::try_deserialize(&mut pool_account.account.data())?;

    let (token_a_mint, token_b_mint) = (pool.token_0_mint, pool.token_1_mint);
    let (vault_a, vault_b) = (pool.token_0_vault, pool.token_1_vault);

    let user_token_a_account = get_associated_token_address(wallet_pk, &token_a_mint);
    let user_token_b_account = get_associated_token_address(wallet_pk, &token_b_mint);
    let user_lp_token_account = get_associated_token_address(wallet_pk, &pool.lp_mint);

    // You may need to calculate minimum amounts based on slippage
    let instruction = Withdraw {
        lp_token_amount: lp_amount,
        minimum_token_0_amount: 0, // Adjust as needed
        minimum_token_1_amount: 0, // Adjust as needed
    };

    let (authority, __bump) =
        Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &raydium_cp_swap::id());

    let accounts = raydium_cp_swap::accounts::Withdraw {
        owner: *wallet_pk,
        authority,
        pool_state: id.pool,
        owner_lp_token: user_lp_token_account,
        token_0_account: user_token_a_account,
        token_1_account: user_token_b_account,
        token_0_vault: vault_a,
        token_1_vault: vault_b,
        token_program: spl_token_id(),
        token_program_2022: spl_token_2022_id(),
        vault_0_mint: token_a_mint,
        vault_1_mint: token_b_mint,
        memo_program: spl_memo_id(),
        lp_mint: pool.lp_mint,
    };

    let withdraw_instruction = Instruction {
        program_id: raydium_cp_swap::id(),
        accounts: accounts.to_account_metas(None),
        data: instruction.data(),
    };

    let result = SwapInstruction {
        instruction: withdraw_instruction,
        out_pubkey: user_token_a_account, // Adjust as needed
        out_mint: token_a_mint,
        in_amount_offset: 8,
        cu_estimate: Some(40_000),
    };

    Ok(result)
}
