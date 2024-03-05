//! Support for the Netflow 5 protocol
mod protocol;
use std::net::UdpSocket;
use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};
use super::FlowbeeRecipient;
pub(crate) use protocol::*;

pub(crate) struct Netflow5 {
    socket: UdpSocket,
    sequence: u32,
    target: String,
}

impl Netflow5 {
    pub(crate) fn new(target: String) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:12212")?;
        Ok(Self { socket, sequence: 0, target })
    }
}

impl FlowbeeRecipient for Netflow5 {
    fn send(&mut self, key: FlowbeeKey, data: FlowbeeData) {
        if let Ok((packet1, packet2)) = to_netflow_5(&key, &data) {
            let header = Netflow5Header::new(self.sequence);
            let header_bytes = unsafe { std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of::<Netflow5Header>()) };
            let packet1_bytes = unsafe { std::slice::from_raw_parts(&packet1 as *const _ as *const u8, std::mem::size_of::<Netflow5Record>()) };
            let packet2_bytes = unsafe { std::slice::from_raw_parts(&packet2 as *const _ as *const u8, std::mem::size_of::<Netflow5Record>()) };
            let mut buffer = Vec::with_capacity(header_bytes.len() + packet1_bytes.len() + packet2_bytes.len());
            buffer.extend_from_slice(header_bytes);
            buffer.extend_from_slice(packet1_bytes);
            buffer.extend_from_slice(packet2_bytes);

            //log::debug!("Sending netflow packet to {target}", target = self.target);
            self.socket.send_to(&buffer, &self.target).unwrap();

            self.sequence = self.sequence.wrapping_add(2);
        }
    }
}
