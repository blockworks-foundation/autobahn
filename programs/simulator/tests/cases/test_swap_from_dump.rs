use anyhow::{Context, Error};
use bonfida_test_utils::error::TestError;
use bonfida_test_utils::ProgramTestContextExt;
use log::{debug, error, info, warn};
use router_test_lib::execution_dump::{ExecutionDump, ExecutionItem};
use router_test_lib::{execution_dump, serialize};
use solana_program::clock::{Clock, Epoch};
use solana_program::instruction::Instruction;
use solana_program::program_pack::Pack;
use solana_program::program_stubs::{set_syscall_stubs, SyscallStubs};
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::SysvarId;
use solana_program_test::BanksClientError;
use solana_program_test::{ProgramTest, ProgramTestContext};
use solana_sdk::account::{Account, AccountSharedData, ReadableAccount};
use solana_sdk::epoch_info::EpochInfo;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::Transaction;
use spl_associated_token_account::get_associated_token_address;
use spl_token::state::AccountState;
use spl_token_2022::state::AccountState as AccountState2022;
use std::collections::HashMap;
use std::process::exit;
use std::str::FromStr;

struct TestLogSyscallStubs;
impl SyscallStubs for TestLogSyscallStubs {
    fn sol_log(&self, message: &str) {
        info!("{}", message)
    }

    fn sol_log_data(&self, _fields: &[&[u8]]) {
        // do nothing
    }
}

#[tokio::test]
async fn test_quote_match_swap_for_orca() -> anyhow::Result<()> {
    run_all_swap_from_dump("orca_swap.lz4").await?
}

#[tokio::test]
async fn test_quote_match_swap_for_cropper() -> anyhow::Result<()> {
    run_all_swap_from_dump("cropper_swap.lz4").await?
}

#[tokio::test]
async fn test_quote_match_swap_for_saber() -> anyhow::Result<()> {
    run_all_swap_from_dump("saber_swap.lz4").await?
}

#[tokio::test]
async fn test_quote_match_swap_for_raydium() -> anyhow::Result<()> {
    run_all_swap_from_dump("raydium_swap.lz4").await?
}

#[tokio::test]
async fn test_quote_match_swap_for_raydium_cp() -> anyhow::Result<()> {
    run_all_swap_from_dump("raydium_cp_swap.lz4").await?
}

#[tokio::test]
async fn test_quote_match_swap_for_openbook_v2() -> anyhow::Result<()> {
    run_all_swap_from_dump("openbook_v2_swap.lz4").await?
}

#[tokio::test]
async fn test_quote_match_swap_for_infinity() -> anyhow::Result<()> {
    run_all_swap_from_dump("infinity_swap.lz4").await?
}

