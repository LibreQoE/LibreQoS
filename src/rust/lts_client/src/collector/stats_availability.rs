//! Message type to be sent to the long-term stats thread when
//! data is available.

use lqos_config::ShapedDevice;

use super::{ThroughputSummary, network_tree::NetworkTreeEntry};

#[derive(Debug)]
/// Messages to/from the stats collection thread
pub enum StatsUpdateMessage {
  /// Fresh throughput stats have been collected
  ThroughputReady(Box<(ThroughputSummary, Vec<(usize, NetworkTreeEntry)>)>),
  /// ShapedDevices.csv has changed and the server needs new data
  ShapedDevicesChanged(Vec<ShapedDevice>),
  /// It's time to collate the session buffer
  CollationTime,
  /// The daemon is exiting
  Quit,
  /// Time to gather UISP data
  UispCollationTime,
}