use std::ops::AddAssign;
use serde::{Deserialize, Serialize};
use zerocopy::FromBytes;
use crate::units::UpDownOrder;

/// Provides strong download/upload separation for
/// stored statistics to eliminate confusion.
#[repr(C)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize, FromBytes, Default, Ord, PartialOrd)]
pub struct DownUpOrder<T> {
    pub down: T,
    pub up: T,
}

impl <T> DownUpOrder<T>
where T: std::cmp::Ord + num_traits::Zero + Copy + num_traits::CheckedSub
    + num_traits::CheckedAdd + num_traits::SaturatingSub + num_traits::SaturatingMul
    + num_traits::FromPrimitive
{
    pub fn new(down: T, up: T) -> Self {
        Self { down, up }
    }
    
    pub fn dir(&self, direction: usize) -> T {
        if direction == 0 {
            self.down
        } else {
            self.up
        }
    }

    pub fn zeroed() -> Self {
        Self { down: T::zero(), up: T::zero() }
    }
    
    pub fn both_less_than(&self, limit: T) -> bool {
        self.down < limit && self.up < limit
    }

    pub fn sum_exceeds(&self, limit: T) -> bool {
        self.down + self.up > limit
    }
    
    pub fn checked_sub_or_zero(&self, rhs: DownUpOrder<T>) -> DownUpOrder<T> {
        let down = T::checked_sub(&self.down, &rhs.down).unwrap_or(T::zero());
        let up = T::checked_sub(&self.up, &rhs.up).unwrap_or(T::zero());
        DownUpOrder { down, up }
    }

    pub fn checked_add(&mut self, rhs: DownUpOrder<T>) {
        self.down = self.down.checked_add(&rhs.down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&rhs.up).unwrap_or(T::zero());
    }

    pub fn checked_add_direct(&mut self, down: T, up: T) {
        self.down = self.down.checked_add(&down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&up).unwrap_or(T::zero());
    }

    pub fn sum(&self) -> T {
        self.down + self.up
    }

    pub fn to_bits_from_bytes(&self) -> DownUpOrder<T> {
        DownUpOrder {
            down: self.down.saturating_mul(&T::from_u32(8).unwrap()),
            up: self.up.saturating_mul(&T::from_u32(8).unwrap()),
        }
    }
}

impl <T> Into<UpDownOrder<T>> for DownUpOrder<T> {
    fn into(self) -> UpDownOrder<T> {
        UpDownOrder {
            up: self.down,
            down: self.up
        }
    }
}

impl <T> AddAssign for DownUpOrder<T>
where T: std::cmp::Ord + num_traits::Zero + Copy + num_traits::CheckedAdd
{
    fn add_assign(&mut self, rhs: Self) {
        self.down = self.down.checked_add(&rhs.down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&rhs.up).unwrap_or(T::zero());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_reverse() {
        let a = UpDownOrder::new(1, 2);
        let b: DownUpOrder<i32> = a.into();
        assert_eq!(a.down, b.up);
    }

    #[test]
    fn test_checked_sub() {
        let a = DownUpOrder::new(1u64, 1);
        let b= DownUpOrder::new(1, 1);
        let c = a.checked_sub_or_zero(b);
        assert_eq!(c.up, 0);
        assert_eq!(c.down, 0);

        let b= DownUpOrder::new(2, 2);
        let c = a.checked_sub_or_zero(b);
        assert_eq!(c.up, 0);
        assert_eq!(c.down, 0);
    }

    #[test]
    fn test_checked_add() {
        let mut a = DownUpOrder::new(u64::MAX, u64::MAX);
        let b = DownUpOrder::new(1, 1);
        a.checked_add(b);
        assert_eq!(a.down, 0);
        assert_eq!(a.up, 0);
        let mut a = DownUpOrder::new(1, 2);
        a.checked_add(b);
        assert_eq!(a.down, 2);
        assert_eq!(a.up, 3);
    }

    #[test]
    fn test_checked_add_direct() {
        let mut a = DownUpOrder::new(u64::MAX, u64::MAX);
        a.checked_add_direct(1, 1);
        assert_eq!(a.down, 0);
        assert_eq!(a.up, 0);
        let mut a = DownUpOrder::new(1, 2);
        a.checked_add_direct(1, 1);
        assert_eq!(a.down, 2);
        assert_eq!(a.up, 3);
    }
}