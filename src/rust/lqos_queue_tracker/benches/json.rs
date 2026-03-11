//! Benchmarks for JSON serialization and gathering data from TC.
//! This benchmark creates a unique dummy interface and then
//! will destructively clear and then create TC queues.
//! On abort, or completion, it does not presently remove that
//! dummy interface. FIXME.

use criterion::{Criterion, criterion_group, criterion_main};
use lqos_queue_tracker::*;
use std::hint::black_box;
use std::process::{Command, id};

const EXAMPLE_JSON: &str = include_str!("./example.json");
const TC: &str = "/sbin/tc";
const SUDO: &str = "/bin/sudo";
const IP: &str = "ip";

// FIXME: The max interface name is limited to 15 characters
// Using a ASCII64 encoding would help

fn setup_dummy_interface(interface: &str) -> String {
    let interface = format!("{}{}{}", "t_", interface, id());
    if interface.len() > 15 {
        panic!("Interface Queue Length must be less than 15 characters");
    }
    let status = Command::new(SUDO)
        .args([
            IP,
            "link",
            "add",
            "name",
            interface.as_str(),
            "type",
            "dummy",
        ])
        .status()
        .expect("file not found");
    if !status.success() {
        panic!("Dummy device is not supported on this OS: {status}");
    }
    interface
}

fn clear_queues(interface: &str) {
    Command::new(SUDO)
        .args([TC, "qdisc", "delete", "dev", interface, "root"])
        .output()
        .unwrap();
}

fn setup_mq(interface: &str) {
    Command::new(SUDO)
        .args([
            TC, "qdisc", "replace", "dev", interface, "root", "handle", "7FFF:", "mq",
        ])
        .output()
        .unwrap();
}

fn setup_parent_htb(interface: &str) {
    Command::new(SUDO)
        .args([
            TC, "qdisc", "add", "dev", interface, "parent", "7FFF:0x1", "handle", "0x1:", "htb",
            "default", "2",
        ])
        .output()
        .unwrap();

    #[rustfmt::skip]
  Command::new(SUDO)
    .args([
      TC, "class", "add", "dev", interface, "parent", "0x1:", "classid",
      "0x1:1", "htb", "rate", "10000mbit", "ceil", "10000mbit",
    ])
    .output()
    .unwrap();

    #[rustfmt::skip]
  Command::new(SUDO)
    .args([
      TC, "qdisc", "add", "dev", interface, "parent", "0x1:1", "cake",
      "diffserv4",
    ])
    .output()
    .unwrap();
}

fn add_client_pair(interface: &str, queue_number: u32) {
    let class_id = format!("0x1:{queue_number:x}");
    Command::new(SUDO)
        .args([
            TC, "class", "add", "dev", interface, "parent", "0x1:1", "classid", &class_id, "htb",
            "rate", "2500mbit", "ceil", "9999mbit", "prio", "5",
        ])
        .output()
        .unwrap();

    #[rustfmt::skip]
  Command::new(SUDO)
    .args([
      TC, "qdisc", "add", "dev", interface, "parent", &class_id, "cake",
      "diffserv4",
    ])
    .output()
    .unwrap();
}

pub fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("deserialize_cake", |b| {
        b.iter(|| {
            deserialize_tc_tree(EXAMPLE_JSON).unwrap();
        });
    });

    let binding = setup_dummy_interface("qt");
    let interface = binding.as_str();
    //let interface = "eth1";

    const QUEUE_COUNTS: [u32; 1] = [5000];
    for queue_count in QUEUE_COUNTS.iter() {
        let no_stdbuf = format!("NO-STBUF, {queue_count} queues: tc qdisc show -s -j");
        let stdbuf = format!("STBUF -i1024, {queue_count} queues: tc qdisc show -s -j");

        clear_queues(interface);
        setup_mq(interface);
        setup_parent_htb(interface);
        for i in 0..*queue_count {
            let queue_handle = (i + 1) * 2;
            add_client_pair(interface, queue_handle);
        }

        c.bench_function(&no_stdbuf, |b| {
            b.iter(|| {
                let command_output = Command::new("/sbin/tc")
                    .args(["-s", "-j", "qdisc", "show", "dev", "eth1"])
                    .output()
                    .unwrap();
                let json = String::from_utf8(command_output.stdout).unwrap();
                black_box(json);
            });
        });

        c.bench_function(&stdbuf, |b| {
            b.iter(|| {
                let command_output = Command::new("/usr/bin/stdbuf")
                    .args([
                        "-i0", "-o1024M", "-e0", TC, "-s", "-j", "qdisc", "show", "dev", "eth1",
                    ])
                    .output()
                    .unwrap();
                let json = String::from_utf8(command_output.stdout).unwrap();
                black_box(json);
            });
        });
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
