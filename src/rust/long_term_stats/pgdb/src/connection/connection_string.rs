//! Manages access to the safely stored connection string, in `/etc/lqdb`.
//! Failure to obtain a database connection is a fatal error.
//! The connection string is read once, on the first call to `get_connection_string()`.
//! Please be careful to never include `/etc/lqdb` in any git commits.

use std::path::Path;
use std::fs::File;
use std::io::Read;
use once_cell::sync::Lazy;

pub static CONNECTION_STRING: Lazy<String> = Lazy::new(read_connection_string);

/// Read the connection string from /etc/lqdb
/// Called by the `Lazy` on CONNECTION_STRING
fn read_connection_string() -> String {
    let path = Path::new("/etc/lqdb");
    if !path.exists() {
        log::error!("{} does not exist", path.display());
        panic!("{} does not exist", path.display());
    }

    match File::open(path) {
        Ok(mut file) => {
            let mut buf = String::new();
            if let Ok(_size) = file.read_to_string(&mut buf) {
                buf
            } else {
                log::error!("Could not read {}", path.display());
                panic!("Could not read {}", path.display());
            }
        }
        Err(e) => {
            log::error!("Could not open {}: {e:?}", path.display());
            panic!("Could not open {}: {e:?}", path.display());
        }
    }
}
