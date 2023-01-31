use lazy_static::*;
use lqos_bus::BusResponse;
use parking_lot::Mutex;
use std::process::Command;
use tokio::task::spawn_blocking;

lazy_static! {
  static ref TEST_BUSY: Mutex<bool> = Mutex::new(false);
}

pub fn lqos_daht_test() -> BusResponse {
  spawn_blocking(|| {
    if let Some(_lock) = TEST_BUSY.try_lock() {
      Command::new("/bin/ssh")
        .args([
          "-t",
          "lqtest@lqos.taht.net",
          "\"/home/lqtest/bin/v6vsv4.sh\"",
        ])
        .output()
        .unwrap();
    }
  });
  BusResponse::Ack
}
