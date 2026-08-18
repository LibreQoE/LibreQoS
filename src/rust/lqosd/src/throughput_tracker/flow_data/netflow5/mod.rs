//! Support for the Netflow 5 protocol
//! Mostly taken from: https://netflow.caligare.com/netflow_v5.htm
mod protocol;
use super::{FlowAnalysis, FlowbeeLocalData};
use crossbeam_channel::Sender;
use lqos_sys::flowbee_data::FlowbeeKey;
pub(crate) use protocol::*;
use std::{net::UdpSocket, sync::atomic::AtomicU32};

pub(crate) struct Netflow5 {}

impl Netflow5 {
    pub(crate) fn start(
        target: String,
    ) -> anyhow::Result<Sender<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>> {
        let (tx, rx) =
            crossbeam_channel::bounded::<(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))>(65535);

        std::thread::Builder::new()
            .name("Netflow5".to_string())
            .spawn(move || {
                // Create socket once and reuse it
                let socket = match UdpSocket::bind("0.0.0.0:0") {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!("Failed to create Netflow5 UDP socket: {}", e);
                        return;
                    }
                };

                let sequence = AtomicU32::new(0);
                let mut accumulator = Vec::with_capacity(NETFLOW5_MAX_FLOWS_PER_PACKET);
                let mut last_sent = std::time::Instant::now();
                while let Ok((key, (data, analysis))) = rx.recv() {
                    // Exclude one-way flows
                    if (data.bytes_sent.sum()) == 0 {
                        continue;
                    }

                    accumulator.push((key, (data, analysis)));

                    if accumulator.len() >= NETFLOW5_MAX_FLOWS_PER_PACKET
                        || last_sent.elapsed().as_secs() > 1
                    {
                        Self::flush_accumulator(&accumulator, &socket, &target, &sequence);
                        accumulator.clear();
                        last_sent = std::time::Instant::now();
                    }
                }

                // Handle any remaining flows when shutting down
                if !accumulator.is_empty() {
                    Self::flush_accumulator(&accumulator, &socket, &target, &sequence);
                }
            })?;

        Ok(tx)
    }

    fn flush_accumulator(
        accumulator: &[(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))],
        socket: &UdpSocket,
        target: &str,
        sequence: &AtomicU32,
    ) {
        for chunk in accumulator.chunks(NETFLOW5_MAX_FLOWS_PER_PACKET) {
            Self::queue_handler(chunk, socket, target, sequence);
        }
    }

    fn queue_handler(
        accumulator: &[(FlowbeeKey, (FlowbeeLocalData, FlowAnalysis))],
        socket: &UdpSocket,
        target: &str,
        sequence: &AtomicU32,
    ) {
        if accumulator.is_empty() {
            return;
        }

        if accumulator.len() > NETFLOW5_MAX_FLOWS_PER_PACKET {
            for chunk in accumulator.chunks(NETFLOW5_MAX_FLOWS_PER_PACKET) {
                Self::queue_handler(chunk, socket, target, sequence);
            }
            return;
        }

        let mut records = Vec::with_capacity(accumulator.len() * 2);
        for (key, (data, _)) in accumulator {
            if let Ok((packet1, packet2)) = to_netflow_5(key, data) {
                records.push(packet1);
                records.push(packet2);
            }
        }

        let Ok(num_records) = u16::try_from(records.len()) else {
            tracing::error!("NetFlow5 record count exceeded u16::MAX; dropping export packet");
            return;
        };

        if num_records == 0 {
            return;
        }

        let sequence_number = sequence.load(std::sync::atomic::Ordering::Relaxed);
        let header = Netflow5Header::new(sequence_number, num_records);
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                &header as *const _ as *const u8,
                std::mem::size_of::<Netflow5Header>(),
            )
        };

        let mut buffer = Vec::with_capacity(
            header_bytes.len() + (records.len() * std::mem::size_of::<Netflow5Record>()),
        );

        buffer.extend_from_slice(header_bytes);
        for record in &records {
            let record_bytes = unsafe {
                std::slice::from_raw_parts(
                    record as *const _ as *const u8,
                    std::mem::size_of::<Netflow5Record>(),
                )
            };
            buffer.extend_from_slice(record_bytes);
        }

        if let Err(e) = socket.send_to(&buffer, target) {
            tracing::error!("Failed to send Netflow5 data to {}: {}", target, e);
            // Don't increment sequence on failure to maintain consistency
        } else {
            sequence.fetch_add(u32::from(num_records), std::sync::atomic::Ordering::Relaxed);
        }
    }
}
