use crate::chain_data::ChainDataArcRw;
use crate::dex::{
    AccountProviderView, ChainDataAccountProvider, DexEdge, DexEdgeIdentifier, DexInterface,
};
use crate::test_tools::rpc;
use anchor_spl::associated_token::get_associated_token_address;
use itertools::Itertools;
use mango_feeds_connector::chain_data::AccountData;
use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_test_lib::{execution_dump, serialize};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use solana_sdk::account::ReadableAccount;
use solana_sdk::clock::Clock;
use solana_sdk::config::program;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::signer::Signer;
use solana_sdk::sysvar::SysvarId;
use solana_sdk::bpf_loader_upgradeable::UpgradeableLoaderState;
use std::sync::Arc;
use tracing::{debug, error};
use std::str::FromStr;

pub async fn run_dump_mainnet_data(
    dex: Arc<dyn DexInterface>,
    rpc_client: RouterRpcClient,
    chain_data: ChainDataArcRw,
) -> anyhow::Result<()> {
    run_dump_mainnet_data_with_custom_amount(
        dex,
        rpc_client,
        chain_data,
        Box::new(|_edge| 1_000_000),
    )
    .await
}

pub async fn run_dump_mainnet_data_with_custom_amount(
    dex: Arc<dyn DexInterface>,
    mut rpc_client: RouterRpcClient,
    chain_data: ChainDataArcRw,
    q: Box<dyn Fn(Arc<dyn DexEdge>) -> u64>,
) -> anyhow::Result<()> {
    let wallet = Keypair::from_base58_string(
        "4bGce2QrnYZSYT27nNTxKmmhPxRwGEuDBzETq6xLHtVZ3TUoBec6Wnp7MpVjXXmpvyk64dfW8Q4SHqbgKxUfLgP3",
    );

    // always insert clock into chain data so it's available in dump during test run
    let clock_account = rpc_client.get_account(&Clock::id()).await?;
    let clock = clock_account.deserialize_data::<Clock>()?;
    let clock_data = AccountData {
        slot: clock.slot,
        write_version: 0,
        account: clock_account.to_account_shared_data(),
    };
    chain_data
        .write()
        .unwrap()
        .update_account(Clock::id(), clock_data);

    let chain_data =
        Arc::new(ChainDataAccountProvider::new(chain_data.clone())) as AccountProviderView;

    rpc::load_subscriptions(&mut rpc_client, dex.clone()).await?;
    rpc::load_programs(&mut rpc_client, dex.clone()).await?;

    let edges_identifiers = get_edges_identifiers(&dex);

    let mut errors = 0;
    let mut skipped = 0;
    let mut success = 0;

    let mut accounts_needed = dex.program_ids();
    for id in edges_identifiers {
        accounts_needed.insert(id.input_mint());
        accounts_needed.insert(id.output_mint());

        let Ok(edge) = dex.load(&id, &chain_data) else {
            errors += 1;
            continue;
        };

        let Ok(quote) = dex.quote(&id, &edge, &chain_data, q(edge.clone())) else {
            errors += 1;
            continue;
        };

        if quote.in_amount == 0 {
            skipped += 1;
            continue;
        }

        if quote.out_amount == 0 {
            skipped += 1;
            continue;
        }

        let Ok(swap_ix) = dex.build_swap_ix(
            &id,
            &chain_data,
            &wallet.pubkey(),
            quote.in_amount,
            quote.out_amount,
            1000,
        ) else {
            errors += 1;
            continue;
        };

        accounts_needed.extend(
            swap_ix
                .instruction
                .accounts
                .iter()
                .map(|x| x.pubkey)
                .filter(|x| {
                    !is_ata(&x, &wallet.pubkey(), &id.input_mint())
                        && !is_ata(&x, &wallet.pubkey(), &id.output_mint())
                        && !chain_data.account(&x).is_ok()
                }),
        );
        success += 1;
    }

    println!("Adding some {} accounts", accounts_needed.len());
    for x in accounts_needed.iter().take(10) {
        println!("- {} ", x);
    }
    let accounts = rpc_client.get_multiple_accounts(&accounts_needed).await?;

    for (_, account) in accounts {
        // get buffer for upgradable programs
        if account.owner == solana_sdk::bpf_loader_upgradeable::ID {
            let state = bincode::deserialize::<UpgradeableLoaderState>(&account.data).unwrap();
            if let UpgradeableLoaderState::Program { programdata_address } = state {
                rpc_client.get_account(&programdata_address).await?;
            }
        }
    }

    println!("Error count: {}", errors);
    println!("Skipped count: {}", skipped);
    println!("Success count: {}", success);

    assert!(!accounts_needed.is_empty());
    Ok(())
}