async fn run_all_swap_from_dump(dump_name: &str) -> Result<Result<(), Error>, Error> {
    let mut skip_count = option_env!("SKIP_COUNT")
        .map(|x| u32::from_str(x).unwrap_or(0))
        .unwrap_or(0);
    let mut stop_at = u32::MAX;
    let skip_ixs_index = vec![];

    let mut run_lot_size = option_env!("RUN_LOT_SIZE")
        .map(|x| u32::from_str(x).unwrap_or(500))
        .unwrap_or(500);

    if let Some(run_lot) = option_env!("RUN_LOT").map(|x| u32::from_str(x).unwrap_or(0)) {
        skip_count = run_lot_size * run_lot;
        stop_at = run_lot_size * (1 + run_lot);
    }

    set_syscall_stubs(Box::new(TestLogSyscallStubs {}));

    let data = serialize::deserialize_from_file::<execution_dump::ExecutionDump>(
        &format!("tests/fixtures/{}", dump_name).to_string(),
    )?;
    let wallet = Keypair::from_base58_string(data.wallet_keypair.as_str());

    let mut success = 0;
    let mut index = 0;

    let clock_account = data
        .accounts
        .get(&Clock::id())
        .ok_or("invalid dump doesnt contain clock sysvar")
        .unwrap();
    let clock = clock_account.deserialize_data::<Clock>()?;

    let mut cus_required = vec![];
    for quote in &data.cache {
        if quote.is_exact_out {
            continue;
        }

        index += 1;
        if skip_count > 0 {
            skip_count -= 1;
            continue;
        }
        if index > stop_at {
            continue;
        }
        if skip_ixs_index.contains(&(index)) {
            continue;
        }

        let mut ctx = setup_test_chain(&data.programs, &clock).await;

        create_wallet(&mut ctx, wallet.pubkey());

        let initial_in_balance = quote.input_amount * 2;
        let initial_out_balance = 1_000_000;

        let instruction = deserialize_instruction(&quote.instruction)?;

        initialize_instruction_accounts(&mut ctx, &data, &instruction).await?;

        let input_mint_is_2022 = is_2022(&data.accounts, quote.input_mint).await;
        let output_mint_is_2022 = is_2022(&data.accounts, quote.output_mint).await;

        set_balance(
            &mut ctx,
            wallet.pubkey(),
            quote.input_mint,
            initial_in_balance,
            input_mint_is_2022,
        )
        .await?;
        set_balance(
            &mut ctx,
            wallet.pubkey(),
            quote.output_mint,
            initial_out_balance,
            output_mint_is_2022,
        )
        .await?;

        if let Some(cus) = simulate_cu_usage(&mut ctx, &wallet, &instruction).await {
            cus_required.push(cus);
        }

        match swap(&mut ctx, &wallet, &instruction).await {
            Ok(_) => Ok(()),
            Err(e) => {
                debug_print_ix(
                    &mut success,
                    &mut index,
                    quote,
                    &mut ctx,
                    &instruction,
                    input_mint_is_2022,
                    output_mint_is_2022,
                )
                .await;

                Err(e)
            }
        }?;

        let post_in_balance = get_balance(
            &mut ctx,
            wallet.pubkey(),
            quote.input_mint,
            input_mint_is_2022,
        )
        .await?;
        let post_out_balance = get_balance(
            &mut ctx,
            wallet.pubkey(),
            quote.output_mint,
            output_mint_is_2022,
        )
        .await?;

        let sent_in_amount = initial_in_balance.saturating_sub(post_in_balance);
        let received_out_amount = post_out_balance.saturating_sub(initial_out_balance);

        info!(
            "Swapped #{index}: {} ({}) -> {} ({})",
            sent_in_amount, quote.input_mint, received_out_amount, quote.output_mint
        );
        info!(
            "Expected: {} -> {}",
            quote.input_amount, quote.output_amount
        );

        let unexpected_in_amount = quote.input_amount < sent_in_amount;
        let unexpected_out_amount = if quote.is_exact_out {
            quote.output_amount > received_out_amount
        } else {
            quote.output_amount != received_out_amount
        };

        if unexpected_in_amount || unexpected_out_amount {
            debug_print_ix(
                &mut success,
                &mut index,
                quote,
                &mut ctx,
                &instruction,
                input_mint_is_2022,
                output_mint_is_2022,
            )
            .await;
        }

        if quote.is_exact_out {
            assert!(quote.input_amount >= sent_in_amount);
            assert!(quote.output_amount <= received_out_amount);
        } else {
            assert!(quote.input_amount >= sent_in_amount);
            assert_eq!(quote.output_amount, received_out_amount);
        }

        success += 1;
    }

    cus_required.sort();
    let count = cus_required.len();
    if count > 0 {
        let median_index = count / 2;
        let p75_index = count * 75 / 100;
        let p95_index = count * 95 / 100;
        let p99_index = count * 99 / 100;
        println!("Cu usage stats");
        println!(
            "Count: {}, Min :{}, Max: {}, Median: {}, p75:{}, p95: {}, p99:{}",
            count,
            cus_required[0],
            cus_required[count - 1],
            cus_required[median_index],
            cus_required[p75_index],
            cus_required[p95_index],
            cus_required[p99_index]
        );
    }

    info!("Successfully ran {} swaps", success);

    Ok(Ok(()))
}

