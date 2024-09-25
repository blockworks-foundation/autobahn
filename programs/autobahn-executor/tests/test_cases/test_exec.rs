#![cfg(feature = "test-bpf")]

use bonfida_test_utils::ProgramTestContextExt;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::pubkey::Pubkey;
use std::collections::HashMap;
use std::str::FromStr;

use autobahn_executor;
use autobahn_executor::swap_ix::generate_swap_ix_data;
use solana_sdk::signer::{keypair::Keypair, Signer};

use crate::utils::*;

#[tokio::test]
async fn should_do_a_one_hop_execution() {
    let mut test: TestData = TestData::new(
        &["Alice".to_string(), "Bob".to_string()],
        &["USDC".to_string(), "EURC".to_string()],
        HashMap::from([
            ("Alice:USDC".to_string(), 100_000_000),
            ("Bob:EURC".to_string(), 100_000_000),
        ]),
    )
    .await;

    assert_ata_balance(
        &mut test.program_test_context,
        test.users_ata["Alice:USDC"],
        100_000_000,
    )
    .await;
    assert_ata_balance(
        &mut test.program_test_context,
        test.users_ata["Alice:EURC"],
        0,
    )
    .await;

    // Exec swap

    let swap_a_to_b = build_mock_swap_ix(
        &test.users["Alice"],
        &test.users["Bob"],
        test.mock_swap_program_id,
        test.users_ata["Alice:USDC"],
        test.users_ata["Alice:EURC"],
        test.users_ata["Bob:USDC"],
        test.users_ata["Bob:EURC"],
        5_000_000u64,
        25_000_000u64,
    );

    let ix = generate_swap_ix_data(
        20_000_000,
        &[swap_a_to_b],
        &[0],
        test.users_ata["Alice:USDC"],
        &[test.users_ata["Alice:EURC"]],
        autobahn_executor::id(),
        0,
    );

    test.program_test_context
        .sign_send_instructions(&[ix], &[&test.users["Alice"], &test.users["Bob"]])
        .await
        .unwrap();

    assert_ata_balance(
        &mut test.program_test_context,
        test.users_ata["Alice:USDC"],
        95_000_000,
    )
    .await;
    assert_ata_balance(
        &mut test.program_test_context,
        test.users_ata["Alice:EURC"],
        25_000_000,
    )
    .await;
}

#[tokio::test]
async fn should_do_a_two_hops_execution() {
    let mut test = TestData::new(
        &["Alice".to_string(), "Bob".to_string()],
        &["USDC".to_string(), "EURC".to_string()],
        HashMap::from([
            ("Alice:USDC".to_string(), 100_000_000),
            ("Bob:USDC".to_string(), 100_000_000),
            ("Bob:EURC".to_string(), 100_000_000),
        ]),
    )
    .await;

    // Exec swap

    let swap_a_to_b = build_mock_swap_ix(
        &test.users["Alice"],
        &test.users["Bob"],
        test.mock_swap_program_id,
        test.users_ata["Alice:USDC"],
        test.users_ata["Alice:EURC"],
        test.users_ata["Bob:USDC"],
        test.users_ata["Bob:EURC"],
        5_000_000u64,
        25_000_000u64,
    );
    let swap_b_to_a = build_mock_swap_ix(
        &test.users["Alice"],
        &test.users["Bob"],
        test.mock_swap_program_id,
        test.users_ata["Alice:EURC"],
        test.users_ata["Alice:USDC"],
        test.users_ata["Bob:EURC"],
        test.users_ata["Bob:USDC"],
        30_000_000u64,
        6_000_000u64,
    );

    let ix = generate_swap_ix_data(
        0,
        &[swap_a_to_b, swap_b_to_a],
        &[0, 0],
        test.users_ata["Alice:USDC"],
        &[test.users_ata["Alice:EURC"], test.users_ata["Alice:USDC"]],
        autobahn_executor::id(),
        0,
    );

    test.program_test_context
        .sign_send_instructions(&[ix], &[&test.users["Alice"], &test.users["Bob"]])
        .await
        .unwrap();

    assert_ata_balances(
        &mut test,
        vec![
            ("Alice:USDC", 101_000_000),
            ("Alice:EURC", 0),
            ("Bob:USDC", 99_000_000),
            ("Bob:EURC", 100_000_000),
        ],
    )
    .await;
}

