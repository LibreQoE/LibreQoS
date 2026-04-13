use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    lqos_topology::start_topology().await;
    Ok(())
}
