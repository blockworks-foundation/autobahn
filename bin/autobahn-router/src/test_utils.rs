use solana_program::pubkey::Pubkey;

pub trait ToPubkey {
    fn to_pubkey(&self) -> Pubkey;
}

impl ToPubkey for u8 {
    fn to_pubkey(&self) -> Pubkey {
        Pubkey::new_from_array([*self; 32])
    }
}
