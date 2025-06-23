use self::protocol::to_netflow_9;
use super::{FlowAnalysis, FlowbeeLocalData};
use crate::throughput_tracker::flow_data::netflow9::protocol::{
    header::Netflow9Header, template_ipv4::template_data_ipv4, template_ipv6::template_data_ipv6,
};
use crossbeam_channel::Sender;
use lqos_sys::flowbee_data::FlowbeeKey;
use std::{net::UdpSocket, sync::atomic::AtomicU32};
mod protocol;

pub(crate) struct Netflow9 {}

impl Netflow9 {
    pub(crate) fn new(
        target: String,
    ) -> anyhow::Result<Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>> {
        let (tx, rx) =
            crossbeam_channel::bounded::<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>(65535);
        let socket = UdpSocket::bind("0.0.0.0:0")?;

        std::thread::Builder::new()
            .name("Netflow9".to_string())
            .spawn(move || {
                let mut accumulator = Vec::with_capacity(14);
                let sequence = AtomicU32::new(0);
                let mut last_sent = std::time::Instant::now();
                while let Ok((key, (data, analysis))) = rx.recv() {
                    // Exclude one-way flows
                    if (data.bytes_sent.sum()) == 0 {
                        continue;
                    }

                    accumulator.push((key, (data, analysis)));

                    // Send if there is more than 15 records AND it has been more than 1 second since the last send
                    if accumulator.len() >= 14 && last_sent.elapsed().as_secs() > 1 {
                        for chunk in accumulator.chunks(14) {
                            Self::queue_handler(chunk, &socket, &target, &sequence);
                        }
                        accumulator.clear();
                        last_sent = std::time::Instant::now();
                    }
                }
                
                // Handle any remaining flows when shutting down
                if !accumulator.is_empty() {
                    for chunk in accumulator.chunks(14) {
                        Self::queue_handler(chunk, &socket, &target, &sequence);
                    }
                }
            })?;

        Ok(tx)
    }

    fn queue_handler(
        accumulator: &[(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))],
        socket: &UdpSocket,
        target: &str,
        sequence: &AtomicU32,
    ) {
        let num_records = (accumulator.len() * 2) as u16 + 2; // +2 to include templates
        let sequence_num = sequence.load(std::sync::atomic::Ordering::Relaxed);
        let header = Netflow9Header::new(sequence_num, num_records);
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                &header as *const _ as *const u8,
                std::mem::size_of::<Netflow9Header>(),
            )
        };
        let template1 = template_data_ipv4();
        let template2 = template_data_ipv6();
        let mut buffer = Vec::with_capacity(
            header_bytes.len() + template1.len() + template2.len() + (num_records as usize * 140),
        );
        buffer.extend_from_slice(header_bytes);
        buffer.extend_from_slice(&template1);
        buffer.extend_from_slice(&template2);

        for (key, (data, _)) in accumulator {
            if let Ok((packet1, packet2)) = to_netflow_9(key, data) {
                buffer.extend_from_slice(&packet1);
                buffer.extend_from_slice(&packet2);
            }
        }
        if let Err(e) = socket.send_to(&buffer, target) {
            tracing::error!("Failed to send Netflow9 data to {}: {}", target, e);
            // Don't increment sequence on failure to maintain consistency
        } else {
            sequence.fetch_add(num_records as u32, std::sync::atomic::Ordering::Relaxed);
        }
    }
}
