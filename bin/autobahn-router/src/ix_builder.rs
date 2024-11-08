use crate::routing_types::{Route, RouteStep};
use crate::swap::Swap;
use anchor_lang::Id;
use anchor_spl::associated_token::get_associated_token_address;
use anchor_spl::token::Token;
use autobahn_executor::swap_ix::generate_swap_ix_data;
use router_lib::dex::{AccountProviderView, SwapInstruction, SwapMode};
use solana_program::instruction::Instruction;
use solana_program::pubkey::Pubkey;
use std::str::FromStr;

const CU_PER_HOP_DEFAULT: u32 = 80_000;
const CU_BASE: u32 = 150_000;

pub trait SwapStepInstructionBuilder {
    fn build_ix(
        &self,
        wallet_pk: &Pubkey,
        step: &RouteStep,
        max_slippage_bps: i32,
        swap_mode: SwapMode,
        other_amount: u64,
    ) -> anyhow::Result<SwapInstruction>; // TODO handle multi hop from same edge ?
}

pub trait SwapInstructionsBuilder {
    fn build_ixs(
        &self,
        wallet_pk: &Pubkey,
        route: &Route,
        wrap_and_unwrap_sol: bool,
        auto_create_out: bool,
        max_slippage_bps: i32,
        other_amount_threshold: u64,
        swap_mode: SwapMode,
    ) -> anyhow::Result<Swap>;
}

pub struct SwapStepInstructionBuilderImpl {
    pub chain_data: AccountProviderView,
}

impl SwapStepInstructionBuilder for SwapStepInstructionBuilderImpl {
    fn build_ix(
        &self,
        wallet_pk: &Pubkey,
        step: &RouteStep,
        max_slippage_bps: i32,
        swap_mode: SwapMode,
        other_amount: u64,
    ) -> anyhow::Result<SwapInstruction> {
        let in_amount = match swap_mode {
            SwapMode::ExactIn => step.in_amount,
            SwapMode::ExactOut => other_amount,
        };

        step.edge.build_swap_ix(
            &self.chain_data,
            wallet_pk,
            in_amount,
            step.out_amount,
            max_slippage_bps,
        )
    }
}

pub struct SwapInstructionsBuilderImpl<T: SwapStepInstructionBuilder> {
    ix_builder: T,
    router_version: u8,
}

impl<T: SwapStepInstructionBuilder> SwapInstructionsBuilderImpl<T> {
    pub fn new(ix_builder: T, router_version: u8) -> SwapInstructionsBuilderImpl<T> {
        Self {
            ix_builder,
            router_version,
        }
    }
}

