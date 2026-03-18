#[repr(C)]
#[derive(Clone, Default)]
pub struct IpHashData {
    pub cpu: u32,
    pub tc_handle: u32,
    pub circuit_id: u64,
    pub device_id: u64,
}

#[cfg(test)]
mod test {
    use super::IpHashData;

    #[test]
    fn ip_hash_data_size() {
        assert_eq!(std::mem::size_of::<IpHashData>(), 24);
    }
}
