use crate::dex::SwapMode;
use crate::model::quote_response::QuoteResponse;
use crate::model::swap_request::SwapRequest;
use crate::model::swap_response::SwapIxResponse;
use crate::utils::http_error_handling;
use anyhow::Context;
use solana_sdk::address_lookup_table::AddressLookupTableAccount;
use solana_sdk::hash::Hash;
use solana_sdk::message::VersionedMessage;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, NullSigner};
use solana_sdk::signer::Signer;
use solana_sdk::transaction::VersionedTransaction;
use std::str::FromStr;
use std::time::Duration;

pub struct RouterClient {
    pub http_client: reqwest::Client,
    pub router_url: String,
}

impl RouterClient {
    pub async fn simulate_swap<F>(
        &self,
        lookup_tables: F,
        quote_response: QuoteResponse,
        wallet_pk: &Pubkey,
        latest_blockhash: Hash,
        wrap_unwrap_sol: bool,
    ) -> anyhow::Result<VersionedTransaction>
    where
        F: Fn(Pubkey) -> Option<AddressLookupTableAccount>,
    {
        let message = self
            .build_swap_message(
                lookup_tables,
                quote_response,
                wallet_pk,
                latest_blockhash,
                wrap_unwrap_sol,
            )
            .await?;
        let tx = VersionedTransaction::try_new(message, &[&NullSigner::new(wallet_pk)])?;

        Ok(tx)
    }

    pub async fn swap<F>(
        &self,
        lookup_tables: F,
        quote_response: QuoteResponse,
        wallet: &Keypair,
        latest_blockhash: Hash,
    ) -> anyhow::Result<VersionedTransaction>
    where
        F: Fn(Pubkey) -> Option<AddressLookupTableAccount>,
    {
        let message = self
            .build_swap_message(
                lookup_tables,
                quote_response,
                &wallet.pubkey(),
                latest_blockhash,
                true,
            )
            .await?;
        let tx = VersionedTransaction::try_new(message, &[&wallet])?;

        Ok(tx)
    }

    async fn build_swap_message<F>(
        &self,
        lookup_tables: F,
        quote_response: QuoteResponse,
        wallet: &Pubkey,
        latest_blockhash: Hash,
        wrap_and_unwrap_sol: bool,
    ) -> anyhow::Result<VersionedMessage>
    where
        F: Fn(Pubkey) -> Option<AddressLookupTableAccount>,
    {
        let query_args: Vec<String> = vec![];

        let request = SwapRequest {
            user_public_key: wallet.to_string(),
            wrap_and_unwrap_sol,
            auto_create_out_ata: true,
            use_shared_accounts: false,
            fee_account: None,
            compute_unit_price_micro_lamports: None,
            as_legacy_transaction: false,
            use_token_ledger: false,
            destination_token_account: None,
            quote_response,
        };

        let response = self
            .http_client
            .post(format!("{}/swap-instructions", self.router_url))
            .query(&query_args)
            .json(&request)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("swap request to autobahn-router")?;

        let swap: SwapIxResponse = http_error_handling(response).await.with_context(|| {
            format!(
                "error requesting swap between {} and {}",
                request.quote_response.input_mint, request.quote_response.output_mint
            )
        })?;

        let mut instructions: Vec<solana_sdk::instruction::Instruction> = vec![];

        if let Some(ixs) = swap.compute_budget_instructions {
            for ix in ixs {
                instructions.push(ix.to_ix()?);
            }
        }

        if let Some(ixs) = swap.setup_instructions {
            for ix in ixs {
                instructions.push(ix.to_ix()?);
            }
        }

        instructions.push(swap.swap_instruction.to_ix()?);

        if let Some(ixs) = swap.cleanup_instructions {
            for ix in ixs {
                instructions.push(ix.to_ix()?);
            }
        }

        // TODO
        // swap.token_ledger_instruction

        let mut address_lookup_table_accounts = vec![];
        if let Some(alt) = swap.address_lookup_table_addresses {
            for alt_addr_str in alt {
                let alt_addr = Pubkey::from_str(&alt_addr_str).unwrap();
                if let Some(alt_acc) = lookup_tables(alt_addr) {
                    address_lookup_table_accounts.push(alt_acc.clone());
                }
            }
        }

        let v0_message = solana_sdk::message::v0::Message::try_compile(
            wallet,
            &instructions,
            address_lookup_table_accounts.as_slice(),
            latest_blockhash,
        )?;
        let message = VersionedMessage::V0(v0_message);

        Ok(message)
    }

    pub async fn quote(
        &self,
        input_mint: Pubkey,
        output_mint: Pubkey,
        amount: u64,
        slippage_bps: u64,
        only_direct_routes: bool,
        max_account: usize,
        swap_mode: SwapMode,
    ) -> anyhow::Result<QuoteResponse> {
        let query_args = vec![
            ("inputMint", input_mint.to_string()),
            ("outputMint", output_mint.to_string()),
            ("amount", format!("{}", amount)),
            ("slippageBps", format!("{}", slippage_bps)),
            ("onlyDirectRoutes", only_direct_routes.to_string()),
            ("maxAccounts", format!("{}", max_account)),
            ("isExactOut", format!("{}", swap_mode.to_string())),
        ];

        let response = self
            .http_client
            .get(format!("{}/quote", self.router_url))
            .query(&query_args)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .context("quote request to autobahn-router")?;

        let quote: QuoteResponse = http_error_handling(response).await.with_context(|| {
            format!("error requesting route between {input_mint} and {output_mint}")
        })?;

        Ok(quote)
    }
}
