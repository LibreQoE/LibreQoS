//! K3 retirement gate: the removed kernel-datapath, structure, and feed
//! tokens must not reappear.
//!
//! K3-A deleted the XDP/TC attach, the bakery, the `lqos_sys` skeleton and
//! pinned maps, the kernel IP-mapping surface, and the kernel Heimdall
//! transport. K3-B deleted the remaining kernel-only structures and feeds:
//! the netflow exporter and finished-flow feed, the kernel flowbee ring
//! buffer and throughput writer, the kernel `queuingStructure` + queue
//! monitor, the CPU-affinity surfaces, the always-false `runtime_virtualized`
//! schema fields, and the kernel-mode config surfaces (on-a-stick
//! accessors, TreeGuard config).
//!
//! The blackboard / remote-command / scheduler-control chain was deleted by
//! K3-B as part of that chain but is NOT gated: ruled it
//! attach-orphans (bus/lts2-fed, not kernel-data consumers) and the 3a LTS
//! reinstatement moved their spawns to the common startup path.
//!
//! This workspace scan (the treeguard/K2 pattern) fails if any of those
//! mechanisms is reintroduced. This gate file is self-excluded.

use std::path::{Path, PathBuf};

/// Tokens owned by the deleted kernel datapath / bakery. Each must never be
/// referenced again.
const FORBIDDEN: &[&str] = &[
    // K3-A: kernel datapath, bakery, lqos_sys maps, kernel Heimdall.
    "LibreQoSKernels",
    "BakeryApply",
    "BakeryCommands",
    "lqos_bakery",
    "start_bakery",
    "bpf_garbage_collector",
    "add_ip_to_tc",
    "BPF_SKELETON",
    "bpf_iterator",
    "preflight_checks",
    "recompute_stick_offset",
    "xdp_iphash_to_cpu_cmdline",
    "MapIpToFlow",
    "heimdall_expire",
    "perf_interface",
    "HeimdallMode",
    "BpfMap",
    // K3-B: kernel telemetry chain.
    "HIGH_WATERMARK",
    "FinishedFlowAnalysis",
    "netflow5",
    "netflow9",
    "netflow_common",
    "FlowActor",
    "get_rtt_events_per_second",
    // NOTE: the blackboard / remote-command / scheduler-control chain is NOT
 // gated — ruled those attach-orphans (not kernel-data consumers)
    // and the 3a LTS reinstatement moved their spawns to the common startup.
    // K3-B: kernel structures / queues / cpu affinity / schema.
    "QUEUE_STRUCTURE",
    "QUEUE_STRUCTURE_CHANGED_STORMGUARD",
    "reload_queue_structure",
    "spawn_queue_structure_monitor",
    "cpu_affinity",
    "runtime_virtualized",
    // K3-B: kernel-mode config surfaces.
    "on_a_stick_mode",
    "stick_vlans",
    "TreeguardConfig",
    "TreeguardCpuMode",
    "TreeguardCircuitsConfig",
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            if name == "target" || name == ".git" {
                continue;
            }
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn kernel_datapath_tokens_do_not_reappear() {
    let rust_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("lqosd manifest should have a parent")
        .to_path_buf();
    let gate_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/k3_retirement_gate.rs")
        .canonicalize()
        .expect("gate file should canonicalize");

    let mut files = Vec::new();
    collect_rs_files(&rust_root, &mut files);
    assert!(
        files.len() > 100,
        "workspace scan should find many Rust files, found {}",
        files.len()
    );

    let mut violations = Vec::new();
    for path in files {
        if path.canonicalize().ok().as_deref() == Some(gate_path.as_path()) {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        for token in FORBIDDEN {
            if contents.contains(token) {
                violations.push(format!("{}: {token}", path.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "K3 kernel retirement grep gate failed (the XDP/TC attach, bakery, \
         lqos_sys maps, kernel IP-mapping, kernel Heimdall transport, and \
         the K3-B kernel structures/feeds/config surfaces are gone — they \
         must not reappear):\n{}",
        violations.join("\n")
    );
}
