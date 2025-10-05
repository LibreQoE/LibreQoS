//! AtomicDownUp is a struct that contains two atomic u64 values, one for down and one for up.
//! We frequently order things down and then up in kernel maps, keeping the ordering explicit
//! helps reduce directional confusion/bugs.

use crate::units::DownUpOrder;

/// Provides strong download/upload separation for
/// stored statistics to eliminate confusion.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct UpDownOrder<T> {
    /// The up value
    pub up: T,
    /// The down value
    pub down: T,
}

impl<T> UpDownOrder<T>
where
    T: std::cmp::Ord + num_traits::Zero + Copy + num_traits::CheckedSub + num_traits::CheckedAdd,
{
    /// Create a new UpDownOrder with the given up and down values.
    pub fn new(up: T, down: T) -> Self {
        Self { up, down }
    }

    /// Return a new UpDownOrder with both up and down set to zero.
    pub fn zeroed() -> Self {
        Self {
            down: T::zero(),
            up: T::zero(),
        }
    }

    /// Check if both up and down are less than the given limit.
    pub fn both_less_than(&self, limit: T) -> bool {
        self.down < limit && self.up < limit
    }

    /// Subtract the given UpDownOrder from this one, returning a new UpDownOrder.
    /// If the subtraction would result in a negative value, the result is set to zero.
    pub fn checked_sub_or_zero(&self, rhs: UpDownOrder<T>) -> UpDownOrder<T> {
        let down = T::checked_sub(&self.down, &rhs.down).unwrap_or(T::zero());
        let up = T::checked_sub(&self.up, &rhs.up).unwrap_or(T::zero());
        UpDownOrder { down, up }
    }

    /// Add the given UpDownOrder to this one, updating the values in place.
    /// Overflowing values are set to zero.
    pub fn checked_add(&mut self, rhs: UpDownOrder<T>) {
        self.down = self.down.checked_add(&rhs.down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&rhs.up).unwrap_or(T::zero());
    }
}

impl<T> From<UpDownOrder<T>> for DownUpOrder<T> {
    fn from(val: UpDownOrder<T>) -> Self {
        DownUpOrder {
            up: val.down,
            down: val.up,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reverse() {
        let a = DownUpOrder::new(1, 2);
        let b: UpDownOrder<i32> = a.into();
        assert_eq!(a.down, b.up);
    }

    #[test]
    fn test_checked_sub() {
        let a = UpDownOrder::new(1u64, 1);
        let b = UpDownOrder::new(1, 1);
        let c = a.checked_sub_or_zero(b);
        assert_eq!(c.up, 0);
        assert_eq!(c.down, 0);

        let b = UpDownOrder::new(2, 2);
        let c = a.checked_sub_or_zero(b);
        assert_eq!(c.up, 0);
        assert_eq!(c.down, 0);
    }

    #[test]
    fn test_checked_add() {
        let mut a = UpDownOrder::new(u64::MAX, u64::MAX);
        let b = UpDownOrder::new(1, 1);
        a.checked_add(b);
        assert_eq!(a.down, 0);
        assert_eq!(a.up, 0);
        let mut a = UpDownOrder::new(1, 2);
        a.checked_add(b);
        assert_eq!(a.down, 3);
        assert_eq!(a.up, 2);
    }
}
