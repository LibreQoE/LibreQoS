use lazy_static::*;
use parking_lot::RwLock;

lazy_static! {
    /// Global storage of current CPU usage
    pub static ref CPU_USAGE : RwLock<Vec<f32>> = RwLock::new(Vec::new());
}

lazy_static! {
    /// Global storage of current RAM usage
    pub static ref MEMORY_USAGE : RwLock<Vec<u64>> = RwLock::new(vec![0, 0]);
}