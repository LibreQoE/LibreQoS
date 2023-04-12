/// Scale a number of packets to a human readable string.
/// 
/// ## Parameters
/// * `n`: the number of packets to scale
pub fn scale_packets(n: u64) -> String {
  if n >= 1_000_000_000 {
    format!("{:.2} gpps", n as f32 / 1_000_000_000.0)
  } else if n >= 1_000_000 {
    format!("{:.2} mpps", n as f32 / 1_000_000.0)
  } else if n >= 1_000 {
    format!("{:.2} kpps", n as f32 / 1_000.0)
  } else {
    format!("{n} pps")
  }
}

/// Scale a number of bits to a human readable string.
/// 
/// ## Parameters
/// * `n`: the number of bits to scale
pub fn scale_bits(n: u64) -> String {
  if n >= 1_000_000_000 {
    format!("{:.2} gbit/s", n as f32 / 1_000_000_000.0)
  } else if n >= 1_000_000 {
    format!("{:.2} mbit/s", n as f32 / 1_000_000.0)
  } else if n >= 1_000 {
    format!("{:.2} kbit/s", n as f32 / 1_000.0)
  } else {
    format!("{n} bit/s")
  }
}

#[cfg(test)]
mod test {
  #[test]
  fn test_scale_packets() {
    assert_eq!(super::scale_packets(1), "1 pps");
    assert_eq!(super::scale_packets(1000), "1.00 kpps");
    assert_eq!(super::scale_packets(1000000), "1.00 mpps");
    assert_eq!(super::scale_packets(1000000000), "1.00 gpps");
  }

  #[test]
  fn test_scale_bits() {
    assert_eq!(super::scale_bits(1), "1 bit/s");
    assert_eq!(super::scale_bits(1000), "1.00 kbit/s");
    assert_eq!(super::scale_bits(1000000), "1.00 mbit/s");
    assert_eq!(super::scale_bits(1000000000), "1.00 gbit/s");}
}