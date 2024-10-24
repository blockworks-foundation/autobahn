use anyhow::Error;
use litesvm::LiteSVM;
use log::{error, info, warn};
use router_test_lib::execution_dump::{ExecutionDump, ExecutionItem};
use router_test_lib::{execution_dump, serialize};
use sha2::Digest;
use sha2::Sha256;
use solana_program::clock::Clock;
use solana_program::instruction::Instruction;
use solana_program::program_pack::Pack;
use solana_program::program_stubs::{set_syscall_stubs, SyscallStubs};
use solana_program::pubkey::Pubkey;
use solana_program::sysvar::SysvarId;
use solana_sdk::account::{Account, AccountSharedData, ReadableAccount};
use solana_sdk::bpf_loader_upgradeable::UpgradeableLoaderState;
use solana_sdk::message::{Message, VersionedMessage};
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;
use spl_associated_token_account::{
    get_associated_token_address, get_associated_token_address_with_program_id,
};
use spl_token::state::AccountState;
use spl_token_2022::state::AccountState as AccountState2022;
use std::collections::HashMap;
use std::path::PathBuf;
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
    tracing_subscriber::fmt::init();

    let mut skip_count = option_env!("SKIP_COUNT")
        .map(|x| u32::from_str(x).unwrap_or(0))
        .unwrap_or(0);
    let mut stop_at = u32::MAX;
    let skip_ixs_index = vec![];

    let run_lot_size = option_env!("RUN_LOT_SIZE")
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

        let instruction = deserialize_instruction(&quote.instruction)?;

        let programs = data.programs.iter().copied().collect();
        let mut ctx = setup_test_chain(&programs, &clock, &data, &instruction.program_id)?;

        create_wallet(&mut ctx, wallet.pubkey());

        let initial_in_balance = quote.input_amount * 2;
        let initial_out_balance = 1_000_000;

        // let slot = ctx.banks_client.get_root_slot().await.unwrap();
        // ctx.warp_to_slot(slot+3).unwrap();

        let input_mint_is_2022 = is_2022(&data.accounts, quote.input_mint).await;
        let output_mint_is_2022 = is_2022(&data.accounts, quote.output_mint).await;

        set_balance(
            &mut ctx,
            wallet.pubkey(),
            quote.input_mint,
            initial_in_balance,
            input_mint_is_2022,
        )?;
        set_balance(
            &mut ctx,
            wallet.pubkey(),
            quote.output_mint,
            initial_out_balance,
            output_mint_is_2022,
        )?;

        for meta in &instruction.accounts {
            let Some(account) = ctx.get_account(&meta.pubkey) else {
                log::warn!("missing account : {:?}", meta.pubkey);
                continue;
            };

            // keep code to test hashses
            let mut hasher = Sha256::new();
            hasher.update(account.data());
            let result = hasher.finalize();
            let base64 = base64::encode(result);
            log::debug!(
                "account : {:?} dump : {base64:?} executable : {}",
                meta.pubkey,
                account.executable()
            );
        }

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
    ctx: &mut LiteSVM,
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
            .get_account(&acc.pubkey)
            .map(|x| (x.executable, x.owner.to_string()))
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

fn initialize_accounts(program_test: &mut LiteSVM, dump: &ExecutionDump) -> anyhow::Result<()> {
    log::debug!("initializing accounts : {:?}", dump.accounts.len());
    let accounts_to_load = dump.accounts.clone();
    for (pk, account) in &accounts_to_load {
        if *account.owner() == solana_sdk::bpf_loader_upgradeable::ID {
            log::debug!("{pk:?} has upgradable loader");
            let state = bincode::deserialize::<UpgradeableLoaderState>(&account.data()).unwrap();
            if let UpgradeableLoaderState::Program {
                programdata_address,
            } = state
            {
                // load buffer accounts first
                match accounts_to_load.get(&programdata_address) {
                    Some(program_buffer) => {
                        log::debug!("loading buffer:  {programdata_address:?}");
                        program_test.set_account(
                            programdata_address,
                            solana_sdk::account::Account {
                                lamports: program_buffer.lamports(),
                                owner: *program_buffer.owner(),
                                data: program_buffer.data().to_vec(),
                                rent_epoch: program_buffer.rent_epoch(),
                                executable: program_buffer.executable(),
                            },
                        )?;
                    }
                    None => {
                        error!("{programdata_address:?} is not there");
                    }
                }
            }
        }
        log::debug!(
            "Setting data for {} with owner {} and is executable {}",
            pk,
            account.owner(),
            account.executable()
        );

        log::debug!("Setting data for {}", pk);
        program_test.set_account(
            *pk,
            solana_sdk::account::Account {
                lamports: account.lamports(),
                owner: *account.owner(),
                data: account.data().to_vec(),
                rent_epoch: account.rent_epoch(),
                executable: account.executable(),
            },
        )?;
    }

    Ok(())
}

