#!/usr/bin/env python3
"""
LibreQoS + Bakery integration test harness

Purpose
- Verify incremental vs full Bakery reload behavior for both tiered and flat topologies
  by driving LibreQoS.py and inspecting lqosd logs.

Important
- This test applies real TC/XDP changes via lqosd. Use on a test box or during a
  maintenance window. It is NOT intended for production service units under systemd.

Manual steps (run in two terminals)
1) Start lqosd manually as root with logs to a file:
     sudo RUST_LOG=info lqosd 2>&1 | tee /tmp/lqosd.log
   - Ensure /etc/lqos.conf interfaces and bandwidths are set correctly for this host.
   - Stop systemd units first if they are enabled (optional):
       sudo systemctl stop lqosd lqos_scheduler lqos_api
2) In another terminal from this directory (LibreQoS/src), run the tests:
     python3 bakery_integration_test.py --log-file /tmp/lqosd.log
   - You can limit scope with --tiered-only or --flat-only.

What this script does
- Backs up `network.json` and `ShapedDevices.csv` in the current directory.
- Writes small tiered and flat fixtures and calls LibreQoS.refreshShapers() after each change.
- Parses lqosd logs to decide whether a full reload occurred or an incremental update happened.
- Restores your original files at the end unless --no-restore is used.

Notes
- The first commit after (re)starting lqosd will do a full reload (MQ init). The
  test allows that and asserts the subsequent expected behaviors.
- Flat networks do not have explicit sites, so site add/remove checks are skipped there.

Usage examples
- Tiered + Flat (default):
    python3 bakery_integration_test.py --log-file /tmp/lqosd.log
- Tiered only:
    python3 bakery_integration_test.py --log-file /tmp/lqosd.log --tiered-only
- Flat only:
    python3 bakery_integration_test.py --log-file /tmp/lqosd.log --flat-only

Exit code
- 0 on success, 1 on assertion failure or precondition failure.
"""

from __future__ import annotations

import argparse
import contextlib
import csv
import json
import os
import re
import shutil
import sys
import time
from dataclasses import dataclass
from typing import Dict, List, Optional, Tuple


# Import LibreQoS and lqosd helpers without invoking the __main__ block
try:
    import LibreQoS  # type: ignore
except Exception as e:
    print(f"ERROR: Failed to import LibreQoS.py: {e}")
    sys.exit(1)

try:
    from liblqos_python import is_lqosd_alive  # type: ignore
    # Optional: bus log message helper (newer liblqos_python)
    try:
        from liblqos_python import log_info  # type: ignore
    except Exception:
        log_info = None  # type: ignore
except Exception:
    # Provide a soft fallback if binding is unavailable
    def is_lqosd_alive() -> bool:
        return False

def _mark_step(name: str) -> None:
    msg = f"STARTING TEST ({name})"
    try:
        if 'log_info' in globals() and callable(log_info):  # type: ignore[name-defined]
            try:
                ok = log_info(msg)  # type: ignore[misc]
                if not ok:
                    print(msg)
            except Exception:
                print(msg)
        else:
            print(msg)
    except Exception:
        print(msg)


# -----------------------
# Log parsing and waiting
# -----------------------

FULL_RELOAD_PAT = re.compile(r"Bakery: Full reload triggered")
FULL_RELOAD_SUMMARY_PAT = re.compile(r"Bakery full reload:.*")
MQ_INIT_PAT = re.compile(r"MQ not created, performing full reload\.")
CHANGES_PAT = re.compile(
    r"Bakery changes: sites_speed=(\d+), circuits_added=(\d+), removed=(\d+), speed=(\d+), ip=(\d+)"
)
NO_CHANGES_PAT = re.compile(r"No changes detected in batch, skipping processing\.")


@dataclass
class LogResult:
    full_reload: bool
    mq_init: bool
    changes: Optional[Dict[str, int]]
    raw_lines: List[str]


