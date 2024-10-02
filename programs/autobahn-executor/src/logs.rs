use bytemuck::{Pod, Zeroable};
use solana_program::{program_error::ProgramError, pubkey::Pubkey};
use std::mem::size_of;

/// Serialize and log an event
///
/// Note that this is done instead of a self-CPI, which would be more reliable
/// as explained here
/// <https://github.com/coral-xyz/anchor/blob/59ee310cfa18524e7449db73604db21b0e04780c/lang/attribute/event/src/lib.rs#L104>
/// because the goal of this program is to minimize the number of input
/// accounts, so including the signer for the self CPI is not worth it.
/// Also, be compatible with anchor parsing clients.
#[inline(never)] // ensure fresh stack frame
pub fn emit_stack<T: bytemuck::Pod + Discriminant>(e: T) -> Result<(), ProgramError> {
    // stack buffer, stack frames are 4kb
    let mut buffer: [u8; 3000] = [0u8; 3000];

    buffer[..8].copy_from_slice(&T::discriminant());

    *get_mut_helper::<T>(&mut buffer, 8) = e;

    solana_program::log::sol_log_data(&[&buffer[..(size_of::<T>() + 8)]]);

    Ok(())
}

/// Read a struct of type T in an array of data at a given index.
pub fn get_mut_helper<T: bytemuck::Pod>(data: &mut [u8], index_usize: usize) -> &mut T {
    bytemuck::from_bytes_mut(&mut data[index_usize..index_usize + size_of::<T>()])
}

pub trait Discriminant {
    fn discriminant() -> [u8; 8];
}

macro_rules! discriminant {
    ($type_name:ident, $value:ident, $test_name:ident) => {
        impl Discriminant for $type_name {
            fn discriminant() -> [u8; 8] {
                $value
            }
        }

        #[test]
        fn $test_name() {
            let mut buffer: [u8; 8] = [0u8; 8];
            let discriminant: u64 = get_discriminant::<$type_name>().unwrap();
            buffer[..8].copy_from_slice(&u64::to_le_bytes(discriminant));
            assert_eq!(buffer, $type_name::discriminant());
        }
    };
}

#[repr(C)]
#[derive(Clone, Copy, Zeroable, Pod)]
pub struct SwapEvent {
    pub input_mint: Pubkey,
    pub input_amount: u64,
    pub output_mint: Pubkey,
    pub output_amount: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Zeroable, Pod)]
pub struct PlatformFeeLog {
    pub user: Pubkey,
    pub platform_token_account: Pubkey,
    pub platform_fee: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Zeroable, Pod)]
pub struct ReferrerFeeLog {
    pub referee: Pubkey,
    pub referer_token_account: Pubkey,
    pub referrer_fee: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Zeroable, Pod)]
pub struct ReferrerWithdrawLog {
    pub referer: Pubkey,
    pub referer_token_account: Pubkey,
    pub amount: u64,
}

pub const PLATFORM_FEE_LOG_DISCRIMINANT: [u8; 8] = [160, 183, 104, 34, 255, 190, 119, 188];
pub const REFERRER_FEE_LOG_DISCRIMINANT: [u8; 8] = [198, 149, 221, 27, 28, 103, 76, 95];
pub const REFERRER_WITHDRAW_LOG_DISCRIMINANT: [u8; 8] = [25, 7, 239, 41, 67, 36, 141, 92];
pub const SWAP_EVENT_DISCRIMINANT: [u8; 8] = [56, 178, 48, 245, 42, 152, 27, 75];

discriminant!(
    PlatformFeeLog,
    PLATFORM_FEE_LOG_DISCRIMINANT,
    test_platform_fee_log
);

discriminant!(
    ReferrerFeeLog,
    REFERRER_FEE_LOG_DISCRIMINANT,
    test_referrer_fee_log
);

discriminant!(
    ReferrerWithdrawLog,
    REFERRER_WITHDRAW_LOG_DISCRIMINANT,
    test_referrer_withdraw_log
);
discriminant!(SwapEvent, SWAP_EVENT_DISCRIMINANT, test_swap_event);

/// Canonical discriminant of the given struct. It is the hash of program ID and
/// the name of the type.
#[cfg(test)]
pub fn get_discriminant<T>() -> Result<u64, ProgramError> {
    let type_name: &str = std::any::type_name::<T>();
    let discriminant: u64 = u64::from_le_bytes(
        solana_program::keccak::hashv(&[crate::ID.as_ref(), type_name.as_bytes()]).as_ref()[..8]
            .try_into()
            .map_err(|_| ProgramError::InvalidAccountData)?,
    );
    Ok(discriminant)
}
