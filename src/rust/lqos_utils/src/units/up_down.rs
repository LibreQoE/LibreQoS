use crate::units::DownUpOrder;

/// Provides strong download/upload separation for
/// stored statistics to eliminate confusion.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct UpDownOrder<T> {
    pub up: T,
    pub down: T,
}

impl<T> UpDownOrder<T>
where T: std::cmp::Ord + num_traits::Zero + Copy + num_traits::CheckedSub
    + num_traits::CheckedAdd
{
    pub fn new(up: T, down: T) -> Self {
        Self {
            up, down
        }
    }

    pub fn zeroed() -> Self {
        Self { down: T::zero(), up: T::zero() }
    }

    pub fn both_less_than(&self, limit: T) -> bool {
        self.down < limit && self.up < limit
    }

    pub fn checked_sub_or_zero(&self, rhs: UpDownOrder<T>) -> UpDownOrder<T> {
        let down = T::checked_sub(&self.down, &rhs.down).unwrap_or(T::zero());
        let up = T::checked_sub(&self.up, &rhs.up).unwrap_or(T::zero());
        UpDownOrder { down, up }
    }

    pub fn checked_add(&mut self, rhs: UpDownOrder<T>) {
        self.down = self.down.checked_add(&rhs.down).unwrap_or(T::zero());
        self.up = self.up.checked_add(&rhs.up).unwrap_or(T::zero());
    }
}

impl <T> Into<DownUpOrder<T>> for UpDownOrder<T> {
    fn into(self) -> DownUpOrder<T> {
        DownUpOrder {
            up: self.down,
            down: self.up,
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
        let b= UpDownOrder::new(1, 1);
        let c = a.checked_sub_or_zero(b);
        assert_eq!(c.up, 0);
        assert_eq!(c.down, 0);

        let b= UpDownOrder::new(2, 2);
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