/// Currently unused, represents the current operation mode of the Heimdall
/// sub-system. Defaults to 1.
#[repr(u8)]
pub enum HeimdallMode {
  /// Do not monitor
  Off = 0,
  /// Only look at flows on hosts we are watching via the circuit monitor
  WatchOnly = 1,
  /// Capture detailed packet data from flows
  Analysis = 2,
}

/// Configuration options passed to Heimdall
#[derive(Default, Clone)]
#[repr(C)]
pub struct HeimdalConfig {
  /// Current operation mode
  pub mode: u32,
}