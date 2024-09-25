use serde_derive::{Deserialize, Serialize};
use solana_sdk::account::Account;
use solana_sdk::pubkey::Pubkey;

// assign a magic version to ensure that snapshot is preferred over all versions from gRPC sources
pub const SNAP_ACCOUNT_WRITE_VERSION: u64 = 17777777777777777777u64;

pub fn account_write_from(
    pubkey: Pubkey,
    slot: u64,
    write_version: u64,
    account: Account,
) -> AccountWrite {
    AccountWrite {
        pubkey,
        slot,
        write_version,
        lamports: account.lamports,
        owner: account.owner,
        executable: account.executable,
        rent_epoch: account.rent_epoch,
        data: account.data,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountWrite {
    pub pubkey: Pubkey,
    pub slot: u64,
    pub write_version: u64,
    pub lamports: u64,
    pub owner: Pubkey,
    pub executable: bool,
    pub rent_epoch: u64,
    pub data: Vec<u8>,
    // is_selected < removed
}

#[derive(Debug)]
pub enum AccountOrSnapshotUpdate {
    AccountUpdate(AccountWrite),
    SnapshotUpdate(Vec<AccountWrite>),
}
