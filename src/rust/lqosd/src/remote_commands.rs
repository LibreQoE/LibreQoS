use tracing::{debug, warn};
use lts2_sys::RemoteCommand;

pub fn start_remote_commands() {
    debug!("Starting remote commands system");
    let _ = std::thread::Builder::new()
        .name("Remote Command Handler".to_string())
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(30));
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                debug!("Checking for remote commands");

                if lts2_sys::remote_command_count() > 0 {
                    let commands = lts2_sys::remote_commands();
                    commands.into_iter().for_each(run_command);
                }
            }
        });
}

fn run_command(command: RemoteCommand) {
    match command {
        RemoteCommand::Log(msg) => {
            warn!("Message from Insight: {}", msg);
        }
    }
}