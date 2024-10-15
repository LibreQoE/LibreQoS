use tracing::warn;
use lqos_utils::units::DownUpOrder;
use crate::NetworkJsonNode;

/// Type used while updating the network tree with new data.
/// Rather than have a race condition while the updates are performed
/// (and potentially new requests come in, and receive invalid data),
/// we copy the network tree into this structure, and then update this
/// structure. Once the updates are complete, we copy the data back
/// into the main network tree.
pub struct NetworkJsonCounting {
    pub nodes: Vec<NetworkJsonNode>,
}

impl NetworkJsonCounting {
    /// Starts an update cycle. This clones the nodes into
    /// the `NetworkJsonCounting` structure - work will be performed on the clone.
    pub fn begin_update_cycle(nodes: Vec<NetworkJsonNode>) -> Self {
        Self { nodes }
    }

    /// Sets all current throughput values to zero
    /// Note that due to interior mutability, this does not require mutable
    /// access.
    pub fn zero_throughput_and_rtt(&mut self) {
        //log::warn!("Locking network tree for throughput cycle");
        self.nodes.iter_mut().for_each(|n| {
            n.current_throughput.set_to_zero();
            n.current_tcp_retransmits.set_to_zero();
            n.rtts.clear();
            n.current_drops.set_to_zero();
            n.current_marks.set_to_zero();
        });
    }

    /// Add throughput numbers to node entries. Note that this does *not* require
    /// mutable access due to atomics and interior mutability - so it is safe to use
    /// a read lock.
    pub fn add_throughput_cycle(
        &mut self,
        targets: &[usize],
        bytes: (u64, u64),
    ) {
        for idx in targets {
            // Safety first: use "get" to ensure that the node exists
            if let Some(node) = self.nodes.get_mut(*idx) {
                node.current_throughput.checked_add_tuple(bytes);
            } else {
                warn!("No network tree entry for index {idx}");
            }
        }
    }

    /// Record RTT time in the tree. Note that due to interior mutability,
    /// this does not require mutable access.
    pub fn add_rtt_cycle(&self, targets: &[usize], rtt: f32) {
        for idx in targets {
            // Safety first: use "get" to ensure that the node exists
            if let Some(node) = self.nodes.get(*idx) {
                node.rtts.insert((rtt * 100.0) as u16);
            } else {
                warn!("No network tree entry for index {idx}");
            }
        }
    }

    /// Record TCP Retransmits in the tree.
    pub fn add_retransmit_cycle(&mut self, targets: &[usize], tcp_retransmits: DownUpOrder<u64>) {
        for idx in targets {
            // Safety first; use "get" to ensure that the node exists
            if let Some(node) = self.nodes.get_mut(*idx) {
                node.current_tcp_retransmits.checked_add(tcp_retransmits);
            } else {
                warn!("No network tree entry for index {idx}");
            }
        }
    }

    /// Adds a series of CAKE marks and drops to the tree structure.
    pub fn add_queue_cycle(&mut self, targets: &[usize], marks: &DownUpOrder<u64>, drops: &DownUpOrder<u64>) {
        for idx in targets {
            // Safety first; use "get" to ensure that the node exists
            if let Some(node) = self.nodes.get_mut(*idx) {
                node.current_marks.checked_add(*marks);
                node.current_drops.checked_add(*drops);
            } else {
                warn!("No network tree entry for index {idx}");
            }
        }
    }
}