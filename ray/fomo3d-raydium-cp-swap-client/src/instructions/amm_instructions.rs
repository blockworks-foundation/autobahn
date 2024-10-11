use anchor_client::{Client, Cluster};
use anchor_lang::Key;
use anyhow::Result;
use raydium_cp_swap::accounts::CreateAmmConfig;
use solana_sdk::compute_budget::ComputeBudgetInstruction;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::{instruction::Instruction, pubkey::Pubkey, system_program, sysvar};

use raydium_cp_swap::accounts as raydium_cp_accounts;
use raydium_cp_swap::instruction as raydium_cp_instructions;
use raydium_cp_swap::{
    states::{AMM_CONFIG_SEED, OBSERVATION_SEED, POOL_SEED, POOL_VAULT_SEED},
    AUTH_SEED,
};
use std::rc::Rc;

use super::super::{read_keypair_file, ClientConfig};

pub fn collect_protocol_fee_instr(
    config: &ClientConfig,
    pool_id: Pubkey,
    token_0_vault: Pubkey,
    token_1_vault: Pubkey,
    token_0_mint: Pubkey,
    token_1_mint: Pubkey,
    recipient_token_0_account: Pubkey,
    recipient_token_1_account: Pubkey,
    amount_0_requested: u64,
    amount_1_requested: u64,
    amm_config: Pubkey,
) -> Result<Vec<Instruction>> {
    let payer = read_keypair_file(&config.payer_path);
    let url = Cluster::Custom(config.http_url.clone(), config.ws_url.clone());
    // Client.
    let client = Client::new(url, Rc::new(payer.expect("Failed to get payer keypair")));
    let program = client.program(config.raydium_cp_program)?;

    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program.id());

    let instructions = program
        .request()
        .accounts(raydium_cp_accounts::CollectProtocolFee {
            authority,
            pool_state: pool_id,
            token_0_vault,
            amm_config,
            owner: program.payer(),
            token_1_vault,
            recipient_token_0_account,
            recipient_token_1_account,
            token_program: spl_token::id(),
            token_program_2022: spl_token_2022::id(),
            vault_0_mint: token_0_mint,
            vault_1_mint: token_1_mint,
        })
        .args(raydium_cp_instructions::CollectProtocolFee {
            amount_0_requested,
            amount_1_requested,
        })
        .instructions()?;
    Ok(instructions)
}
pub fn collect_fund_fee_instr(
    config: &ClientConfig,
    pool_id: Pubkey,
    amm_config: Pubkey,
    token_0_vault: Pubkey,
    token_1_vault: Pubkey,
    token_0_mint: Pubkey,
    token_1_mint: Pubkey,
    recipient_token_0_account: Pubkey,
    recipient_token_1_account: Pubkey,
    amount_0_requested: u64,
    amount_1_requested: u64,
) -> Result<Vec<Instruction>> {
    let payer = read_keypair_file(&config.payer_path);
    let url = Cluster::Custom(config.http_url.clone(), config.ws_url.clone());
    // Client.
    let client = Client::new(url, Rc::new(payer.expect("Failed to get payer keypair")));
    let program = client.program(config.raydium_cp_program)?;

    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program.id());

    let mut instructions = program
        .request()
        .accounts(raydium_cp_accounts::CollectFundFee {
            owner: program.payer(),
            authority,
            pool_state: pool_id,
            amm_config,
            token_0_vault,
            token_1_vault,
            vault_0_mint: token_0_mint,
            vault_1_mint: token_1_mint,
            recipient_token_0_account,
            recipient_token_1_account,
            token_program: spl_token::id(),
            token_program_2022: spl_token_2022::id(),
        })
        .args(raydium_cp_instructions::CollectFundFee {
            amount_0_requested,
            amount_1_requested,
        })
        .instructions()?;

    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);
    instructions.insert(0, compute_budget_ix);
    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_price(3333333);
    instructions.insert(0, compute_budget_ix);
    Ok(instructions)
}
pub fn initialize_amm_config_instr(
    config: &ClientConfig,
    amm_config_index: u64,
    token_0_creator_rate: u64,
    token_1_lp_rate: u64,
    token_0_lp_rate: u64,
    token_1_creator_rate: u64,
) -> Result<Vec<Instruction>> {
    let payer = read_keypair_file(&config.payer_path);
    let url = Cluster::Custom(config.http_url.clone(), config.ws_url.clone());
    let client = Client::new(url, Rc::new(payer.expect("Failed to get payer keypair")));
    let program = client.program(config.raydium_cp_program)?;

    let (amm_config_key, _bump) = Pubkey::find_program_address(
        &[AMM_CONFIG_SEED.as_bytes(), &amm_config_index.to_be_bytes()],
        &program.id(),
    );

    let accounts = CreateAmmConfig {
        amm_config: amm_config_key,
        owner: program.payer(),
        system_program: system_program::ID,
    };

    let ix = program
        .request()
        .accounts(accounts)
        .args(raydium_cp_instructions::CreateAmmConfig {
            index: amm_config_index,
            token_0_creator_rate,
            token_1_lp_rate,
            token_0_lp_rate,
            token_1_creator_rate,
        })
        .instructions()?;

    Ok(ix)
}
pub fn initialize_pool_instr(
    config: &ClientConfig,
    token_0_mint: Pubkey,
    token_1_mint: Pubkey,
    token_0_program: Pubkey,
    token_1_program: Pubkey,
    user_token_0_account: Pubkey,
    user_token_1_account: Pubkey,
    init_amount_0: u64,
    init_amount_1: u64,
    open_time: u64,
    symbol: String,
    uri: String,
    name: String,
    lp_mint: Pubkey,
    amm_config_index: u64,
) -> Result<Vec<Instruction>> {
    let payer = read_keypair_file(&config.payer_path);
    let url = Cluster::Custom(config.http_url.clone(), config.ws_url.clone());
    // Client.
    let client = Client::new(url, Rc::new(payer.expect("Failed to get payer keypair")));
    let program = client.program(config.raydium_cp_program)?;

    let (amm_config_key, __bump) = Pubkey::find_program_address(
        &[AMM_CONFIG_SEED.as_bytes(), &amm_config_index.to_be_bytes()],
        &program.id(),
    );

    let (pool_account_key, __bump) = Pubkey::find_program_address(
        &[
            POOL_SEED.as_bytes(),
            amm_config_key.to_bytes().as_ref(),
            token_0_mint.to_bytes().as_ref(),
            token_1_mint.to_bytes().as_ref(),
        ],
        &program.id(),
    );
    println!("pool_account_key: {}", pool_account_key);
    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program.id());
    let (token_0_vault, __bump) = Pubkey::find_program_address(
        &[
            POOL_VAULT_SEED.as_bytes(),
            pool_account_key.to_bytes().as_ref(),
            token_0_mint.to_bytes().as_ref(),
        ],
        &program.id(),
    );
    let (token_1_vault, __bump) = Pubkey::find_program_address(
        &[
            POOL_VAULT_SEED.as_bytes(),
            pool_account_key.to_bytes().as_ref(),
            token_1_mint.to_bytes().as_ref(),
        ],
        &program.id(),
    );
    let (observation_key, __bump) = Pubkey::find_program_address(
        &[
            OBSERVATION_SEED.as_bytes(),
            pool_account_key.to_bytes().as_ref(),
        ],
        &program.id(),
    );

    let (lp_mint_key, bump) = Pubkey::find_program_address(
        &[
            "pool_lp_mint".as_bytes(),
            pool_account_key.to_bytes().as_ref(),
        ],
        &program.id(),
    );
    let mut instructions = program
        .request()
        .accounts(raydium_cp_accounts::Initialize {
            creator: program.payer(),
            winna_winna_chickum_dinna: program.payer(),
            amm_config: amm_config_key,
            authority,
            pool_state: pool_account_key,
            token_0_mint,
            token_1_mint,
            lp_mint: lp_mint_key,
            creator_token_0: user_token_0_account,
            creator_token_1: user_token_1_account,
            creator_lp_token: spl_associated_token_account::get_associated_token_address(
                &program.payer(),
                &lp_mint_key,
            ),
            token_0_vault,
            token_1_vault,
            observation_state: observation_key,
            token_program: spl_token::id(),
            token_0_program,
            token_1_program,
            associated_token_program: spl_associated_token_account::id(),
            system_program: system_program::id(),
            rent: sysvar::rent::id(),
        })
        .args(raydium_cp_instructions::Initialize {
            init_amount_0,
            init_amount_1,
            open_time,
        })
        .instructions()?;
    // Extend with compute budget instruction
    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);
    instructions.insert(0, compute_budget_ix);
    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_price(3333333);
    instructions.insert(0, compute_budget_ix);
    // Initialize metadata for the LP token
    let metadata_instruction = program
        .request()
        .accounts(raydium_cp_accounts::InitializeMetadata {
            creator: program.payer(),
            authority,
            pool_state: pool_account_key,
            observation_state: observation_key,
            lp_mint: lp_mint_key,
            token_metadata_program: mpl_token_metadata::ID,
            metadata: Pubkey::find_program_address(
                &[
                    b"metadata",
                    mpl_token_metadata::ID.as_ref(),
                    lp_mint_key.as_ref(),
                ],
                &mpl_token_metadata::ID,
            )
            .0,
            system_program: system_program::id(),
            rent: sysvar::rent::id(),
            amm_config: amm_config_key,
        })
        .args(raydium_cp_instructions::InitializeMetadata {
            name: name.clone(),
            symbol: symbol.clone(),
            uri: uri.clone(),
        })
        .instructions()?;

    instructions.extend(metadata_instruction);
    Ok(instructions)
}