class LogReader:
    def __init__(self, path: str):
        self.path = path

    def snapshot(self) -> int:
        try:
            return os.path.getsize(self.path)
        except Exception:
            return 0

    def read_since(self, offset: int) -> List[str]:
        lines: List[str] = []
        try:
            with open(self.path, "r", errors="replace") as f:
                f.seek(offset)
                for line in f:
                    lines.append(line.rstrip("\n"))
        except Exception:
            pass
        return lines

    def wait_for_events(self, offset: int, timeout_s: float = 20.0) -> LogResult:
        deadline = time.time() + timeout_s
        print(f"Waiting for Bakery events (timeout {timeout_s:.1f}s)...")
        collected: List[str] = []
        full_reload = False
        mq_init = False
        changes: Optional[Dict[str, int]] = None
        printed_sleep_note = False

        while time.time() < deadline:
            new_lines = self.read_since(offset)
            if new_lines:
                collected.extend(new_lines)
                # Seek to last byte each time
                offset += sum(len(l) + 1 for l in new_lines)

                # Parse new chunk
                for ln in new_lines:
                    if FULL_RELOAD_PAT.search(ln) or FULL_RELOAD_SUMMARY_PAT.search(ln):
                        full_reload = True
                    if MQ_INIT_PAT.search(ln):
                        mq_init = True
                    m = CHANGES_PAT.search(ln)
                    if m:
                        changes = {
                            "sites_speed": int(m.group(1)),
                            "circuits_added": int(m.group(2)),
                            "removed": int(m.group(3)),
                            "speed": int(m.group(4)),
                            "ip": int(m.group(5)),
                        }
                # Heuristic: stop when we saw an outcome (changes or full reload or no changes)
                if changes or full_reload or any(NO_CHANGES_PAT.search(x) for x in new_lines):
                    break

            if not printed_sleep_note:
                print("No new logs yet; sleeping 0.4s...")
                printed_sleep_note = True
            time.sleep(0.4)

        return LogResult(full_reload=full_reload, mq_init=mq_init, changes=changes, raw_lines=collected)


# -----------------------
# Fixtures and mutators
# -----------------------

TIERED_NETWORK_BASE = {
    "Site_A": {
        "downloadBandwidthMbps": 100,
        "uploadBandwidthMbps": 100,
        "type": "Site",
        "children": {
            "AP_A": {
                "downloadBandwidthMbps": 50,
                "uploadBandwidthMbps": 50,
                "type": "AP",
            }
        },
    },
}


def write_network_json(obj: dict) -> None:
    with open("network.json", "w") as f:
        json.dump(obj, f, indent=2)


CSV_HEADER = [
    "Circuit ID",
    "Circuit Name",
    "Device ID",
    "Device Name",
    "Parent Node",
    "MAC",
    "IPv4",
    "IPv6",
    "Download Min Mbps",
    "Upload Min Mbps",
    "Download Max Mbps",
    "Upload Max Mbps",
    "Comment",
    "sqm",
]


def write_circuits(rows: List[Dict[str, str | int | float]]) -> None:
    with open("ShapedDevices.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=CSV_HEADER)
        w.writeheader()
        for r in rows:
            w.writerow(r)


def tiered_circuits_base() -> List[Dict[str, str | int | float]]:
    # Two circuits hanging off AP_A
    return [
        {
            "Circuit ID": "1",
            "Circuit Name": "C1",
            "Device ID": "1",
            "Device Name": "D1",
            "Parent Node": "AP_A",
            "MAC": "",
            "IPv4": "100.64.0.1",
            "IPv6": "",
            "Download Min Mbps": 1,
            "Upload Min Mbps": 1,
            "Download Max Mbps": 10,
            "Upload Max Mbps": 10,
            "Comment": "",
            "sqm": "",
        },
        {
            "Circuit ID": "2",
            "Circuit Name": "C2",
            "Device ID": "2",
            "Device Name": "D2",
            "Parent Node": "AP_A",
            "MAC": "",
            "IPv4": "100.64.0.2",
            "IPv6": "",
            "Download Min Mbps": 1,
            "Upload Min Mbps": 1,
            "Download Max Mbps": 10,
            "Upload Max Mbps": 10,
            "Comment": "",
            "sqm": "",
        },
    ]


def flat_network_base() -> dict:
    # Flat = empty object
    return {}


def flat_circuits_base() -> List[Dict[str, str | int | float]]:
    # Parent Node is ignored when flat, but we still populate a value.
    return [
        {
            "Circuit ID": "10",
            "Circuit Name": "F1",
            "Device ID": "10",
            "Device Name": "FD1",
            "Parent Node": "none",
            "MAC": "",
            "IPv4": "100.64.1.1",
            "IPv6": "",
            "Download Min Mbps": 1,
            "Upload Min Mbps": 1,
            "Download Max Mbps": 10,
            "Upload Max Mbps": 10,
            "Comment": "",
            "sqm": "",
        },
        {
            "Circuit ID": "11",
            "Circuit Name": "F2",
            "Device ID": "11",
            "Device Name": "FD2",
            "Parent Node": "none",
            "MAC": "",
            "IPv4": "100.64.1.2",
            "IPv6": "",
            "Download Min Mbps": 1,
            "Upload Min Mbps": 1,
            "Download Max Mbps": 10,
            "Upload Max Mbps": 10,
            "Comment": "",
            "sqm": "",
        },
    ]


# -----------------------
# Test runner utilities
# -----------------------


def run_refresh_and_wait(log: LogReader, timeout_s: float) -> LogResult:
    offset = log.snapshot()
    # Call the core refresh function; this does not require running as __main__
    LibreQoS.refreshShapers()
    # Allow lqosd time to process the commit and log
    print("Sleeping 0.5s to allow lqosd to commit and log...")
    time.sleep(0.5)
    return log.wait_for_events(offset, timeout_s=timeout_s)


def assert_no_full_reload(tag: str, res: LogResult, step_started_at: Optional[float] = None) -> Tuple[bool, str]:
    if res.full_reload and not res.mq_init:
        ts = (
            time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(step_started_at))
            + f" (epoch {step_started_at:.3f})"
            if step_started_at is not None
            else "unknown"
        )
        return False, f"{tag}: Unexpected full reload detected (step started {ts})"
    return True, f"{tag}: OK (no full reload)"


