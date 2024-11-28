use once_cell::sync::OnceCell;
use solana_program::pubkey::Pubkey;
use std::collections::HashSet;

/// Used in routing
/// we want to be sure that all accounts used to prices are in the "accepted filter" from the grpc subscriptions
static GLOBAL_ACCOUNTS_FILTERS: OnceCell<HashSet<Pubkey>> = OnceCell::new();

pub fn set_global_filters(filters: &HashSet<Pubkey>) {
    GLOBAL_ACCOUNTS_FILTERS.try_insert(filters.clone()).unwrap();
}

pub fn is_in_global_filters(address: &Pubkey) -> bool {
    GLOBAL_ACCOUNTS_FILTERS.get().unwrap().contains(address)
}

pub fn name(mint: &Pubkey) -> String {
    let m = mint.to_string();

    if m == "So11111111111111111111111111111111111111112" {
        "SOL".to_string()
    } else if m == "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" {
        "USDC".to_string()
    } else if m == "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB" {
        "USDT".to_string()
    } else if m == "USDH1SM1ojwWUga67PGrgFWUHibbjqMvuMaDkRJTgkX" {
        "USDH".to_string()
    } else if m == "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn" {
        "JitoSOL".to_string()
    } else if m == "jupSoLaHXQiZZTSfEWMTRRgpnyFm8f6sZdosWBjx93v" {
        "JupSol".to_string()
    } else if m == "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So" {
        "mSol".to_string()
    } else if m == "JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN" {
        "JUP".to_string()
    } else if m == "5oVNBeEEQvYi1cX3ir8Dx5n1P7pdxydbGF2X4TxVusJm" {
        "INF".to_string()
    } else if m == "27G8MtK7VtTcCHkpASjSDdkWWYfoqT6ggEuKidVJidD4" {
        "JLP".to_string()
    } else if m == "MangoCzJ36AjZyKwVj3VnYU4GTonjfVEnJmvvWaxLac" {
        "MNGO".to_string()
    } else if m == "hntyVP6YFm1Hg25TN9WGLqM12b8TQmcknKrdu1oxWux" {
        "HNT".to_string()
    } else if m == "KMNo3nJsBXfcpJTVhZcXLW7RmTwTt4GVFE7suUBo9sS" {
        "KMNO".to_string()
    } else if m == "DriFtupJYLTosbwoN8koMbEYSx54aFAVLddWsbksjwg7" {
        "DRIFT".to_string()
    } else if m == "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr" {
        "POPCAT".to_string()
    } else if m == "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R" {
        "RAY".to_string()
    } else if m == "3jsFX1tx2Z8ewmamiwSU851GzyzM2DJMq7KWW5DM8Py3" {
        "CHAI".to_string()
    } else if m == "rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBof" {
        "RENDER".to_string()
    } else if m == "nosXBVoaCTtYdLvKY6Csb4AC8JCdQKKAaWYtx2ZMoo7" {
        "NOS".to_string()
    } else if m == "METAewgxyPbgwsseH8T16a39CQ5VyVxZi9zXiDPY18m" {
        "MPLX".to_string()
    } else if m == "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263" {
        "BONK".to_string()
    } else if m == "6CNHDCzD5RkvBWxxyokQQNQPjFWgoHF94D7BmC73X6ZK" {
        "GECKO".to_string()
    } else if m == "LMDAmLNduiDmSiMxgae1gW7ubArfEGdAfTpKohqE5gn" {
        "LMDA".to_string()
    } else if m == "NeonTjSjsuo3rexg9o6vHuMXw62f9V7zvmu8M8Zut44" {
        "Neon".to_string()
    } else if m == "SHDWyBxihqiCj6YekG2GUr7wqKLeLAMK1gHZck9pL6y" {
        "Shadow".to_string()
    } else if m == "ukHH6c7mMyiWCf1b9pnWe25TSpkDDt3H5pQZgZ74J82" {
        "BOME".to_string()
    } else if m == "3S8qX1MsMqRbiwKg2cQyx7nis1oHMgaCuc9c4VfvVdPN" {
        "MOTHER".to_string()
    } else if m == "AKEWE7Bgh87GPp171b4cJPSSZfmZwQ3KaqYqXoKLNAEE" {
        "USDC (hyperlane)".to_string()
    } else {
        m
    }
}