impl<T: SwapStepInstructionBuilder> SwapInstructionsBuilder for SwapInstructionsBuilderImpl<T> {
    fn build_ixs(
        &self,
        wallet_pk: &Pubkey,
        route: &Route,
        auto_wrap_sol: bool,
        auto_create_out: bool,
        max_slippage_bps: i32,
        other_amount_threshold: u64,
        swap_mode: SwapMode,
    ) -> anyhow::Result<Swap> {
        if route.steps.len() == 0 {
            anyhow::bail!("Can't generate instructions for empty route");
        }

        let mut setup_instructions = vec![];
        let mut cleanup_instructions = vec![];

        let exec_program_id: Pubkey = autobahn_executor::id();
        let sol_mint: Pubkey =
            Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();

        if auto_wrap_sol && route.input_mint == sol_mint {
            Self::create_ata(&wallet_pk, &mut setup_instructions, &sol_mint);
            let wsol_account = get_associated_token_address(wallet_pk, &sol_mint);

            let in_amount = match swap_mode {
                SwapMode::ExactIn => route.in_amount,
                SwapMode::ExactOut => other_amount_threshold,
            };

            setup_instructions.push(solana_program::system_instruction::transfer(
                &wallet_pk,
                &wsol_account,
                in_amount,
            ));
            setup_instructions.push(anchor_spl::token::spl_token::instruction::sync_native(
                &Token::id(),
                &wsol_account,
            )?);

            Self::close_wsol_ata(&wallet_pk, &mut cleanup_instructions, &wsol_account)?;
        }

        // We don't really care about Orca/Raydium/Openbook min out amount
        //   since we are checking it at the end of execution anyway
        //   .. and it prevent using the "overquote" heuristic
        let max_slippage_for_hop_bps = max_slippage_bps * 2;

        let swap_instructions = route
            .steps
            .iter()
            .map(|x| {
                self.ix_builder.build_ix(
                    wallet_pk,
                    x,
                    max_slippage_for_hop_bps,
                    swap_mode,
                    other_amount_threshold,
                )
            })
            .collect::<anyhow::Result<Vec<SwapInstruction>>>()?;

        let mut cu_estimate = CU_BASE;

        for step in &swap_instructions {
            if auto_create_out || (step.out_mint == sol_mint && auto_wrap_sol) {
                Self::create_ata(&wallet_pk, &mut setup_instructions, &step.out_mint);
                cu_estimate += 12_000;
            }

            if step.out_mint == sol_mint && auto_wrap_sol {
                let wsol_account = get_associated_token_address(wallet_pk, &sol_mint);
                Self::close_wsol_ata(&wallet_pk, &mut cleanup_instructions, &wsol_account)?;
                cu_estimate += 12_000;
            }

            cu_estimate += step.cu_estimate.unwrap_or(CU_PER_HOP_DEFAULT);
        }

        let (instructions, in_out): (Vec<_>, Vec<_>) = swap_instructions
            .into_iter()
            .map(|x| (x.instruction, (x.in_amount_offset, x.out_pubkey)))
            .unzip();
        let (in_amount_offsets, out_account_pubkeys): (Vec<_>, Vec<_>) = in_out.into_iter().unzip();

        let min_out_amount = match swap_mode {
            SwapMode::ExactIn => other_amount_threshold,
            SwapMode::ExactOut => route.out_amount,
        };

        let swap_instruction = generate_swap_ix_data(
            min_out_amount,
            instructions.as_slice(),
            in_amount_offsets.as_slice(),
            get_associated_token_address(&wallet_pk, &route.input_mint),
            out_account_pubkeys.as_slice(),
            exec_program_id,
            self.router_version,
        );

        Ok(Swap {
            setup_instructions,
            swap_instruction,
            cleanup_instructions,
            cu_estimate,
        })
    }
}

impl<T: SwapStepInstructionBuilder> SwapInstructionsBuilderImpl<T> {
    fn close_wsol_ata(
        wallet_pk: &&Pubkey,
        cleanup_instructions: &mut Vec<Instruction>,
        wsol_account: &Pubkey,
    ) -> anyhow::Result<()> {
        cleanup_instructions.push(anchor_spl::token::spl_token::instruction::close_account(
            &Token::id(),
            &wsol_account,
            &wallet_pk,
            &wallet_pk,
            &[&wallet_pk],
        )?);
        Ok(())
    }

