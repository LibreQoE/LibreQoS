use crate::lts2_sys::shared_types::{IngestSession, SiteCakeDrops};

pub(crate) fn add_site_cake_drops(message: &mut IngestSession, queue: &mut Vec<SiteCakeDrops>) {
    while let Some(circuit_cake_marks) = queue.pop() {
        message.site_cake_drops.push(circuit_cake_marks);
    }
}
