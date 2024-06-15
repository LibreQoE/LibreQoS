use colored::Colorize;

pub fn success(s: &str) {
    println!("{} - {s}", "OK".bright_green());
}

pub fn warn(s: &str) {
    println!("{} - {s}", "WARN".bright_yellow());
}

pub fn error(s: &str) {
    println!("{} - {s}", "ERROR".bright_red());
}