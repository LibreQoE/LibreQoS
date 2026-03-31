use anyhow::Result;
use clap::{Parser, Subcommand};
use lqos_bus::{BusRequest, bus_request};
use lqos_config::{UserRole, WebUsers};
use std::process::exit;

#[derive(Parser)]
#[command()]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add or update a user
    Add {
        /// Username
        #[arg(long)]
        username: String,

        /// Role
        #[arg(long)]
        role: UserRole,

        /// Password
        #[arg(long)]
        password: String,
    },
    /// Remove a user
    Del {
        /// Username to remove
        username: String,
    },
    /// List users
    List,
}

fn notify_auth_cache_invalidated() {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        eprintln!("Warning: unable to create Tokio runtime for auth cache invalidation");
        return;
    };

    if let Err(e) = runtime.block_on(bus_request(vec![BusRequest::InvalidateAuthCache])) {
        eprintln!("Warning: updated lqusers.toml but could not notify lqosd: {e}");
    }
}

fn main() -> Result<()> {
    let cli = Args::parse();
    let mut users = WebUsers::load_or_create()?;
    match cli.command {
        Some(Commands::Add {
            username,
            role,
            password,
        }) => {
            users.add_or_update_user(&username, &password, role)?;
            notify_auth_cache_invalidated();
        }
        Some(Commands::Del { username }) => {
            users.remove_user(&username)?;
            notify_auth_cache_invalidated();
        }
        Some(Commands::List) => {
            println!("All Users\n");
            users.print_users()?;
        }
        None => {
            println!("Run with --help to see instructions");
            exit(0);
        }
    }

    Ok(())
}
