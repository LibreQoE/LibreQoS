use crate::{
    queue_diff::{make_queue_diff, QueueDiff},
    queue_types::QueueType,
    NUM_QUEUE_HISTORY,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
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
        let new_diff_up = make_queue_diff(self.prev_upload.as_ref().unwrap(), &self.current_upload);
        let new_diff_dn =
            make_queue_diff(self.prev_download.as_ref().unwrap(), &self.current_download);

        if let (Ok(new_diff_dn), Ok(new_diff_up)) = (new_diff_dn, new_diff_up) {
            self.history[self.history_head] = (new_diff_dn, new_diff_up);
            self.history_head += 1;
            if self.history_head >= NUM_QUEUE_HISTORY {
                self.history_head = 0;
            }
        }
    }
}
