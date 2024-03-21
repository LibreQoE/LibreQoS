mod bus;
mod top_level_ui;
mod ui_base;
use anyhow::Result;
use bus::BusMessage;
use ui_base::UiBase;
pub mod widgets;

fn main() -> Result<()> {
    // Create an async channel for seinding data into the bus system.
    let (tx, rx) = tokio::sync::mpsc::channel::<BusMessage>(100);

    // Create a tokio runtime in a single thread
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move { bus::bus_loop(rx).await });
    });


    // Initialize the UI
    let mut ui = UiBase::new(tx)?;
    ui.event_loop()?;

    // Return OK
    Ok(())
}
