use allocative_derive::Allocative;
use lqos_utils::hex_string::read_hex_string;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::error;

/// Provides consistent handling of TC handle types.
#[derive(Copy, Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq, Hash, Allocative)]
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
    pub fn from_string(handle: &str) -> Result<Self, TcHandleParseError> {
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
                let major = read_hex_string(parts[0])
                    .map_err(|_| TcHandleParseError::InvalidInput(handle.to_string()))?;
                let minor = read_hex_string(parts[1])
                    .map_err(|_| TcHandleParseError::InvalidInput(handle.to_string()))?;
                if major >= (1 << 16) || minor >= (1 << 16) {
                    return Err(TcHandleParseError::InvalidInput(handle.to_string()));
                }
                Ok(Self((major << 16) | minor))
            }
        }
    }

    /// Construct a TC handle from a raw 32-bit unsigned integer.
    pub fn from_u32(tc: u32) -> Self {
        Self(tc)
    }

    /// Retreives a TC handle as a raw 32-bit unsigned integer.
    pub fn as_u32(&self) -> u32 {
        self.0
    }

    /// Construct a zeroed TC handle.
    pub fn zero() -> Self {
        Self(0)
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
    #[error("Unable to convert string to C-compatible string. Check your unicode!")]
    CString,
    #[error("Invalid input")]
    InvalidInput(String),
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn make_root() {
        let tc = TcHandle::from_string("root").unwrap();
        assert_eq!(tc.0, TC_H_ROOT);
    }

    #[test]
    fn make_unspecified() {
        let tc = TcHandle::from_string("none").unwrap();
        assert_eq!(tc.0, TC_H_UNSPEC);
    }

    #[test]
    fn test_invalid() {
        let tc = TcHandle::from_string("not_a_number");
        assert!(tc.is_err());
    }

    #[test]
    fn oversize_major() {
        let tc = TcHandle::from_string("65540:0");
        assert!(tc.is_err());
    }

    #[test]
    fn oversize_minor() {
        let tc = TcHandle::from_string("0:65540");
        assert!(tc.is_err());
    }

    #[test]
    fn zero() {
        let tc = TcHandle::from_string("0:0").unwrap();
        assert_eq!(tc.0, 0);
    }

    #[test]
    fn roundtrip() {
        let tc = TcHandle::from_string("1:2").unwrap();
        assert_eq!(tc.to_string(), "1:2");
    }

    #[test]
    fn hex() {
        let tc = TcHandle::from_string("7FFF:2").unwrap();
        assert_eq!(tc.to_string().to_uppercase(), "7FFF:2");
    }

    #[test]
    fn roundtrip_extreme() {
        for major in 0..2000 {
            for minor in 0..2000 {
                let handle = format!("{major:x}:{minor:x}");
                let tc = TcHandle::from_string(&handle).unwrap();
                assert_eq!(tc.to_string(), handle);
            }
        }
    }

    #[test]
    fn blank_minor() {
        let tc = TcHandle::from_string("7FFF:").unwrap();
        assert_eq!(tc.to_string().to_uppercase(), "7FFF:0");
    }
}
