use crate::utils::{read_u64, read_u8};
use solana_program::account_info::AccountInfo;
use solana_program::entrypoint::ProgramResult;
use solana_program::instruction::{AccountMeta, Instruction};
use solana_program::program::invoke;

/// Instruction that forwards to openbook v2 PlaceTakeOrder
///
/// Data:
/// - in_amount: u64 in native
/// - is_bid: u8 is 1 or 0
/// - limit: u8
///
/// Accounts:
/// - openbook v2 program
/// - openbook v2 PlaceTakeOrder accounts
pub fn execute_openbook_v2_swap(
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let (in_amount, instruction_data) = read_u64(instruction_data);
    let (is_bid, instruction_data) = read_u8(instruction_data);
    let (limit, _instruction_data) = read_u8(instruction_data);

    // Load the openbook market to figure out the lot sizes
    let market_data = accounts[3].try_borrow_data()?;
    assert_eq!(market_data.len(), 848);
    let quote_lot_size = i64::from_le_bytes(market_data[448..456].try_into().unwrap());
    let base_lot_size = i64::from_le_bytes(market_data[456..464].try_into().unwrap());
    drop(market_data);

    let is_bid = is_bid == 1;
    let side: u8;
    let max_base_lots;
    let max_quote_lots;
    let price_lots;
    if is_bid {
        side = 0;
        price_lots = i64::MAX;
        max_quote_lots = in_amount as i64 / quote_lot_size;
        max_base_lots = i64::MAX / base_lot_size;
    } else {
        side = 1;
        price_lots = 1;
        max_quote_lots = i64::MAX / quote_lot_size;
        max_base_lots = in_amount as i64 / base_lot_size;
    }
    let mut data = Vec::with_capacity(40);
    data.extend_from_slice(&[3, 44, 71, 3, 26, 199, 203, 85]); // PlaceTakeOrder discriminator
    data.push(side);
    data.extend_from_slice(&price_lots.to_le_bytes());
    data.extend_from_slice(&max_base_lots.to_le_bytes());
    data.extend_from_slice(&max_quote_lots.to_le_bytes());
    data.push(1u8); // ImmediateOrCancel
    data.push(limit);

    let instruction = Instruction {
        program_id: *accounts[0].key,
        accounts: accounts[1..]
            .iter()
            .map(|ai| AccountMeta {
                pubkey: *ai.key,
                is_signer: ai.is_signer,
                is_writable: ai.is_writable,
            })
            .collect::<Vec<_>>(),
        data,
    };

    invoke(&instruction, accounts)?;

    Ok(())
}
