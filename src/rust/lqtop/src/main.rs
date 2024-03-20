mod bus;
mod top_level_ui;
mod ui_base;
use anyhow::Result;
use ui_base::UiBase;
pub mod widgets;

#[tokio::main]
async fn main() -> Result<()> {
    // Spawn the bus as an async background task and retrieve
    // the command sender.
    let bus_commander = bus::bus_loop().await;

    // Initialize the UI
    let mut ui = UiBase::new(bus_commander.clone())?;
    ui.event_loop().await?;

    // Return OK
    Ok(())
}
