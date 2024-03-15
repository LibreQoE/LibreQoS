//! Support for the Netflow 5 protocol
//! Mostly taken from: https://netflow.caligare.com/netflow_v5.htm
mod protocol;
use super::{FlowAnalysis, FlowbeeLocalData, FlowbeeRecipient};
use lqos_sys::flowbee_data::FlowbeeKey;
pub(crate) use protocol::*;
use std::{
    net::UdpSocket,
    sync::{atomic::AtomicU32, Arc, Mutex},
};

pub(crate) struct Netflow5 {
    socket: UdpSocket,
    sequence: AtomicU32,
    target: String,
    send_queue: Mutex<Vec<(FlowbeeKey, FlowbeeLocalData)>>,
}

impl Netflow5 {
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

            let send_chunks = lock.chunks(15);
            for to_send in send_chunks {
                let num_records = (to_send.len() * 2) as u16;
                let sequence = self.sequence.load(std::sync::atomic::Ordering::Relaxed);
                let header = Netflow5Header::new(sequence, num_records);
                let header_bytes = unsafe {
                    std::slice::from_raw_parts(
                        &header as *const _ as *const u8,
                        std::mem::size_of::<Netflow5Header>(),
                    )
                };
    
                let mut buffer = Vec::with_capacity(
                    header_bytes.len() + to_send.len() * 2 * std::mem::size_of::<Netflow5Record>(),
                );
    
                buffer.extend_from_slice(header_bytes);
                for (key, data) in to_send {
                    if let Ok((packet1, packet2)) = to_netflow_5(key, data) {
                        let packet1_bytes = unsafe {
                            std::slice::from_raw_parts(
                                &packet1 as *const _ as *const u8,
                                std::mem::size_of::<Netflow5Record>(),
                            )
                        };
                        let packet2_bytes = unsafe {
                            std::slice::from_raw_parts(
                                &packet2 as *const _ as *const u8,
                                std::mem::size_of::<Netflow5Record>(),
                            )
                        };
                        buffer.extend_from_slice(packet1_bytes);
                        buffer.extend_from_slice(packet2_bytes);
                    }
                }

                self.socket.send_to(&buffer, &self.target).unwrap();
                self.sequence.fetch_add(num_records as u32, std::sync::atomic::Ordering::Relaxed);    
            }
            lock.clear();
        }
    }
}

impl FlowbeeRecipient for Netflow5 {
    fn enqueue(&self, key: FlowbeeKey, data: FlowbeeLocalData, _analysis: FlowAnalysis) {
        let mut lock = self.send_queue.lock().unwrap();
        lock.push((key, data));
    }
}
