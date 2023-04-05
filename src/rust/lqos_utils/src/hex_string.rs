use log::error;
use thiserror::Error;

/// `read_hex_string` converts a string from C-friendly Hex format
/// (e.g. `0xC12`) into a hexadecimal `u32`.
///
/// ## Parameters
///
/// * `s`: the string to attempt to parse.
///
/// ## Returns
///
/// Either a converted `u32` or a `HexParseError`.
///
/// ## Example
///
/// ```rust
/// use lqos_utils::hex_string::read_hex_string;
/// assert_eq!(read_hex_string("0x12AD").unwrap(), 4781);
/// ```
pub fn read_hex_string(s: &str) -> Result<u32, HexParseError> {
  if s.is_empty() {
    return Ok(0);
  }
  let result = u32::from_str_radix(&s.replace("0x", ""), 16);
  match result {
    Ok(data) => Ok(data),
    Err(e) => {
      error!("Unable to convert {s} to a u32");
      error!("{:?}", e);
      Err(HexParseError::ParseError)
    }
  }
}

/// `HexParseError` is an error type defining what can go wrong
/// parsing a string into a `u32` hex number.
#[derive(Error, Debug)]
pub enum HexParseError {
  /// The hex string could not be decoded
  #[error("Unable to decode string into valid hex")]
  ParseError,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hex_string_success() {
    assert_eq!(read_hex_string("0x12AD").unwrap(), 4781);
    assert_eq!(read_hex_string("12AD").unwrap(), 4781);
    assert_eq!(read_hex_string("0x12ad").unwrap(), 4781);
    assert_eq!(read_hex_string("12Ad").unwrap(), 4781);
  }

  #[test]
  fn hex_string_fail() {
    assert!(read_hex_string("0xG00F").is_err());
    assert!(read_hex_string("G00F").is_err());
  }
}
