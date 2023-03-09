use lqos_bus::BusResponse;
use std::{process::Command, sync::atomic::AtomicBool};
use tokio::task::spawn_blocking;

static TEST_BUSY: AtomicBool = AtomicBool::new(false);

pub fn lqos_daht_test() -> BusResponse {
  spawn_blocking(|| {
    if TEST_BUSY.compare_exchange(
      false,
      true,
      std::sync::atomic::Ordering::Relaxed,
      std::sync::atomic::Ordering::Relaxed,
    ) == Ok(false)
    {
      let result = Command::new("/bin/ssh")
        .args(["-t", "lqtest@lqos.taht.net", "\"/home/lqtest/bin/v6vsv4.sh\""])
        .output();
      if result.is_err() {
        log::warn!("Unable to call dtaht test: {:?}", result);
      }

      TEST_BUSY.store(false, std::sync::atomic::Ordering::Relaxed);
    }
  });
  BusResponse::Ack
}
