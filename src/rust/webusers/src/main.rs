use anyhow::Result;
use clap::{Parser, Subcommand};
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

    /// CPU id to connect
    #[arg(long)]
    password: String,
  },
  /// Remove a user
  Del {
    /// Username to remove
    username: String,
  },
  /// List all mapped IPs.
  List,
}

fn main() -> Result<()> {
  let cli = Args::parse();
  let mut users = WebUsers::load_or_create()?;
  match cli.command {
    Some(Commands::Add { username, role, password }) => {
      users.add_or_update_user(&username, &password, role)?;
    }
    Some(Commands::Del { username }) => {
      users.remove_user(&username)?;
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
