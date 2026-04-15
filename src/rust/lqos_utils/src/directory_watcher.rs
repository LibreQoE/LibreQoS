use notify::{Config, Event, RecursiveMode, Watcher};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    time::{Duration, Instant},
};
use thiserror::Error;
use tracing::{debug, error};

const DIRECTORY_WATCH_DEBOUNCE_MILLIS: u64 = 250;

/// Watches one directory and returns the coalesced changed paths for each batch
/// of file-system notifications.
pub struct DirectoryWatcher {
    nice_name: String,
    path: PathBuf,
    debounce_window: Duration,
    rx: Option<crossbeam_channel::Receiver<Result<Event, notify::Error>>>,
    watcher: Option<notify::RecommendedWatcher>,
}

impl DirectoryWatcher {
    /// Creates a new directory watcher with the default debounce window.
    pub fn new<S: ToString>(nice_name: S, path: PathBuf) -> Self {
        Self {
            nice_name: nice_name.to_string(),
            path,
            debounce_window: Duration::from_millis(DIRECTORY_WATCH_DEBOUNCE_MILLIS),
            rx: None,
            watcher: None,
        }
    }

    /// Waits for the next batch of changed paths in the watched directory.
    pub fn watch(&mut self) -> Result<Vec<PathBuf>, WatchedDirectoryError> {
        if !self.path.is_dir() {
            return Err(WatchedDirectoryError::NotADirectory(self.path.clone()));
        }

        self.ensure_watcher_started()?;
        let Some(rx) = &self.rx else {
            return Err(WatchedDirectoryError::WatchReceiveError);
        };

        loop {
            let first_event = rx
                .recv()
                .map_err(|_| WatchedDirectoryError::WatchReceiveError)?;
            let mut changed_paths = BTreeSet::new();
            collect_paths(first_event, &mut changed_paths);

            let mut deadline = Instant::now() + self.debounce_window;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }

                match rx.recv_timeout(remaining) {
                    Ok(event) => {
                        collect_paths(event, &mut changed_paths);
                        deadline = Instant::now() + self.debounce_window;
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => break,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        return Err(WatchedDirectoryError::WatchReceiveError);
                    }
                }
            }

            if !changed_paths.is_empty() {
                debug!("{} changed: {:?}", self.nice_name, changed_paths);
                return Ok(changed_paths.into_iter().collect());
            }
        }
    }

    fn ensure_watcher_started(&mut self) -> Result<(), WatchedDirectoryError> {
        if self.watcher.is_some() {
            return Ok(());
        }

        let (tx, rx) = crossbeam_channel::bounded(64);
        let maybe_watcher = notify::RecommendedWatcher::new(tx, Config::default());
        let Ok(mut watcher) = maybe_watcher else {
            if let Err(err) = maybe_watcher {
                error!("Unable to create watcher for {}", self.nice_name);
                error!("{err:?}");
            }
            return Err(WatchedDirectoryError::CreateWatcherError);
        };

        watcher
            .watch(&self.path, RecursiveMode::NonRecursive)
            .map_err(|_| WatchedDirectoryError::StartWatcherError)?;

        self.rx = Some(rx);
        self.watcher = Some(watcher);
        Ok(())
    }
}

fn collect_paths(event: Result<Event, notify::Error>, changed_paths: &mut BTreeSet<PathBuf>) {
    match event {
        Ok(event) => {
            changed_paths.extend(event.paths);
        }
        Err(err) => {
            error!("Directory watcher event error: {err:?}");
        }
    }
}

/// Errors that can occur while watching a directory.
#[derive(Debug, Error)]
pub enum WatchedDirectoryError {
    /// The requested watch path does not exist as a directory.
    #[error("Watch path is not a directory: {0}")]
    NotADirectory(PathBuf),

    /// Unable to create the underlying notify watcher.
    #[error("Unable to create watcher")]
    CreateWatcherError,

    /// Unable to start the underlying notify watcher.
    #[error("Unable to start watcher")]
    StartWatcherError,

    /// The watcher channel stopped delivering events.
    #[error("Unable to receive watcher events")]
    WatchReceiveError,
}
