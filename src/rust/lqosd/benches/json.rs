//! Benchmarks for JSON serialization and gathering data from TC.
//! Please select an interface in `test_interface.txt` (no enter character
//! at the end). This benchmark will destructively clear and then create
//! TC queues - so don't select an interface that you need!

use criterion::{criterion_group, criterion_main, Criterion, black_box};
use lqosd::*;
use std::process::Command;

const EXAMPLE_JSON: &str = include_str!("./example_json.txt");
const TC: &str = "/sbin/tc";
const SUDO: &str = "/bin/sudo";

fn clear_queues(interface: &str) {
    Command::new(SUDO)
        .args([TC, "qdisc", "delete", "dev", interface, "root"])
        .output()
        .unwrap();
}

fn setup_mq(interface: &str) {
    Command::new(SUDO)
        .args([TC, "qdisc", "replace", "dev", interface, "root", "handle", "7FFF:", "mq"])
        .output()
        .unwrap();
}

fn setup_parent_htb(interface: &str) {
    Command::new(SUDO)
        .args([TC, "qdisc", "add", "dev", interface, "parent", "7FFF:0x1", "handle", "0x1:", "htb", "default", "2"])
        .output()
        .unwrap();
    
    Command::new(SUDO)
        .args([TC, "class", "add", "dev", interface, "parent", "0x1:", "classid", "0x1:1", "htb", "rate", "10000mbit", "ceil", "10000mbit"])
        .output()
        .unwrap();

    Command::new(SUDO)
        .args([TC, "qdisc", "add", "dev", interface, "parent", "0x1:1", "cake", "diffserv4"])
        .output()
        .unwrap();
}

fn add_client_pair(interface: &str, queue_number: u32) {
    let class_id = format!("0x1:{:x}", queue_number);
    Command::new(SUDO)
        .args([TC, "class", "add", "dev", interface, "parent", "0x1:1", "classid", &class_id, "htb", "rate", "2500mbit", "ceil", "9999mbit", "prio", "5"])
        .output()
        .unwrap();

    Command::new(SUDO)
        .args([TC, "qdisc", "add", "dev", interface, "parent", &class_id, "cake", "diffserv4"])
        .output()
        .unwrap();
}

pub fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("deserialize_cake", |b| {
        b.iter(|| {
            deserialize_tc_tree(EXAMPLE_JSON).unwrap();
        });
    });

    const INTERFACE: &str = include_str!("test_interface.txt");
    const QUEUE_COUNTS: [u32; 4] = [10, 100, 1000, 2000];
    for queue_count in QUEUE_COUNTS.iter() {
        let no_stdbuf = format!("NO-STBUF, {queue_count} queues: tc qdisc show -s -j");
        let stdbuf = format!("STBUF -i1024, {queue_count} queues: tc qdisc show -s -j");

        clear_queues(INTERFACE);
        setup_mq(INTERFACE);
        setup_parent_htb(INTERFACE);
        for i in 0 .. *queue_count {
            let queue_handle = (i+1) * 2;
            add_client_pair(INTERFACE, queue_handle);
        }

        c.bench_function(&no_stdbuf, |b| {
            b.iter(|| {
                let command_output = Command::new("/sbin/tc")
                    .args(["-s", "-j", "qdisc", "show", "dev", "eth1"])
                    .output().unwrap();
                let json = String::from_utf8(command_output.stdout).unwrap();
                black_box(json);
            });
        });

        c.bench_function(&stdbuf, |b| {
            b.iter(|| {
                let command_output = Command::new("/usr/bin/stdbuf")
                    .args(["-i0", "-o1024M", "-e0", TC, "-s", "-j", "qdisc", "show", "dev", "eth1"])
                    .output().unwrap();
                let json = String::from_utf8(command_output.stdout).unwrap();
                black_box(json);
            });
        });
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);