async fn debug_print_ix(
    success: &mut i32,
    index: &mut u32,
    quote: &ExecutionItem,
    ctx: &mut ProgramTestContext,
    instruction: &Instruction,
    input_mint_is_2022: bool,
    output_mint_is_2022: bool,
) {
    error!(
        "Faulty swapping #{} quote{}: \r\n{} -> {} ({} -> {})\r\n (successfully run {} swap)",
        index,
        if quote.is_exact_out {
            " (ExactOut)"
        } else {
            ""
        },
        quote.input_mint,
        quote.output_mint,
        quote.input_amount,
        quote.output_amount,
        success
    );

    error!("Faulty ix: {:?}", instruction);
    error!(
        "* input mint: {} (is 2022 -> {})",
        quote.input_mint, input_mint_is_2022
    );
    error!(
        "* output mint: {} (is 2022 -> {})",
        quote.output_mint, output_mint_is_2022
    );

    for acc in &instruction.accounts {
        let account = ctx
            .banks_client
            .get_account(acc.pubkey)
            .await
            .map(|x| {
                x.map(|y| (y.executable, y.owner.to_string()))
                    .unwrap_or((false, "???".to_string()))
            })
            .unwrap_or((false, "???".to_string()));

        warn!(
            "Account: {} (exec={}) is owned by {} ",
            acc.pubkey, account.0, account.1
        );
    }
}

async fn is_2022(accounts: &HashMap<Pubkey, AccountSharedData>, mint: Pubkey) -> bool {
    let result = accounts.get(&mint);
    let Some(result) = result else {
        warn!("Missing Mint: {}", mint);
        return false;
    };

    *result.owner() == Pubkey::from_str("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb").unwrap()
}

fn deserialize_instruction(swap_ix: &Vec<u8>) -> anyhow::Result<Instruction> {
    let instruction: Instruction = bincode::deserialize(swap_ix.as_slice())?;
    Ok(instruction)
}

async fn initialize_instruction_accounts(
    ctx: &mut ProgramTestContext,
    dump: &ExecutionDump,
    instruction: &Instruction,
) -> anyhow::Result<()> {
    for account_meta in &instruction.accounts {
        if dump.programs.contains(&account_meta.pubkey) {
            continue;
        }
        if let Some(account) = dump.accounts.get(&account_meta.pubkey) {
            if account.executable() {
                continue;
            }
            debug!("Setting data for {}", account_meta.pubkey);
            ctx.set_account(&account_meta.pubkey, account);
        } else {
            if ctx
                .banks_client
                .get_account(account_meta.pubkey)
                .await?
                .is_none()
            {
                debug!("Missing data for {}", account_meta.pubkey); // Can happen for empty oracle account...
            }
        }
    }

    Ok(())
}

async fn simulate_cu_usage(
    ctx: &mut ProgramTestContext,
    owner: &Keypair,
    instruction: &Instruction,
) -> Option<u64> {
    let mut transaction =
        Transaction::new_with_payer(&[instruction.clone()], Some(&ctx.payer.pubkey()));

    transaction.sign(&[&ctx.payer, owner], ctx.last_blockhash);
    let sim = ctx
        .banks_client
        .simulate_transaction(transaction.clone())
        .await;
    match sim {
        Ok(sim) => {
            log::debug!("{:?}", sim.result);
            if sim.result.is_some() && sim.result.unwrap().is_ok() {
                let simulation_details = sim.simulation_details.unwrap();
                let cus = simulation_details.units_consumed;
                log::debug!("units consumed : {}", cus);
                log::debug!("----logs");
                for log in simulation_details.logs {
                    log::debug!("{log:?}");
                }
                Some(cus)
            } else {
                None
            }
        }
        Err(e) => {
            log::warn!("Error simulating : {}", e);
            None
        }
    }
}

