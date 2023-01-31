#[repr(C)]
#[derive(Clone, Default)]
pub struct IpHashData {
  pub cpu: u32,
  pub tc_handle: u32,
}
