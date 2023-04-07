use lqos_bus::long_term_stats::StatsSubmission;
use once_cell::sync::Lazy;
use tokio::sync::Mutex;

struct QueueSubmission {
    attempts: u8,
    body: StatsSubmission,
    sent: bool,
}

pub(crate) struct Queue {
    queue: Mutex<Vec<QueueSubmission>>,
}

impl Queue {
    fn new() -> Self {
        Self {
            queue: Mutex::new(Vec::new()),
        }
    }

    pub async fn push(&self, data: lqos_bus::long_term_stats::StatsSubmission, host: &str) {
        {
            let mut lock = self.queue.lock().await;
            lock.push(QueueSubmission {
                attempts: 0,
                sent: false,
                body: data,
            });
        }
        tokio::spawn(send_queue(host.to_string()));
    }
}

pub(crate) static QUEUE: Lazy<Queue> = Lazy::new(Queue::new);

async fn send_queue(host: String) {
    let url = format!("http://{host}/submit");

    let mut lock = QUEUE.queue.lock().await;
    if lock.is_empty() {
        return;
    }

    for s in lock.iter_mut() {
        let client = reqwest::Client::new();
        let res = client.post(&url)
            .json(&s.body)
            .send();

        match res.await {
            Ok(_) => {
                s.sent = true;
            }
            Err(e) => {
                log::warn!("Error sending stats: {}", e);
                s.attempts += 1;
            }
        }
    }

    lock.retain(|s| !s.sent);
    lock.retain(|s| s.attempts < 200);
}