pub fn deposit_instr(
    config: &ClientConfig,
    pool_id: Pubkey,
    token_0_mint: Pubkey,
    token_1_mint: Pubkey,
    token_lp_mint: Pubkey,
    token_0_vault: Pubkey,
    token_1_vault: Pubkey,
    user_token_0_account: Pubkey,
    user_token_1_account: Pubkey,
    user_token_lp_account: Pubkey,
    lp_token_amount: u64,
    maximum_token_0_amount: u64,
    maximum_token_1_amount: u64,
) -> Result<Vec<Instruction>> {
    let payer = read_keypair_file(&config.payer_path);
    let url = Cluster::Custom(config.http_url.clone(), config.ws_url.clone());
    // Client.
    let client = Client::new(url, Rc::new(payer.expect("Failed to get payer keypair")));
    let program = client.program(config.raydium_cp_program)?;

    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program.id());

    let mut instructions = program
        .request()
        .accounts(raydium_cp_accounts::Deposit {
            owner: program.payer(),
            authority,
            pool_state: pool_id,
            owner_lp_token: user_token_lp_account,
            token_0_account: user_token_0_account,
            token_1_account: user_token_1_account,
            token_0_vault,
            token_1_vault,
            token_program: spl_token::id(),
            token_program_2022: spl_token_2022::id(),
            vault_0_mint: token_0_mint,
            vault_1_mint: token_1_mint,
            lp_mint: token_lp_mint,
        })
        .args(raydium_cp_instructions::Deposit {
            lp_token_amount,
            maximum_token_0_amount: u64::MAX,
            maximum_token_1_amount: u64::MAX,
        })
        .instructions()?;

    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);
    instructions.insert(0, compute_budget_ix);
    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_price(3333333);
    instructions.insert(0, compute_budget_ix);
    Ok(instructions)
}

