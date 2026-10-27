#!/usr/bin/env python3
"""Developer-experience demo (docs/dev_experience_bootstrap_monorepo_card.md §3.2).

The <=10-minute first experience: a 16-circuit shaping tree running through the
DPDK `net_af_packet` engine on a local veth pair, with the v3 `lqosd` web UI
serving live statistics. No second NIC and no production install are touched.

Design notes (from the §3.1 fresh review,:
  * The engine is the DPDK `net_af_packet` PMD on veths (the in-tree
    `scripts/local_testbed.py` exercises). The Rust-native AF_PACKET engine is
    a scheduler-less passthrough and cannot shape, so it is NOT used here.
  * `circuit_capacity` is AUTO-DERIVED from the live leaf count (leaves + 1
    best-effort slot), never a raw default -- the shared host 17x lesson.
  * This script is v3-side only; it never edits the engine checkout.

Run through `./build_rust.sh demo`. Needs root (veth/netns + AF_PACKET +
/run/lqos); the script re-execs itself under sudo when needed.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

# --- constants --------------------------------------------------------------
NAMESPACE = "lqos-demo"
LAN = "lqos-demo-lan"          # DPDK-facing LAN veth (host)
SUB = "lqos-demo-sub"          # subscriber peer (moved into NAMESPACE)
WAN = "lqos-demo-wan"          # DPDK-facing WAN veth (host)
UPLINK = "lqos-demo-uplink"    # WAN peer (host)
GATEWAY = "198.18.0.1"
SUBSCRIBER_IP = "198.18.0.2"
SUBSCRIBER_PREFIX = 24

DEMO_CIRCUITS = 16
DEMO_RATE_KBPS = 25_000        # 25 Mbps commit/ceil per circuit
FILE_PREFIX = "lqos-demo"
WEBSERVER_PORT = 9123
BRIDGE_BOOT_MARKER = "Bridge starting with"

# --- pure helpers (unit-testable without root) ------------------------------
def derive_capacity(leaf_count: int) -> int:
    """Capacity = live leaves + one best-effort slot.

    The historical convention is leaves + 1 (``DEFAULT_CIRCUIT_CAPACITY = 17``
    for 16 circuits, bridge_rs/src/config.rs). The demo never ships a raw
    default like 4096: the boot advisory warns at ``capacity >= leaves * 8``.
    """
    if leaf_count < 1:
        raise ValueError("leaf_count must be >= 1 to size a shaping tree")
    return leaf_count + 1


def build_transaction(leaf_count: int = DEMO_CIRCUITS,
                      rate_kbps: int = DEMO_RATE_KBPS) -> dict:
    """A deterministic flat 16-circuit transaction (the demo fixture).

    Mirrors the in-tree 16-circuit fixture shape (scripts/test/fixtures/
    fanout5_transaction.json) but maps the demo subscriber range so traffic
    from the local namespace classifies onto the circuits.
    """
    leaf_ops = []
    mapping_ops = []
    for index in range(leaf_count):
        circuit = f"demo-circuit-{index + 1}"
        address = f"198.18.0.{index + 2}"
        leaf_ops.append({
            "op": "upsert",
            "circuit_id": circuit,
            "parent_node_id": "root",
            "download": {"commit_kbps": rate_kbps, "ceil_kbps": rate_kbps},
            "upload": {"commit_kbps": rate_kbps, "ceil_kbps": rate_kbps},
            "aqm": {"kind": "none"},
            "admin_up": True,
        })
        mapping_ops.append({
            "op": "map",
            "prefix_len": 32,
            "ip": address,
            "circuit_id": circuit,
            "device_id": f"demo-device-{index + 1}",
        })
    return {
        "generation": "dev-demo-1",
        "schema_version": 3,
        "inner_ops": [],
        "leaf_ops": leaf_ops,
        "mapping_ops": mapping_ops,
    }


def count_leaf_upserts(transaction: dict) -> int:
    return sum(1 for op in transaction.get("leaf_ops", [])
               if op.get("op") == "upsert")


def build_bridge_config(capacity: int) -> dict:
    """The typed bridge config selected by LIBREQOS_DPDK_CONFIG_FILE."""
    return {"schema_version": 1, "circuit_capacity": capacity}


def build_demo_config(template: str, v3_src: Path, state_dir: Path,
                      webserver_port: int) -> str:
    """Overlay the demo overrides onto the shipped lqos.conf template.

    Top-level keys must precede tables in TOML, so ``webserver_listen`` is
    injected next to the other top-level fields, never appended.
    """
    lines = template.splitlines()
    out: list[str] = []
    inserted_listen = False
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("lqos_directory"):
            out.append(f'lqos_directory = "{v3_src}"')
        elif stripped.startswith("state_directory"):
            out.append(f'state_directory = "{state_dir}"')
        elif stripped.startswith("node_name"):
            out.append('node_name = "LibreQoS Demo"')
            out.append(f'webserver_listen = "0.0.0.0:{webserver_port}"')
            inserted_listen = True
        elif stripped.startswith("gather_stats"):
            out.append("gather_stats = false")
        elif stripped.startswith("license_key"):
            out.append('license_key = ""')
        else:
            out.append(line)
    if not inserted_listen:
        out.insert(1, f'webserver_listen = "0.0.0.0:{webserver_port}"')
    return "\n".join(out) + "\n"


# --- runtime ----------------------------------------------------------------
def log(message: str) -> None:
    print(f"[demo] {message}", flush=True)


def die(message: str, code: int = 1) -> None:
    print(f"[demo] ERROR: {message}", file=sys.stderr, flush=True)
    raise SystemExit(code)


def run(argv: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(argv, check=True, **kwargs)


def resolve_paths(args: argparse.Namespace) -> tuple[Path, Path, Path, Path]:
    here = Path(__file__).resolve()
    src_dir = here.parent.parent          # <repo>/src/src
    repo_root = src_dir.parent.parent     # <repo>
    # Monorepo: the engine is in-tree; there is no separate engine checkout.
    bridge_bin = Path(args.bridge_bin or
                      repo_root / "rust/libreqos_dpdk/target/release/bridge_rs")
    lqosd_bin = Path(args.lqosd_bin or
                     src_dir / "rust/target/release/lqosd")
    return src_dir, repo_root, bridge_bin, lqosd_bin


def preflight(src_dir: Path, bridge_bin: Path, lqosd_bin: Path) -> None:
    missing = []
    if not bridge_bin.is_file():
        missing.append(f"bridge_rs ({bridge_bin}) -- run './build_rust.sh build'")
    if not lqosd_bin.is_file():
        missing.append(f"lqosd ({lqosd_bin}) -- run './build_rust.sh build'")
    static2 = src_dir / "bin/static2"
    if not static2.is_dir():
        missing.append(f"web UI assets ({static2}) -- run './build_rust.sh build'")
    if shutil.which("ip") is None:
        missing.append("iproute2 ('ip')")
    if missing:
        die("missing prerequisites:\n  - " + "\n  - ".join(missing))
    ldd = subprocess.run(["ldd", str(bridge_bin)], capture_output=True, text=True)
    not_found = [line for line in ldd.stdout.splitlines() if "not found" in line]
    if not_found:
        die("bridge_rs has unresolved shared libraries (install/bundle DPDK):\n  "
            + "\n  ".join(not_found))


def write_work_files(src_dir: Path, work: Path, leaves: int) -> tuple[Path, Path]:
    transaction = build_transaction(leaves)
    actual = count_leaf_upserts(transaction)
    if actual != leaves:
        die(f"internal error: expected {leaves} leaves, built {actual}")
    capacity = derive_capacity(actual)
    work.mkdir(parents=True, exist_ok=True)
    tx_path = work / "transaction.json"
    tx_path.write_text(json.dumps(transaction, indent=2), encoding="utf-8")
    cfg_path = work / "bridge-config.json"
    cfg_path.write_text(json.dumps(build_bridge_config(capacity)), encoding="utf-8")
    log(f"{actual} circuits, derived circuit_capacity={capacity}")
    return tx_path, cfg_path


def setup_network() -> None:
    log("creating namespace + veth pair")
    run(["ip", "netns", "add", NAMESPACE])
    run(["ip", "link", "add", LAN, "type", "veth", "peer", "name", SUB])
    run(["ip", "link", "add", WAN, "type", "veth", "peer", "name", UPLINK])
    run(["ip", "link", "set", SUB, "netns", NAMESPACE])
    for dev in (LAN, WAN, UPLINK):
        run(["ip", "link", "set", dev, "up"])
    run(["ip", "addr", "add", f"{GATEWAY}/{SUBSCRIBER_PREFIX}", "dev", UPLINK])
    run(["ip", "netns", "exec", NAMESPACE, "ip", "link", "set", "lo", "up"])
    run(["ip", "netns", "exec", NAMESPACE, "ip", "link", "set", SUB, "up"])
    run(["ip", "netns", "exec", NAMESPACE, "ip", "addr", "add",
         f"{SUBSCRIBER_IP}/{SUBSCRIBER_PREFIX}", "dev", SUB])
    run(["ip", "netns", "exec", NAMESPACE, "ip", "route", "add",
         "default", "via", GATEWAY])
    for dev in (LAN, WAN, UPLINK):
        for feature in ("gso", "tso", "gro", "lro"):
            subprocess.run(["ethtool", "-K", dev, feature, "off"],
                           capture_output=True)


def bridge_argv(bridge_bin: Path, workers: int) -> list[str]:
    return [
        str(bridge_bin),
        "-l", "0-1", "-n", "1",
        "--no-pci", "--no-huge", "--legacy-mem", "-m", "512",
        f"--file-prefix={FILE_PREFIX}",
        f"--vdev=net_af_packet0,iface={WAN},qpairs={workers}",
        f"--vdev=net_af_packet1,iface={LAN},qpairs={workers}",
        "--",
        "--workers", str(workers),
        "--rx-queues", str(workers),
        "--tx-queues", str(workers),
    ]


def bridge_env(work: Path, tx_path: Path, cfg_path: Path) -> dict:
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("LQOS_", "LIBREQOS_"))}
    env.update({
        "RUST_LOG": "info",
        "LQOS_CONTROL_CHANNEL": "0",
        "LIBREQOS_DPDK_USE_LPM_CLASSIFIER": "1",
        "LIBREQOS_DPDK_PLACEMENT_OWNERSHIP": "off",
        "LIBREQOS_DPDK_TRANSACTION_FILE": str(tx_path),
        "LIBREQOS_DPDK_CONFIG_FILE": str(cfg_path),
        "LIBREQOS_ANALYTICS_SOCKET": str(work / "analytics.sock"),
    })
    return env


def wait_for(predicate, timeout: float, what: str) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.2)
    die(f"timed out waiting for {what}")


def launch() -> None:
    """Set up networking, launch the bridge + lqosd, and block until Ctrl-C."""
    args = parse_args()
    src_dir, repo_root, bridge_bin, lqosd_bin = resolve_paths(args)
    preflight(src_dir, bridge_bin, lqosd_bin)

    work = Path(args.work_dir or "/run/lqos-demo")
    tx_path, cfg_path = write_work_files(src_dir, work, args.circuits)

    template_path = (src_dir / "rust/lqos_config/src/etc/v15/example.toml")
    if not template_path.is_file():
        die(f"lqos.conf template not found: {template_path}")
    conf_path = work / "lqos.conf"
    conf_path.write_text(
        build_demo_config(template_path.read_text(encoding="utf-8"),
                          src_dir, work / "state", args.port),
        encoding="utf-8")

    setup_network()

    log_path = work / "bridge.log"
    log_handle = log_path.open("a", encoding="utf-8")
    bridge = subprocess.Popen(bridge_argv(bridge_bin, args.workers),
                              env=bridge_env(work, tx_path, cfg_path),
                              cwd=work, stdout=log_handle,
                              stderr=subprocess.STDOUT, start_new_session=True)
    log_handle.close()

    def bridge_ready() -> bool:
        if bridge.poll() is not None:
            die(f"bridge_rs exited during startup ({bridge.returncode}); see {log_path}")
        if not log_path.is_file():
            return False
        text = log_path.read_text(encoding="utf-8", errors="replace")
        return BRIDGE_BOOT_MARKER in text

    def socket_ready() -> bool:
        return (work / "analytics.sock").exists()

    wait_for(bridge_ready, 15.0, "bridge startup")
    wait_for(socket_ready, 15.0, "analytics socket")
    log("bridge APPLIED; analytics socket is live")

    (work / "state").mkdir(parents=True, exist_ok=True)
    Path("/run/lqos").mkdir(parents=True, exist_ok=True)

    lqosd_path = work / "lqosd.log"
    lqosd_env = {k: v for k, v in os.environ.items()
                 if not k.startswith(("LQOS_", "LIBREQOS_"))}
    lqosd_env.update({
        "RUST_LOG": "info",
        "LQOS_CONFIG": str(conf_path),
        "LIBREQOS_ANALYTICS_SOCKET": str(work / "analytics.sock"),
    })
    lqosd_handle = lqosd_path.open("a", encoding="utf-8")
    lqosd = subprocess.Popen([str(lqosd_bin)], env=lqosd_env, cwd=src_dir,
                             stdout=lqosd_handle, stderr=subprocess.STDOUT,
                             start_new_session=True)
    lqosd_handle.close()

    def port_open() -> bool:
        if lqosd.poll() is not None:
            die(f"lqosd exited during startup ({lqosd.returncode}); see {lqosd_path}")
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            probe.settimeout(0.3)
            return probe.connect_ex(("127.0.0.1", args.port)) == 0

    wait_for(port_open, 25.0, "the lqosd web UI")
    log(f"dashboard: http://127.0.0.1:{args.port}/")
    log(f"logs: {log_path} , {lqosd_path}")

    if args.traffic:
        run_traffic(args.traffic_seconds)

    log("running; press Ctrl-C to stop and clean up")
    try:
        while True:
            if bridge.poll() is not None:
                die(f"bridge_rs exited ({bridge.returncode}); see {log_path}")
            if lqosd.poll() is not None:
                die(f"lqosd exited ({lqosd.returncode}); see {lqosd_path}")
            time.sleep(0.5)
    finally:
        stop(bridge, lqosd)
        cleanup_network(args.keep_work)


def run_traffic(seconds: int) -> None:
    log(f"sending a short UDP burst through the bridge ({seconds}s)")
    script = (
        "import socket,time\n"
        "s=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)\n"
        "payload=b'x'*1200\n"
        "end=time.time()+" + str(seconds) + "\n"
        "while time.time()<end:\n"
        "    s.sendto(payload,('" + GATEWAY + "',9))\n"
    )
    subprocess.run(["ip", "netns", "exec", NAMESPACE, "python3", "-c", script],
                   check=False)


def stop(bridge, lqosd) -> None:
    for proc in (bridge, lqosd):
        try:
            if proc.poll() is None:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
        except (ProcessLookupError, PermissionError):
            pass
    for proc in (bridge, lqosd):
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass


def cleanup_network(keep_work: bool) -> None:
    log("cleaning up namespace + veths")
    subprocess.run(["ip", "netns", "del", NAMESPACE], check=False)
    for dev in (LAN, WAN, UPLINK):
        subprocess.run(["ip", "link", "del", dev], check=False,
                       capture_output=True)
    if not keep_work:
        shutil.rmtree("/run/lqos-demo", ignore_errors=True)
    else:
        log("kept /run/lqos-demo")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--circuits", type=int, default=DEMO_CIRCUITS)
    parser.add_argument("--workers", type=int, default=1)
    parser.add_argument("--port", type=int, default=WEBSERVER_PORT)
    parser.add_argument("--bridge-bin", default=None)
    parser.add_argument("--lqosd-bin", default=None)
    parser.add_argument("--work-dir", default=None)
    parser.add_argument("--traffic-seconds", type=int, default=5)
    parser.add_argument("--no-traffic", dest="traffic", action="store_false",
                        default=True)
    parser.add_argument("--keep-work", action="store_true")
    parser.add_argument("--dry-run", action="store_true",
                        help="print the plan and exit (no root, no changes)")
    parser.add_argument("--selftest", action="store_true",
                        help="run the pure-helper checks and exit")
    return parser.parse_args()


def selftest() -> int:
    assert derive_capacity(16) == 17
    assert derive_capacity(1) == 2
    tx = build_transaction(16)
    assert count_leaf_upserts(tx) == 16
    assert len(tx["mapping_ops"]) == 16
    assert tx["mapping_ops"][0]["ip"] == "198.18.0.2"
    cfg = build_bridge_config(derive_capacity(16))
    assert cfg == {"schema_version": 1, "circuit_capacity": 17}
    overlay = build_demo_config(
        'version = "1.5"\nlqos_directory = "/opt/libreqos/src"\n'
        'state_directory = "/opt/libreqos/state"\nnode_name = "X"\n'
        '[long_term_stats]\ngather_stats = true\nlicense_key = "(data)"\n',
        Path("/v3/src"), Path("/run/lqos-demo/state"), 9123)
    assert 'lqos_directory = "/v3/src"' in overlay
    assert 'state_directory = "/run/lqos-demo/state"' in overlay
    assert overlay.index("webserver_listen") < overlay.index("[long_term_stats]")
    assert "gather_stats = false" in overlay
    assert 'license_key = ""' in overlay
    print("[demo] selftest OK")
    return 0


def main() -> int:
    args = parse_args()
    if args.selftest:
        return selftest()
    if args.dry_run:
        src_dir, repo_root, bridge_bin, lqosd_bin = resolve_paths(args)
        tx = build_transaction(args.circuits)
        capacity = derive_capacity(count_leaf_upserts(tx))
        print(f"[demo] dry-run: repo={repo_root}")
        print(f"[demo] dry-run: bridge={bridge_bin}")
        print(f"[demo] dry-run: lqosd={lqosd_bin}")
        print(f"[demo] dry-run: {args.circuits} circuits -> capacity {capacity}")
        print(f"[demo] dry-run: dashboard on :{args.port}")
        return 0
    if os.geteuid() != 0:
        log("not root; re-executing under sudo (needed for veth/netns + AF_PACKET)")
        os.execvp("sudo", ["sudo", "-E", sys.executable,
                           os.path.abspath(__file__), *sys.argv[1:]])
    launch()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
