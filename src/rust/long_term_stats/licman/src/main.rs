use anyhow::Result;
use clap::{Parser, Subcommand};
use std::process::exit;

#[derive(Parser)]
#[command()]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage stats hosts
    Hosts {
        #[command(subcommand)]
        command: Option<HostsCommands>,
    },
}

#[derive(Subcommand)]
enum HostsCommands {
    Add { hostname: String },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    env_logger::init_from_env(
        env_logger::Env::default()
          .filter_or(env_logger::DEFAULT_FILTER_ENV, "warn"),
      );

    // Get the database connection pool
    let pool = pgdb::get_connection_pool(5).await;
    if pool.is_err() {
        log::error!("Unable to connect to the database");
        log::error!("{pool:?}");
        return Err(anyhow::Error::msg("Unable to connect to the database"));
    }
    let pool = pool.unwrap();

    let cli = Args::parse();
    match cli.command {
        Some(Commands::Hosts {
            command: Some(HostsCommands::Add { hostname }),
        }) => {
            match pgdb::add_stats_host(pool, hostname).await {
                Err(e) => {
                    log::error!("Unable to add stats host: {e:?}");
                    exit(1);
                }
                Ok(new_id) => {
                    println!("Added stats host with id {}", new_id);
                }
            }
        }
        Some(Commands::Hosts { command: None }) | None => {
            println!("Run with --help to see instructions");
            exit(0);
        }
    }

    Ok(())
}
