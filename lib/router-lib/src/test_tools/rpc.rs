use crate::chain_data::ChainDataArcRw;
use crate::dex::{DexInterface, DexSubscriptionMode};
use mango_feeds_connector::chain_data::{AccountData, ChainData};
use router_feed_lib::account_write::AccountWrite;
use router_feed_lib::router_rpc_client::{RouterRpcClient, RouterRpcClientTrait};
use router_feed_lib::router_rpc_wrapper::RouterRpcWrapper;
use router_test_lib::serialize;
use serde::{Deserialize, Serialize};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_client::rpc_config::RpcProgramAccountsConfig;
use solana_client::rpc_filter::{Memcmp, RpcFilterType};
use solana_sdk::account::{Account, AccountSharedData};
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[derive(Serialize, Deserialize)]
struct RpcDump {
    pub cache: HashMap<Pubkey, Option<Account>>,
    pub cache_gpa: HashMap<(Pubkey, String), Option<Vec<AccountWrite>>>,
}

pub struct DumpRpcClient {
    dump: RpcDump,
    rpc: RouterRpcClient,
    path: String,
    chain_data: Arc<RwLock<ChainData>>,
}

pub struct ReplayerRpcClient {
    dump: RpcDump,
}

#[async_trait::async_trait]
impl RouterRpcClientTrait for ReplayerRpcClient {
    async fn get_account(&mut self, pubkey: &Pubkey) -> anyhow::Result<Account> {
        match self.dump.cache.get(pubkey).unwrap() {
            Some(x) => Ok(x.clone()),
            None => anyhow::bail!("Invalid account"),
        }
    }

    async fn get_multiple_accounts(
        &mut self,
        pubkeys: &HashSet<Pubkey>,
    ) -> anyhow::Result<Vec<(Pubkey, Account)>> {
        let mut result = vec![];

        for x in pubkeys {
            let acc = self.dump.cache.get(x);
            if acc.is_some() {
                let acc = acc.unwrap();
                if acc.is_some() {
                    result.push((*x, acc.clone().unwrap()));
                }
            }
        }

        Ok(result)
    }

    async fn get_program_accounts_with_config(
        &mut self,
        pubkey: &Pubkey,
        config: RpcProgramAccountsConfig,
    ) -> anyhow::Result<Vec<AccountWrite>> {
        let config_serialized = serde_json::to_string(&config)?;
        match self
            .dump
            .cache_gpa
            .get(&(*pubkey, config_serialized))
            .unwrap()
        {
            Some(x) => Ok(x.clone()),
            None => anyhow::bail!("Invalid gpa"),
        }
    }

    fn is_gpa_compression_enabled(&self) -> bool {
        false
    }
}

#[async_trait::async_trait]
impl RouterRpcClientTrait for DumpRpcClient {
    async fn get_account(&mut self, pubkey: &Pubkey) -> anyhow::Result<Account> {
        match self.rpc.get_account(pubkey).await {
            Ok(r) => {
                insert_into_arc_chain_data(&self.chain_data, *pubkey, r.clone());
                self.dump.cache.insert(*pubkey, Some(r.clone()));
                Ok(r)
            }
            Err(e) => {
                self.dump.cache.insert(*pubkey, None);
                Err(e)
            }
        }
    }

    async fn get_multiple_accounts(
        &mut self,
        pubkeys: &HashSet<Pubkey>,
    ) -> anyhow::Result<Vec<(Pubkey, Account)>> {
        match self.rpc.get_multiple_accounts(pubkeys).await {
            Ok(r) => {
                for acc in &r {
                    insert_into_arc_chain_data(&self.chain_data, acc.0, acc.1.clone());
                }

                for x in &r {
                    self.dump.cache.insert(x.0, Some(x.1.clone()));
                }

                Ok(r)
            }
            Err(e) => Err(e),
        }
    }

    async fn get_program_accounts_with_config(
        &mut self,
        pubkey: &Pubkey,
        config: RpcProgramAccountsConfig,
    ) -> anyhow::Result<Vec<AccountWrite>> {
        let config_serialized = serde_json::to_string(&config)?;
        match self
            .rpc
            .get_program_accounts_with_config(pubkey, config.clone())
            .await
        {
            Ok(r) => {
                for acc in &r {
                    insert_into_arc_chain_data(
                        &self.chain_data,
                        acc.pubkey,
                        account_from_a_write(acc.clone()),
                    );
                }

                self.dump
                    .cache_gpa
                    .insert((*pubkey, config_serialized), Some(r.clone()));
                Ok(r)
            }
            Err(e) => {
                self.dump
                    .cache_gpa
                    .insert((*pubkey, config_serialized), None);
                Err(e)
            }
        }
    }

    fn is_gpa_compression_enabled(&self) -> bool {
        false
    }
}

impl Drop for DumpRpcClient {
    fn drop(&mut self) {
        serialize::serialize_to_file(&self.dump, &self.path);
    }
}

