use std::process::Command;

pub(crate) fn get_proc_version() -> anyhow::Result<String> {
    let output = Command::new("/bin/cat")
        .args(["/proc/version"])
        .output()?;
    let stdout = String::from_utf8(output.stdout)?;
    Ok(stdout)
}