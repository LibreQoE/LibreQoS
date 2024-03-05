use std::{net::UdpSocket, time::Instant};

use lqos_sys::flowbee_data::{FlowbeeData, FlowbeeKey};

use self::protocol::{to_netflow_9, Netflow9Header};

use super::FlowbeeRecipient;

mod protocol;

pub(crate) struct Netflow9 {
    socket: UdpSocket,
    sequence: u32,
    target: String,
    last_sent_template: Option<Instant>,
}

impl Netflow9 {
    pub(crate) fn new(target: String) -> anyhow::Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:12212")?;
        Ok(Self { socket, sequence: 0, target, last_sent_template: None})
    }
}

impl FlowbeeRecipient for Netflow9 {
    fn send(&mut self, key: FlowbeeKey, data: FlowbeeData) {
        let mut needs_template = false;
        if let Some(last_sent_template) = self.last_sent_template {
            if last_sent_template.elapsed().as_secs() > 60 {
                needs_template = true;
            }
        } else {
            needs_template = true;
        }

        if needs_template {
            let template = protocol::template_data_ipv4(self.sequence);
            self.socket.send_to(&template, &self.target).unwrap();
            self.last_sent_template = Some(Instant::now());
        }

        if let Ok((packet1, packet2)) = to_netflow_9(&key, &data) {
            let header = Netflow9Header::new(self.sequence);
            let header_bytes = unsafe { std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of::<Netflow9Header>()) };
            let mut buffer = Vec::with_capacity(header_bytes.len() + packet1.len() + packet2.len());
            buffer.extend_from_slice(header_bytes);
            buffer.extend_from_slice(&packet1);
            buffer.extend_from_slice(&packet2);

            //log::debug!("Sending netflow packet to {target}", target = self.target);
            self.socket.send_to(&buffer, &self.target).unwrap();

            self.sequence = self.sequence.wrapping_add(2);
        }
    }
}