use std::collections::HashSet;

use crate::account_write::AccountWrite;
use solana_client::rpc_config::RpcProgramAccountsConfig;
use solana_sdk::account::Account;
use solana_sdk::pubkey::Pubkey;

#[async_trait::async_trait]
pub trait RouterRpcClientTrait: Sync + Send {
    async fn get_account(&mut self, pubkey: &Pubkey) -> anyhow::Result<Account>;

    async fn get_multiple_accounts(
        &mut self,
        pubkeys: &HashSet<Pubkey>,
    ) -> anyhow::Result<Vec<(Pubkey, Account)>>;

    async fn get_program_accounts_with_config(
        &mut self,
        pubkey: &Pubkey,
        config: RpcProgramAccountsConfig,
        compression_enabled: bool,
    ) -> anyhow::Result<Vec<AccountWrite>>;
}

pub struct RouterRpcClient {
    pub rpc: Box<dyn RouterRpcClientTrait + Send + Sync + 'static>,
}

#[async_trait::async_trait]
impl RouterRpcClientTrait for RouterRpcClient {
    async fn get_account(&mut self, pubkey: &Pubkey) -> anyhow::Result<Account> {
        self.rpc.get_account(pubkey).await
    }

    async fn get_multiple_accounts(
        &mut self,
        pubkeys: &HashSet<Pubkey>,
    ) -> anyhow::Result<Vec<(Pubkey, Account)>> {
        self.rpc.get_multiple_accounts(pubkeys).await
    }

    async fn get_program_accounts_with_config(
        &mut self,
        pubkey: &Pubkey,
        config: RpcProgramAccountsConfig,
        compression_enabled: bool,
    ) -> anyhow::Result<Vec<AccountWrite>> {
        self.rpc
            .get_program_accounts_with_config(pubkey, config, compression_enabled)
            .await
    }
}
