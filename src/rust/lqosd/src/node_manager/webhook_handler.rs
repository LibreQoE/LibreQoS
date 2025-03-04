use axum::extract::Json;
use lqos_config::load_config;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::{
    mpsc::{Sender, channel},
    Mutex,
};
use tracing::{info, error};

use crate::shaped_devices_tracker::WEBHOOK_REFRESH_COMPLETED;

/// List of valid UISP webhook event types that impact client's services
const VALID_EVENT_TYPES: &[&str] = &[
    "service.activate",
    "service.add",
    "service.archive",
    "service.edit",
    "service.end",
    "service.suspend",
    "service.suspend_cancel",
];

/// Webhook refresh queue state
pub struct WebhookRefreshState {
    /// Channel to signal refresh tasks
    refresh_tx: Sender<()>,
    /// Flag to track if a refresh is currently running
    is_refreshing: Arc<Mutex<bool>>,
}

impl WebhookRefreshState {
    /// Create a new webhook refresh state manager
    pub fn new() -> Self {
        // Create a channel with a small buffer to allow queuing
        let (refresh_tx, mut refresh_rx) = channel(10);
        let is_refreshing = Arc::new(Mutex::new(false));
        
        let is_refreshing_clone = is_refreshing.clone();
        
        // Background task to manage refresh queue
        tokio::spawn(async move {
            while let Some(_) = refresh_rx.recv().await {
                // Ensure only one refresh runs at a time
                {
                    let mut refreshing = is_refreshing_clone.lock().await;
                    if *refreshing {
                        continue; // Skip if already refreshing
                    }
                    *refreshing = true;
                }
                
                // Reset the completion flag before starting
                WEBHOOK_REFRESH_COMPLETED.store(false, Ordering::Relaxed);
                
                // Perform the refresh
                match load_config() {
                    Ok(config) => {
                        let lqos_dir = Path::new(&config.lqos_directory);
                        let scheduler_py = lqos_dir.join("scheduler.py");
                        
                        info!("Running scheduler.py with --webhook flag from: {}", scheduler_py.display());
                        
                        // Run scheduler.py and wait for its completion
                        match tokio::process::Command::new("python3")
                            .arg(&scheduler_py)
                            .arg("--webhook")
                            .current_dir(lqos_dir)
                            .output()
                            .await
                        {
                            Ok(output) => {
                                info!("Scheduler.py webhook handler completed with status: {}", output.status);
                                if !output.status.success() {
                                    let stderr = String::from_utf8_lossy(&output.stderr);
                                    error!("Scheduler.py webhook handler stderr: {}", stderr);
                                }
                                
                                // Wait for completion with timeout
                                let start = Instant::now();
                                loop {
                                    if WEBHOOK_REFRESH_COMPLETED.load(Ordering::Relaxed) {
                                        break;
                                    }
                                    
                                    if start.elapsed() > Duration::from_secs(30) {
                                        error!("Refresh watchers timed out");
                                        break;
                                    }
                                    
                                    tokio::time::sleep(Duration::from_millis(100)).await;
                                }
                            },
                            Err(e) => error!("Failed to run scheduler.py: {}", e),
                        }
                    },
                    Err(e) => error!("Failed to load config: {}", e),
                }
                
                // Mark refresh as complete
                {
                    let mut refreshing = is_refreshing_clone.lock().await;
                    *refreshing = false;
                }
            }
        });
        
        Self { 
            refresh_tx, 
            is_refreshing 
        }
    }

    /// Handle incoming webhook
    pub async fn handle_webhook(&self, payload: Json<Value>) -> String {
        if let Some(event) = payload.get("eventName").and_then(Value::as_str) {
            let valid_event_types: HashSet<&str> = VALID_EVENT_TYPES.iter().cloned().collect();
            if valid_event_types.contains(event) {
                info!("Received valid webhook event: {}", event);
                
                // Check if a refresh is already in progress
                let is_refreshing = self.is_refreshing.lock().await;
                
                // Always try to send a refresh signal 
                if let Err(e) = self.refresh_tx.try_send(()) {
                    error!("Could not queue webhook refresh: {}", e);
                    return serde_json::json!({
                        "status": "error",
                        "message": "Failed to queue LibreQOS refresh"
                    }).to_string();
                }
                
                return serde_json::json!({
                    "status": "success",
                    "message": "LibreQOS partial reload queued by webhook",
                    "is_refreshing": *is_refreshing
                }).to_string();
            }
        }
        
        serde_json::json!({
            "status": "ignored",
            "message": "LibreQOS ignored webhook, event not relevant"
        }).to_string()
    }
}

/// Global webhook refresh state management
#[derive(Clone)]
pub struct WebhookManager {
    state: Arc<WebhookRefreshState>,
}

impl WebhookManager {
    /// Create a new webhook manager
    pub async fn new() -> Self {
        Self {
            state: Arc::new(WebhookRefreshState::new()),
        }
    }

    /// Get a clone of the internal state for routing
    pub fn get_state(&self) -> Arc<WebhookRefreshState> {
        self.state.clone()
    }
}