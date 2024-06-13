//! Provides the start of a text-mode support tool for LibreQoS. It will double as
//! a library (see `lib.rs`) to provide similar functionality from the GUI.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use clap::{Parser, Subcommand};
use colored::Colorize;
use lqos_config::load_config;
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
    /// Gather Support Info and Send it to the LibreQoS Team. Note that LTS users and donors get priority, we don't guarantee that we'll help anyone else. Please make sure you've tried Zulip first ( https://chat.libreqos.io/ )
    Submit,
    /// Summarize the contents of a support dump
    Summarize {
        /// The filename to read
        filename: String
    },
    /// Expand all submitted data from a support dump into a given directory
    Expand {
        /// The filename to read from
        filename: String,
        /// The target directory
        target: String,
    }
}

fn read_line() -> String {
    use std::io::{stdin,stdout,Write};
    let mut s = String::new();
    stdin().read_line(&mut s).expect("Did not enter a correct string");
    s.trim().to_string()
}

fn get_lts_key() -> String {
    if let Ok(cfg) = load_config() {
        if let Some(key) = cfg.long_term_stats.license_key {
            return key.clone();
        }
    }

    println!();
    println!("{}", "No LTS Key Found!".bright_red());
    println!("We prioritize helping Long-Term Stats Subscribers and Donors.");
    println!("Please enter your LTS key (from your email), or ENTER for none:");
    return read_line();
}

fn get_name() -> String {
    let mut candidate = String::new();
    while candidate.is_empty() {
        println!("Please enter your name, email address and Zulip handle in a single line (ENTER when done).");
        candidate = read_line();
    }
    candidate
}

fn get_comments() -> String {
    println!("Anything you'd like to tell us about? (Comments)");
    read_line()
}

fn gather_dump() {
    let name = get_name();
    let lts_key = get_lts_key();
    let comments = get_comments();

    let dump = gather_all_support_info(&name, &comments, &lts_key).unwrap();
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
            println!("Sent by: {}", decoded.sender);
            println!("Comments: {}", decoded.comment);
            println!("LTS Key: {}", decoded.lts_key);

            println!("{:50} {:10} {}", "Sanity Check", "Success?", "Comment");
            for entry in decoded.sanity_checks.results.iter() {
                println!("{:50} {:10} {}", entry.name, entry.success, entry.comments);
            }
            println!();

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

fn expand(filename: &str, target: &str) {
    // Check inputs
    let in_path = Path::new(filename);
    if !in_path.exists() {
        println!("{} not found", filename);
        return;
    }
    let out_path = Path::new(target);
    if !out_path.exists() {
        println!("{} not found", target);
        return;
    }
    if !out_path.is_dir() {
        println!("{} is not a directory", target);
        return;
    }

    // Load the data
    let bytes = std::fs::read(&in_path).unwrap();
    if let Ok(decoded) = SupportDump::from_bytes(&bytes) {
        // Save the header
        let header = format!("From: {}\nComment: {}\nLTS Key: {}\n", decoded.sender, decoded.comment, decoded.lts_key);
        let header_path = out_path.join("header.txt");
        std::fs::write(header_path, header.as_bytes()).unwrap();

        // Save the sanity check results
        let mut sanity = String::new();
        for check in decoded.sanity_checks.results.iter() {
            sanity += &format!("{} - Success? {}: {}\n", check.name, check.success, check.comments);
        }
        let sanity_path = out_path.join("sanity_checks.txt");
        std::fs::write(sanity_path, sanity.as_bytes()).unwrap();

        // Save the files
        for (idx, dump) in decoded.entries.iter().enumerate() {
            let trimmed = dump.name.trim().replace(' ', "").to_lowercase().replace('(', "").replace(')', "");
            let dump_path = out_path.join(&format!("{idx:0>2}_{}", trimmed));
            std::fs::write(dump_path, dump.contents.as_bytes()).unwrap();
        }
    }

    println!("Expanded data written to {}", target);
}

fn submit() {
    // Get header
    let name = get_name();
    let lts_key = get_lts_key();
    let comments = get_comments();

    // Get the data
    let dump = gather_all_support_info(&name, &comments, &lts_key).unwrap();

    // Send it
    lqos_support_tool::submit_to_network(dump);
}

fn main() {
     let cli = Cli::parse();

    match cli.command {
        Some(Commands::Sanity) => sanity_checks(),
        Some(Commands::Gather) => gather_dump(),
        Some(Commands::Summarize { filename }) => summarize(&filename),
        Some(Commands::Expand { filename, target }) => expand(&filename, &target),
        Some(Commands::Submit) => submit(),
        _ => {}
    }
}