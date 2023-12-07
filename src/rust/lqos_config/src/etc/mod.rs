//! Manages the `/etc/lqos.conf` file.

mod etclqos_migration;
pub use etclqos_migration::*;
mod v15;
mod python_migration;
mod migration;

#[cfg(test)]
pub mod test_data;

pub use migration::migrate_if_needed;