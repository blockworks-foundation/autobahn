#![cfg(feature = "test-bpf")]

use crate::utils::*;
use autobahn_executor;
use autobahn_executor::Instructions;
use bonfida_test_utils::ProgramTestContextExt;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::pubkey::Pubkey;
use solana_program::system_program;
use solana_sdk::signer::Signer;
use std::collections::HashMap;

#[tokio::test]
async fn should_charge_platform_fee() {
    for pct in [0, 50, 100, 150] {
        println!("With {pct} %");

        let mut test: TestData = TestData::new(
            &["Alice".to_string(), "Platform".to_string()],
            &["USDC".to_string()],
            HashMap::from([
                ("Alice:USDC".to_string(), 100_000_000),
                ("Platform:USDC".to_string(), 100_000_000),
            ]),
        )
        .await;

        let data = build_fee_ix_data(2_000_000u64, pct);

        let ix = Instruction {
            program_id: autobahn_executor::id(),
            accounts: vec![
                AccountMeta::new_readonly(spl_token::ID, false),
                AccountMeta::new(test.users_ata["Alice:USDC"], false),
                AccountMeta::new(test.users_ata["Platform:USDC"], false),
                AccountMeta::new_readonly(test.users["Alice"].pubkey(), true),
            ],
            data,
        };

        test.program_test_context
            .sign_send_instructions(&[ix], &[&test.users["Alice"]])
            .await
            .unwrap();

        assert_ata_balances(
            &mut test,
            vec![("Alice:USDC", 98_000_000), ("Platform:USDC", 102_000_000)],
        )
        .await;
    }
}

#[tokio::test]
async fn should_split_fee_between_platform_and_referrer() {
    for (pct, expected_alice, expected_bob, expected_platform) in [
        (80, 98_000_000, 100_400_000, 101_600_000),
        (50, 98_000_000, 101_000_000, 101_000_000),
        (120, 98_000_000, 100_000_000, 102_000_000),
        (0, 98_000_000, 102_000_000, 100_000_000),
    ] {
        println!("With {pct} %");

        let mut test: TestData = TestData::new(
            &[
                "Alice".to_string(),
                "Bob".to_string(),
                "Platform".to_string(),
            ],
            &["USDC".to_string()],
            HashMap::from([
                ("Alice:USDC".to_string(), 100_000_000),
                ("Bob:USDC".to_string(), 100_000_000),
                ("Platform:USDC".to_string(), 100_000_000),
            ]),
        )
        .await;

        let data = build_fee_ix_data(2_000_000u64, pct);

        let ix = Instruction {
            program_id: autobahn_executor::id(),
            accounts: vec![
                AccountMeta::new_readonly(spl_token::ID, false),
                AccountMeta::new(test.users_ata["Alice:USDC"], false),
                AccountMeta::new(test.users_ata["Platform:USDC"], false),
                AccountMeta::new_readonly(test.users["Alice"].pubkey(), true),
                AccountMeta::new(test.users_ata["Bob:USDC"], false),
            ],
            data,
        };

        test.program_test_context
            .sign_send_instructions(&[ix], &[&test.users["Alice"]])
            .await
            .unwrap();

        assert_ata_balances(
            &mut test,
            vec![
                ("Alice:USDC", expected_alice),
                ("Bob:USDC", expected_bob),
                ("Platform:USDC", expected_platform),
            ],
        )
        .await;
    }
}

#[tokio::test]
async fn should_work_with_referrer_derived_address() {
    for (pct, expected_alice, expected_bob, expected_derived_bob, expected_platform) in [
        (80, 98_000_000, 100_000_000, 400_000, 101_600_000),
        (50, 98_000_000, 100_000_000, 1_000_000, 101_000_000),
        (120, 98_000_000, 100_000_000, 0, 102_000_000),
        (0, 98_000_000, 100_000_000, 2_000_000, 100_000_000),
    ] {
        println!("With {pct} %");

        let mut test: TestData = TestData::new(
            &[
                "Alice".to_string(),
                "Bob".to_string(),
                "Platform".to_string(),
            ],
            &["USDC".to_string()],
            HashMap::from([
                ("Alice:USDC".to_string(), 100_000_000),
                ("Bob:USDC".to_string(), 100_000_000),
                ("Platform:USDC".to_string(), 100_000_000),
            ]),
        )
        .await;

        let (vault_pubkey, vault_bump_seed) = Pubkey::find_program_address(
            &[
                b"referrer",
                test.users["Bob"].pubkey().as_ref(),
                test.mint_keys["USDC"].as_ref(),
            ],
            &autobahn_executor::id(),
        );

        // Bob refers Alice, Alice creates the vault for Bob if it doesn't exist yet
        let accounts = vec![
            AccountMeta::new(test.users["Alice"].pubkey(), true),
            AccountMeta::new_readonly(test.users["Bob"].pubkey(), false),
            AccountMeta::new(vault_pubkey, false),
            AccountMeta::new_readonly(test.mint_keys["USDC"], false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ];

        let create_referral_ix = Instruction {
            program_id: autobahn_executor::id(),
            accounts,
            data: vec![Instructions::CreateReferral as u8, vault_bump_seed],
        };

        test.program_test_context
            .sign_send_instructions(&[create_referral_ix], &[&test.users["Alice"]])
            .await
            .unwrap();

        let data = build_fee_ix_data(2_000_000u64, pct);

        let charge_fees_ix = Instruction {
            program_id: autobahn_executor::id(),
            accounts: vec![
                AccountMeta::new_readonly(spl_token::ID, false),
                AccountMeta::new(test.users_ata["Alice:USDC"], false),
                AccountMeta::new(test.users_ata["Platform:USDC"], false),
                AccountMeta::new_readonly(test.users["Alice"].pubkey(), true),
                AccountMeta::new(vault_pubkey, false),
            ],
            data,
        };

        test.program_test_context
            .sign_send_instructions(&[charge_fees_ix], &[&test.users["Alice"]])
            .await
            .unwrap();

        assert_ata_balances(
            &mut test,
            vec![
                ("Alice:USDC", expected_alice),
                ("Bob:USDC", expected_bob),
                ("Platform:USDC", expected_platform),
            ],
        )
        .await;

        assert_ata_balance(
            &mut test.program_test_context,
            vault_pubkey,
            expected_derived_bob,
        )
        .await;

        // bob should be able to transfer from his derived account to himself
        let accounts = vec![
            AccountMeta::new(test.users["Bob"].pubkey(), true),
            AccountMeta::new(vault_pubkey, false),
            AccountMeta::new_readonly(test.mint_keys["USDC"], false),
            AccountMeta::new(test.users_ata["Bob:USDC"], false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ];

        let withdraw_ix = Instruction {
            program_id: autobahn_executor::id(),
            accounts,
            data: vec![Instructions::WithdrawReferral as u8, vault_bump_seed],
        };

        test.program_test_context
            .sign_send_instructions(&[withdraw_ix], &[&test.users["Bob"]])
            .await
            .unwrap();

        assert_ata_balance(&mut test.program_test_context, vault_pubkey, 0).await;

        assert_ata_balance(
            &mut test.program_test_context,
            test.users_ata["Bob:USDC"],
            expected_bob + expected_derived_bob,
        )
        .await;
    }
}

fn build_fee_ix_data(fee_amount: u64, platform_fee_pct: u8) -> Vec<u8> {
    let mut data = vec![];
    data.push(Instructions::ChargeFees as u8);
    data.extend_from_slice(fee_amount.to_le_bytes().as_slice());
    data.push(platform_fee_pct);
    data
}
