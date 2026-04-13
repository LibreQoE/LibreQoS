use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use lqos_netplan_helper::inspect_network_mode;
use lqos_netplan_helper::protocol::{ApplyMode, ApplyRequest};
use lqos_netplan_helper::transaction::{
    HelperPaths, PendingChildren, apply_transaction, confirm_transaction, helper_status,
    retry_shaping_transaction, revert_transaction, rollback_transaction,
};

#[derive(Parser)]
#[command(name = "lqos_netplan_helper")]
#[command(about = "LibreQoS managed netplan helper")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Status,
    Inspect,
    Preview,
    Apply {
        #[arg(long, default_value = "cli")]
        source: String,
        #[arg(long)]
        operator_username: Option<String>,
    },
    Confirm {
        operation_id: String,
    },
    Revert {
        operation_id: String,
    },
    Rollback {
        backup_id: String,
    },
    RetryShaping,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = HelperPaths::default();
    let mut pending_children = PendingChildren::default();
    match cli.command {
        Commands::Status => {
            let response = helper_status(&paths, &mut pending_children)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        Commands::Inspect => {
            let config = lqos_config::load_config().context("Unable to load current config")?;
            println!(
                "{}",
                serde_json::to_string_pretty(&inspect_network_mode(config.as_ref()))?
            );
            Ok(())
        }
        Commands::Preview => {
            let config = lqos_config::load_config().context("Unable to load current config")?;
            let inspection = inspect_network_mode(config.as_ref());
            if let Some(preview) = inspection.managed_preview_yaml {
                println!("{preview}");
            } else if let Some(note) = inspection.preview_note {
                println!("{note}");
            }
            Ok(())
        }
        Commands::Apply {
            source,
            operator_username,
        } => {
            let config = lqos_config::load_config().context("Unable to load current config")?;
            let response = apply_transaction(
                &paths,
                &mut pending_children,
                ApplyRequest {
                    config: (*config).clone(),
                    source,
                    operator_username,
                    mode: ApplyMode::Apply,
                    confirm_dangerous_changes: true,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        Commands::Confirm { operation_id } => {
            let response = confirm_transaction(&paths, &mut pending_children, &operation_id)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        Commands::Revert { operation_id } => {
            let response = revert_transaction(&paths, &mut pending_children, &operation_id)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        Commands::Rollback { backup_id } => {
            let response = rollback_transaction(&paths, &mut pending_children, &backup_id)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
        Commands::RetryShaping => {
            let response = retry_shaping_transaction(&paths, &mut pending_children)?;
            println!("{}", serde_json::to_string_pretty(&response)?);
            Ok(())
        }
    }
}
