use crate::lts2_sys::RemoteCommand;
use tracing::{debug, warn};

pub fn start_remote_commands() {
    debug!("Starting remote commands system");
    let _ = std::thread::Builder::new()
        .name("Remote Command Handler".to_string())
        .spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(30));
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                debug!("Checking for remote commands");

                if crate::lts2_sys::remote_command_count() > 0 {
                    let commands = crate::lts2_sys::remote_commands();
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
        RemoteCommand::SetInsightControlledTopology { enabled } => {
            if let Ok(config) = lqos_config::load_config() {
                let mut config = (*config).clone();
                config.long_term_stats.enable_insight_topology = Some(enabled);
                if let Err(e) = lqos_config::update_config(&config) {
                    tracing::error!("Failed to update config: {}", e);
                }
                let _ = crate::scheduler_control::enable_scheduler();
                let _ = crate::scheduler_control::restart_scheduler();
            }
        }
        RemoteCommand::SetInsightRole { role } => {
            if let Ok(config) = lqos_config::load_config() {
                let mut config = (*config).clone();
                config.long_term_stats.insight_topology_role = Some(role);
                if let Err(e) = lqos_config::update_config(&config) {
                    tracing::error!("Failed to update config: {}", e);
                }
                let _ = crate::scheduler_control::enable_scheduler();
                let _ = crate::scheduler_control::restart_scheduler();
            }
        }
        RemoteCommand::RestartLqosd => {
            // Gracefully detach XDP/TC before exiting to avoid stale attachments
            if let Ok(cfg) = lqos_config::load_config() {
                if cfg.on_a_stick_mode() {
                    let _ = lqos_sys::unload_xdp_from_interface(&cfg.internet_interface());
                } else {
                    let _ = lqos_sys::unload_xdp_from_interface(&cfg.internet_interface());
                    let _ = lqos_sys::unload_xdp_from_interface(&cfg.isp_interface());
                }
            }
            std::process::exit(0);
        }
        RemoteCommand::RestartScheduler => {
            let _ = crate::scheduler_control::restart_scheduler();
        }
    }
}
