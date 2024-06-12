//! Provides the start of a text-mode support tool for LibreQoS. It will double as
//! a library (see `lib.rs`) to provide similar functionality from the GUI.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use clap::{Parser, Subcommand};
use lqos_support_tool::{gather_all_support_info, run_sanity_checks, SupportDump};

#[derive(Parser)]
#[command(version = "1.0", about = "LibreQoS Support Tool", long_about = None, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Sanity Checks your Configuration against your hardware
    Sanity,
    /// Gather Support Info and Save it to /tmp
    Gather,
    /// Summarize the contents of a support dump
    Summarize {
        /// The filename to read
        filename: String
    },
}

fn gather_dump() {
    let dump = gather_all_support_info().unwrap();
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let filename = format!("/tmp/libreqos_{}.support", timestamp);
    let path = Path::new(&filename);
    std::fs::write(&path, dump.serialize_and_compress().unwrap()).unwrap();
    lqos_support_tool::console::success(&format!("Dump written to {}", filename));
}

fn summarize(filename: &str) {
    let path = Path::new(filename);
    if !path.exists() {
        println!("Dump not found at {filename}");
    } else {
        let bytes = std::fs::read(&path).unwrap();
        if let Ok(decoded) = SupportDump::from_bytes(&bytes) {
            println!("{:40} {:10} : {:40?}", "ENTRY", "SIZE", "FILENAME");
            for entry in decoded.entries.iter() {
                println!("{:40} {:10} : {:40?}", entry.name, entry.contents.len(), entry.filename);
            }
        } else {
            println!("Dump did not decode");
        }
    }
}

fn sanity_checks() {
    if let Err(e) = run_sanity_checks() {
        println!("Sanity Check Failed: {e:?}");
    }
}

fn main() {
     let cli = Cli::parse();

    match cli.command {
        Some(Commands::Sanity) => sanity_checks(),
        Some(Commands::Gather) => gather_dump(),
        Some(Commands::Summarize { filename }) => summarize(&filename),
        _ => {}
    }
}