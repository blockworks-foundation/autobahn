#![allow(dead_code)]

use solana_program::instruction::InstructionError;
use solana_program::program_error::ProgramError;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Keypair;
use solana_sdk::transaction::TransactionError;
use solana_sdk::transport::TransportError;

pub fn clone_keypair(keypair: &Keypair) -> Keypair {
    Keypair::from_base58_string(&keypair.to_base58_string())
}

// Add clone() to Keypair, totally safe in tests
pub trait ClonableKeypair {
    fn clone(&self) -> Self;
}
impl ClonableKeypair for Keypair {
    fn clone(&self) -> Self {
        clone_keypair(self)
    }
}

/// A Keypair-like struct that's Clone and Copy and can be into()ed to a Keypair
///
/// The regular Keypair is neither Clone nor Copy because the key data is sensitive
/// and should not be copied needlessly. That just makes things difficult for tests.
#[derive(Clone, Copy, Debug)]
pub struct TestKeypair([u8; 64]);
impl TestKeypair {
    pub fn new() -> Self {
        Keypair::new().into()
    }

    pub fn to_keypair(&self) -> Keypair {
        Keypair::from_bytes(&self.0).unwrap()
    }

    pub fn pubkey(&self) -> Pubkey {
        solana_sdk::signature::Signer::pubkey(&self.to_keypair())
    }
}
impl Default for TestKeypair {
    fn default() -> Self {
        Self([0; 64])
    }
}
impl<T: std::borrow::Borrow<Keypair>> From<T> for TestKeypair {
    fn from(k: T) -> Self {
        Self(k.borrow().to_bytes())
    }
}
impl Into<Keypair> for &TestKeypair {
    fn into(self) -> Keypair {
        self.to_keypair()
    }
}

#[macro_export]
macro_rules! assert_eq_f64 {
    ($value:expr, $expected:expr, $max_error:expr $(,)?) => {
        let value = $value;
        let expected = $expected;
        let ok = (value - expected).abs() < $max_error;
        if !ok {
            println!("comparison failed: value: {value}, expected: {expected}");
        }
        assert!(ok);
    };
}

#[macro_export]
macro_rules! assert_eq_fixed_f64 {
    ($value:expr, $expected:expr, $max_error:expr $(,)?) => {
        assert_eq_f64!($value.to_num::<f64>(), $expected, $max_error);
    };
}
