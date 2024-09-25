use crate::chain_data::ChainDataArcRw;
use mango_feeds_connector::chain_data::AccountData;
use router_feed_lib::router_rpc_client::RouterRpcClient;
use serde_derive::{Deserialize, Serialize};
use solana_sdk::instruction::Instruction;
use solana_sdk::pubkey::Pubkey;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
pub struct SwapInstruction {
    pub instruction: Instruction,
    pub out_pubkey: Pubkey,
    pub out_mint: Pubkey,
    pub in_amount_offset: u16,
    pub cu_estimate: Option<u32>,
}

// TODO: likely this should also have the input and output mint?
#[derive(Clone, Debug)]
pub struct Quote {
    pub in_amount: u64,
    pub out_amount: u64,
    pub fee_amount: u64,
    pub fee_mint: Pubkey,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MixedDexSubscription {
    pub accounts: HashSet<Pubkey>,
    pub programs: HashSet<Pubkey>,
    pub token_accounts_for_owner: HashSet<Pubkey>,
}

#[derive(Clone, Copy, Hash, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum SwapMode {
    #[default]
    ExactIn = 0,
    ExactOut = 1,
}

#[derive(Debug)]
pub struct ParseSwapModeError;

impl FromStr for SwapMode {
    type Err = ParseSwapModeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "ExactIn" {
            Ok(Self::ExactIn)
        } else if s == "ExactOut" {
            Ok(Self::ExactOut)
        } else {
            Err(ParseSwapModeError)
        }
    }
}

impl ToString for SwapMode {
    fn to_string(&self) -> String {
        match &self {
            SwapMode::ExactIn => "ExactIn".to_string(),
            SwapMode::ExactOut => "ExactOut".to_string(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DexSubscriptionMode {
    Disabled, // used to prevent setting up subscriptions for disabled dex adapters
    Accounts(HashSet<Pubkey>),
    Programs(HashSet<Pubkey>),
    Mixed(MixedDexSubscription),
}

impl Display for DexSubscriptionMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            DexSubscriptionMode::Disabled => {
                write!(f, "disabled")
            }
            DexSubscriptionMode::Accounts(a) => {
                write!(f, "gma to {} accounts", a.len())
            }
            DexSubscriptionMode::Programs(p) => {
                write!(f, "gpa to {} programs", p.len())
            }
            DexSubscriptionMode::Mixed(m) => {
                write!(
                    f,
                    "mixed: gpa {} gta {} gma {}",
                    m.programs.len(),
                    m.token_accounts_for_owner.len(),
                    m.accounts.len()
                )
            }
        }
    }
}

pub trait DexEdgeIdentifier: Sync + Send {
    fn key(&self) -> Pubkey;
    fn desc(&self) -> String;
    fn input_mint(&self) -> Pubkey;
    fn output_mint(&self) -> Pubkey;
    fn accounts_needed(&self) -> usize;
    fn as_any(&self) -> &dyn Any;
}

pub trait DexEdge {
    fn as_any(&self) -> &dyn Any;
}

pub trait AccountProvider: Sync + Send {
    fn account(&self, address: &Pubkey) -> anyhow::Result<AccountData>;
    fn newest_processed_slot(&self) -> u64;
}

pub struct ChainDataAccountProvider {
    chain_data: ChainDataArcRw,
}

impl ChainDataAccountProvider {
    pub fn new(chain_data: ChainDataArcRw) -> Self {
        Self { chain_data }
    }
}

impl AccountProvider for ChainDataAccountProvider {
    fn account(&self, address: &Pubkey) -> anyhow::Result<AccountData> {
        self.chain_data.read().unwrap().account(address).cloned()
    }

    fn newest_processed_slot(&self) -> u64 {
        self.chain_data.read().unwrap().newest_processed_slot()
    }
}

pub type AccountProviderView = Arc<dyn AccountProvider>;

#[async_trait::async_trait]
pub trait DexInterface: Sync + Send {
    async fn initialize(
        rpc: &mut RouterRpcClient,
        options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized;

    fn name(&self) -> String;

    fn subscription_mode(&self) -> DexSubscriptionMode;

    fn edges_per_pk(&self) -> HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>;
    fn program_ids(&self) -> HashSet<Pubkey>;

    fn load(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        // TODO: put behind interface so we can adapt for BanksClient
        chain_data: &AccountProviderView,
    ) -> anyhow::Result<Arc<dyn DexEdge>>;

    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote>;

    fn build_swap_ix(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        // TODO: put behind interface so we can adapt for BanksClient
        chain_data: &AccountProviderView,
        wallet_pk: &Pubkey,
        in_amount: u64,
        out_amount: u64,
        max_slippage_bps: i32,
    ) -> anyhow::Result<SwapInstruction>;

    fn supports_exact_out(&self, id: &Arc<dyn DexEdgeIdentifier>) -> bool;

    fn quote_exact_out(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        out_amount: u64,
    ) -> anyhow::Result<Quote>;
    // TODO: list of all program_ids to fetch for testing
}
