mod connection;
mod license;

pub mod sqlx {
    pub use sqlx::*;
}

pub use connection::get_connection_pool;
pub use license::{get_stats_host_for_key, insert_or_update_node_public_key, fetch_public_key};