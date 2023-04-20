use std::time::Instant;

use lqos_sys::{rtt_for_each, throughput_for_each};

fn main() {
  println!("LibreQoS Map Performance Tool");

  // Test the RTT map
  let mut rtt_count = 0;
  let now = Instant::now();
  rtt_for_each(&mut |_rtt, _tracker| {
    rtt_count += 1;
  });
  let elapsed = now.elapsed();
  println!("RTT map: {} entries in {} µs", rtt_count, elapsed.as_micros());

  let mut tp_count = 0;
  let now = Instant::now();
  throughput_for_each(&mut |_ip, _hosts| {
    tp_count += 1;
  });
  let elapsed = now.elapsed();
  println!("TP map: {} entries in {} µs", tp_count, elapsed.as_micros());
}
