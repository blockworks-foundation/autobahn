pub mod test {
    use router_feed_lib::router_rpc_client::RouterRpcClient;
    use router_lib::dex::{
        AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode, Quote,
        SwapInstruction,
    };
    use solana_program::pubkey::Pubkey;
    use std::any::Any;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    pub(crate) struct MockDexIdentifier {
        pub key: Pubkey,
        pub input_mint: Pubkey,
        pub output_mint: Pubkey,
        pub price: f64,
    }

    impl DexEdgeIdentifier for MockDexIdentifier {
        fn key(&self) -> Pubkey {
            self.key
        }

        fn desc(&self) -> String {
            format!("{} - {}", self.input_mint, self.output_mint)
        }

        fn input_mint(&self) -> Pubkey {
            self.input_mint
        }

        fn output_mint(&self) -> Pubkey {
            self.output_mint
        }

        fn accounts_needed(&self) -> usize {
            0
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    pub struct MockDexInterface {}

    pub struct MockEdge {}
    impl DexEdge for MockEdge {
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[async_trait::async_trait]
    impl DexInterface for MockDexInterface {
        async fn initialize(
            _rpc: &mut RouterRpcClient,
            _options: HashMap<String, String>,
        ) -> anyhow::Result<Arc<dyn DexInterface>>
        where
            Self: Sized,
        {
            todo!()
        }

        fn name(&self) -> String {
            todo!()
        }

        fn subscription_mode(&self) -> DexSubscriptionMode {
            todo!()
        }

        fn edges_per_pk(&self) -> HashMap<Pubkey, Vec<Arc<dyn DexEdgeIdentifier>>> {
            todo!()
        }

        fn program_ids(&self) -> HashSet<Pubkey> {
            todo!()
        }

        fn load(
            &self,
            _id: &Arc<dyn DexEdgeIdentifier>,
            _chain_data: &AccountProviderView,
        ) -> anyhow::Result<Arc<dyn DexEdge>> {
            Ok(Arc::new(MockEdge {}) as Arc<dyn DexEdge>)
        }

        fn quote(
            &self,
            id: &Arc<dyn DexEdgeIdentifier>,
            _edge: &Arc<dyn DexEdge>,
            _chain_data: &AccountProviderView,
            in_amount: u64,
        ) -> anyhow::Result<Quote> {
            let id = id.as_any().downcast_ref::<MockDexIdentifier>().unwrap();
            let out_amount = (id.price * in_amount as f64).round() as u64;

            Ok(Quote {
                in_amount,
                out_amount,
                fee_amount: 0,
                fee_mint: id.input_mint,
            })
        }

        fn build_swap_ix(
            &self,
            _id: &Arc<dyn DexEdgeIdentifier>,
            _chain_data: &AccountProviderView,
            _wallet_pk: &Pubkey,
            _in_amount: u64,
            _out_amount: u64,
            _max_slippage_bps: i32,
        ) -> anyhow::Result<SwapInstruction> {
            todo!()
        }

        fn supports_exact_out(&self, _id: &Arc<dyn DexEdgeIdentifier>) -> bool {
            true
        }

        fn quote_exact_out(
            &self,
            id: &Arc<dyn DexEdgeIdentifier>,
            _edge: &Arc<dyn DexEdge>,
            _chain_data: &AccountProviderView,
            out_amount: u64,
        ) -> anyhow::Result<Quote> {
            let id = id.as_any().downcast_ref::<MockDexIdentifier>().unwrap();
            let in_amount = (out_amount as f64 / id.price).round() as u64;

            Ok(Quote {
                in_amount,
                out_amount,
                fee_amount: 0,
                fee_mint: id.input_mint,
            })
        }
    }
}