pub fn withdraw_instr(
    config: &ClientConfig,
    pool_id: Pubkey,
    token_0_mint: Pubkey,
    token_1_mint: Pubkey,
    token_lp_mint: Pubkey,
    token_0_vault: Pubkey,
    token_1_vault: Pubkey,
    user_token_0_account: Pubkey,
    user_token_1_account: Pubkey,
    user_token_lp_account: Pubkey,
    lp_token_amount: u64,
    minimum_token_0_amount: u64,
    minimum_token_1_amount: u64,
) -> Result<Vec<Instruction>> {
    let payer = read_keypair_file(&config.payer_path);
    let url = Cluster::Custom(config.http_url.clone(), config.ws_url.clone());
    // Client.
    let client = Client::new(url, Rc::new(payer.expect("Failed to get payer keypair")));
    let program = client.program(config.raydium_cp_program)?;

    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program.id());

    let mut instructions = program
        .request()
        .accounts(raydium_cp_accounts::Withdraw {
            owner: program.payer(),
            authority,
            pool_state: pool_id,
            owner_lp_token: user_token_lp_account,
            token_0_account: user_token_0_account,
            token_1_account: user_token_1_account,
            token_0_vault,
            token_1_vault,
            token_program: spl_token::id(),
            token_program_2022: spl_token_2022::id(),
            vault_0_mint: token_0_mint,
            vault_1_mint: token_1_mint,
            memo_program: spl_memo::ID,
            lp_mint: token_lp_mint,
        })
        .args(raydium_cp_instructions::Withdraw {
            lp_token_amount,
            minimum_token_0_amount,
            minimum_token_1_amount,
        })
        .instructions()?;

    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);
    instructions.insert(0, compute_budget_ix);
    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_price(3333333);
    instructions.insert(0, compute_budget_ix);
    Ok(instructions)
}

