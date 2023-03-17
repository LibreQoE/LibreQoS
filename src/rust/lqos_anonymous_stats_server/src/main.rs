mod stats_server;
mod db;
use axum::{routing::get, Router};
use db::dump_all_to_string;
use tokio::spawn;

async fn stats_viewer() -> anyhow::Result<()> {
  let app = Router::new().route("/", get(handler));

  axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
      .serve(app.into_make_service())
      .await?;

  Ok(())
}

async fn handler() -> String {
  dump_all_to_string().unwrap()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  // Start the logger
  env_logger::init_from_env(
    env_logger::Env::default()
      .filter_or(env_logger::DEFAULT_FILTER_ENV, "warn"),
  );

  db::create_if_not_exist();
  db::check_id();

  spawn(stats_viewer());

  let _ = stats_server::gather_stats().await;
  Ok(())
}