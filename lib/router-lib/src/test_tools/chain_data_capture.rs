use crate::dex::{AccountProvider, AccountProviderView};
use mango_feeds_connector::chain_data::AccountData;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::RwLock;

pub struct ChainDataCaptureAccountProvider {
    chain_data: AccountProviderView,
    pub accounts: RwLock<HashMap<Pubkey, AccountData>>,
}

impl AccountProvider for ChainDataCaptureAccountProvider {
    fn account(&self, address: &Pubkey) -> anyhow::Result<AccountData> {
        let result = self.chain_data.account(address)?;

        self.accounts
            .write()
            .unwrap()
            .insert(*address, result.clone());

        Ok(result)
    }

    fn newest_processed_slot(&self) -> u64 {
        self.chain_data.newest_processed_slot()
    }
}

impl ChainDataCaptureAccountProvider {
    pub fn new(chain_data: AccountProviderView) -> Self {
        Self {
            chain_data,
            accounts: Default::default(),
        }
    }
}
