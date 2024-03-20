mod bus;
mod top_level_ui;
mod ui_base;
use anyhow::Result;
use ui_base::UiBase;
pub mod widgets;

fn main() -> Result<()> {
    // Create a tokio runtime in a single thread
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { bus::bus_loop().await });
    });


    // Initialize the UI
    let mut ui = UiBase::new()?;
    ui.event_loop()?;

    // Return OK
    Ok(())
}
