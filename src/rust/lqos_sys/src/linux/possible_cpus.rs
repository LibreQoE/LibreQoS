use std::{fs::read_to_string, path::Path};

use tracing::error;
use thiserror::Error;

const POSSIBLE_CPUS_PATH: &str = "/sys/devices/system/cpu/possible";

/// Query the number of available CPUs from `/sys/devices/system/cpu/possible`,
/// and return the last digit (it will be formatted 0-3 or similar) plus one.
/// So on a 16 CPU system, `0-15` will return `16`.
pub fn num_possible_cpus() -> Result<u32, PossibleCpuError> {
  let path = Path::new(POSSIBLE_CPUS_PATH);
  if !path.exists() {
    error!("Unable to read /sys/devices/system/cpu/possible");
    return Err(PossibleCpuError::FileNotFound);
  };

  let file_contents = read_to_string(path);
  if file_contents.is_err() {
    error!("Unable to read contents of /sys/devices/system/cpu/possible");
    error!("{file_contents:?}");
    return Err(PossibleCpuError::UnableToRead);
  }
  let file_contents = file_contents.unwrap();

  parse_cpu_string(&file_contents)
}

fn parse_cpu_string(possible_cpus: &str) -> Result<u32, PossibleCpuError> {
  if let Some(last_digit) = possible_cpus.trim().split('-').last() {
    if let Ok(n) = last_digit.parse::<u32>() {
      Ok(n + 1)
    } else {
      error!("Unable to parse /sys/devices/system/cpu/possible");
      error!("{possible_cpus}");
      Err(PossibleCpuError::ParseError)
    }
  } else {
    error!("Unable to parse /sys/devices/system/cpu/possible");
    error!("{possible_cpus}");
    Err(PossibleCpuError::ParseError)
  }
}

#[derive(Error, Debug, Clone, PartialEq)]
pub enum PossibleCpuError {
  #[error("Unable to access /sys/devices/system/cpu/possible")]
  FileNotFound,
  #[error("Unable to read /sys/devices/system/cpu/possible")]
  UnableToRead,
  #[error("Unable to parse contents of /sys/devices/system/cpu/possible")]
  ParseError,
}

#[cfg(test)]
mod test {
  use super::*;

  #[test]
  fn test_unable_to_parse() {
    assert_eq!(parse_cpu_string("blah").err().unwrap(), PossibleCpuError::ParseError);
  }

  #[test]
  fn test_four_cpus() {
    assert_eq!(4, parse_cpu_string("0-3").unwrap());
  }

  #[test]
  fn test_sixteen_cpus() {
    assert_eq!(16, parse_cpu_string("0-15").unwrap());
  }

  #[test]
  fn test_againt_c() {
    let cpu_count = unsafe { libbpf_sys::libbpf_num_possible_cpus() } as u32;
    assert_eq!(cpu_count, num_possible_cpus().unwrap());
  }
}
