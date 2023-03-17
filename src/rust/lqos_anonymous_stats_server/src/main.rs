mod stats_server;
mod db;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  // Start the logger
  env_logger::init_from_env(
    env_logger::Env::default()
      .filter_or(env_logger::DEFAULT_FILTER_ENV, "warn"),
  );

  db::create_if_not_exist();

  let _ = stats_server::gather_stats().await;
  Ok(())
}