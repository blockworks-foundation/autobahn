use autobahn_executor;
use autobahn_executor::process_instruction;
use bonfida_test_utils::{ProgramTestContextExt, ProgramTestExt};
use solana_program::pubkey::Pubkey;
use solana_program_test::ProgramTestContext;
use solana_sdk::account::AccountSharedData;
use std::collections::HashMap;

use {
    solana_program_test::{processor, ProgramTest},
    solana_sdk::signer::{keypair::Keypair, Signer},
};
pub struct TestData {
    pub mock_swap_program_id: Pubkey,
    pub users: HashMap<String, Keypair>,
    pub mint_authority: Keypair,
    pub mint_keys: HashMap<String, Pubkey>,
    pub users_ata: HashMap<String, Pubkey>,
    pub program_test_context: ProgramTestContext,
}

impl TestData {
    pub async fn new(users: &[String], mints: &[String], balances: HashMap<String, u64>) -> Self {
        ////
        // Setup
        ////
        let mut program_test = ProgramTest::new(
            "autobahn_executor",
            autobahn_executor::id(),
            processor!(process_instruction),
        );

        let mock_swap_program_id = Keypair::new();
        program_test.add_program("mock_swap", mock_swap_program_id.pubkey(), None);
        program_test.set_compute_max_units(300_000);

        let mint_authority = Keypair::new();
        let users: HashMap<String, Keypair> =
            users.iter().map(|x| (x.clone(), Keypair::new())).collect();
        let mint_keys: HashMap<String, Pubkey> = mints
            .iter()
            .map(|x| {
                (
                    x.clone(),
                    program_test.add_mint(None, 6, &mint_authority.pubkey()).0,
                )
            })
            .collect();

        ////
        // Create test context
        ////
        let mut program_test_context = program_test.start_with_context().await;

        // init sol balances
        for user in &users {
            program_test_context.set_account(
                &user.1.pubkey(),
                &AccountSharedData::new(1_000_000_000, 0, &solana_program::system_program::ID),
            );
        }

        // Initialize user token accounts:
        let mut users_ata: HashMap<String, Pubkey> = HashMap::new();
        for user in &users {
            for mint in &mint_keys {
                let key = format!("{}:{}", user.0, mint.0);
                users_ata.insert(
                    key.clone(),
                    initialize_ata_with_balance(
                        *mint.1,
                        &mut program_test_context,
                        balances.get(&key).map(|x| *x).unwrap_or(0),
                        user.1.pubkey(),
                        &mint_authority,
                    )
                    .await,
                );
            }
        }

        Self {
            program_test_context,
            mock_swap_program_id: mock_swap_program_id.pubkey(),
            mint_authority,
            mint_keys,
            users,
            users_ata,
        }
    }
}

pub async fn initialize_ata_with_balance(
    mint: Pubkey,
    prg_test_ctx: &mut ProgramTestContext,
    balance: u64,
    owner: Pubkey,
    authority: &Keypair,
) -> Pubkey {
    let ata = prg_test_ctx
        .initialize_token_accounts(mint, &[owner])
        .await
        .unwrap()[0];
    prg_test_ctx
        .mint_tokens(authority, &mint, &ata, balance)
        .await
        .unwrap();
    ata
}

pub async fn assert_ata_balance(prg_test_ctx: &mut ProgramTestContext, ata: Pubkey, i: u64) {
    let alice_token_account_balance = prg_test_ctx.get_token_account(ata).await.unwrap().amount;
    assert_eq!(alice_token_account_balance, i);
}

pub async fn assert_ata_balances(test: &mut TestData, expected_balances: Vec<(&str, u64)>) {
    for (user_ata, expected_balance) in expected_balances {
        assert_ata_balance(
            &mut test.program_test_context,
            test.users_ata[user_ata],
            expected_balance,
        )
        .await;
    }
}
