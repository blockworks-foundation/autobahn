use async_trait::async_trait;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::address_lookup_table::state::AddressLookupTable;
use solana_program::address_lookup_table::AddressLookupTableAccount;
use solana_program::pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[async_trait]
pub trait AltProvider {
    async fn get_alt(&self, address: Pubkey) -> anyhow::Result<AddressLookupTableAccount>;
}

pub struct RpcAltProvider {
    pub rpc_client: RpcClient,
    pub cache: RwLock<HashMap<Pubkey, (Instant, Option<AddressLookupTableAccount>)>>,
}

#[async_trait]
impl AltProvider for RpcAltProvider {
    async fn get_alt(&self, address: Pubkey) -> anyhow::Result<AddressLookupTableAccount> {
        {
            let locked = self.cache.read().unwrap();
            if let Some((update, hash)) = locked.get(&address) {
                if Instant::now().duration_since(*update) < Duration::from_secs(60 * 5) {
                    if let Some(acc) = hash.clone() {
                        return Ok(acc);
                    } else {
                        anyhow::bail!("address not found");
                    }
                }
            }
        }

        let Ok(alt_data) = self.rpc_client.get_account(&address).await else {
            let mut locked = self.cache.write().unwrap();
            locked.insert(address, (Instant::now(), None));
            anyhow::bail!("failed to load ALT");
        };

        let account = AddressLookupTableAccount {
            key: address,
            addresses: AddressLookupTable::deserialize(alt_data.data.as_slice())
                .unwrap()
                .addresses
                .to_vec(),
        };
        let mut locked = self.cache.write().unwrap();
        locked.insert(address, (Instant::now(), Some(account.clone())));
        Ok(account)
    }
}
