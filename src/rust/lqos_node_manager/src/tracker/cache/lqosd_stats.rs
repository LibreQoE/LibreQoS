use once_cell::sync::Lazy;
use std::sync::RwLock;

pub static HOST_COUNTS: Lazy<RwLock<(u32, u32)>> =
  Lazy::new(|| RwLock::new((0, 0)));
