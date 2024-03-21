use std::fmt::Display;

use lqos_sys::flowbee_data::FlowbeeKey;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FlowProtocol {
    Smtp,
    Ftp,
    Http,
    Https,
    Ssh,
    Telnet,
    Imap,
    Rdp,
    Dns,
    Pop3,
    Quic,
    Other { proto: u8, src_port: u16, dst_port: u16 }
}

impl FlowProtocol {
    pub fn new(key: &FlowbeeKey) -> Self {
        match key.ip_protocol {
            6 => Self::tcp(key),
            17 => Self::udp(key),
            _ => Self::Other { 
                proto: key.ip_protocol, 
                src_port: key.src_port, 
                dst_port: key.dst_port, 
            }
        }
    }

    fn tcp(key: &FlowbeeKey) -> Self {
        match key.src_port {
            25 => Self::Smtp,
            80 => Self::Http,
            443 => Self::Https,
            21 | 20 => Self::Ftp,
            22 => Self::Ssh,
            23 => Self::Telnet,
            3389 => Self::Rdp,
            143 => Self::Imap,
            53 => Self::Dns,
            110 => Self::Pop3,
            _ => Self::Other { 
                proto: key.ip_protocol, 
                src_port: key.src_port, 
                dst_port: key.dst_port, 
            }
        }
    }

    fn udp(key: &FlowbeeKey) -> Self {
        match key.src_port {
            53 => Self::Dns,
            80 | 443 => Self::Quic,            
            _ => Self::Other { 
                proto: key.ip_protocol, 
                src_port: key.src_port, 
                dst_port: key.dst_port, 
            }
        }
    }
}

impl Display for FlowProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Smtp => write!(f, "SMTP"),
            Self::Ftp => write!(f, "FTP"),
            Self::Http => write!(f, "HTTP"),
            Self::Https => write!(f, "HTTPS"),
            Self::Ssh => write!(f, "SSH"),
            Self::Telnet => write!(f, "Telnet"),
            Self::Imap => write!(f, "IMAP"),
            Self::Rdp => write!(f, "RDP"),
            Self::Dns => write!(f, "DNS"),
            Self::Pop3 => write!(f, "POP3"),
            Self::Quic => write!(f, "QUIC"),
            Self::Other { proto, src_port, dst_port } => write!(f, "{} {}/{}", proto_name(proto), src_port, dst_port),
        }
    }
}

fn proto_name(proto: &u8) -> &'static str {
    match proto {
        6 => "TCP",
        17 => "UDP",
        1 => "ICMP",
        _ => "Other",
    }
}