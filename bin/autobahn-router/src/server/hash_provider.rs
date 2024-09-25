use async_trait::async_trait;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_program::hash::Hash;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[async_trait]
pub trait HashProvider {
    async fn get_latest_hash(&self) -> anyhow::Result<Hash>;
}

pub struct RpcHashProvider {
    pub rpc_client: RpcClient,
    pub last_update: RwLock<Option<(Instant, Hash)>>,
}

#[async_trait]
impl HashProvider for RpcHashProvider {
    async fn get_latest_hash(&self) -> anyhow::Result<Hash> {
        {
            let locked = self.last_update.read().unwrap();
            if let Some((update, hash)) = *locked {
                if Instant::now().duration_since(update) < Duration::from_millis(500) {
                    return Ok(hash);
                }
            }
        }

        let hash = self.rpc_client.get_latest_blockhash().await?;
        let mut locked = self.last_update.write().unwrap();
        *locked = Some((Instant::now(), hash));
        Ok(hash)
    }
}
