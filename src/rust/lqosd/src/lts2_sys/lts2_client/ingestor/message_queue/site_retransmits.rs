use crate::lts2_sys::shared_types::{IngestSession, SiteRetransmits};

pub(crate) fn add_site_retransmits(message: &mut IngestSession, queue: &mut Vec<SiteRetransmits>) {
    while let Some(site_cake_retransmits) = queue.pop() {
        message.site_retransmits.push(SiteRetransmits {
            timestamp: site_cake_retransmits.timestamp,
            site_hash: site_cake_retransmits.site_hash,
            tcp_retransmits_down: site_cake_retransmits.tcp_retransmits_down,
            tcp_retransmits_up: site_cake_retransmits.tcp_retransmits_up,
        });
    }
}
