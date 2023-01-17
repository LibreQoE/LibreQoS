use crate::{deserialize_tc_tree, queue_types::QueueType};
use anyhow::Result;
use lqos_bus::TcHandle;
use std::process::Command;

const TC: &str = "/sbin/tc";

pub fn read_named_queue_from_interface(
    interface: &str,
    tc_handle: TcHandle,
) -> Result<Vec<QueueType>> {
    let command_output = Command::new(TC)
        .args([
            "-s",
            "-j",
            "qdisc",
            "show",
            "dev",
            interface,
            "parent",
            &tc_handle.to_string(),
        ])
        .output()?;

    let json = String::from_utf8(command_output.stdout)?;
    let result = deserialize_tc_tree(&json);
    Ok(result?)
}
