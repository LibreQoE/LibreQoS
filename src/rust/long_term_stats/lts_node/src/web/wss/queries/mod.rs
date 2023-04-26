//! Provides pre-packaged queries for obtaining data, that will
//! then be used by the web server to respond to requests.

mod packet_counts;
pub use packet_counts::send_packets_for_all_nodes;