async fn simulate_cu_usage(
    ctx: &mut LiteSVM,
    owner: &Keypair,
    instruction: &Instruction,
) -> Option<u64> {
    let tx = VersionedTransaction::try_new(
        VersionedMessage::Legacy(Message::new(&[instruction.clone()], Some(&owner.pubkey()))),
        &[owner],
    )
    .unwrap();

    let sim = ctx.simulate_transaction(tx);
    match sim {
        Ok(sim) => {
            let cus = sim.compute_units_consumed;
            log::debug!("----logs");
            for log in sim.logs {
                log::debug!("{log:?}");
            }
            if cus > 0 {
                Some(cus)
            } else {
                None
            }
        }
        Err(e) => {
            log::warn!("Error simulating : {:?}", e);
            None
        }
    }
}

async fn swap(ctx: &mut LiteSVM, owner: &Keypair, instruction: &Instruction) -> anyhow::Result<()> {
    let tx = VersionedTransaction::try_new(
        VersionedMessage::Legacy(Message::new(&[instruction.clone()], Some(&owner.pubkey()))),
        &[owner],
    )
    .unwrap();

    let result = ctx.send_transaction(tx);
    match result {
        Ok(_) => Ok(()),
        Err(e) => {
            log::debug!("------------- LOGS ------------------");
            for log in &e.meta.logs {
                log::debug!("{log:?}");
            }
            Err(anyhow::format_err!("Failed to swap {:?}", e.err))
        }
    }
}

async fn get_balance(
    ctx: &mut LiteSVM,
    owner: Pubkey,
    mint: Pubkey,
    is_2022: bool,
) -> anyhow::Result<u64> {
    let ata_address = get_associated_token_address(&owner, &mint);

    let Some(ata) = ctx.get_account(&ata_address) else {
        return Ok(0);
    };

    if is_2022 {
        let ata = spl_token_2022::state::Account::unpack(&ata.data);
        if let Ok(ata) = ata {
            return Ok(ata.amount);
        }
    };

    if let Ok(ata) = spl_token::state::Account::unpack(&ata.data) {
        Ok(ata.amount)
    } else {
        Ok(0u64)
    }
}

fn set_balance(
    ctx: &mut LiteSVM,
    owner: Pubkey,
    mint: Pubkey,
    amount: u64,
    is_2022: bool,
) -> anyhow::Result<()> {
    let token_program_id = if is_2022 {
        spl_token_2022::ID
    } else {
        spl_token::ID
    };

    let ata_address =
        get_associated_token_address_with_program_id(&owner, &mint, &token_program_id);
    let mut data = vec![0u8; 165];

    if is_2022 {
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
    } else {
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
    };

    ctx.set_account(
        ata_address,
        Account {
            lamports: 1_000_000_000,
            data: data,
            owner: token_program_id,
            executable: false,
            rent_epoch: u64::MAX,
        },
    )?;

    Ok(())
}

fn create_wallet(ctx: &mut LiteSVM, address: Pubkey) {
    let _ = ctx.airdrop(&address, 1_000_000_000);
}

pub fn find_file(filename: &str) -> Option<PathBuf> {
    for dir in default_shared_object_dirs() {
        let candidate = dir.join(filename);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn default_shared_object_dirs() -> Vec<PathBuf> {
    let mut search_path = vec![];
    if let Ok(bpf_out_dir) = std::env::var("BPF_OUT_DIR") {
        search_path.push(PathBuf::from(bpf_out_dir));
    } else if let Ok(bpf_out_dir) = std::env::var("SBF_OUT_DIR") {
        search_path.push(PathBuf::from(bpf_out_dir));
    }
    search_path.push(PathBuf::from("tests/fixtures"));
    if let Ok(dir) = std::env::current_dir() {
        search_path.push(dir);
    }
    log::trace!("SBF .so search path: {:?}", search_path);
    search_path
}

fn setup_test_chain(
    _programs: &Vec<Pubkey>,
    clock: &Clock,
    dump: &ExecutionDump,
    _instruction_program: &Pubkey,
) -> anyhow::Result<LiteSVM> {
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

    let mut program_test = LiteSVM::new();
    program_test.set_sysvar(clock);

    initialize_accounts(&mut program_test, dump)?;
    let path = find_file(format!("autobahn_executor.so").as_str()).unwrap();
    log::debug!("Adding program: {:?} at {path:?}", autobahn_executor::ID);
    program_test.add_program_from_file(autobahn_executor::ID, path)?;

    // TODO: make this dynamic based on routes
    let mut cb = solana_program_runtime::compute_budget::ComputeBudget::default();
    cb.compute_unit_limit = 1_400_000;
    program_test.set_compute_budget(cb);

    Ok(program_test)
}
