use std::collections::HashSet;

use async_trait::async_trait;
use itertools::Itertools;
use solana_account_decoder::UiAccountEncoding;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig};
use solana_sdk::account::Account;
use solana_sdk::pubkey::Pubkey;

use crate::account_write::AccountWrite;
use crate::get_program_account::{
    fetch_multiple_accounts, get_compressed_program_account_rpc,
    get_uncompressed_program_account_rpc,
};
use crate::router_rpc_client::RouterRpcClientTrait;

pub struct RouterRpcWrapper {
    pub rpc: RpcClient,
}

#[async_trait]
impl RouterRpcClientTrait for RouterRpcWrapper {
    async fn get_account(&mut self, pubkey: &Pubkey) -> anyhow::Result<Account> {
        let response = self
            .rpc
            .get_account_with_config(
                pubkey,
                RpcAccountInfoConfig {
                    encoding: Some(UiAccountEncoding::Base64),
                    data_slice: None,
                    commitment: Some(self.rpc.commitment()),
                    min_context_slot: None,
                },
            )
            .await?;

        match response.value {
            None => Err(anyhow::format_err!("missing account")),
            Some(x) => Ok(x),
        }
    }

    async fn get_multiple_accounts(
        &mut self,
        pubkeys: &HashSet<Pubkey>,
    ) -> anyhow::Result<Vec<(Pubkey, Account)>> {
        let keys = pubkeys.iter().cloned().collect_vec();
        let result = fetch_multiple_accounts(&self.rpc, keys.as_slice(), 100).await?;
        Ok(result)
    }

    async fn get_program_accounts_with_config(
        &mut self,
        pubkey: &Pubkey,
        config: RpcProgramAccountsConfig,
    ) -> anyhow::Result<Vec<AccountWrite>> {
        let disable_compressed = std::env::var::<String>("DISABLE_COMRPESSED_GPA".to_string())
            .unwrap_or("false".to_string());
        let disable_compressed: bool = disable_compressed.trim().parse().unwrap();
        if disable_compressed {
            Ok(
                get_uncompressed_program_account_rpc(&self.rpc, &HashSet::from([*pubkey]), config)
                    .await?
                    .1,
            )
        } else {
            Ok(
                get_compressed_program_account_rpc(&self.rpc, &HashSet::from([*pubkey]), config)
                    .await?
                    .1,
            )
        }
    }
}
