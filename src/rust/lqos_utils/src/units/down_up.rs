//! AtomicDownUp is a struct that contains two atomic u64 values, one for down and one for up.
//! We frequently order things down and then up in kernel maps, keeping the ordering explicit
//! helps reduce directional confusion/bugs.

use crate::units::UpDownOrder;
use allocative_derive::Allocative;
use serde::{Deserialize, Serialize};
use std::ops::AddAssign;
use zerocopy::FromBytes;

/// Provides strong download/upload separation for
/// stored statistics to eliminate confusion. This is a generic
/// type: you can control the type stored inside.
#[repr(C)]
#[derive(
    Copy,
    Clone,
    Debug,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    FromBytes,
    Default,
    Ord,
    PartialOrd,
    Allocative,
)]
pub struct DownUpOrder<T> {
    /// The down value
    pub down: T,
    /// The up value
    pub up: T,
}

impl<T> DownUpOrder<T>
where
    T: std::cmp::Ord
        + num_traits::Zero
        + Copy
        + num_traits::CheckedSub
        + num_traits::CheckedAdd
        + num_traits::SaturatingSub
        + num_traits::SaturatingMul
        + num_traits::FromPrimitive
        + num_traits::SaturatingAdd
        + Default,
{
    /// Create a new DownUpOrder with the given down and up values.
    pub fn new(down: T, up: T) -> Self {
        Self { down, up }
    }

    /// In the C code, it's common to refer to a "direction" byte:
    ///
    /// * 0: down
    /// * 1: up
    /// * >1: error
    ///
    /// This is a helper function to translate that byte into the
    /// appropriate value.
    pub fn dir(&self, direction: usize) -> T {
        if direction == 0 { self.down } else { self.up }
    }

    /// Return a new DownUpOrder with both down and up set to zero.
    pub fn zeroed() -> Self {
        Self {
            down: T::zero(),
            up: T::zero(),
        }
    }

    /// Check if both down and up are less than the given limit.
    /// Returns `true` if they are both less than the limit, `false` otherwise.
    pub fn both_less_than(&self, limit: T) -> bool {
        self.down < limit && self.up < limit
    }

    /// Check if the sum of down and up exceeds the given limit.
    pub fn sum_exceeds(&self, limit: T) -> bool {
        self.down + self.up > limit
    }

    /// Subtract the given DownUpOrder from this one, returning a new DownUpOrder.
    /// If the result would be negative, it is clamped to zero.
    pub fn checked_sub_or_zero(&self, rhs: DownUpOrder<T>) -> DownUpOrder<T> {
        let down = T::checked_sub(&self.down, &rhs.down).unwrap_or(T::zero());
        let up = T::checked_sub(&self.up, &rhs.up).unwrap_or(T::zero());
        DownUpOrder { down, up }
    }

    /// Add the given DownUpOrder to this one. If the result would overflow,
    /// it is set to zero.
    pub fn checked_add(&mut self, rhs: DownUpOrder<T>) {
        self.down = self.down.checked_add(&rhs.down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&rhs.up).unwrap_or(T::zero());
    }

    /// Add the given down and up values to this DownUpOrder. If the result would overflow,
    /// it is set to zero.
    pub fn checked_add_direct(&mut self, down: T, up: T) {
        self.down = self.down.checked_add(&down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&up).unwrap_or(T::zero());
    }

    /// Add the given tuple of down and up values to this DownUpOrder. If the result would overflow,
    /// it is set to zero.
    pub fn checked_add_tuple(&mut self, (down, up): (T, T)) {
        self.down = self.down.checked_add(&down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&up).unwrap_or(T::zero());
    }

    /// Add the `down` and `up` values, giving a total.
    pub fn sum(&self) -> T {
        self.down.saturating_add(&self.up)
    }

    /// Multiply the `down` and `up` values by 8, giving the total number of bits, assuming
    /// that the previous value was bytes.
    pub fn to_bits_from_bytes(&self) -> DownUpOrder<T> {
        DownUpOrder {
            down: self
                .down
                .saturating_mul(&T::from_u32(8).unwrap_or_default()),
            up: self.up.saturating_mul(&T::from_u32(8).unwrap_or_default()),
        }
    }

    /// Get the `down` value.
    pub fn get_down(&self) -> T {
        self.down
    }

    /// Get the `up` value.
    pub fn get_up(&self) -> T {
        self.up
    }

    /// Set both the `down` and `up` values to zero.
    pub fn set_to_zero(&mut self) {
        self.down = T::zero();
        self.up = T::zero();
    }

    /// Check if both down and up are zero.
    pub fn not_zero(&self) -> bool {
        self.down != T::zero() || self.up != T::zero()
    }
}

impl<T> From<DownUpOrder<T>> for UpDownOrder<T> {
    fn from(val: DownUpOrder<T>) -> Self {
        UpDownOrder {
            up: val.down,
            down: val.up,
        }
    }
}

impl<T> AddAssign for DownUpOrder<T>
where
    T: std::cmp::Ord + num_traits::Zero + Copy + num_traits::CheckedAdd,
{
    fn add_assign(&mut self, rhs: Self) {
        self.down = self.down.checked_add(&rhs.down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&rhs.up).unwrap_or(T::zero());
    }
}

/// Divides two DownUpOrder values, returning a tuple of the results.
pub fn down_up_divide(left: DownUpOrder<u64>, right: DownUpOrder<u64>) -> (f64, f64) {
    #[inline(always)]
    fn safe_div(n: u64, d: u64) -> f64 {
        if d == 0 { 0.0 } else { n as f64 / d as f64 }
    }
    (safe_div(left.down, right.down), safe_div(left.up, right.up))
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
        let b = DownUpOrder::new(1, 1);
        let c = a.checked_sub_or_zero(b);
        assert_eq!(c.up, 0);
        assert_eq!(c.down, 0);

        let b = DownUpOrder::new(2, 2);
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
