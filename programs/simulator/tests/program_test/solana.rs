#![allow(dead_code)]

use std::cell::RefCell;
use std::io::{Cursor, Write};

use super::utils::TestKeypair;
// use anchor_lang::AccountDeserialize;
// use anchor_spl::token::TokenAccount;
use solana_program::{program_pack::Pack, rent::*, system_instruction};
use solana_program_test::*;
use solana_sdk::{
    account::ReadableAccount,
    instruction::Instruction,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use spl_token::*;

pub struct SolanaCookie {
    pub context: RefCell<ProgramTestContext>,
    pub rent: Rent,
    pub last_transaction_log: RefCell<Vec<String>>,
}

impl SolanaCookie {
    pub async fn process_transaction(
        &self,
        instructions: &[Instruction],
        signers: Option<&[TestKeypair]>,
    ) -> Result<BanksTransactionResultWithMetadata, BanksClientError> {
        let mut context = self.context.borrow_mut();
        let blockhash = context.get_new_latest_blockhash().await?;

        let mut transaction =
            Transaction::new_with_payer(&instructions, Some(&context.payer.pubkey()));

        let mut all_signers = vec![&context.payer];
        let signer_keypairs =
            signers.map(|signers| signers.iter().map(|s| s.into()).collect::<Vec<Keypair>>());
        let signer_keypair_refs = signer_keypairs
            .as_ref()
            .map(|kps| kps.iter().map(|kp| kp).collect::<Vec<&Keypair>>());

        if let Some(signer_keypair_refs) = signer_keypair_refs {
            all_signers.extend(signer_keypair_refs.iter());
        }

        transaction.sign(&all_signers, blockhash);

        let result = context
            .banks_client
            .process_transaction_with_metadata(transaction)
            .await;

        *self.last_transaction_log.borrow_mut() = result
            .as_ref()
            .ok()
            .and_then(|r| r.metadata.as_ref())
            .map(|v| v.log_messages.clone())
            .unwrap_or_default();

        drop(context);

        result
    }

    pub async fn clock(&self) -> solana_program::clock::Clock {
        self.context
            .borrow_mut()
            .banks_client
            .get_sysvar::<solana_program::clock::Clock>()
            .await
            .unwrap()
    }

    pub fn set_clock(&self, clock: &solana_program::clock::Clock) {
        self.context.borrow_mut().set_sysvar(clock);
    }

    pub async fn clock_timestamp(&self) -> u64 {
        self.clock().await.unix_timestamp.try_into().unwrap()
    }

    pub async fn set_clock_timestamp(&self, timestamp: u64) {
        let mut clock = self.clock().await;
        clock.unix_timestamp = timestamp.try_into().unwrap();
        self.set_clock(&clock);
    }

    pub async fn advance_by_slots(&self, slots: u64) {
        let clock = self.clock().await;
        self.context
            .borrow_mut()
            .warp_to_slot(clock.slot + slots + 1)
            .unwrap();
    }

    pub async fn advance_clock_to(&self, target: i64) {
        let mut clock = self.clock().await;

        // just advance enough to ensure we get changes over last_updated in various ix
        // if this gets too slow for our tests, remove and replace with manual time offset
        // which is configurable
        while clock.unix_timestamp <= target {
            self.context
                .borrow_mut()
                .warp_to_slot(clock.slot + 50)
                .unwrap();
            clock = self.clock().await;
        }
    }

    pub async fn advance_clock_to_next_multiple(&self, window: i64) {
        let ts = self.clock().await.unix_timestamp;
        self.advance_clock_to(ts / window * window + window).await
    }

    pub async fn advance_clock(&self) {
        let clock = self.clock().await;
        self.advance_clock_to(clock.unix_timestamp + 1).await
    }

    pub async fn get_newest_slot_from_history(&self) -> u64 {
        self.context
            .borrow_mut()
            .banks_client
            .get_sysvar::<solana_program::slot_history::SlotHistory>()
            .await
            .unwrap()
            .newest()
    }

    pub async fn create_account_from_len(&self, owner: &Pubkey, len: usize) -> Pubkey {
        let key = TestKeypair::new();
        let rent = self.rent.minimum_balance(len);
        let create_account_instr = solana_sdk::system_instruction::create_account(
            &self.context.borrow().payer.pubkey(),
            &key.pubkey(),
            rent,
            len as u64,
            &owner,
        );
        self.process_transaction(&[create_account_instr], Some(&[key]))
            .await
            .unwrap();
        key.pubkey()
    }

    pub async fn create_account_for_type<T>(&self, owner: &Pubkey) -> Pubkey {
        let key = TestKeypair::new();
        let len = 8 + std::mem::size_of::<T>();
        let rent = self.rent.minimum_balance(len);
        let create_account_instr = solana_sdk::system_instruction::create_account(
            &self.context.borrow().payer.pubkey(),
            &key.pubkey(),
            rent,
            len as u64,
            &owner,
        );
        self.process_transaction(&[create_account_instr], Some(&[key]))
            .await
            .unwrap();
        key.pubkey()
    }

    pub async fn create_token_account(&self, owner: &Pubkey, mint: Pubkey) -> Pubkey {
        let keypair = TestKeypair::new();
        let rent = self.rent.minimum_balance(spl_token::state::Account::LEN);

        let instructions = [
            system_instruction::create_account(
                &self.context.borrow().payer.pubkey(),
                &keypair.pubkey(),
                rent,
                spl_token::state::Account::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_account(
                &spl_token::id(),
                &keypair.pubkey(),
                &mint,
                owner,
            )
            .unwrap(),
        ];

        self.process_transaction(&instructions, Some(&[keypair]))
            .await
            .unwrap();
        return keypair.pubkey();
    }

    // Note: Only one table can be created per authority per slot!
    pub async fn create_address_lookup_table(
        &self,
        authority: TestKeypair,
        payer: TestKeypair,
    ) -> Pubkey {
        let (instruction, alt_address) =
            solana_address_lookup_table_program::instruction::create_lookup_table(
                authority.pubkey(),
                payer.pubkey(),
                self.get_newest_slot_from_history().await,
            );
        self.process_transaction(&[instruction], Some(&[authority, payer]))
            .await
            .unwrap();
        alt_address
    }

    pub async fn get_account_data(&self, address: Pubkey) -> Option<Vec<u8>> {
        Some(
            self.context
                .borrow_mut()
                .banks_client
                .get_account(address)
                .await
                .unwrap()?
                .data()
                .to_vec(),
        )
    }

    /*
    pub async fn get_account_opt<T: AccountDeserialize>(&self, address: Pubkey) -> Option<T> {
        let data = self.get_account_data(address).await?;
        let mut data_slice: &[u8] = &data;
        AccountDeserialize::try_deserialize(&mut data_slice).ok()
    }

    // Use when accounts are too big for the stack
    pub async fn get_account_boxed<T: AccountDeserialize>(&self, address: Pubkey) -> Box<T> {
        let data = self.get_account_data(address).await.unwrap();
        let mut data_slice: &[u8] = &data;
        Box::new(AccountDeserialize::try_deserialize(&mut data_slice).unwrap())
    }

    pub async fn get_account<T: AccountDeserialize>(&self, address: Pubkey) -> T {
        self.get_account_opt(address).await.unwrap()
    }

    pub async fn set_account<T: anchor_lang::Discriminator + anchor_lang::ZeroCopy>(
        &self,
        address: Pubkey,
        data: &T,
    ) {
        let mut buffer = Cursor::new(Vec::new());
        buffer.write_all(&T::DISCRIMINATOR).unwrap();
        buffer.write_all(bytemuck::bytes_of(data)).unwrap();

        let mut account = self
            .context
            .borrow_mut()
            .banks_client
            .get_account(address)
            .await
            .unwrap()
            .unwrap();
        account.data = buffer.into_inner();
        self.context
            .borrow_mut()
            .set_account(&address, &account.into());
    }

    pub async fn token_account_balance(&self, address: Pubkey) -> u64 {
        self.get_account::<TokenAccount>(address).await.amount
    } */

    pub fn program_log(&self) -> Vec<String> {
        self.last_transaction_log.borrow().clone()
    }

    /*
    pub fn program_log_events<T: anchor_lang::Event + anchor_lang::AnchorDeserialize>(
        &self,
    ) -> Vec<T> {
        self.program_log()
            .iter()
            .filter_map(|data| {
                if let Some(event) = data.strip_prefix("Program data: ") {
                    let bytes = base64::decode(event).ok()?;
                    if bytes[0..8] != T::discriminator() {
                        return None;
                    }
                    T::try_from_slice(&bytes[8..]).ok()
                } else {
                    None
                }
            })
            .collect()
    }
     */
}
