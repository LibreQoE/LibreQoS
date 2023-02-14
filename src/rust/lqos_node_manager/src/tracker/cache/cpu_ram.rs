use std::sync::atomic::AtomicU64;

use lazy_static::*;
use parking_lot::RwLock;

lazy_static! {
    /// Global storage of current CPU usage
    pub static ref CPU_USAGE : RwLock<Vec<f32>> = RwLock::new(Vec::with_capacity(128));
}

pub static RAM_USED: AtomicU64 = AtomicU64::new(0);
pub static TOTAL_RAM: AtomicU64 = AtomicU64::new(0);