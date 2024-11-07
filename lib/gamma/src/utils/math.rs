pub trait CheckedCeilDiv: Sized {
    /// Perform ceiling division
    fn checked_ceil_div(&self, rhs: Self) -> Option<(Self, Self)>;
}

impl CheckedCeilDiv for u128 {
    fn checked_ceil_div(&self, mut rhs: Self) -> Option<(Self, Self)> {
        let mut quotient = self.checked_div(rhs)?;
        if quotient == 0 {
            if self.checked_mul(2 as u128)? >= rhs {
                return Some((1, 0));
            } else {
                return Some((0, 0));
            }
        }

        let remainder = self.checked_rem(rhs)?;
        if remainder > 0 {
            quotient = quotient.checked_add(1)?;
            rhs = self.checked_div(quotient)?;
            let remainder = self.checked_rem(quotient)?;
            if remainder > 0 {
                rhs = rhs.checked_add(1)?;
            }
        }
        Some((quotient, rhs))
    }
}
