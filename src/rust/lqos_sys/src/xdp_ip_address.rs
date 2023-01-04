use byteorder::{BigEndian, ByteOrder};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// XdpIpAddress provides helpful conversion between the XDP program's
/// native storage of IP addresses in `[u8; 16]` blocks of bytes and
/// Rust `IpAddr` types.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct XdpIpAddress(pub [u8; 16]);

impl Default for XdpIpAddress {
    fn default() -> Self {
        Self ([0xFF; 16])
    }
}

impl XdpIpAddress {
    /// Converts a Rust `IpAddr` type into an `XdpIpAddress`.
    /// 
    /// # Arguments
    /// 
    /// * `ip` - the IP Address to convert.
    pub fn from_ip(ip: IpAddr) -> Self {
        let mut result = Self::default();
        match ip {
            IpAddr::V4(ip) => {
                result.0[12] = ip.octets()[0];
                result.0[13] = ip.octets()[1];
                result.0[14] = ip.octets()[2];
                result.0[15] = ip.octets()[3];
            }
            IpAddr::V6(ip) => {
                for i in 0..8 {
                    let base = i * 2;
                    result.0[base] = ip.octets()[base];
                    result.0[base+1] = ip.octets()[base + 1];
                }
            }
        }

        result
    }

    /// Converts an `XdpIpAddress` type to a Rust `IpAddr` type
    pub fn as_ip(&self) -> IpAddr {
        if self.0[0] == 0xFF
            && self.0[1] == 0xFF
            && self.0[2] == 0xFF
            && self.0[3] == 0xFF
            && self.0[4] == 0xFF
            && self.0[5] == 0xFF
            && self.0[6] == 0xFF
            && self.0[7] == 0xFF
            && self.0[8] == 0xFF
            && self.0[9] == 0xFF
            && self.0[10] == 0xFF
            && self.0[11] == 0xFF
        {
            // It's an IPv4 Address
            IpAddr::V4(Ipv4Addr::new(
                self.0[12],
                self.0[13],
                self.0[14],
                self.0[15],
            ))
        } else {
            // It's an IPv6 address
            IpAddr::V6(Ipv6Addr::new(
                BigEndian::read_u16(&self.0[0..2]),
                BigEndian::read_u16(&self.0[2..4]),
                BigEndian::read_u16(&self.0[4..6]),
                BigEndian::read_u16(&self.0[6..8]),
                BigEndian::read_u16(&self.0[8..10]),
                BigEndian::read_u16(&self.0[10..12]),
                BigEndian::read_u16(&self.0[12..14]),
                BigEndian::read_u16(&self.0[14..]),
            ))
        }
    }
}

impl Into<IpAddr> for XdpIpAddress {
    fn into(self) -> IpAddr {
        self.as_ip()
    }
}

impl From<IpAddr> for XdpIpAddress {
    fn from(ip: IpAddr) -> Self {
        Self::from_ip(ip)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_default_xdp_ip() {
        let default = XdpIpAddress::default();
        assert_eq!(default.0, [0xFF; 16]);
    }

    #[test]
    fn test_from_ipv4() {
        let ip = XdpIpAddress::from_ip("1.2.3.4".parse().unwrap());
        for n in 0..12 {
            assert_eq!(ip.0[n], 0xFF);
        }
        assert_eq!(ip.0[12], 1);
        assert_eq!(ip.0[13], 2);
        assert_eq!(ip.0[14], 3);
        assert_eq!(ip.0[15], 4);
    }

    #[test]
    fn test_to_ipv4() {
        let raw_ip = XdpIpAddress([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3, 4]);
        let ip = raw_ip.as_ip();
        let intended_ip : IpAddr = "1.2.3.4".parse().unwrap();
        assert_eq!(ip, intended_ip);
    }

    #[test]
    fn test_ipv6_round_trip() {
        let ipv6 = IpAddr::V6("2001:db8:85a3::8a2e:370:7334".parse().unwrap());
        let xip = XdpIpAddress::from_ip(ipv6);
        let test = xip.as_ip();
        assert_eq!(ipv6, test);
    }
}