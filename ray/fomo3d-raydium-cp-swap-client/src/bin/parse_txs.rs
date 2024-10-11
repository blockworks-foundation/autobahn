use anyhow::Result;
use rand::seq::SliceRandom; // Import the necessary trait for random selection
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_client::RpcClient;
use solana_rpc_client_api::config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_rpc_client_api::filter::{Memcmp, MemcmpEncodedBytes, RpcFilterType};
use solana_sdk::pubkey::Pubkey;
use std::fs::File;
use std::io::Write;

fn main() -> Result<()> {
    // List of RPC URLs
    let rpc_urls = vec![
        "https://burned-young-snowflake.solana-mainnet.quiknode.pro/96e3f49289f987ccdd62dacc40990b20bd21f5ad/",
        "https://skilled-sly-choice.solana-mainnet.quiknode.pro/5db92b766fd9b7ec4cc7e89101473c1d579aa98a/",
        "https://aged-billowing-firefly.solana-mainnet.quiknode.pro/714c2bc2cba308a8c5fe4aee343d31b83b9f42d1/",
        "https://distinguished-dry-sea.solana-mainnet.quiknode.pro/79528918b82740044a48a73406c3139caf8e729d/",
        "https://solitary-yolo-ensemble.solana-mainnet.quiknode.pro/82fe22445068e050d80b27275910aa62734e2520/",
        "https://summer-orbital-gas.solana-mainnet.quiknode.pro/dff876e9e6cb916bc741a761367a91f50ff5dd92/",
        "https://serene-cosmopolitan-arrow.solana-mainnet.quiknode.pro/e5024a662e59587220837fbb749fe7cce477ca09/",
        "https://neat-snowy-bird.solana-mainnet.quiknode.pro/14c0721161ba1af1c4ef91b0a568e2b24edeb9c5/"
    ];

    // Randomly select an RPC URL
    let rpc_url = rpc_urls
        .choose(&mut rand::thread_rng())
        .expect("No RPC URLs available");

    // Initialize RPC client with the selected URL
    let client = RpcClient::new(rpc_url.to_string());

    // Program ID
    let program_id = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C".parse::<Pubkey>()?;

    // Set up filters
    let filters = vec![
        RpcFilterType::DataSize(637),
        RpcFilterType::Memcmp(Memcmp::new(
            0,
            MemcmpEncodedBytes::Base58("iUE1qg7KXeV".to_string()),
        )),
    ];

    // Configure the RPC request
    let config = RpcProgramAccountsConfig {
        filters: Some(filters),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            ..RpcAccountInfoConfig::default()
        },
        ..RpcProgramAccountsConfig::default()
    };

    // Fetch program accounts
    let accounts = client.get_program_accounts_with_config(&program_id, config)?;

    println!("Found {} matching accounts:", accounts.len());

    // Open file in write mode
    let mut file = File::create("cp-swap.txt")?;

    for (pubkey, account) in accounts {
        // Write each pubkey to the file
        writeln!(file, "{}", pubkey)?;
    }

    Ok(())
}
