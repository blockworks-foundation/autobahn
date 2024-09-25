#![allow(dead_code)]

use std::cell::RefCell;
use std::str::FromStr;
use std::sync::Arc;

use solana_program::{program_option::COption, program_pack::Pack};
use solana_program_test::*;
use solana_sdk::pubkey::Pubkey;
use spl_token::{state::*, *};

pub use cookies::*;
pub use solana::*;
pub use utils::*;

pub mod cookies;
pub mod solana;
pub mod utils;

pub struct TestContextBuilder {
    test: ProgramTest,
    mint0: Pubkey,
}

impl TestContextBuilder {
    pub fn new() -> Self {
        // We need to intercept logs to capture program log output
        let log_filter = "solana_rbpf=trace,\
                    solana_runtime::message_processor=debug,\
                    solana_runtime::system_instruction_processor=trace,\
                    solana_program_test=info";
        let env_logger =
            env_logger::Builder::from_env(env_logger::Env::new().default_filter_or(log_filter))
                .format_timestamp_nanos()
                .build();
        let _ = log::set_boxed_logger(Box::new(env_logger));

        let mut test = ProgramTest::new("autobahn-executor", autobahn_executor::id(), None);

        // // intentionally set to as tight as possible, to catch potential problems early
        // test.set_compute_max_units(80000);

        Self {
            test,
            mint0: Pubkey::new_unique(),
        }
    }

    pub fn test(&mut self) -> &mut ProgramTest {
        &mut self.test
    }

    pub fn create_mints(&mut self) -> Vec<MintCookie> {
        let mut mints: Vec<MintCookie> = vec![];
        mints
    }

    pub fn create_users(&mut self, mints: &[MintCookie]) -> Vec<UserCookie> {
        let num_users = 3;
        let mut users = Vec::new();
        users
    }

    pub async fn start_default(mut self) -> TestContext {
        let mints = self.create_mints();
        let users = self.create_users(&mints);

        let solana = self.start().await;

        TestContext {
            solana: solana.clone(),
            mints,
            users,
        }
    }

    pub async fn start(self) -> Arc<SolanaCookie> {
        let mut context = self.test.start_with_context().await;
        let rent = context.banks_client.get_rent().await.unwrap();

        let solana = Arc::new(SolanaCookie {
            context: RefCell::new(context),
            rent,
            last_transaction_log: RefCell::new(vec![]),
        });

        solana
    }
}

pub struct TestContext {
    pub solana: Arc<SolanaCookie>,
    pub mints: Vec<MintCookie>,
    pub users: Vec<UserCookie>,
}

impl TestContext {
    pub async fn new() -> Self {
        TestContextBuilder::new().start_default().await
    }
}
