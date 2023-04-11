use log::{error, info};
use notify::{Config, RecursiveMode, Watcher};
use std::{
  path::PathBuf,
  time::{Duration, Instant},
};
use thiserror::Error;

const SLEEP_UNTIL_EXISTS_SECONDS: u64 = 10;
const SLEEP_AFTER_CREATION_SECONDS: u64 = 3;
const SLEEP_AFTER_CHANGE_SECONDS: u64 = 3;
const SLEEP_DEBOUNCE_DURATION: u64 = 1;

/// Provides a convenient mechanism for watching a file for changes.
/// On Linux, it uses `inotify` - this varies for other operating systems.
///
/// Do not create the structure directly: use new(), followed by
/// setting the appropriate callbacks.
///
/// ## Example
///
/// ```rust
/// use lqos_utils::file_watcher::FileWatcher;
/// use std::path::Path;
///
/// let path = Path::new("/opt/libreqos/src").join("ShapedDevices.csv");
/// let mut watcher = FileWatcher::new("ShapedDevices.csv", path);
/// watcher.set_file_changed_callback(|| println!("ShapedDevices.csv has changed"));
/// //let _ = watcher.watch(); // Commented out because the test will hang
/// ```
pub struct FileWatcher {
  nice_name: String,
  path: PathBuf,
  file_created_callback: Option<fn()>,
  file_exists_callback: Option<fn()>,
  file_changed_callback: Option<fn()>,
}

impl FileWatcher {
  /// Creates a new `FileWatcher`.
  ///
  /// ## Arguments
  ///
  /// * `nice_name` - the print-friendly (short) name of the file to watch.
  /// * `path` - a generated `PathBuf` pointing to the file to watch.
  pub fn new<S: ToString>(nice_name: S, path: PathBuf) -> Self {
    Self {
      nice_name: nice_name.to_string(),
      path,
      file_created_callback: None,
      file_exists_callback: None,
      file_changed_callback: None,
    }
  }

  /// Set a callback function to run if the file did not exist
  /// initially, and has been created since execution started.
  pub fn set_file_created_callback(&mut self, callback: fn()) {
    self.file_created_callback = Some(callback);
  }

  /// Set a callback function to run if the file exists when
  /// the watching process being.
  pub fn set_file_exists_callback(&mut self, callback: fn()) {
    self.file_exists_callback = Some(callback);
  }

  /// Set a callback function to run whenever the file changes.
  pub fn set_file_changed_callback(&mut self, callback: fn()) {
    self.file_changed_callback = Some(callback);
  }

  /// Start watching the file. NOTE: this function will only
  /// return if something bad happens. It is designed to be
  /// executed in a thread, and take over the executing thread.
  pub fn watch(&mut self) -> Result<(), WatchedFileError> {
    // Handle the case in which the file does not yet exist
    if !self.path.exists() {
      info!(
        "{} does not exist yet. Waiting for it to appear.",
        self.nice_name
      );
      loop {
        std::thread::sleep(Duration::from_secs(SLEEP_UNTIL_EXISTS_SECONDS));
        if self.path.exists() {
          info!("{} has been created. Waiting a second.", self.nice_name);
          std::thread::sleep(Duration::from_secs(
            SLEEP_AFTER_CREATION_SECONDS,
          ));
          if let Some(callback) = &mut self.file_created_callback {
            callback();
          }
          break;
        }
      }
    } else if let Some(callback) = &mut self.file_exists_callback {
      callback();
    }

    // Build the watcher
    let (tx, rx) = std::sync::mpsc::channel();
    let watcher = notify::RecommendedWatcher::new(tx, Config::default());
    if watcher.is_err() {
      error!("Unable to create watcher for ShapedDevices.csv");
      error!("{:?}", watcher);
      return Err(WatchedFileError::CreateWatcherError);
    }
    let mut watcher = watcher.unwrap();

    // Try to start watching for changes
    let retval = watcher.watch(&self.path, RecursiveMode::NonRecursive);
    if retval.is_err() {
      error!("Unable to start watcher for ShapedDevices.csv");
      error!("{:?}", retval);
      return Err(WatchedFileError::StartWatcherError);
    }
    let mut last_event: Option<Instant> = None;
    loop {
      let ret = rx.recv();
      if ret.is_err() {
        error!("Error from monitor thread, watching {}", self.nice_name);
        error!("{:?}", ret);
      }

      // A change event has arrived
      // Are we taking a short break to avoid duplicates?
      let mut process = true;
      if let Some(last_event) = last_event {
        if last_event.elapsed().as_secs() < SLEEP_DEBOUNCE_DURATION {
          process = false;
          //info!("Ignoring duplicate event");
        }
      }

      if process {
        std::thread::sleep(Duration::from_secs(SLEEP_AFTER_CHANGE_SECONDS));
        last_event = Some(Instant::now());
        info!("{} changed", self.nice_name);
        if let Some(callback) = &mut self.file_changed_callback {
          callback();
          return Ok(()); // Bail out to restart
        }
      }
    }
  }
}

/// Errors that can occur when watching a file.
#[derive(Error, Debug)]
pub enum WatchedFileError {
  /// Unable to create the file watcher.
  #[error("Unable to create watcher")]
  CreateWatcherError,

  /// Unable to start the file watcher system.
  #[error("Unable to start watcher")]
  StartWatcherError,
}
