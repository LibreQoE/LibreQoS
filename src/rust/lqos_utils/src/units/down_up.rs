//! AtomicDownUp is a struct that contains two atomic u64 values, one for down and one for up.
//! We frequently order things down and then up in kernel maps, keeping the ordering explicit
//! helps reduce directional confusion/bugs.

use crate::units::UpDownOrder;
use allocative_derive::Allocative;
use serde::{Deserialize, Serialize};
use std::ops::AddAssign;
use zerocopy::FromBytes;

/// Strongly-typed count of TCP retransmit events.
#[repr(transparent)]
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    Ord,
    PartialOrd,
    Hash,
    Allocative,
)]
#[serde(transparent)]
pub struct RetransmitCount(pub u64);

impl RetransmitCount {
    /// Returns the underlying retransmit count.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Strongly-typed count of TCP packets used as the retransmit denominator.
#[repr(transparent)]
#[derive(
    Copy,
    Clone,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    Ord,
    PartialOrd,
    Hash,
    Allocative,
)]
#[serde(transparent)]
pub struct TcpPacketCount(pub u64);

impl TcpPacketCount {
    /// Returns the underlying packet count.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Strongly-typed retransmit fraction in the inclusive range `0.0..=1.0`.
#[repr(transparent)]
#[derive(
    Copy, Clone, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize, Allocative,
)]
#[serde(transparent)]
pub struct RetransmitFraction(f64);

impl RetransmitFraction {
    /// Constructs a retransmit fraction if the provided value is within `0.0..=1.0`.
    pub fn new(value: f64) -> Option<Self> {
        if (0.0..=1.0).contains(&value) {
            Some(Self(value))
        } else {
            None
        }
    }

    /// Returns the underlying fraction value.
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Converts the fraction to a `0.0..=100.0` percentage.
    pub fn percent_0_to_100(self) -> f64 {
        self.0 * 100.0
    }
}

/// Keeps TCP retransmit numerator and denominator together so callers cannot mix units.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize, Allocative)]
pub struct TcpRetransmitSample {
    /// The retransmit count for the sample window.
    pub retransmits: RetransmitCount,
    /// The TCP packet count for the same sample window.
    pub packets: TcpPacketCount,
}

impl TcpRetransmitSample {
    /// Creates a new retransmit sample from raw counts.
    pub const fn new(retransmits: u64, packets: u64) -> Self {
        Self {
            retransmits: RetransmitCount(retransmits),
            packets: TcpPacketCount(packets),
        }
    }

    /// Computes the retransmit fraction for the sample window.
    pub fn fraction(self) -> Option<RetransmitFraction> {
        let packets = self.packets.get();
        if packets == 0 {
            return None;
        }
        RetransmitFraction::new(self.retransmits.get() as f64 / packets as f64)
    }

    /// Computes the retransmit percentage for the sample window.
    pub fn percent_0_to_100(self) -> Option<f64> {
        self.fraction().map(RetransmitFraction::percent_0_to_100)
    }
}

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

impl<T> DownUpOrder<T> {
    /// Create a new DownUpOrder with the given down and up values.
    pub const fn new(down: T, up: T) -> Self {
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
    pub fn dir(&self, direction: usize) -> T
    where
        T: Copy,
    {
        if direction == 0 { self.down } else { self.up }
    }

    /// Get the `down` value.
    pub fn get_down(&self) -> T
    where
        T: Copy,
    {
        self.down
    }

    /// Get the `up` value.
    pub fn get_up(&self) -> T
    where
        T: Copy,
    {
        self.up
    }
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

/// Pairs retransmit counts with TCP packet counts in the same directional ordering.
pub fn down_up_retransmit_sample(
    retransmits: DownUpOrder<u64>,
    packets: DownUpOrder<u64>,
) -> DownUpOrder<TcpRetransmitSample> {
    DownUpOrder::new(
        TcpRetransmitSample::new(retransmits.down, packets.down),
        TcpRetransmitSample::new(retransmits.up, packets.up),
    )
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

    #[test]
    fn retransmit_fraction_rejects_invalid_values() {
        assert!(RetransmitFraction::new(-0.1).is_none());
        assert!(RetransmitFraction::new(1.1).is_none());
        assert_eq!(RetransmitFraction::new(0.25).map(|f| f.get()), Some(0.25));
    }

    #[test]
    fn retransmit_sample_computes_fraction_and_percent() {
        let sample = TcpRetransmitSample::new(5, 200);
        let fraction = sample.fraction().expect("fraction should exist");
        assert_eq!(fraction.get(), 0.025);
        assert_eq!(sample.percent_0_to_100(), Some(2.5));

        let empty = TcpRetransmitSample::new(5, 0);
        assert!(empty.fraction().is_none());
        assert!(empty.percent_0_to_100().is_none());
    }
}
