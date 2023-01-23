pub fn scale_packets(n: u64) -> String {
  if n > 1_000_000_000 {
    format!("{:.2} gpps", n as f32 / 1_000_000_000.0)
  } else if n > 1_000_000 {
    format!("{:.2} mpps", n as f32 / 1_000_000.0)
  } else if n > 1_000 {
    format!("{:.2} kpps", n as f32 / 1_000.0)
  } else {
    format!("{n} pps")
  }
}

pub fn scale_bits(n: u64) -> String {
  if n > 1_000_000_000 {
    format!("{:.2} gbit/s", n as f32 / 1_000_000_000.0)
  } else if n > 1_000_000 {
    format!("{:.2} mbit/s", n as f32 / 1_000_000.0)
  } else if n > 1_000 {
    format!("{:.2} kbit/s", n as f32 / 1_000.0)
  } else {
    format!("{n} bit/s")
  }
}