async fn swap(
    ctx: &mut ProgramTestContext,
    owner: &Keypair,
    instruction: &Instruction,
) -> anyhow::Result<()> {
    ctx.get_new_latest_blockhash().await?;

    log::info!("swapping");
    let result = ctx
        .sign_send_instructions(&[instruction.clone()], &[&owner])
        .await;

    match result {
        Ok(()) => Ok(()),
        Err(e) => Err(anyhow::format_err!("Failed to swap {:?}", e)),
    }
}

async fn get_balance(
    ctx: &mut ProgramTestContext,
    owner: Pubkey,
    mint: Pubkey,
    is_2022: bool,
) -> anyhow::Result<u64> {
    let ata_address = get_associated_token_address(&owner, &mint);

    if is_2022 {
        let Ok(ata) = ctx.banks_client.get_account(ata_address).await else {
            return Ok(0);
        };

        let Some(ata) = ata else {
            return Ok(0);
        };

        let ata = spl_token_2022::state::Account::unpack(&ata.data);
        if let Ok(ata) = ata {
            return Ok(ata.amount);
        }
    };

    if let Ok(ata) = ctx.get_token_account(ata_address).await {
        Ok(ata.amount)
    } else {
        Ok(0u64)
    }
}

async fn set_balance(
    ctx: &mut ProgramTestContext,
    owner: Pubkey,
    mint: Pubkey,
    amount: u64,
    is_2022: bool,
) -> anyhow::Result<()> {
    let ata_address = get_associated_token_address(&owner, &mint);

    if is_2022 {
        let mut data = vec![0u8; 165];
        let account = spl_token_2022::state::Account {
            mint,
            owner,
            amount,
            delegate: Default::default(),
            state: AccountState2022::Initialized,
            is_native: Default::default(),
            delegated_amount: 0,
            close_authority: Default::default(),
        };
        account.pack_into_slice(data.as_mut_slice());

        ctx.set_account(
            &ata_address,
            &AccountSharedData::from(Account {
                lamports: 1_000_000_000,
                data: data,
                owner: spl_token_2022::ID,
                executable: false,
                rent_epoch: 0,
            }),
        );

        return Ok(());
    }

    let mut data = vec![0u8; 165];
    let account = spl_token::state::Account {
        mint,
        owner,
        amount,
        delegate: Default::default(),
        state: AccountState::Initialized,
        is_native: Default::default(),
        delegated_amount: 0,
        close_authority: Default::default(),
    };
    account.pack_into_slice(data.as_mut_slice());

    ctx.set_account(
        &ata_address,
        &AccountSharedData::from(Account {
            lamports: 1_000_000_000,
            data: data,
            owner: spl_token::ID,
            executable: false,
            rent_epoch: 0,
        }),
    );

    Ok(())
}

fn create_wallet(ctx: &mut ProgramTestContext, address: Pubkey) {
    ctx.set_account(
        &address,
        &AccountSharedData::from(Account {
            lamports: 1_000_000_000,
            data: vec![],
            owner: address,
            executable: false,
            rent_epoch: 0,
        }),
    );
}

async fn setup_test_chain(programs: &Vec<Pubkey>, clock: &Clock) -> ProgramTestContext {
    // We need to intercept logs to capture program log output
    let log_filter = "solana_rbpf=trace,\
                    solana_runtime::message_processor=debug,\
                    solana_runtime::system_instruction_processor=trace,\
                    solana_program_test=info,\
                    solana_metrics::metrics=warn,\
                    tarpc=error,\
                    info";
    let env_logger =
        env_logger::Builder::from_env(env_logger::Env::new().default_filter_or(log_filter))
            .format_timestamp_nanos()
            .build();
    let _ = log::set_boxed_logger(Box::new(env_logger));

    let mut program_test = ProgramTest::default();
    for &key in programs {
        program_test.add_program(key.to_string().as_str(), key, None);
    }
    program_test.add_program("autobahn_executor", autobahn_executor::ID, None);

    // TODO: make this dynamic based on routes
    program_test.set_compute_max_units(1_400_000);

    let program_test_context = program_test.start_with_context().await;

    // Set clock
    program_test_context.set_sysvar(clock);

    info!("Setting clock to: {}", clock.unix_timestamp);

    program_test_context
}
