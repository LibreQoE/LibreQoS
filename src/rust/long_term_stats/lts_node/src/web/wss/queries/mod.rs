//! Provides pre-packaged queries for obtaining data, that will
//! then be used by the web server to respond to requests.

mod packet_counts;
mod throughput;
pub use packet_counts::send_packets_for_all_nodes;
pub use throughput::send_throughput_for_all_nodes;