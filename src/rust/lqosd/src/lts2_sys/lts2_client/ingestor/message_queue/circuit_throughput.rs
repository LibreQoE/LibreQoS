use crate::lts2_sys::shared_types::{CircuitThroughput, IngestSession};

pub(crate) fn add_circuit_throughput(message: &mut IngestSession, queue: &mut Vec<CircuitThroughput>) {
    while let Some(msg) = queue.pop() {
        message.circuit_throughput.push(CircuitThroughput {
            timestamp: msg.timestamp,
            circuit_hash: msg.circuit_hash,
            download_bytes: msg.download_bytes,
            upload_bytes: msg.upload_bytes,
            packets_down: msg.packets_down,
            packets_up: msg.packets_up,
            packets_tcp_down: msg.packets_tcp_down,
            packets_tcp_up: msg.packets_tcp_up,
            packets_udp_down: msg.packets_udp_down,
            packets_udp_up: msg.packets_udp_up,
            packets_icmp_down: msg.packets_icmp_down,
            packets_icmp_up: msg.packets_icmp_up,
        });
    }
}