pub fn swap_base_input_instr(
    config: &ClientConfig,
    pool_id: Pubkey,
    amm_config: Pubkey,
    observation_account: Pubkey,
    input_token_account: Pubkey,
    output_token_account: Pubkey,
    input_vault: Pubkey,
    output_vault: Pubkey,
    input_token_mint: Pubkey,
    output_token_mint: Pubkey,
    input_token_program: Pubkey,
    output_token_program: Pubkey,
    amount_in: u64,
    minimum_amount_out: u64,
) -> Result<Vec<Instruction>> {
    let payer = read_keypair_file(&config.payer_path);
    let url = Cluster::Custom(config.http_url.clone(), config.ws_url.clone());
    // Client.
    let client = Client::new(url, Rc::new(payer.expect("Failed to get payer keypair")));
    let program = client.program(config.raydium_cp_program)?;

    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program.id());

    let instructions = program
        .request()
        .accounts(raydium_cp_accounts::Swap {
            payer: program.payer(),
            authority,
            amm_config,
            pool_state: pool_id,
            input_token_account,
            output_token_account,
            input_vault,
            output_vault,
            input_token_program,
            output_token_program,
            input_token_mint,
            output_token_mint,
            observation_state: observation_account,
        })
        .args(raydium_cp_instructions::SwapBaseInput {
            amount_in,
            minimum_amount_out,
        })
        .instructions()?;

    Ok(instructions)
}

pub fn swap_base_output_instr(
    config: &ClientConfig,
    pool_id: Pubkey,
    amm_config: Pubkey,
    observation_account: Pubkey,
    input_token_account: Pubkey,
    output_token_account: Pubkey,
    input_vault: Pubkey,
    output_vault: Pubkey,
    input_token_mint: Pubkey,
    output_token_mint: Pubkey,
    input_token_program: Pubkey,
    output_token_program: Pubkey,
    max_amount_in: u64,
    amount_out: u64,
) -> Result<Vec<Instruction>> {
    let payer = read_keypair_file(&config.payer_path);
    let url = Cluster::Custom(config.http_url.clone(), config.ws_url.clone());
    // Client.
    let client = Client::new(url, Rc::new(payer.expect("Failed to get payer keypair")));
    let program = client.program(config.raydium_cp_program)?;

    let (authority, __bump) = Pubkey::find_program_address(&[AUTH_SEED.as_bytes()], &program.id());

    let mut instructions = program
        .request()
        .accounts(raydium_cp_accounts::Swap {
            payer: program.payer(),
            authority,
            amm_config,
            pool_state: pool_id,
            input_token_account,
            output_token_account,
            input_vault,
            output_vault,
            input_token_program,
            output_token_program,
            input_token_mint,
            output_token_mint,
            observation_state: observation_account,
        })
        .args(raydium_cp_instructions::SwapBaseOutput {
            max_amount_in,
            amount_out,
        })
        .instructions()?;

    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(1_400_000);
    instructions.insert(0, compute_budget_ix);
    let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_price(3333333);
    instructions.insert(0, compute_budget_ix);
    Ok(instructions)
}
