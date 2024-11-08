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
    /// Instruction to be executed by the user to swap through an edge.
    pub instruction: Instruction,
    /// Address of the user's associated token account that will receive
    /// the proceeds of the swap after invoking instruction.
    pub out_pubkey: Pubkey,
    /// Mint of the tokens received from the swap.
    pub out_mint: Pubkey,
    /// Byte offset in Instruction.data that the onchain executor program
    /// will use to replace the input amount with the proceeds of the
    /// previous swap before cpi-invocation of this edge.
    /// instruction.data\[in_amount_offset..in_amount_offset+8\] = in_amount
    pub in_amount_offset: u16,
    /// Conservative upper bound estimate of compute cost. If it is too low
    /// transactions will fail, if it is too high, they will confirm slower.
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
    /// Called on router boot, with the options read from the dex adapters's
    /// config file. Can use RPC to initialize. The result contains usually
    /// self. After calling initialize the returned DexInterface needs to be
    /// able to respond the following methods:
    /// - name()
    /// - subscription_mode()
    /// - edges_per_pk()
    /// - program_ids()
    /// - supports_exact_out()
    /// - load()
    async fn initialize(
        rpc: &mut RouterRpcClient,
        options: HashMap<String, String>,
    ) -> anyhow::Result<Arc<dyn DexInterface>>
    where
        Self: Sized;

    fn name(&self) -> String;

    /// Defines the kind of grpc/quic subscription that should be established
    /// to the RPC/Validator to keep this adapter updated. Also defines the
    /// accounts included in a snapshot for simulation tests.
    /// Right now the subscription mode is static per adapter and changes post
    /// initialization have no effect. This might change in the future.
    fn subscription_mode(&self) -> DexSubscriptionMode;

    /// Defines the relationship between account updates and which
    /// DexEdgeIndentifiers will be reloaded. Once a batch of account updates
    /// has been added to ChainData the corresponding edge identifies will be
    /// passed to DexInterface::load().
    /// The identifies are a symbolic representation for edges, meaning they
    /// should not store any mutable data related to the generation of actual
    /// quotes, but merely expose the immutable description of the possibility
    /// to quote a trade from input mint to output mint.
    fn edges_per_pk(&self) -> HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>>;

    /// Defines the programs that should be included in a snapshot for
    /// simulation tests.
    fn program_ids(&self) -> HashSet<Pubkey>;

    /// Initializes an Edge from ChainData (production) or BanksClient (test).
    /// The Edge will be dropped once a new Edge for the same EdgeIndentifier
    /// has been initialized. After calling initialize the DexInterface needs
    /// to be able to respond to quote() and supports_exact_out() calls that
    /// pass this Edge. It can store immutable data locally.
    /// Performance is critical, optimize implementations well.
    fn load(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        chain_data: &AccountProviderView,
    ) -> anyhow::Result<Arc<dyn DexEdge>>;

    /// Calculates the output amount for a given input amount, will be called
    /// multiple times after an edge has been loaded.
    /// Performance is critical, optimize implementations well.
    fn quote(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        in_amount: u64,
    ) -> anyhow::Result<Quote>;

    /// Calculates the input amount for a given output amount, will be called
    /// multiple times after an edge has been loaded.
    /// Performance is critical, optimize implementations well.
    fn quote_exact_out(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        edge: &Arc<dyn DexEdge>,
        chain_data: &AccountProviderView,
        out_amount: u64,
    ) -> anyhow::Result<Quote>;

    /// Returns true, if the edge supports both quote() and quote_exact_out().
    /// Returns false, if the edge only support quote().
    fn supports_exact_out(&self, id: &Arc<dyn DexEdgeIdentifier>) -> bool;

    /// Constructs a description for call-data passed to the executor program.
    /// Once a route has been selected for the end-user to swap through, the
    /// router will invoke DexInterface::build_swap_ix for every edge in the
    /// route with with it's DexEdgeIdentifier rather than the initialized
    /// DexEdge. The build_swap_ix implementation should use the most recent
    /// data possible to avoid latency between account update, load & quote.
    /// Exact-out is only used during route selection, onchain execution is
    /// always using exact input amounts that get adjusted between edge
    /// cpi-invocations using the in_amount_offset.
    fn build_swap_ix(
        &self,
        id: &Arc<dyn DexEdgeIdentifier>,
        chain_data: &AccountProviderView,
        wallet_pk: &Pubkey,
        in_amount: u64,
        out_amount: u64,
        max_slippage_bps: i32,
    ) -> anyhow::Result<SwapInstruction>;
}
