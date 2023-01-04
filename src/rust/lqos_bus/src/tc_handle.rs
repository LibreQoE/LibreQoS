use std::ffi::CString;
use anyhow::{Result, Error};
use serde::{Serialize, Deserialize};

/// Provides consistent handling of TC handle types.
#[derive(Copy, Clone, Serialize, Deserialize, Debug, Default)]
pub struct TcHandle(u32);

#[allow(non_camel_case_types)]
type __u32 = ::std::os::raw::c_uint;
#[allow(dead_code)]
const TC_H_ROOT: u32 = 4294967295;
#[allow(dead_code)]
const TC_H_UNSPEC: u32 = 0;

extern "C" {
    pub fn get_tc_classid(
        h: *mut __u32,
        str_: *const ::std::os::raw::c_char,
    ) -> ::std::os::raw::c_int;
}

impl TcHandle {
    #[inline(always)]
    pub fn get_major_minor(&self) -> (u16, u16) {
        // According to xdp_pping.c handles are minor:major u16s inside
        // a u32.
        (
            (self.0 >> 16) as u16,
            (self.0 & 0xFFFF) as u16,
        )
    }

    pub fn from_string<S:ToString>(handle: S) -> Result<Self> {
        let mut tc_handle : __u32 = 0;
        let str = CString::new(handle.to_string())?;
        let handle_pointer : *mut __u32 = &mut tc_handle;
        let result = unsafe {
            get_tc_classid(handle_pointer, str.as_ptr())
        };
        if result != 0 {
            Err(Error::msg("Unable to parse TC handle string"))
        } else {
            Ok(Self(tc_handle))
        }
    }

    pub fn from_u32(tc: u32) -> Self {
        Self(tc)
    }

    pub fn as_u32(&self) -> u32 {
        self.0
    }

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
        for major in 0 .. 2000 {
            for minor in 0..2000 {
                let handle = format!("{major:x}:{minor:x}");
                let tc = TcHandle::from_string(&handle).unwrap();
                assert_eq!(tc.to_string(), handle);
            }
        }
        
    }
}