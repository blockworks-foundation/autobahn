use solana_program::pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::warn;

pub type Decimals = u8;

#[derive(Clone, Copy)]
pub struct Token {
    pub mint: Pubkey,
    pub decimals: Decimals,
}

#[derive(Clone)]
pub struct TokenCache {
    tokens: Arc<HashMap<Pubkey, Decimals>>,
}

impl TokenCache {
    pub fn new(data: HashMap<Pubkey, Decimals>) -> Self {
        Self {
            tokens: Arc::new(data),
        }
    }

    // use Result over Option to be compatible
    pub fn token(&self, mint: Pubkey) -> anyhow::Result<Token> {
        self.tokens
            .get(&mint)
            .map(|&decimals| Token { mint, decimals })
            .ok_or_else(|| {
                // this should never happen
                warn!("Token not found in cache: {}", mint);
                anyhow::anyhow!("Token not found in cache")
            })
    }

    pub fn tokens(&self) -> HashSet<Pubkey> {
        self.tokens
            .iter()
            .map(|(k, _)| *k)
            .collect::<HashSet<Pubkey>>()
    }
}
