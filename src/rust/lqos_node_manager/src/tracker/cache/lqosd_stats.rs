use once_cell::sync::Lazy;
use std::sync::RwLock;

//pub static TOP_10_DOWNLOADERS: Lazy<RwLock<Vec<IpStats>>> =
//  Lazy::new(|| RwLock::new(Vec::with_capacity(10)));

//pub static WORST_10_RTT: Lazy<RwLock<Vec<IpStats>>> =
//  Lazy::new(|| RwLock::new(Vec::with_capacity(10)));

pub static RTT_HISTOGRAM: Lazy<RwLock<Vec<u32>>> =
  Lazy::new(|| RwLock::new(Vec::with_capacity(100)));

pub static HOST_COUNTS: Lazy<RwLock<(u32, u32)>> =
  Lazy::new(|| RwLock::new((0, 0)));