def assert_full_reload(tag: str, res: LogResult, step_started_at: Optional[float] = None) -> Tuple[bool, str]:
    if not res.full_reload:
        ts = (
            time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(step_started_at))
            + f" (epoch {step_started_at:.3f})"
            if step_started_at is not None
            else "unknown"
        )
        return False, f"{tag}: Expected full reload, but none detected (step started {ts})"
    return True, f"{tag}: OK (full reload)"


def with_backups(paths: List[str]):
    class _Ctx:
        def __enter__(self):
            self.baks = []
            for p in paths:
                if os.path.exists(p):
                    bak = p + ".bak.integration"
                    shutil.copyfile(p, bak)
                    self.baks.append((p, bak))
            return self

        def __exit__(self, exc_type, exc, tb):
            # Restore originals
            for p, bak in self.baks:
                try:
                    shutil.move(bak, p)
                except Exception:
                    pass
            # Remove any remaining temp backups
            for p in paths:
                bak = p + ".bak.integration"
                with contextlib.suppress(Exception):
                    if os.path.exists(bak):
                        os.remove(bak)

    return _Ctx()


# -----------------------
# Scenarios
# -----------------------


def run_tiered_suite(log: LogReader, timeout_s: float, results: List[str]) -> bool:
    ok = True

    # Baseline
    write_network_json(TIERED_NETWORK_BASE)
    rows = tiered_circuits_base()
    write_circuits(rows)
    res = run_refresh_and_wait(log, timeout_s)
    # First run likely MQ init/full reload; do not assert here
    results.append("tiered: baseline: " + ("ok (initial run)"))

    # Site speed change (same site name & location)
    _mark_step("tiered: site speed change")
    net2 = json.loads(json.dumps(TIERED_NETWORK_BASE))
    net2["Site_A"]["downloadBandwidthMbps"] = 120
    net2["Site_A"]["uploadBandwidthMbps"] = 110
    write_network_json(net2)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("tiered: site speed change", res, step_t0)
    results.append(msg)
    ok &= passed

    # Circuit speed change
    _mark_step("flat: circuit speed change")
    _mark_step("tiered: circuit speed change")
    rows2 = json.loads(json.dumps(rows))
    rows2[0]["Download Max Mbps"] = 20
    rows2[0]["Upload Max Mbps"] = 15
    write_circuits(rows2)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("tiered: circuit speed change", res, step_t0)
    results.append(msg)
    ok &= passed

    # Circuit SQM change
    _mark_step("flat: circuit SQM change")
    _mark_step("tiered: circuit SQM change")
    rows3 = json.loads(json.dumps(rows2))
    rows3[1]["sqm"] = "cake/fq_codel"
    write_circuits(rows3)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("tiered: circuit SQM change", res, step_t0)
    results.append(msg)
    ok &= passed

    # Add circuit
    _mark_step("flat: add circuit")
    _mark_step("tiered: add circuit")
    rows4 = json.loads(json.dumps(rows3))
    rows4.append({
        "Circuit ID": "3",
        "Circuit Name": "C3",
        "Device ID": "3",
        "Device Name": "D3",
        "Parent Node": "AP_A",
        "MAC": "",
        "IPv4": "100.64.0.3",
        "IPv6": "",
        "Download Min Mbps": 1,
        "Upload Min Mbps": 1,
        "Download Max Mbps": 10,
        "Upload Max Mbps": 10,
        "Comment": "",
        "sqm": "",
    })
    write_circuits(rows4)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("tiered: add circuit", res, step_t0)
    results.append(msg)
    ok &= passed

    # Remove circuit
    _mark_step("flat: remove circuit")
    _mark_step("tiered: remove circuit")
    rows5 = [r for r in rows4 if r["Circuit ID"] != "3"]
    write_circuits(rows5)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("tiered: remove circuit", res, step_t0)
    results.append(msg)
    ok &= passed

    # Add site (should trigger full reload)
    _mark_step("tiered: add site")
    net3 = json.loads(json.dumps(net2))
    net3["Site_B"] = {
        "downloadBandwidthMbps": 80,
        "uploadBandwidthMbps": 80,
        "type": "Site",
        "children": {},
    }
    write_network_json(net3)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_full_reload("tiered: add site", res, step_t0)
    results.append(msg)
    ok &= passed

    # Remove site (should trigger full reload)
    _mark_step("tiered: remove site")
    net4 = json.loads(json.dumps(net3))
    del net4["Site_B"]
    write_network_json(net4)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_full_reload("tiered: remove site", res, step_t0)
    results.append(msg)
    ok &= passed

    return ok