#[tokio::test]
async fn should_do_a_three_hops_swap() {
    let mut test = TestData::new(
        &["Alice".to_string(), "Bob".to_string()],
        &[
            "USDC".to_string(),
            "EURC".to_string(),
            "PYTH".to_string(),
            "MNGO".to_string(),
        ],
        HashMap::from([
            ("Alice:USDC".to_string(), 100_000_000),
            ("Bob:EURC".to_string(), 10_000_000),
            ("Bob:PYTH".to_string(), 20_000_000),
            ("Bob:MNGO".to_string(), 40_000_000),
        ]),
    )
    .await;

    let swap_usdc_eurc = build_mock_swap_ix(
        &test.users["Alice"],
        &test.users["Bob"],
        test.mock_swap_program_id,
        test.users_ata["Alice:USDC"],
        test.users_ata["Alice:EURC"],
        test.users_ata["Bob:USDC"],
        test.users_ata["Bob:EURC"],
        12_000_000u64,
        10_000_000u64,
    );
    let swap_eurc_pyth = build_mock_swap_ix(
        &test.users["Alice"],
        &test.users["Bob"],
        test.mock_swap_program_id,
        test.users_ata["Alice:EURC"],
        test.users_ata["Alice:PYTH"],
        test.users_ata["Bob:EURC"],
        test.users_ata["Bob:PYTH"],
        11_000_000u64,
        20_000_000u64,
    );
    let swap_pyth_mngo = build_mock_swap_ix(
        &test.users["Alice"],
        &test.users["Bob"],
        test.mock_swap_program_id,
        test.users_ata["Alice:PYTH"],
        test.users_ata["Alice:MNGO"],
        test.users_ata["Bob:PYTH"],
        test.users_ata["Bob:MNGO"],
        25_000_000u64,
        40_000_000u64,
    );

    let ix = generate_swap_ix_data(
        0,
        &[swap_usdc_eurc, swap_eurc_pyth, swap_pyth_mngo],
        &[0, 0, 0],
        test.users_ata["Alice:USDC"],
        &[
            test.users_ata["Alice:EURC"],
            test.users_ata["Alice:PYTH"],
            test.users_ata["Alice:MNGO"],
        ],
        autobahn_executor::id(),
        0,
    );

    test.program_test_context
        .sign_send_instructions(&[ix], &[&test.users["Alice"], &test.users["Bob"]])
        .await
        .unwrap();

    assert_ata_balances(
        &mut test,
        vec![
            ("Alice:USDC", 88_000_000),
            ("Alice:EURC", 0),
            ("Alice:PYTH", 0),
            ("Alice:MNGO", 40_000_000),
            ("Bob:USDC", 12_000_000),
            ("Bob:EURC", 10_000_000),
            ("Bob:PYTH", 20_000_000),
            ("Bob:MNGO", 0),
        ],
    )
    .await;
}

#[tokio::test]
async fn should_fail_when_max_slippage_is_reached() {
    for (min_out_amount, expected_err) in [(30_000_000, true), (20_000_000, false)] {
        let mut test: TestData = TestData::new(
            &["Alice".to_string(), "Bob".to_string()],
            &["USDC".to_string(), "EURC".to_string()],
            HashMap::from([
                ("Alice:USDC".to_string(), 100_000_000),
                ("Bob:EURC".to_string(), 100_000_000),
            ]),
        )
        .await;

        let swap_a_to_b = build_mock_swap_ix(
            &test.users["Alice"],
            &test.users["Bob"],
            test.mock_swap_program_id,
            test.users_ata["Alice:USDC"],
            test.users_ata["Alice:EURC"],
            test.users_ata["Bob:USDC"],
            test.users_ata["Bob:EURC"],
            5_000_000u64,
            25_000_000u64,
        );

        let ix = generate_swap_ix_data(
            min_out_amount,
            &[swap_a_to_b],
            &[0],
            test.users_ata["Alice:USDC"],
            &[test.users_ata["Alice:EURC"]],
            autobahn_executor::id(),
            0,
        );

        let is_err = test
            .program_test_context
            .sign_send_instructions(&[ix], &[&test.users["Alice"], &test.users["Bob"]])
            .await
            .is_err();

        assert_eq!(is_err, expected_err)
    }
}

fn build_mock_swap_ix(
    swapper: &Keypair,
    other: &Keypair,
    mock_swap: Pubkey,
    swapper_ata_a: Pubkey,
    swapper_ata_b: Pubkey,
    other_ata_a: Pubkey,
    other_ata_b: Pubkey,
    amount_a: u64,
    amount_b: u64,
) -> Instruction {
    let mut swap_data = vec![];
    swap_data.extend_from_slice(&amount_a.to_le_bytes());
    swap_data.extend_from_slice(&amount_b.to_le_bytes());

    Instruction {
        program_id: mock_swap,
        data: swap_data,
        accounts: vec![
            AccountMeta::new(
                Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap(),
                false,
            ),
            AccountMeta::new(swapper.pubkey(), true),
            AccountMeta::new(swapper_ata_a, false),
            AccountMeta::new(other_ata_a, false),
            AccountMeta::new(other.pubkey(), true),
            AccountMeta::new(other_ata_b, false),
            AccountMeta::new(swapper_ata_b, false),
        ],
    }
}
