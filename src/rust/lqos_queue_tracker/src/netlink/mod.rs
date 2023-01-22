mod cake_stats;
mod nla_types;
use anyhow::Result;
use futures_util::TryStreamExt;
use rtnetlink::{new_connection, packet::TcMessage};

/// Very primitive - replace me
pub async fn get_all_queue_stats_with_netlink(
  interface_index: i32,
) -> Result<Vec<TcMessage>> {
  let mut results = Vec::new();
  let (connection, handle, _) = new_connection().unwrap();
  tokio::spawn(connection);
  let mut result = handle.qdisc().get().index(interface_index).execute();
  while let Ok(Some(result)) = result.try_next().await {
    results.push(result);
  }
  Ok(results)
}
