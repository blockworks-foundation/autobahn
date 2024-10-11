//! Large uint types

// required for clippy
#![allow(clippy::assign_op_pattern)]
#![allow(clippy::ptr_offset_with_cast)]
#![allow(clippy::manual_range_contains)]

use uint::construct_uint;

construct_uint! {
    pub struct U256(4);
}
construct_uint! {
    pub struct U192(3);
}

#[allow(dead_code)]
pub const fn to_u256(n: u128) -> U256 {
    U256([n as u64, (n >> 64) as u64, 0, 0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_u256() {
        {
            let from = 0;
            let result = to_u256(from);
            let back = result.as_u128();
            assert_eq!(from, back);
        }
        {
            let from = 1;
            let result = to_u256(from);
            let back = result.as_u128();
            assert_eq!(from, back);
        }
        {
            let from = 1324342342433342342;
            let result = to_u256(from);
            let back = result.as_u128();
            assert_eq!(from, back);
        }
        {
            let from = u64::MAX as u128;
            let result = to_u256(from);
            let back = result.as_u128();
            assert_eq!(from, back);
        }
        {
            let from = u64::MAX as u128 + 1;
            let result = to_u256(from);
            let back = result.as_u128();
            assert_eq!(from, back);
        }
        {
            let from = u64::MAX as u128 + 2;
            let result = to_u256(from);
            let back = result.as_u128();
            assert_eq!(from, back);
        }
        {
            let from = u128::MAX;
            let result = to_u256(from);
            let back = result.as_u128();
            assert_eq!(from, back);
        }
    }
}
