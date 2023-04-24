use anyhow::Result;
use clap::{Parser, Subcommand};
use pgdb::create_free_trial;
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
    /// Manage licenses
    License {
        #[command(subcommand)]
        command: Option<LicenseCommands>,
    },
    /// Manage users
    Users {
        #[command(subcommand)]
        command: Option<UsersCommands>,
    },
}

#[derive(Subcommand)]
enum HostsCommands {
    /// Add a host to the list of available stats storing hosts
    Add { hostname: String, influx_host: String, api_key: String },
}

#[derive(Subcommand)]
enum LicenseCommands {
    /// Create a new free trial license
    FreeTrial { organization: String },
}

#[derive(Subcommand)]
enum UsersCommands {
    /// Add a new user
    Add { key: String, username: String, password: String, nicename: String },
    /// Delete a user
    Delete { key: String, username: String },
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
            command: Some(HostsCommands::Add { hostname, influx_host, api_key }),
        }) => {
            match pgdb::add_stats_host(pool, hostname, influx_host, api_key).await {
                Err(e) => {
                    log::error!("Unable to add stats host: {e:?}");
                    exit(1);
                }
                Ok(new_id) => {
                    println!("Added stats host with id {}", new_id);
                }
            }
        }
        Some(Commands::License{command: Some(LicenseCommands::FreeTrial { organization })}) => {
            match create_free_trial(pool, &organization).await {
                Err(e) => {
                    log::error!("Unable to create free trial: {e:?}");
                    exit(1);
                }
                Ok(key) => {
                    println!("Your new license key is: {}", key);
                }
            }
        }
        Some(Commands::Users{command: Some(UsersCommands::Add { key, username, password, nicename })}) => {
            match pgdb::add_user(pool, &key, &username, &password, &nicename).await {
                Err(e) => {
                    log::error!("Unable to add user: {e:?}");
                    exit(1);
                }
                Ok(_) => {
                    println!("Added user {}", username);
                }
            }
        }
        _ => {
            println!("Run with --help to see instructions");
            exit(0);
        }
    }

    Ok(())
}
