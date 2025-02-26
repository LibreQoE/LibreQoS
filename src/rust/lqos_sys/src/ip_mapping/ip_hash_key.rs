#[repr(C)]
#[derive(Clone)]
pub struct IpHashKey {
    pub prefixlen: u32,
    pub address: [u8; 16],
}

impl Default for IpHashKey {
    fn default() -> Self {
        Self {
            prefixlen: 0,
            address: [0xFF; 16],
        }
    }
}
