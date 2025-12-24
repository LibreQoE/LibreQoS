use crate::{
    NUM_QUEUE_HISTORY,
    queue_diff::{CakeDiffTin, QueueDiff, make_queue_diff},
    queue_types::QueueType,
};
use lqos_bus::{CakeDiffTinTransit, CakeDiffTransit, CakeTransit, QueueStoreTransit};
use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub struct QueueStore {
    history: Vec<(QueueDiff, QueueDiff)>,
    history_head: usize,
    prev_download: Option<QueueType>,
    prev_upload: Option<QueueType>,
    current_download: QueueType,
    current_upload: QueueType,
}

impl QueueStore {
    pub(crate) fn new(download: QueueType, upload: QueueType) -> Self {
        Self {
            history: vec![(QueueDiff::None, QueueDiff::None); NUM_QUEUE_HISTORY],
            history_head: 0,
            prev_upload: None,
            prev_download: None,
            current_download: download,
            current_upload: upload,
        }
    }

    pub(crate) fn update(&mut self, download: &QueueType, upload: &QueueType) {
        self.prev_upload = Some(self.current_upload.clone());
        self.prev_download = Some(self.current_download.clone());
        self.current_download = download.clone();
        self.current_upload = upload.clone();
        let Some(prev_up) = self.prev_upload.as_ref() else {
            tracing::info!("QueueStore.update: previous upload state missing; skipping update");
            return;
        };
        let Some(prev_dn) = self.prev_download.as_ref() else {
            tracing::info!("QueueStore.update: previous download state missing; skipping update");
            return;
        };
        let new_diff_up = make_queue_diff(prev_up, &self.current_upload);
        let new_diff_dn = make_queue_diff(prev_dn, &self.current_download);

        if let (Ok(new_diff_dn), Ok(new_diff_up)) = (new_diff_dn, new_diff_up) {
            self.history[self.history_head] = (new_diff_dn, new_diff_up);
            self.history_head += 1;
            if self.history_head >= NUM_QUEUE_HISTORY {
                self.history_head = 0;
            }
        }
    }
}

// Note: I'm overriding the warning because the "from only" behaviour
// is actually what we want here.
#[allow(clippy::from_over_into)]
impl Into<QueueStoreTransit> for QueueStore {
    fn into(self) -> QueueStoreTransit {
        // Determine queue kinds for display
        let kind_down = match &self.current_download {
            QueueType::Cake(_) => "cake",
            QueueType::FqCodel(_) => "fq_codel",
            _ => "none",
        };
        let kind_up = match &self.current_upload {
            QueueType::Cake(_) => "cake",
            QueueType::FqCodel(_) => "fq_codel",
            _ => "none",
        };
        QueueStoreTransit {
            history: self
                .history
                .iter()
                .cloned()
                .map(|(a, b)| (a.into(), b.into()))
                .collect(),
            history_head: self.history_head,
            //prev_download: self.prev_download.map(|d| d.into()),
            //prev_upload: self.prev_upload.map(|u| u.into()),
            current_download: self.current_download.into(),
            current_upload: self.current_upload.into(),
            kind_down: kind_down.to_string(),
            kind_up: kind_up.to_string(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<CakeDiffTransit> for QueueDiff {
    fn into(self) -> CakeDiffTransit {
        match &self {
            QueueDiff::Cake(c) => CakeDiffTransit {
                bytes: c.bytes,
                packets: c.packets,
                qlen: c.qlen,
                tins: c.tins.iter().cloned().map(|t| t.into()).collect(),
            },
            QueueDiff::FqCodel(c) => {
                // Map fq_codel stats into a Cake-like transit so the UI can render.
                // Pad to 4 tins to match typical diffserv4 rendering assumptions in the UI.
                let mut tins = Vec::with_capacity(4);
                tins.push(CakeDiffTinTransit {
                    sent_bytes: c.bytes,
                    backlog_bytes: c.backlog,
                    drops: c.ddrops,
                    marks: 0,
                    base_delay_us: 0,
                });
                // Add three zeroed tins for UI expectations
                for _ in 0..3 {
                    tins.push(CakeDiffTinTransit::default());
                }
                CakeDiffTransit {
                    bytes: c.bytes,
                    packets: c.packets,
                    qlen: c.backlog,
                    tins,
                }
            }
            _ => CakeDiffTransit::default(),
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<CakeDiffTinTransit> for CakeDiffTin {
    fn into(self) -> CakeDiffTinTransit {
        CakeDiffTinTransit {
            sent_bytes: self.sent_bytes,
            backlog_bytes: self.backlog_bytes,
            drops: self.drops,
            marks: self.marks,
            base_delay_us: self.base_delay_us,
        }
    }
}

#[allow(clippy::from_over_into)]
impl Into<CakeTransit> for QueueType {
    fn into(self) -> CakeTransit {
        if let QueueType::Cake(c) = self {
            CakeTransit {
                //handle: c.handle,
                //parent: c.parent,
                //options: c.options.into(),
                //bytes: c.bytes,
                //packets: c.packets,
                //overlimits: c.overlimits,
                //requeues: c.requeues,
                //backlog: c.backlog,
                //qlen: c.qlen,
                memory_used: c.memory_used,
                //memory_limit: c.memory_limit,
                //capacity_estimate: c.capacity_estimate,
                //min_network_size: c.min_network_size,
                //max_network_size: c.max_network_size,
                //min_adj_size: c.min_adj_size,
                //max_adj_size: c.max_adj_size,
                //avg_hdr_offset: c.avg_hdr_offset,
                //tins: c.tins.iter().cloned().map(|t| t.into()).collect(),
                //drops: c.drops,
            }
        } else {
            CakeTransit::default()
        }
    }
}

/*
#[allow(clippy::from_over_into)]
impl Into<CakeOptionsTransit> for TcCakeOptions {
  fn into(self) -> CakeOptionsTransit {
    CakeOptionsTransit {
      rtt: self.rtt,
      bandwidth: self.bandwidth as u8,
      diffserv: self.diffserv as u8,
      flowmode: self.flowmode as u8,
      ack_filter: self.ack_filter as u8,
      nat: self.nat,
      wash: self.wash,
      ingress: self.ingress,
      split_gso: self.split_gso,
      raw: self.raw,
      overhead: self.overhead,
      fwmark: self.fwmark,
    }
  }
}

#[allow(clippy::from_over_into)]
impl Into<CakeTinTransit> for TcCakeTin {
  fn into(self) -> CakeTinTransit {
    CakeTinTransit {
      //threshold_rate: self.threshold_rate,
      //sent_bytes: self.sent_bytes,
      //backlog_bytes: self.backlog_bytes,
      //target_us: self.target_us,
      //interval_us: self.interval_us,
      //peak_delay_us: self.peak_delay_us,
      //avg_delay_us: self.avg_delay_us,
      //base_delay_us: self.base_delay_us,
      //sent_packets: self.sent_packets,
      //way_indirect_hits: self.way_indirect_hits,
      //way_misses: self.way_misses,
      //way_collisions: self.way_collisions,
      //drops: self.drops,
      //ecn_marks: self.ecn_marks,
      //ack_drops: self.ack_drops,
      //sparse_flows: self.sparse_flows,
      //bulk_flows: self.bulk_flows,
      //unresponsive_flows: self.unresponsive_flows,
      //max_pkt_len: self.max_pkt_len,
      //flow_quantum: self.flow_quantum,
    }
  }
}
*/
