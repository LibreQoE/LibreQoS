#[repr(C)]
#[derive(Clone)]
pub struct IpHashData {
    pub cpu: u32,
    pub tc_handle: u32,
}

impl Default for IpHashData {
    fn default() -> Self {
        Self {
            cpu: 0,
            tc_handle: 0,
        }
    }
}