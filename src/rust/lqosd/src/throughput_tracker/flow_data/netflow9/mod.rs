use crate::throughput_tracker::flow_data::netflow9::protocol::{
    header::Netflow9Header, template_ipv4::template_data_ipv4, template_ipv6::template_data_ipv6,
};
use lqos_sys::flowbee_data::FlowbeeKey;
use std::{net::UdpSocket, sync::{atomic::AtomicU32, Arc, Mutex}};

use self::protocol::to_netflow_9;
use super::{FlowAnalysis, FlowbeeLocalData, FlowbeeRecipient};
mod protocol;

pub(crate) struct Netflow9 {
    socket: UdpSocket,
    sequence: AtomicU32,
    target: String,
    send_queue: Mutex<Vec<(FlowbeeKey, FlowbeeLocalData)>>,
}

impl Netflow9 {
    pub(crate) fn new(target: String) -> anyhow::Result<Arc<Self>> {
        let socket = UdpSocket::bind("0.0.0.0:12212")?;
        let result = Arc::new(Self {
            socket,
            sequence: AtomicU32::new(0),
            target,
            send_queue: Mutex::new(Vec::new()),
        });
        let thread_result = result.clone();
        std::thread::spawn(move || thread_result.queue_handler());
        Ok(result)
    }

    fn queue_handler(&self) {
        loop {
            let mut lock = self.send_queue.lock().unwrap();
            if lock.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }

            let send_chunks = lock.chunks(14);            
            for to_send in send_chunks {
                let num_records = (to_send.len() * 2) as u16 + 2; // +2 to include templates
                let sequence = self.sequence.load(std::sync::atomic::Ordering::Relaxed);
                let header = Netflow9Header::new(sequence, num_records);
                let header_bytes = unsafe { std::slice::from_raw_parts(&header as *const _ as *const u8, std::mem::size_of::<Netflow9Header>()) };
                let template1 = template_data_ipv4();
                let template2 = template_data_ipv6();
                let mut buffer = Vec::with_capacity(header_bytes.len() + template1.len() + template2.len() + (num_records as usize) * 140);
                buffer.extend_from_slice(header_bytes);
                buffer.extend_from_slice(&template1);
                buffer.extend_from_slice(&template2);

                for (key, data) in to_send {
                    if let Ok((packet1, packet2)) = to_netflow_9(key, data) {
                        buffer.extend_from_slice(&packet1);
                        buffer.extend_from_slice(&packet2);
                    }
                }
                self.socket.send_to(&buffer, &self.target).unwrap();
                self.sequence.fetch_add(num_records as u32, std::sync::atomic::Ordering::Relaxed);
            }
            lock.clear();
        }

    }
}

impl FlowbeeRecipient for Netflow9 {
    fn enqueue(&self, key: FlowbeeKey, data: FlowbeeLocalData, _analysis: FlowAnalysis) {
        let mut lock = self.send_queue.lock().unwrap();
        lock.push((key, data));
    }
}
