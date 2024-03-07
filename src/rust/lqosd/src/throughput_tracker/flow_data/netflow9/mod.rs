use std::net::UdpSocket;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use crate::throughput_tracker::flow_data::netflow9::protocol::{header::Netflow9Header, template_ipv4::template_data_ipv4, template_ipv6::template_data_ipv6};

use self::protocol::to_netflow_9;
use super::FlowbeeRecipient;
mod protocol;

pub(crate) struct Netflow9 {
    socket: UdpSocket,
    sequence: u32,
    target: String,
}

impl Netflow9 {
    pub(crate) fn new(target: String) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:12212")?;
        Ok(Self { socket, sequence: 0, target })
    }
}

impl FlowbeeRecipient for Netflow9 {
    fn send(&mut self, key: FlowbeeKey, data: FlowbeeData) {
        if let Ok((packet1, packet2)) = to_netflow_9(&key, &data) {
            let header = Netflow9Header::new(self.sequence, 4);
            let header_bytes = unsafe { std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of::<Netflow9Header>()) };
            let mut buffer = Vec::with_capacity(header_bytes.len() + packet1.len() + packet2.len());
            buffer.extend_from_slice(header_bytes);
            buffer.extend_from_slice(&template_data_ipv4());
            buffer.extend_from_slice(&template_data_ipv6());
            buffer.extend_from_slice(&packet1);
            buffer.extend_from_slice(&packet2);

            log::debug!("Sending netflow9 packet of size {} to {}", buffer.len(), self.target);
            self.socket.send_to(&buffer, &self.target).unwrap();

            self.sequence = self.sequence.wrapping_add(2);
        }
    }
}