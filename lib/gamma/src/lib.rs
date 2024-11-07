pub mod curve;
use std::ops::BitAnd;
pub mod fees;
pub mod utils;

declare_id!("GAMMA7meSFWaBXF25oSUgmGRwaW6sCMFLmBNiMSdbHVT");

anchor_gen::generate_cpi_crate!("src/gamma.json");

pub const AUTH_SEED: &str = "vault_and_lp_mint_auth_seed";

#[derive(PartialEq, Eq)]
pub enum PoolStatusBitFlag {
    Enable,
    Disable,
}

pub enum PoolStatusBitIndex {
    Deposit,
    Withdraw,
    Swap,
}

impl PoolState {
    pub const LEN: usize = 8 + 10 * 32 + 5 * 1 + 7 * 8 + 16 * 4 + 23 * 8;

    pub fn get_status_by_bit(&self, bit: PoolStatusBitIndex) -> bool {
        let status = u8::from(1) << (bit as u8);
        self.status.bitand(status) == 0
    }
}

#[error_code]
pub enum GammaError {
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Invalid fee")]
    InvalidFee,
}