pub fn rpc_dumper_client(url: String, out_path: &str) -> (RouterRpcClient, ChainDataArcRw) {
    let disable_compressed_gpa =
        std::env::var::<String>("DISABLE_COMRPESSED_GPA".to_string()).unwrap_or("true".to_string());
    let gpa_compression_enabled: bool = !disable_compressed_gpa.trim().parse::<bool>().unwrap();

    let chain_data = ChainDataArcRw::new(RwLock::new(ChainData::new()));
    let rpc_client = RouterRpcClient {
        rpc: Box::new(DumpRpcClient {
            dump: RpcDump {
                cache: Default::default(),
                cache_gpa: Default::default(),
            },
            chain_data: chain_data.clone(),
            rpc: RouterRpcClient {
                rpc: Box::new(RouterRpcWrapper {
                    rpc: RpcClient::new_with_timeout_and_commitment(
                        url,
                        Duration::from_secs(60 * 20),
                        CommitmentConfig::finalized(),
                    ),
                    gpa_compression_enabled,
                }),
                gpa_compression_enabled,
            },
            path: out_path.to_string(),
        }),
        gpa_compression_enabled,
    };

    (rpc_client, chain_data)
}

pub fn rpc_replayer_client(in_path: &str) -> (RouterRpcClient, ChainDataArcRw) {
    // note that file might need to go into "bin/autobahn-router" folder!
    assert!(
        PathBuf::from_str(in_path).unwrap().exists(),
        "rpc replayer file not found: {}",
        in_path
    );

    let dump = serialize::deserialize_from_file::<RpcDump>(&in_path.to_string()).unwrap();

    let mut chain_data = ChainData::new();

    for (pubkey, account) in &dump.cache {
        if let Some(account) = account {
            insert_into_chain_data(&mut chain_data, *pubkey, account.clone());
        }
    }
    for x in &dump.cache_gpa {
        if let Some(accounts) = x.1 {
            for account in accounts {
                insert_into_chain_data(
                    &mut chain_data,
                    account.pubkey,
                    account_from_a_write(account.clone()),
                );
            }
        }
    }

    let rpc = ReplayerRpcClient { dump };
    let replayer = RouterRpcClient {
        rpc: Box::new(rpc),
        gpa_compression_enabled: false,
    };
    let chain_data = ChainDataArcRw::new(RwLock::new(chain_data));

    (replayer, chain_data)
}

fn account_from_a_write(account: AccountWrite) -> Account {
    Account {
        lamports: account.lamports,
        data: account.data,
        owner: account.owner,
        executable: account.executable,
        rent_epoch: account.rent_epoch,
    }
}

fn insert_into_chain_data(chain_data: &mut ChainData, key: Pubkey, account: Account) {
    chain_data.update_account(
        key,
        AccountData {
            slot: 0,
            write_version: 0,
            account: AccountSharedData::from(account),
        },
    )
}

fn insert_into_arc_chain_data(chain_data: &ChainDataArcRw, key: Pubkey, account: Account) {
    chain_data.write().unwrap().update_account(
        key,
        AccountData {
            slot: 0,
            write_version: 0,
            account: AccountSharedData::from(account),
        },
    )
}

pub async fn load_programs(
    _rpc_client: &mut RouterRpcClient,
    dex: Arc<dyn DexInterface>,
) -> anyhow::Result<()> {
    let _program_ids = dex.program_ids();
    // TODO ?
    Ok(())
}

pub async fn load_subscriptions(
    rpc_client: &mut RouterRpcClient,
    dex: Arc<dyn DexInterface>,
) -> anyhow::Result<()> {
    match dex.subscription_mode() {
        DexSubscriptionMode::Disabled => {}
        DexSubscriptionMode::Accounts(accounts) => {
            rpc_client.get_multiple_accounts(&accounts).await?;
        }
        DexSubscriptionMode::Programs(program_ids) => {
            for program in program_ids {
                rpc_client
                    .get_program_accounts_with_config(
                        &program,
                        RpcProgramAccountsConfig {
                            filters: None,
                            account_config: Default::default(),
                            with_context: Some(true),
                        },
                    )
                    .await?;
            }
        }
        DexSubscriptionMode::Mixed(m) => {
            rpc_client.get_multiple_accounts(&m.accounts).await?;

            for program in m.programs {
                rpc_client
                    .get_program_accounts_with_config(
                        &program,
                        RpcProgramAccountsConfig {
                            filters: None,
                            account_config: Default::default(),
                            with_context: Some(true),
                        },
                    )
                    .await?;
            }

            for owner in m.token_accounts_for_owner {
                let filters = Some(vec![
                    RpcFilterType::DataSize(165),
                    RpcFilterType::Memcmp(Memcmp::new_base58_encoded(
                        32,
                        owner.to_bytes().as_slice(),
                    )),
                ]);
                rpc_client
                    .get_program_accounts_with_config(
                        &Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap(),
                        RpcProgramAccountsConfig {
                            filters,
                            account_config: Default::default(),
                            with_context: Some(true),
                        },
                    )
                    .await?;
            }
        }
    }

    Ok(())
}
