use mango_feeds_connector::chain_data::AccountData;
use router_lib::dex::AccountProvider;
use solana_client::rpc_client::RpcClient;
use solana_program::pubkey::Pubkey;
use solana_sdk::account::AccountSharedData;
use solana_sdk::commitment_config::CommitmentConfig;

pub struct LiveAccountProvider {
    pub rpc_client: RpcClient,
}

impl AccountProvider for LiveAccountProvider {
    fn account(&self, address: &Pubkey) -> anyhow::Result<AccountData> {
        let response = self
            .rpc_client
            .get_account_with_commitment(address, CommitmentConfig::processed())?;
        let account = response
            .value
            .ok_or(anyhow::format_err!("failed to retrieve account"))?;

        Ok(AccountData {
            slot: response.context.slot,
            write_version: 0,
            account: AccountSharedData::from(account),
        })
    }

    fn newest_processed_slot(&self) -> u64 {
        panic!("not implemented")
    }
}
