use crate::lts2_sys::shared_types::{IngestSession, SiteThroughput};

pub(crate) fn add_site_throughput(message: &mut IngestSession, queue: &mut Vec<SiteThroughput>) {
    while let Some(site_throughput) = queue.pop() {
        message.site_throughput.push(site_throughput);
    }
}
