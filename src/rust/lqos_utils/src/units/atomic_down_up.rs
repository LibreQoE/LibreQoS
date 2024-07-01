use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering::Relaxed;

pub struct AtomicDownUp {
    down: AtomicU64,
    up: AtomicU64,
}

impl AtomicDownUp {
    pub const fn zeroed() -> Self {
        Self {
            down: AtomicU64::new(0),
            up: AtomicU64::new(0),
        }
    }
    
    pub fn set_to_zero(&self) {
        self.up.store(0, Relaxed);
        self.down.store(0, Relaxed);
    }
    
    pub fn checked_add_tuple(&self, n: (u64, u64)) {
        let n0 = self.down.load(std::sync::atomic::Ordering::Relaxed);
        if let Some(n) = n0.checked_add(n.0) {
            self.down.store(n, std::sync::atomic::Ordering::Relaxed);
        }

        let n1 = self.up.load(std::sync::atomic::Ordering::Relaxed);
        if let Some(n) = n1.checked_add(n.1) {
            self.up.store(n, std::sync::atomic::Ordering::Relaxed);
        }
    }
    
    pub fn get_down(&self) -> u64 {
        self.down.load(Relaxed)
    }

    pub fn get_up(&self) -> u64 {
        self.up.load(Relaxed)
    }
}