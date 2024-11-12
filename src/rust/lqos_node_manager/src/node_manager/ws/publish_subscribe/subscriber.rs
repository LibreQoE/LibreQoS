use std::sync::Arc;
use tokio::sync::mpsc::Sender;

pub(super) struct Subscriber {
    pub(super) is_alive: bool,
    pub(super) sender: Sender<Arc<String>>
}

