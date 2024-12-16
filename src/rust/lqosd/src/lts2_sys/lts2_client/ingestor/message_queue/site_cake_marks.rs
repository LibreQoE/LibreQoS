use crate::lts2_sys::shared_types::{IngestSession, SiteCakeMarks};

pub(crate) fn add_site_cake_marks(message: &mut IngestSession, queue: &mut Vec<SiteCakeMarks>) {
    while let Some(site_cake_marks) = queue.pop() {
        message.site_cake_marks.push(site_cake_marks);
    }
}
