//! AtomicDownUp is a struct that contains two atomic u64 values, one for down and one for up.
//! We frequently order things down and then up in kernel maps, keeping the ordering explicit
//! helps reduce directional confusion/bugs.

use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;
use crate::units::DownUpOrder;

/// AtomicDownUp is a struct that contains two atomic u64 values, one for down and one for up.
/// It's typically used for throughput, but can be used for any pairing that needs to keep track
/// of values by direction.
///
/// Note that unlike the DownUpOrder struct, it is not intended for direct serialization, and
/// is not generic.
#[derive(Debug)]
pub struct AtomicDownUp {
    down: AtomicU64,
    up: AtomicU64,
}

impl AtomicDownUp {
    /// Create a new `AtomicDownUp` with both values set to zero.
    pub const fn zeroed() -> Self {
        Self {
            down: AtomicU64::new(0),
            up: AtomicU64::new(0),
        }
    }

    /// Set both down and up to zero.
    pub fn set_to_zero(&self) {
        self.up.store(0, Relaxed);
        self.down.store(0, Relaxed);
    }

    /// Add a tuple of u64 values to the down and up values. The addition
    /// is checked, and will not occur if it would result in an overflow.
    pub fn checked_add_tuple(&self, n: (u64, u64)) {
        let _ = self.down.fetch_update(Relaxed, Relaxed, |x| x.checked_add(n.0));
        let _ = self.up.fetch_update(Relaxed, Relaxed, |x| x.checked_add(n.1));
    }

    /// Add a DownUpOrder to the down and up values. The addition
    /// is checked, and will not occur if it would result in an overflow.
    pub fn checked_add(&self, n: DownUpOrder<u64>) {
        let _ = self.down.fetch_update(Relaxed, Relaxed, |x| x.checked_add(n.down));
        let _ = self.up.fetch_update(Relaxed, Relaxed, |x| x.checked_add(n.up));
    }

    /// Get the down value.
    pub fn get_down(&self) -> u64 {
        self.down.load(Relaxed)
    }

    /// Get the up value.
    pub fn get_up(&self) -> u64 {
        self.up.load(Relaxed)
    }

    /// Set the down value.
    pub fn set_down(&self, n: u64) {
        self.down.store(n, Relaxed);
    }

    /// Set the up value.
    pub fn set_up(&self, n: u64) {
        self.up.store(n, Relaxed);
    }
    
    /// Transform the AtomicDownUp into a `DownUpOrder<u64>`.
    pub fn as_down_up(&self) -> DownUpOrder<u64> {
        DownUpOrder::new(
            self.get_down(),
            self.get_up()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atomic_down_up() {
        let adu = AtomicDownUp::zeroed();
        assert_eq!(adu.get_down(), 0);
        assert_eq!(adu.get_up(), 0);

        adu.set_down(1);
        adu.set_up(2);
        assert_eq!(adu.get_down(), 1);
        assert_eq!(adu.get_up(), 2);

        adu.checked_add(DownUpOrder::new(1, 2));
        assert_eq!(adu.get_down(), 2);
        assert_eq!(adu.get_up(), 4);

        adu.checked_add_tuple((1, 2));
        assert_eq!(adu.get_down(), 3);
        assert_eq!(adu.get_up(), 6);

        adu.set_to_zero();
        assert_eq!(adu.get_down(), 0);
        assert_eq!(adu.get_up(), 0);
    }
}