def run_flat_suite(log: LogReader, timeout_s: float, results: List[str]) -> bool:
    ok = True

    # Baseline
    write_network_json(flat_network_base())
    rows = flat_circuits_base()
    write_circuits(rows)
    res = run_refresh_and_wait(log, timeout_s)
    results.append("flat: baseline: ok (initial run or structural change)")

    # Circuit speed change
    step_t0 = time.time()
    rows2 = json.loads(json.dumps(rows))
    rows2[0]["Download Max Mbps"] = 25
    rows2[0]["Upload Max Mbps"] = 20
    write_circuits(rows2)
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("flat: circuit speed change", res, step_t0)
    results.append(msg)
    ok &= passed

    # Circuit SQM change
    step_t0 = time.time()
    rows3 = json.loads(json.dumps(rows2))
    rows3[1]["sqm"] = "fq_codel/cake"
    write_circuits(rows3)
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("flat: circuit SQM change", res, step_t0)
    results.append(msg)
    ok &= passed

    # Add circuit
    step_t0 = time.time()
    rows4 = json.loads(json.dumps(rows3))
    rows4.append({
        "Circuit ID": "12",
        "Circuit Name": "F3",
        "Device ID": "12",
        "Device Name": "FD3",
        "Parent Node": "none",
        "MAC": "",
        "IPv4": "100.64.1.3",
        "IPv6": "",
        "Download Min Mbps": 1,
        "Upload Min Mbps": 1,
        "Download Max Mbps": 10,
        "Upload Max Mbps": 10,
        "Comment": "",
        "sqm": "",
    })
    write_circuits(rows4)
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("flat: add circuit", res, step_t0)
    results.append(msg)
    ok &= passed

    # Remove circuit
    step_t0 = time.time()
    rows5 = [r for r in rows4 if r["Circuit ID"] != "12"]
    write_circuits(rows5)
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("flat: remove circuit", res, step_t0)
    results.append(msg)
    ok &= passed

    return ok


# -----------------------
# Main
# -----------------------


def main() -> int:
    ap = argparse.ArgumentParser(description="LibreQoS/Bakery integration tests")
    ap.add_argument("--log-file", required=True, help="Path to lqosd log file (stdout/tee capture)")
    ap.add_argument("--timeout", type=float, default=25.0, help="Wait time per step (seconds)")
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--tiered-only", action="store_true", help="Run tiered cases only")
    g.add_argument("--flat-only", action="store_true", help="Run flat cases only")
    ap.add_argument("--no-restore", action="store_true", help="Do not restore original files (debugging)")

    args = ap.parse_args()

    # Pre-flight guidance
    if not os.path.exists(args.log_file):
        print("ERROR: Log file not found. Please run lqosd with logs, e.g.:")
        print("  sudo RUST_LOG=info lqosd 2>&1 | tee /tmp/lqosd.log")
        return 1

    if not is_lqosd_alive():
        print("WARNING: lqosd does not appear to be running. This test will not observe Bakery logs.")
        print("Start lqosd and re-run, or continue if already started but lib binding cannot detect it.")

    log = LogReader(args.log_file)
    results: List[str] = []
    overall_ok = True

    files = ["network.json", "ShapedDevices.csv"]
    ctx = contextlib.nullcontext()
    if not args.no_restore:
        ctx = with_backups(files)

    with ctx:
        # Run tiered suite
        if not args.flat_only:
            ok = run_tiered_suite(log, args.timeout, results)
            overall_ok &= ok

        # Run flat suite
        if not args.tiered_only:
            ok = run_flat_suite(log, args.timeout, results)
            overall_ok &= ok

    print("Results:")
    for r in results:
        print(f"- {r}")

    if not overall_ok:
        print("One or more checks failed. See lqosd logs for details.")
        print("Tips:")
        print("- Start a fresh lqosd before the run to get a clean baseline")
        print("- Confirm RUST_LOG includes 'info' level (or 'debug') for lqos_bakery")
        print("- Verify /etc/lqos.conf is correct for your interfaces")
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