    fn create_ata(wallet_pk: &&Pubkey, setup_instructions: &mut Vec<Instruction>, mint: &Pubkey) {
        setup_instructions.push(
            spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                &wallet_pk,
                &wallet_pk,
                &mint,
                &Token::id(),
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edge::Edge;
    use crate::test_utils::*;
    use router_feed_lib::router_rpc_client::RouterRpcClient;
    use router_lib::dex::{
        AccountProviderView, DexEdge, DexEdgeIdentifier, DexInterface, DexSubscriptionMode, Quote,
    };
    use std::any::Any;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;
    use test_case::test_case;

    struct MockSwapStepInstructionBuilder {}
    struct MockDex {}
    struct MockId {}

    impl DexEdgeIdentifier for MockId {
        fn key(&self) -> Pubkey {
            todo!()
        }

        fn desc(&self) -> String {
            todo!()
        }

        fn input_mint(&self) -> Pubkey {
            todo!()
        }

        fn output_mint(&self) -> Pubkey {
            todo!()
        }

        fn accounts_needed(&self) -> usize {
            todo!()
        }

        fn as_any(&self) -> &dyn Any {
            todo!()
        }
    }

    #[async_trait::async_trait]
    impl DexInterface for MockDex {
        async fn initialize(
            _rpc: &mut RouterRpcClient,
            _options: HashMap<String, String>,
            _enable_compression: bool,
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
            todo!()
        }

        fn quote(
            &self,
            _id: &Arc<dyn DexEdgeIdentifier>,
            _edge: &Arc<dyn DexEdge>,
            _chain_data: &AccountProviderView,
            _in_amount: u64,
        ) -> anyhow::Result<Quote> {
            todo!()
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
            todo!()
        }

        fn quote_exact_out(
            &self,
            _id: &Arc<dyn DexEdgeIdentifier>,
            _edge: &Arc<dyn DexEdge>,
            _chain_data: &AccountProviderView,
            _out_amount: u64,
        ) -> anyhow::Result<Quote> {
            todo!()
        }
    }

    impl SwapStepInstructionBuilder for MockSwapStepInstructionBuilder {
        fn build_ix(
            &self,
            _wallet_pk: &Pubkey,
            step: &RouteStep,
            _max_slippage_bps: i32,
            _swap_mode: SwapMode,
            _other_amount: u64,
        ) -> anyhow::Result<SwapInstruction> {
            Ok(SwapInstruction {
                instruction: Instruction {
                    program_id: Default::default(),
                    accounts: vec![],
                    data: vec![],
                },
                out_pubkey: Default::default(),
                out_mint: step.edge.output_mint,
                in_amount_offset: 0,
                cu_estimate: None,
            })
        }
    }

    #[test]
    fn should_fail_if_there_is_no_step() {
        let builder = SwapInstructionsBuilderImpl::new(MockSwapStepInstructionBuilder {}, 0);
        let wallet = 0.to_pubkey();

        let ixs = builder.build_ixs(
            &wallet,
            &Route {
                input_mint: 1.to_pubkey(),
                output_mint: 2.to_pubkey(),
                in_amount: 1000,
                out_amount: 2000,
                price_impact_bps: 0,
                steps: vec![],
                slot: 0,
                accounts: None,
            },
            false,
            false,
            0,
            0,
            SwapMode::ExactIn,
        );

        assert!(ixs.is_err());
    }

    #[test]
    fn should_fail_if_there_is_no_step_exact_out() {
        let builder = SwapInstructionsBuilderImpl::new(MockSwapStepInstructionBuilder {}, 0);
        let wallet = 0.to_pubkey();

        let ixs = builder.build_ixs(
            &wallet,
            &Route {
                input_mint: 1.to_pubkey(),
                output_mint: 2.to_pubkey(),
                in_amount: 1000,
                out_amount: 2000,
                price_impact_bps: 0,
                steps: vec![],
                slot: 0,
                accounts: None,
            },
            false,
            false,
            0,
            0,
            SwapMode::ExactOut,
        );

        assert!(ixs.is_err());
    }

    #[test_case(true, false, false, 3, 1 ; "when in is SOL")]
    #[test_case(false, true, false, 1, 1 ; "when out is SOL")]
    #[test_case(false, false, false, 0, 0 ; "when none is SOL")]
    #[test_case(true, false, true, 3, 1 ; "when in is SOL exact out")]
    #[test_case(false, true, true, 1, 1 ; "when out is SOL exact out")]
    #[test_case(false, false, true, 0, 0 ; "when none is SOL exact out")]
    fn should_add_wrapping_unwrapping_ix(
        in_mint_is_sol: bool,
        out_mint_is_sol: bool,
        is_exactout: bool,
        expected_setup_len: usize,
        expected_cleanup_len: usize,
    ) {
        let builder = SwapInstructionsBuilderImpl::new(MockSwapStepInstructionBuilder {}, 0);
        let wallet = 0.to_pubkey();
        let sol = Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap();

        let in_mint = if in_mint_is_sol { sol } else { 1.to_pubkey() };
        let out_mint = if out_mint_is_sol { sol } else { 2.to_pubkey() };

        let swap_mode = if is_exactout {
            SwapMode::ExactOut
        } else {
            SwapMode::ExactIn
        };

        let ixs = builder
            .build_ixs(
                &wallet,
                &Route {
                    input_mint: in_mint,
                    output_mint: out_mint,
                    in_amount: 1000,
                    out_amount: 2000,
                    price_impact_bps: 0,
                    slot: 0,
                    accounts: None,
                    steps: vec![RouteStep {
                        edge: Arc::new(Edge {
                            input_mint: in_mint,
                            output_mint: out_mint,
                            dex: Arc::new(MockDex {}),
                            id: Arc::new(MockId {}),
                            accounts_needed: 1,
                            state: Default::default(),
                        }),
                        in_amount: 1000,
                        out_amount: 2000,
                        fee_amount: 0,
                        fee_mint: Default::default(),
                    }],
                },
                true,
                false,
                0,
                0,
                swap_mode,
            )
            .unwrap();

        assert_eq!(ixs.setup_instructions.len(), expected_setup_len);
        assert_eq!(ixs.cleanup_instructions.len(), expected_cleanup_len);
    }

    #[test]
    fn should_build_ixs() {
        let builder = SwapInstructionsBuilderImpl::new(MockSwapStepInstructionBuilder {}, 0);
        let wallet = 0.to_pubkey();

        let ixs = builder
            .build_ixs(
                &wallet,
                &Route {
                    input_mint: 1.to_pubkey(),
                    output_mint: 2.to_pubkey(),
                    in_amount: 1000,
                    out_amount: 2000,
                    price_impact_bps: 0,
                    slot: 0,
                    accounts: None,
                    steps: vec![RouteStep {
                        edge: Arc::new(Edge {
                            input_mint: 1.to_pubkey(),
                            output_mint: 2.to_pubkey(),
                            accounts_needed: 1,
                            dex: Arc::new(MockDex {}),
                            id: Arc::new(MockId {}),
                            state: Default::default(),
                        }),
                        in_amount: 1000,
                        out_amount: 2000,
                        fee_amount: 0,
                        fee_mint: Default::default(),
                    }],
                },
                false,
                false,
                0,
                0,
                SwapMode::ExactIn,
            )
            .unwrap();

        assert_eq!(0, ixs.setup_instructions.len());
        assert_eq!(0, ixs.cleanup_instructions.len());
    }

    #[test]
    fn should_build_ixs_exact_out() {
        let builder = SwapInstructionsBuilderImpl::new(MockSwapStepInstructionBuilder {}, 0);
        let wallet = 0.to_pubkey();

        let ixs = builder
            .build_ixs(
                &wallet,
                &Route {
                    input_mint: 1.to_pubkey(),
                    output_mint: 2.to_pubkey(),
                    in_amount: 1000,
                    out_amount: 2000,
                    price_impact_bps: 0,
                    slot: 0,
                    accounts: None,
                    steps: vec![RouteStep {
                        edge: Arc::new(Edge {
                            input_mint: 1.to_pubkey(),
                            output_mint: 2.to_pubkey(),
                            accounts_needed: 1,
                            dex: Arc::new(MockDex {}),
                            id: Arc::new(MockId {}),
                            state: Default::default(),
                        }),
                        in_amount: 1000,
                        out_amount: 2000,
                        fee_amount: 0,
                        fee_mint: Default::default(),
                    }],
                },
                false,
                false,
                0,
                0,
                SwapMode::ExactOut,
            )
            .unwrap();

        assert_eq!(0, ixs.setup_instructions.len());
        assert_eq!(0, ixs.cleanup_instructions.len());
    }
}
