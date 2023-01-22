//! Direct byte representations of Netlink encoded chunks.

use zerocopy::FromBytes;

/// Represents a u64, with a type indicator
#[repr(C, packed)]
#[derive(Copy, Clone, FromBytes)]
pub struct Nla64 {
  length: u16,
  nla_type: u16,
  value: u64,
}

/// Represents a u32, with a type indicator
#[repr(C, packed)]
#[derive(Copy, Clone, FromBytes)]
pub struct Nla32 {
  length: u16,
  nla_type: u16,
  value: u32,
}
