mod stats_server;
mod db;
mod webserver;
use tokio::spawn;


#[tokio::main]
async fn main() -> anyhow::Result<()> {
  // Start the logger
  env_logger::init_from_env(
    env_logger::Env::default()
      .filter_or(env_logger::DEFAULT_FILTER_ENV, "warn"),
  );

  db::create_if_not_exist();
  db::check_id();

  spawn(webserver::stats_viewer());

  let _ = stats_server::gather_stats().await;
  Ok(())
}