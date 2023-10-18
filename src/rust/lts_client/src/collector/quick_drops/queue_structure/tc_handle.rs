use log::error;
use lqos_utils::hex_string::read_hex_string;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Provides consistent handling of TC handle types.
#[derive(
  Copy, Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash
)]
pub struct TcHandle(u32);

const TC_H_ROOT: u32 = 4294967295;
const TC_H_UNSPEC: u32 = 0;

impl TcHandle {
  /// Returns the TC handle as two values, indicating major and minor
  /// TC handle values.
  #[inline(always)]
  pub fn get_major_minor(&self) -> (u16, u16) {
    // According to xdp_pping.c handles are minor:major u16s inside
    // a u32.
    ((self.0 >> 16) as u16, (self.0 & 0xFFFF) as u16)
  }

  /// Build a TC handle from a string. This is actually a complicated
  /// operation, since it has to handle "root" and other strings as well
  /// as simple "1:2" mappings. Calls a C function to handle this gracefully.
  pub fn from_string(
    handle: &str,
  ) -> Result<Self, TcHandleParseError> {
    let handle = handle.trim();
    match handle {
      "root" => Ok(Self(TC_H_ROOT)),
      "none" => Ok(Self(TC_H_UNSPEC)),
      _ => {
        if !handle.contains(':') {
          if let Ok(major) = read_hex_string(handle) {
            let minor = 0;
            return Ok(Self((major << 16) | minor));
          } else {
            error!("Unable to parse TC handle {handle}. Must contain a colon.");
            return Err(TcHandleParseError::InvalidInput(handle.to_string()));
          }
        }
        let parts: Vec<&str> = handle.split(':').collect();        
        let major = read_hex_string(parts[0]).map_err(|_| TcHandleParseError::InvalidInput(handle.to_string()))?;
        let minor = read_hex_string(parts[1]).map_err(|_| TcHandleParseError::InvalidInput(handle.to_string()))?;
        if major >= (1<<16) || minor >= (1<<16) {
          return Err(TcHandleParseError::InvalidInput(handle.to_string()));
        }
        Ok(Self((major << 16) | minor))
      }
    }
  }  
}

impl ToString for TcHandle {
  fn to_string(&self) -> String {
    let (major, minor) = self.get_major_minor();
    format!("{major:x}:{minor:x}")
  }
}

#[derive(Error, Debug)]
pub enum TcHandleParseError {
  #[error("Invalid input")]
  InvalidInput(String),
}