fn is_ata(acc: &Pubkey, wallet: &Pubkey, mint: &Pubkey) -> bool {
    get_associated_token_address(wallet, mint) == *acc
}

pub async fn run_dump_swap_ix(
    dump_name: &str,
    dex: Arc<dyn DexInterface>,
    chain_data: ChainDataArcRw,
) -> anyhow::Result<()> {
    run_dump_swap_ix_with_custom_amount(dump_name, dex, chain_data, Box::new(|_edge| 1_000_000))
        .await
}

pub async fn run_dump_swap_ix_with_custom_amount(
    dump_name: &str,
    dex: Arc<dyn DexInterface>,
    chain_data: ChainDataArcRw,
    q: Box<dyn Fn(Arc<dyn DexEdge>) -> u64>,
) -> anyhow::Result<()> {
    let wallet = Keypair::from_base58_string(
        "4bGce2QrnYZSYT27nNTxKmmhPxRwGEuDBzETq6xLHtVZ3TUoBec6Wnp7MpVjXXmpvyk64dfW8Q4SHqbgKxUfLgP3",
    );

    let account_provider =
        Arc::new(ChainDataAccountProvider::new(chain_data.clone())) as AccountProviderView;

    let edges_identifiers = get_edges_identifiers(&dex);

    println!("Pools count: {}", edges_identifiers.len());

    let mut dump = execution_dump::ExecutionDump {
        wallet_keypair: wallet.to_base58_string(),
        programs: dex.program_ids().into_iter().collect(),
        cache: vec![],
        accounts: Default::default(),
    };

    // always include clock sysvar in dump to ensure we can set the sysvar during test run
    let clock_data = account_provider.account(&Clock::id())?;
    dump.accounts
        .insert(Clock::id(), clock_data.account.to_account_shared_data());

    let mut errors = 0;
    let mut skipped = 0;
    let mut success = 0;
    let mut exact_out_sucess = 0;

    for id in edges_identifiers {
        let Ok(edge) = dex.load(&id, &account_provider) else {
            errors += 1;
            continue;
        };

        let Ok(quote) = dex.quote(&id, &edge, &account_provider, q(edge.clone())) else {
            errors += 1;
            continue;
        };

        if quote.in_amount == 0 {
            skipped += 1;
            continue;
        }

        if quote.out_amount == 0 {
            skipped += 1;
            continue;
        }

        let Ok(swap_ix) = dex.build_swap_ix(
            &id,
            &account_provider,
            &wallet.pubkey(),
            quote.in_amount,
            quote.out_amount,
            1000,
        ) else {
            errors += 1;
            continue;
        };

        debug!(
            "#{} || quote: {} => {} : {} => {}",
            success,
            id.input_mint(),
            id.output_mint(),
            quote.in_amount,
            quote.out_amount
        );
        success += 1;

        dump.cache.push(execution_dump::ExecutionItem {
            input_mint: id.input_mint(),
            output_mint: id.output_mint(),
            input_amount: quote.in_amount,
            output_amount: quote.out_amount,
            instruction: bincode::serialize(&swap_ix.instruction).unwrap(),
            is_exact_out: false,
        });

        let chain_data_reader = chain_data.read().unwrap();
        for account in swap_ix.instruction.accounts {
            if let Ok(acc) = chain_data_reader.account(&account.pubkey) {
                dump.accounts.insert(account.pubkey, acc.account.clone());
            } else {
                error!("Missing account (needed for swap) {}", account.pubkey);
            }
        }
        let account = chain_data_reader
            .account(&id.input_mint())
            .expect("missing mint");
        dump.accounts
            .insert(id.input_mint(), account.account.clone());
        let account = chain_data_reader
            .account(&id.input_mint())
            .expect("missing mint");
        dump.accounts
            .insert(id.output_mint(), account.account.clone());

        // build exact out tests
        if dex.supports_exact_out(&id) {
            let Ok(mut quote_exact_out) =
                dex.quote_exact_out(&id, &edge, &account_provider, q(edge.clone()))
            else {
                errors += 1;
                continue;
            };
            // add slippage
            quote_exact_out.in_amount = (quote_exact_out.in_amount as f64 * 1.01).ceil() as u64;

            if quote_exact_out.in_amount != u64::MAX && quote_exact_out.out_amount != 0 {
                let Ok(swap_exact_out_ix) = dex.build_swap_ix(
                    &id,
                    &account_provider,
                    &wallet.pubkey(),
                    quote_exact_out.in_amount,
                    quote_exact_out.out_amount,
                    1000,
                ) else {
                    errors += 1;
                    continue;
                };

                debug!(
                    "#{} || quote_exact_out: {} => {} : {} => {}",
                    success,
                    id.input_mint(),
                    id.output_mint(),
                    quote_exact_out.in_amount,
                    quote_exact_out.out_amount
                );
                exact_out_sucess += 1;

                dump.cache.push(execution_dump::ExecutionItem {
                    input_mint: id.input_mint(),
                    output_mint: id.output_mint(),
                    input_amount: quote_exact_out.in_amount,
                    output_amount: quote_exact_out.out_amount,
                    instruction: bincode::serialize(&swap_exact_out_ix.instruction).unwrap(),
                    is_exact_out: true,
                });

                // add exact out accounts
                let chain_data_reader = chain_data.read().unwrap();
                for account in swap_exact_out_ix.instruction.accounts {
                    if let Ok(acc) = chain_data_reader.account(&account.pubkey) {
                        dump.accounts.insert(account.pubkey, acc.account.clone());
                    } else {
                        error!("Missing account (needed for swap) {}", account.pubkey);
                    }
                }
            }
        }
    }

    println!("Error count: {}", errors);
    println!("Skipped count: {}", skipped);
    println!("Success count: {}", success);
    println!("Exactout Success count: {}", exact_out_sucess);

    for program in dump.programs.clone() {
        let program_account = account_provider.account(&program)?;

        dump.accounts.insert(program, program_account.account.clone());
        // use downloaded buffers for the upgradable programs
        if *program_account.account.owner() == solana_sdk::bpf_loader_upgradeable::ID {
            let state = bincode::deserialize::<UpgradeableLoaderState>(program_account.account.data()).unwrap();
            if let UpgradeableLoaderState::Program { programdata_address } = state {
                let program_data_account = account_provider.account(&programdata_address)?;
                dump.accounts.insert(programdata_address, program_data_account.account);
            }
        }
    }

    for program in &dump.programs {
        debug!("program : {program:?}");
    }

    for (pk, program) in &dump.accounts {
        let mut hasher = Sha256::new();
        hasher.update(program.data());
        let result = hasher.finalize();
        let base64 = base64::encode(result);
        debug!("account : {pk:?} dump : {base64:?}");
    }
    serialize::serialize_to_file(
        &dump,
        &format!("../../programs/simulator/tests/fixtures/{}", dump_name).to_string(),
    );

    Ok(())
}

fn get_edges_identifiers(dex: &Arc<dyn DexInterface>) -> Vec<Arc<dyn DexEdgeIdentifier>> {
    dex.edges_per_pk()
        .into_iter()
        .flat_map(|x| x.1)
        .unique_by(|x| (x.key(), x.input_mint()))
        .sorted_by_key(|x| (x.key(), x.input_mint()))
        .collect_vec()
}
