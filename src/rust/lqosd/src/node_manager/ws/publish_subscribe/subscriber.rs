use std::sync::Arc;
use allocative::Allocative;
use tokio::sync::mpsc::Sender;

#[derive(Allocative)]
pub(super) struct Subscriber {
    pub(super) is_alive: bool,
    #[allocative(skip)]
    pub(super) sender: Sender<Arc<String>>,
}
