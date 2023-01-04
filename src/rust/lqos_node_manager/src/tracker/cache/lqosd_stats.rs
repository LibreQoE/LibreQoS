use lazy_static::*;
use lqos_bus::IpStats;
use parking_lot::RwLock;

lazy_static! {
    pub static ref TOP_10_DOWNLOADERS : RwLock<Vec<IpStats>> = RwLock::new(Vec::new());
}

lazy_static! {
    pub static ref WORST_10_RTT : RwLock<Vec<IpStats>> = RwLock::new(Vec::new());
}

lazy_static! {
    pub static ref RTT_HISTOGRAM : RwLock<Vec<u32>> = RwLock::new(Vec::new());
}

lazy_static! {
    pub static ref HOST_COUNTS : RwLock<(u32, u32)> = RwLock::new((0, 0));
}
