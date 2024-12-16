use crate::lts2_sys::shared_types::{IngestSession, SiteRtt};

pub(crate) fn add_site_rtt(message: &mut IngestSession, queue: &mut Vec<SiteRtt>) {
    while let Some(site_rtt) = queue.pop() {
        message.site_rtt.push(SiteRtt {
            timestamp: site_rtt.timestamp,
            site_hash: site_rtt.site_hash,
            median_rtt: site_rtt.median_rtt,
        });
    }
}
