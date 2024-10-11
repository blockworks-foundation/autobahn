use anyhow::{Error, Result};
use bs58;
use rand::seq::SliceRandom;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_client::{GetConfirmedSignaturesForAddress2Config, RpcClient};
use solana_client::rpc_config::RpcTransactionConfig;
use solana_client::rpc_response::RpcSignatureResult;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction};
use std::fs::{create_dir_all, File};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::exit;

const SIGNATURE_BYTES: usize = 64;
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

    // Create a directory to store the transaction files
    let txs_dir = Path::new("cp-swap-txs");
    if let Err(e) = create_dir_all(&txs_dir) {
        eprintln!("Failed to create directory: {}", e);
    }

    // Open the cp-swap.txt file for reading
    let file = match File::open("cp-swap.txt") {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to open cp-swap.txt: {}", e);
            return Ok(());
        }
    };
    let reader = BufReader::new(file);

    let mut line_count = 0;
    // Iterate through each public key in cp-swap.txt
    for line in reader.lines() {
        if line_count >= 333 {
            break; // Exit the loop after processing 111 lines
        }
        let pubkey_str = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Error reading line: {}", e);
                continue;
            }
        };

        let pubkey = match pubkey_str.parse::<Pubkey>() {
            Ok(pk) => pk,
            Err(e) => {
                eprintln!("Invalid public key: {}, Error: {}", pubkey_str, e);
                continue;
            }
        };

        // Create a file to store the transactions for this account
        let tx_file_path = txs_dir.join(format!("{}.txt", pubkey));
        let mut tx_file = match File::create(&tx_file_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Failed to create file for {}: {}", pubkey, e);
                continue;
            }
        };

        // Iterate over the transactions, fetching until there are fewer than 1000 transactions
        let mut before = None;
        loop {
            // Fetch transactions for the given public key, up to 1000 at a time
            let txs = match get_program_transactions(&client, &pubkey, before.clone()) {
                Ok(txs) => txs,
                Err(e) => {
                    eprintln!("Failed to fetch transactions for {}: {}", pubkey, e);
                    break;
                }
            };

            // If there are fewer than 1000 transactions, stop the iteration
            let tx_count = txs.len();
            if tx_count == 0 {
                break;
            }

            // Write the transactions to the file
            for tx in &txs {
                if let Err(e) = writeln!(tx_file, "{}", serde_json::to_string(&tx).unwrap()) {
                    eprintln!("Failed to write transaction to file for {}: {}", pubkey, e);
                    break;
                }
            }

            let last_tx = txs.last().unwrap();
            match &last_tx.transaction.transaction {
                EncodedTransaction::Json(ui_transaction) => {
                    if let Some(first_signature) = ui_transaction.signatures.get(0) {
                        before = Some(first_signature.clone());
                    } else {
                        println!("No signatures available in the last transaction");
                    }
                }
                _ => println!("Unexpected transaction encoding"),
            }

            println!("Fetched {} transactions for {}", tx_count, pubkey);

            if tx_count < 1000 {
                break;
            }
        }

        line_count += 1;
    }

    Ok(())
}

fn decode_base58_to_signature(base58sig: &str) -> Option<Signature> {
    let decoded_bytes = bs58::decode(base58sig).into_vec().ok()?;

    if decoded_bytes.len() != SIGNATURE_BYTES {
        return None;
    }

    let mut byte_array = [0u8; SIGNATURE_BYTES];
    byte_array.copy_from_slice(&decoded_bytes);

    Some(Signature::from(byte_array))
}

// Helper function to get the transactions for a given account
fn get_program_transactions(
    client: &RpcClient,
    pubkey: &Pubkey,
    before: Option<String>,
) -> Result<Vec<EncodedConfirmedTransactionWithStatusMeta>, String> {
    let max_tx_count = 1000; // Fetch up to 1000 transactions at a time
    let mut config = RpcTransactionConfig::default();
    config.encoding = Some(solana_transaction_status::UiTransactionEncoding::JsonParsed);
    config.max_supported_transaction_version = Some(0);
    let before_sig = match before {
        Some(bs58sig) => {
            let decoded_bytes = bs58::decode(bs58sig)
                .into_vec()
                .expect("Failed to decode Base58 string");

            if decoded_bytes.len() != 64 {
                return Err("lengths don't match".to_string());
            }
            // Convert Vec<u8> to [u8; SIGNATURE_BYTES]
            let mut byte_array = [0u8; 64];
            byte_array.copy_from_slice(&decoded_bytes);

            // Create a Signature object from the byte array
            let signature = Signature::from(byte_array);
            Some(signature)
        }
        None => None,
    };

    // Fetch the signatures for the transactions related to the given account
    let transactions = match client.get_signatures_for_address_with_config(
        pubkey,
        GetConfirmedSignaturesForAddress2Config {
            before: before_sig,
            until: None,
            limit: Some(max_tx_count),
            ..GetConfirmedSignaturesForAddress2Config::default()
        },
    ) {
        Ok(txs) => txs,
        Err(e) => return Err(e.to_string()),
    };

    // Fetch full transaction details for each signature
    let mut full_txs = vec![];
    for signature_info in transactions {
        let signature_typed = decode_base58_to_signature(signature_info.signature.as_str());
        if let Some(signature) = signature_typed {
            match client.get_transaction_with_config(&signature, config) {
                Ok(tx) => full_txs.push(tx),
                Err(e) => eprintln!("Failed to fetch transaction {}: {}", signature, e),
            }
        }
    }

    Ok(full_txs)
}
