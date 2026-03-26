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
1) Either:
   - Start lqosd manually as root with logs to a file:
       sudo RUST_LOG=info lqosd 2>&1 | tee /tmp/lqosd.log
     and run the harness with `--log-file /tmp/lqosd.log`, or
   - Run against the systemd-managed daemon and let the harness read `journalctl -u lqosd`.
   - Ensure /etc/lqos.conf interfaces and bandwidths are set correctly for this host.
2) In another terminal from this directory (LibreQoS/src), run the tests:
     python3 bakery_integration_test.py
   - The default run is the quicker tiered suite.
   - Use --full-suite for the whole harness, or --flat-only/--treeguard-only/etc. for a focused run.

What this script does
- Backs up `network.json` and `ShapedDevices.csv` in the current directory.
- Writes focused tiered/flat fixtures plus a larger synthetic TreeGuard runtime fixture and calls LibreQoS.refreshShapers() after each change.
- Parses lqosd logs to decide whether a full reload occurred or an incremental update happened.
- Restores your original files at the end unless --no-restore is used.

Notes
- The first commit after (re)starting lqosd will do a full reload (MQ init). The
  test allows that and asserts the subsequent expected behaviors.
- Flat networks do not have explicit sites, so site add/remove checks are skipped there.
- Newer Bakery builds log incremental completion as `Bakery mapped circuit decision (...)`
  rather than only the older `Bakery changes: ...` line. This harness accepts both.
- In `cpu_aware` TreeGuard mode, the live TreeGuard suite may legitimately report a
  skip if no CPU pressure is induced on the box during the test window.

Usage examples
- Quick default run (tiered only):
    python3 bakery_integration_test.py
- Use a tee'd manual log file:
    python3 bakery_integration_test.py --log-file /tmp/lqosd.log
- Full suite:
    python3 bakery_integration_test.py --full-suite
- Tiered only:
    python3 bakery_integration_test.py --tiered-only
- TreeGuard runtime suite only:
    python3 bakery_integration_test.py --treeguard-only --treeguard-timeout 90
- Fault-injection reload escalation suite only:
    python3 bakery_integration_test.py --fault-reload-only
  This destructive safety-path test is opt-in and is not included in --full-suite.
- Queue mode toggle suite only:
    sudo python3 bakery_integration_test.py --queue-mode-only
- Flat only:
    python3 bakery_integration_test.py --flat-only

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
from pathlib import Path
from typing import Dict, List, Optional, Set, Tuple, Union
import subprocess


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
    # Optional helpers for interface name and PN speeds
    try:
        from liblqos_python import interface_a  # type: ignore
    except Exception:
        interface_a = None  # type: ignore
    try:
        from liblqos_python import interface_b  # type: ignore
    except Exception:
        interface_b = None  # type: ignore
    try:
        from liblqos_python import generated_pn_download_mbps, generated_pn_upload_mbps  # type: ignore
    except Exception:
        generated_pn_download_mbps = None  # type: ignore
        generated_pn_upload_mbps = None  # type: ignore
    try:
        from liblqos_python import on_a_stick  # type: ignore
    except Exception:
        on_a_stick = None  # type: ignore
    try:
        from liblqos_python import queue_mode  # type: ignore
    except Exception:
        queue_mode = None  # type: ignore
    try:
        from liblqos_python import sync_lqosd_config_from_disk  # type: ignore
    except Exception:
        sync_lqosd_config_from_disk = None  # type: ignore
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
MAPPED_DECISION_PAT = re.compile(
    r"Bakery mapped circuit decision \(([^)]+)\): requested=(\d+), allowed=(\d+), dropped=(\d+),"
)
NO_CHANGES_PAT = re.compile(r"No changes detected in batch, skipping processing\.")
TEST_FAULT_ONCE_PATH = "/tmp/lqos_bakery_fail_purpose_once.txt"


@dataclass
class LogResult:
    full_reload: bool
    mq_init: bool
    changes: Optional[Dict[str, int]]
    incremental_event: Optional[str]
    raw_lines: List[str]


SnapshotToken = Union[int, float]


class LogReader:
    def __init__(self, path: str):
        self.path = path

    def snapshot(self) -> SnapshotToken:
        try:
            return os.path.getsize(self.path)
        except Exception:
            return 0

    def read_since(self, offset: SnapshotToken) -> Tuple[SnapshotToken, List[str]]:
        lines: List[str] = []
        try:
            with open(self.path, "r", errors="replace") as f:
                f.seek(int(offset))
                for line in f:
                    lines.append(line.rstrip("\n"))
        except Exception:
            pass
        next_offset = int(offset) + sum(len(l) + 1 for l in lines)
        return next_offset, lines

    def wait_for_events(self, offset: SnapshotToken, timeout_s: float = 20.0) -> LogResult:
        deadline = time.time() + timeout_s
        print(f"Waiting for Bakery events (timeout {timeout_s:.1f}s)...")
        collected: List[str] = []
        full_reload = False
        mq_init = False
        changes: Optional[Dict[str, int]] = None
        incremental_event: Optional[str] = None
        printed_sleep_note = False

        while time.time() < deadline:
            offset, new_lines = self.read_since(offset)
            if new_lines:
                collected.extend(new_lines)
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
                    mapped = MAPPED_DECISION_PAT.search(ln)
                    if mapped:
                        incremental_event = mapped.group(1)
                        changes = {
                            "requested": int(mapped.group(2)),
                            "allowed": int(mapped.group(3)),
                            "dropped": int(mapped.group(4)),
                        }
                        if "full reload" in incremental_event.lower():
                            full_reload = True
                # Heuristic: stop when we saw an outcome (changes or full reload or no changes)
                if changes or full_reload or any(NO_CHANGES_PAT.search(x) for x in new_lines):
                    break

            if not printed_sleep_note:
                print("No new logs yet; sleeping 0.4s...")
                printed_sleep_note = True
            time.sleep(0.4)

        return LogResult(
            full_reload=full_reload,
            mq_init=mq_init,
            changes=changes,
            incremental_event=incremental_event,
            raw_lines=collected,
        )


class JournalctlLogReader:
    def __init__(self, unit: str = "lqosd"):
        self.unit = unit

    def snapshot(self) -> SnapshotToken:
        return time.time()

    def _since_arg(self, token: SnapshotToken) -> str:
        return f"@{float(token):.6f}"

    def read_since(self, token: SnapshotToken) -> Tuple[SnapshotToken, List[str]]:
        start = float(token)
        proc = subprocess.run(
            [
                "journalctl",
                "-u",
                self.unit,
                "--since",
                self._since_arg(start),
                "--no-pager",
                "-o",
                "short-unix",
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        if proc.returncode != 0:
            return start, []

        lines: List[str] = []
        next_token = start
        for raw_line in proc.stdout.splitlines():
            line = raw_line.rstrip("\n")
            if not line:
                continue
            lines.append(line)
            timestamp_token = line.split(" ", 1)[0].strip()
            try:
                next_token = max(next_token, float(timestamp_token) + 0.000001)
            except Exception:
                next_token = max(next_token, time.time())

        return next_token, lines

    def wait_for_events(self, offset: SnapshotToken, timeout_s: float = 20.0) -> LogResult:
        deadline = time.time() + timeout_s
        print(f"Waiting for Bakery events from journalctl (timeout {timeout_s:.1f}s)...")
        collected: List[str] = []
        full_reload = False
        mq_init = False
        changes: Optional[Dict[str, int]] = None
        incremental_event: Optional[str] = None
        printed_sleep_note = False

        while time.time() < deadline:
            offset, new_lines = self.read_since(offset)
            if new_lines:
                collected.extend(new_lines)
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
                    mapped = MAPPED_DECISION_PAT.search(ln)
                    if mapped:
                        incremental_event = mapped.group(1)
                        changes = {
                            "requested": int(mapped.group(2)),
                            "allowed": int(mapped.group(3)),
                            "dropped": int(mapped.group(4)),
                        }
                        if "full reload" in incremental_event.lower():
                            full_reload = True
                if changes or full_reload or any(NO_CHANGES_PAT.search(x) for x in new_lines):
                    break

            if not printed_sleep_note:
                print("No new logs yet; sleeping 0.4s...")
                printed_sleep_note = True
            time.sleep(0.4)

        return LogResult(
            full_reload=full_reload,
            mq_init=mq_init,
            changes=changes,
            incremental_event=incremental_event,
            raw_lines=collected,
        )


AnyLogReader = Union[LogReader, JournalctlLogReader]


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


VIRTUALIZED_TIERED_NETWORK_BASE = {
    "Site_A": {
        "downloadBandwidthMbps": 120,
        "uploadBandwidthMbps": 120,
        "type": "Site",
        "children": {
            "Town_V": {
                "downloadBandwidthMbps": 80,
                "uploadBandwidthMbps": 80,
                "type": "Site",
                "virtual": True,
                "children": {
                    "AP_V": {
                        "downloadBandwidthMbps": 40,
                        "uploadBandwidthMbps": 40,
                        "type": "AP",
                    }
                },
            },
            "AP_A": {
                "downloadBandwidthMbps": 50,
                "uploadBandwidthMbps": 50,
                "type": "AP",
            },
        },
    },
}


TREEGUARD_RUNTIME_REGION_COUNT = 8
TREEGUARD_RUNTIME_POPS_PER_REGION = 5
TREEGUARD_RUNTIME_APS_PER_POP = 5
TREEGUARD_RUNTIME_CIRCUITS_PER_AP = 5
TREEGUARD_RUNTIME_TOTAL_CIRCUITS = (
    TREEGUARD_RUNTIME_REGION_COUNT
    * TREEGUARD_RUNTIME_POPS_PER_REGION
    * TREEGUARD_RUNTIME_APS_PER_POP
    * TREEGUARD_RUNTIME_CIRCUITS_PER_AP
)


def _treeguard_runtime_region_name(region_index: int) -> str:
    return f"REGION_{region_index:02d}"


def _treeguard_runtime_pop_name(region_index: int, pop_index: int) -> str:
    ordinal = (region_index - 1) * TREEGUARD_RUNTIME_POPS_PER_REGION + pop_index
    return f"POP_{ordinal:03d}"


def _treeguard_runtime_ap_name(region_index: int, pop_index: int, ap_index: int) -> str:
    ordinal = (
        (region_index - 1) * TREEGUARD_RUNTIME_POPS_PER_REGION * TREEGUARD_RUNTIME_APS_PER_POP
        + (pop_index - 1) * TREEGUARD_RUNTIME_APS_PER_POP
        + ap_index
    )
    return f"AP_{ordinal:04d}"


def _treeguard_runtime_circuit_ordinal(region_index: int, pop_index: int, ap_index: int, circuit_index: int) -> int:
    ap_ordinal = (
        (region_index - 1) * TREEGUARD_RUNTIME_POPS_PER_REGION * TREEGUARD_RUNTIME_APS_PER_POP
        + (pop_index - 1) * TREEGUARD_RUNTIME_APS_PER_POP
        + ap_index
    )
    return (ap_ordinal - 1) * TREEGUARD_RUNTIME_CIRCUITS_PER_AP + circuit_index


def _treeguard_runtime_circuit_id(region_index: int, pop_index: int, ap_index: int, circuit_index: int) -> str:
    return f"{100000 + _treeguard_runtime_circuit_ordinal(region_index, pop_index, ap_index, circuit_index):06d}"


def _treeguard_runtime_circuit_ipv4(circuit_ordinal: int) -> str:
    # Keep addresses in CGNAT space while avoiding .0 and .255 octets.
    third_octet = 1 + ((circuit_ordinal - 1) // 250)
    fourth_octet = 1 + ((circuit_ordinal - 1) % 250)
    return f"100.90.{third_octet}.{fourth_octet}"


def treeguard_runtime_fixture_metadata() -> dict[str, str | int]:
    return {
        "virtualized_region": _treeguard_runtime_region_name(1),
        "promoted_pop": _treeguard_runtime_pop_name(1, 1),
        "promoted_leaf": _treeguard_runtime_ap_name(1, 1, 1),
        "promoted_leaf_circuit": _treeguard_runtime_circuit_id(1, 1, 1, 1),
        "sibling_pop": _treeguard_runtime_pop_name(1, 2),
        "sibling_leaf": _treeguard_runtime_ap_name(1, 2, 1),
        "sibling_leaf_circuit": _treeguard_runtime_circuit_id(1, 2, 1, 1),
        "low_value_sibling": _treeguard_runtime_region_name(TREEGUARD_RUNTIME_REGION_COUNT),
        "total_circuits": TREEGUARD_RUNTIME_TOTAL_CIRCUITS,
    }


def treeguard_runtime_network_base() -> dict:
    network: dict[str, dict] = {}
    for region_index in range(1, TREEGUARD_RUNTIME_REGION_COUNT + 1):
        region_name = _treeguard_runtime_region_name(region_index)
        pop_children: dict[str, dict] = {}
        for pop_index in range(1, TREEGUARD_RUNTIME_POPS_PER_REGION + 1):
            pop_name = _treeguard_runtime_pop_name(region_index, pop_index)
            ap_children: dict[str, dict] = {}
            for ap_index in range(1, TREEGUARD_RUNTIME_APS_PER_POP + 1):
                ap_name = _treeguard_runtime_ap_name(region_index, pop_index, ap_index)
                ap_children[ap_name] = {
                    "downloadBandwidthMbps": max(12, 60 - region_index - pop_index - ap_index),
                    "uploadBandwidthMbps": max(12, 40 - region_index - pop_index - ap_index),
                    "type": "AP",
                }
            pop_children[pop_name] = {
                "downloadBandwidthMbps": max(60, 260 - (region_index * 12) - (pop_index * 8)),
                "uploadBandwidthMbps": max(40, 180 - (region_index * 8) - (pop_index * 5)),
                "type": "Site",
                "children": ap_children,
            }

        if region_index == 1:
            region_download = 1200
            region_upload = 800
        elif region_index == TREEGUARD_RUNTIME_REGION_COUNT:
            region_download = 180
            region_upload = 120
        else:
            region_download = max(260, 760 - (region_index * 55))
            region_upload = max(180, 520 - (region_index * 35))

        network[region_name] = {
            "downloadBandwidthMbps": region_download,
            "uploadBandwidthMbps": region_upload,
            "type": "Site",
            "children": pop_children,
        }
    return network


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


def virtualized_tiered_circuits_base() -> List[Dict[str, str | int | float]]:
    return [
        {
            "Circuit ID": "501",
            "Circuit Name": "VIRTUAL_DIRECT",
            "Device ID": "501",
            "Device Name": "VD1",
            "Parent Node": "Town_V",
            "MAC": "",
            "IPv4": "100.64.2.1",
            "IPv6": "",
            "Download Min Mbps": 1,
            "Upload Min Mbps": 1,
            "Download Max Mbps": 20,
            "Upload Max Mbps": 10,
            "Comment": "",
            "sqm": "",
        },
        {
            "Circuit ID": "502",
            "Circuit Name": "VIRTUAL_CHILD",
            "Device ID": "502",
            "Device Name": "VD2",
            "Parent Node": "AP_V",
            "MAC": "",
            "IPv4": "100.64.2.2",
            "IPv6": "",
            "Download Min Mbps": 1,
            "Upload Min Mbps": 1,
            "Download Max Mbps": 15,
            "Upload Max Mbps": 8,
            "Comment": "",
            "sqm": "",
        },
        {
            "Circuit ID": "503",
            "Circuit Name": "SIBLING_REAL",
            "Device ID": "503",
            "Device Name": "VD3",
            "Parent Node": "AP_A",
            "MAC": "",
            "IPv4": "100.64.2.3",
            "IPv6": "",
            "Download Min Mbps": 1,
            "Upload Min Mbps": 1,
            "Download Max Mbps": 12,
            "Upload Max Mbps": 6,
            "Comment": "",
            "sqm": "",
        },
    ]


def treeguard_runtime_circuits_base() -> List[Dict[str, str | int | float]]:
    rows: List[Dict[str, str | int | float]] = []
    for region_index in range(1, TREEGUARD_RUNTIME_REGION_COUNT + 1):
        for pop_index in range(1, TREEGUARD_RUNTIME_POPS_PER_REGION + 1):
            for ap_index in range(1, TREEGUARD_RUNTIME_APS_PER_POP + 1):
                ap_name = _treeguard_runtime_ap_name(region_index, pop_index, ap_index)
                for circuit_index in range(1, TREEGUARD_RUNTIME_CIRCUITS_PER_AP + 1):
                    circuit_ordinal = _treeguard_runtime_circuit_ordinal(
                        region_index, pop_index, ap_index, circuit_index
                    )
                    circuit_id = _treeguard_runtime_circuit_id(
                        region_index, pop_index, ap_index, circuit_index
                    )
                    rows.append(
                        {
                            "Circuit ID": circuit_id,
                            "Circuit Name": (
                                f"TG_R{region_index:02d}_P{pop_index:02d}_A{ap_index:02d}_C{circuit_index:02d}"
                            ),
                            "Device ID": circuit_id,
                            "Device Name": f"TG-D{circuit_ordinal:04d}",
                            "Parent Node": ap_name,
                            "MAC": "",
                            "IPv4": _treeguard_runtime_circuit_ipv4(circuit_ordinal),
                            "IPv6": "",
                            "Download Min Mbps": 1,
                            "Upload Min Mbps": 1,
                            "Download Max Mbps": 8 + ((circuit_index + ap_index) % 6),
                            "Upload Max Mbps": 4 + ((circuit_index + pop_index) % 4),
                            "Comment": "",
                            "sqm": "",
                        }
                    )
    if len(rows) != TREEGUARD_RUNTIME_TOTAL_CIRCUITS:
        raise RuntimeError(
            f"treeguard runtime fixture generated {len(rows)} circuits, expected {TREEGUARD_RUNTIME_TOTAL_CIRCUITS}"
        )
    return rows


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


REALISTIC_TIERED_NETWORK = {
    "NET1": {
        "downloadBandwidthMbps": 900,
        "uploadBandwidthMbps": 600,
        "type": "Site",
        "children": {
            "NET1-1": {
                "downloadBandwidthMbps": 400,
                "uploadBandwidthMbps": 250,
                "type": "Site",
                "children": {
                    "NET1-1-1": {
                        "downloadBandwidthMbps": 180,
                        "uploadBandwidthMbps": 90,
                        "type": "AP",
                    },
                    "NET1-1-2": {
                        "downloadBandwidthMbps": 160,
                        "uploadBandwidthMbps": 80,
                        "type": "AP",
                    },
                },
            },
            "NET1-2": {
                "downloadBandwidthMbps": 350,
                "uploadBandwidthMbps": 210,
                "type": "AP",
            },
        },
    },
    "NET2": {
        "downloadBandwidthMbps": 800,
        "uploadBandwidthMbps": 500,
        "type": "Site",
        "children": {
            "NET2-1": {
                "downloadBandwidthMbps": 300,
                "uploadBandwidthMbps": 200,
                "type": "Site",
                "children": {
                    "NET2-1-1": {
                        "downloadBandwidthMbps": 150,
                        "uploadBandwidthMbps": 75,
                        "type": "AP",
                    },
                },
            },
            "NET2-2": {
                "downloadBandwidthMbps": 220,
                "uploadBandwidthMbps": 110,
                "type": "AP",
            },
        },
    },
    "NET3": {
        "downloadBandwidthMbps": 700,
        "uploadBandwidthMbps": 450,
        "type": "Site",
        "children": {
            "NET3-1": {
                "downloadBandwidthMbps": 260,
                "uploadBandwidthMbps": 150,
                "type": "AP",
            },
            "NET3-2": {
                "downloadBandwidthMbps": 240,
                "uploadBandwidthMbps": 130,
                "type": "AP",
            },
        },
    },
}


def _collect_nodes_from_network(net: Dict[str, dict]) -> List[Tuple[str, int, int]]:
    """Flatten a nested network dict into (name, dl, ul) tuples."""
    nodes: List[Tuple[str, int, int]] = []

    def walk(node_map: Dict[str, dict]) -> None:
        for name, node in node_map.items():
            if not isinstance(node, dict):
                continue
            dl = node.get("downloadBandwidthMbps")
            ul = node.get("uploadBandwidthMbps")
            try:
                dl_int = int(dl) if dl is not None else 0
            except Exception:
                dl_int = 0
            try:
                ul_int = int(ul) if ul is not None else 0
            except Exception:
                ul_int = 0
            nodes.append((name, dl_int, ul_int))
            ch = node.get("children")
            if isinstance(ch, dict):
                walk(ch)

    walk(net)
    return nodes


def realistic_tiered_circuits_base() -> List[Dict[str, str | int | float]]:
    """Return asymmetric circuits for a realistic multi-site tree.

    At least two circuits are attached to each node, plus a pool of
    orphan circuits that will be assigned to generated PNs.
    """
    rows: List[Dict[str, str | int | float]] = []
    nodes = _collect_nodes_from_network(REALISTIC_TIERED_NETWORK)
    circuit_id = 2001
    base_third_octet = 10

    # Two circuits per node in the tree
    for idx, (parent_name, dl, ul) in enumerate(nodes):
        third = base_third_octet + idx
        for j in range(2):
            # Keep circuit rates comfortably below the parent and asymmetric
            if dl > 0:
                max_dl = int(max(5, dl * (0.4 - 0.03 * j)))
            else:
                max_dl = 10
            if ul > 0:
                max_ul = int(max(2, ul * (0.3 - 0.03 * j)))
            else:
                max_ul = 5
            if max_ul >= max_dl:
                max_ul = max(1, max_dl - 1)

            ip_addr = f"100.64.{third}.{j + 1}"
            if j == 0:
                ip_addr = f"{ip_addr},100.64.{third}.{j + 101}"
            row: Dict[str, str | int | float] = {
                "Circuit ID": str(circuit_id),
                "Circuit Name": f"CIRCUIT_{circuit_id:04d}",
                "Device ID": str(circuit_id),
                "Device Name": f"DEV_{circuit_id:04d}",
                "Parent Node": parent_name,
                "MAC": "",
                "IPv4": ip_addr,
                "IPv6": "",
                "Download Min Mbps": 1,
                "Upload Min Mbps": 1,
                "Download Max Mbps": max_dl,
                "Upload Max Mbps": max_ul,
                "Comment": "",
                "sqm": "",
            }
            rows.append(row)
            circuit_id += 1

    # Add a pool of orphan circuits that will be placed under generated PNs
    pn_dl_default = 100
    pn_ul_default = 50
    if generated_pn_download_mbps and generated_pn_upload_mbps:  # type: ignore[name-defined]
        try:
            pn_dl_default = int(generated_pn_download_mbps())  # type: ignore[misc]
            pn_ul_default = int(generated_pn_upload_mbps())  # type: ignore[misc]
        except Exception:
            pass
    orphan_dl = max(5, min(80, pn_dl_default - 5))
    orphan_ul = max(2, min(30, pn_ul_default - 5, orphan_dl - 1))

    for k in range(10):
        ip_addr = f"100.64.250.{k + 1}"
        row = {
            "Circuit ID": str(circuit_id),
            "Circuit Name": f"ORPHAN_{circuit_id:04d}",
            "Device ID": str(circuit_id),
            "Device Name": f"DEV_ORPHAN_{circuit_id:04d}",
            "Parent Node": "none",
            "MAC": "",
            "IPv4": ip_addr,
            "IPv6": "",
            "Download Min Mbps": 1,
            "Upload Min Mbps": 1,
            "Download Max Mbps": orphan_dl,
            "Upload Max Mbps": orphan_ul,
            "Comment": "",
            "sqm": "",
        }
        rows.append(row)
        circuit_id += 1

    return rows


# -----------------------
# Test runner utilities
# -----------------------


def run_refresh_and_wait(log: AnyLogReader, timeout_s: float) -> LogResult:
    offset = log.snapshot()
    # Call the core refresh function; this does not require running as __main__
    LibreQoS.refreshShapers()
    # Allow lqosd time to process the commit and log
    print("Sleeping 0.5s to allow lqosd to commit and log...")
    time.sleep(0.5)
    return log.wait_for_events(offset, timeout_s=timeout_s)


def run_refresh_subprocess_and_wait(log: AnyLogReader, timeout_s: float) -> LogResult:
    offset = log.snapshot()
    proc = subprocess.run(
        [sys.executable, "LibreQoS.py"],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.stdout:
        print(proc.stdout, end="" if proc.stdout.endswith("\n") else "\n")
    if proc.stderr:
        print(proc.stderr, end="" if proc.stderr.endswith("\n") else "\n", file=sys.stderr)
    if proc.returncode != 0:
        raise RuntimeError(f"queue-mode suite: LibreQoS.py subprocess exited {proc.returncode}")
    print("Sleeping 0.5s to allow lqosd to commit and log...")
    time.sleep(0.5)
    return log.wait_for_events(offset, timeout_s=timeout_s)


def settle_initial_bakery_logs(log: AnyLogReader, settle_s: float = 1.5) -> None:
    offset = log.snapshot()
    time.sleep(settle_s)
    _ = log.wait_for_events(offset, timeout_s=0.5)


def clear_bakery_fault_once() -> None:
    try:
        os.remove(TEST_FAULT_ONCE_PATH)
    except FileNotFoundError:
        pass


def arm_bakery_fault_once(selector: str) -> None:
    with open(TEST_FAULT_ONCE_PATH, "w") as f:
        f.write(selector.strip())
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())


# -----------------------
# TC and structure checks
# -----------------------

def _tc(args: List[str]) -> Tuple[int, str, str]:
    try:
        proc = subprocess.run(["/sbin/tc"] + args, capture_output=True, text=True, check=False)
        return proc.returncode, proc.stdout, proc.stderr
    except Exception as e:
        return 127, "", str(e)


def check_default_classes_present_on_iface() -> Tuple[bool, str]:
    """Verify that on the first interface, default classes exist: class <maj>:2 parent <maj>:1.

    Returns (passed, message).
    """
    if not interface_a or not callable(interface_a):  # type: ignore[name-defined]
        return False, "tc check: interface_a() binding unavailable"
    try:
        iface = interface_a()  # type: ignore[misc]
    except Exception as e:
        return False, f"tc check: failed to get interface_a(): {e}"
    rc, out, err = _tc(["class", "show", "dev", iface])
    if rc != 0:
        return False, f"tc check: 'tc class show dev {iface}' failed: {err.strip()}"
    # Look for any line where child is :2 and parent is same major :1
    pat = re.compile(r"^class\s+htb\s+(\w+):2\s+parent\s+\1:1\b", re.MULTILINE)
    if pat.search(out):
        return True, f"tc check: default class present on {iface}"
    return False, f"tc check: default class not found on {iface}"


def check_default_classes_present_on_iface_b() -> Tuple[bool, str]:
    """Same as above but for interface_b when available."""
    if not interface_b or not callable(interface_b):  # type: ignore[name-defined]
        return True, "tc check: interface_b() unavailable; skipping"
    try:
        iface = interface_b()  # type: ignore[misc]
    except Exception as e:
        return False, f"tc check: failed to get interface_b(): {e}"
    rc, out, err = _tc(["class", "show", "dev", iface])
    if rc != 0:
        return False, f"tc check: 'tc class show dev {iface}' failed: {err.strip()}"
    # Match: class htb <maj>:2 parent <maj>:1
    pat = re.compile(r"^class\s+htb\s+(\w+):2\s+parent\s+\1:1\b", re.MULTILINE)
    if pat.search(out):
        return True, f"tc check: default class present on {iface}"
    return False, f"tc check: default class not found on {iface}"


def _load_queuing_structure() -> Optional[dict]:
    try:
        with open("queuingStructure.json", "r") as f:
            return json.load(f)
    except Exception:
        return None


def assert_generated_pns_present() -> Tuple[bool, List[str]]:
    msgs: List[str] = []
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["queuingStructure.json not found or unreadable"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["queuingStructure.json has no 'Network' dict"]
    gpns = qs.get("generatedPNs")
    if not isinstance(gpns, list) or not gpns:
        return False, ["No generatedPNs listed in queuingStructure.json"]
    # Two shapes are valid:
    # 1) Binpacked: Generated_PN_* appear under CpueQueue* children.
    # 2) Non-binpacked: Generated_PN_* appear at top-level of Network.
    cpu_bins = [(k, v) for k, v in net.items() if isinstance(k, str) and k.startswith("CpueQueue")]
    if cpu_bins:
        found_under_cpu = 0
        for k, v in cpu_bins:
            ch = v.get("children") if isinstance(v, dict) else None
            if isinstance(ch, dict) and any(isinstance(nm, str) and nm.startswith("Generated_PN_") for nm in ch.keys()):
                found_under_cpu += 1
        if found_under_cpu == 0:
            return False, ["No Generated_PN_* found under any CpueQueue* in queuingStructure.json"]
        msgs.append(f"generated PN presence: found under {found_under_cpu} CPU bins")
        return True, msgs
    # Top-level fallback
    top_level_pn = [k for k in net.keys() if isinstance(k, str) and k.startswith("Generated_PN_")]
    if not top_level_pn:
        return False, ["No Generated_PN_* found at top-level of queuingStructure.json"]
    msgs.append(f"generated PN presence: found {len(top_level_pn)} at top-level")
    return True, msgs


def _find_pn_nodes(net: Dict[str, dict]) -> List[Tuple[str, dict]]:
    result: List[Tuple[str, dict]] = []
    def walk(node_map: Dict[str, dict]):
        for k, v in node_map.items():
            if not isinstance(v, dict):
                continue
            if isinstance(k, str) and k.startswith("Generated_PN_"):
                result.append((k, v))
            ch = v.get("children") if isinstance(v, dict) else None
            if isinstance(ch, dict):
                walk(ch)
    walk(net)
    return result


def _hex_to_int(s: str) -> Optional[int]:
    try:
        if isinstance(s, str):
            return int(s, 16) if s.startswith("0x") else int(s)
        if isinstance(s, int):
            return int(s)
    except Exception:
        return None
    return None


def _tc_id_token_to_int(tok: str) -> Optional[int]:
    """Parse tc class/qdisc ID tokens.

    The live `tc` text output commonly emits bare hex tokens like `10f` rather than
    `0x10f`. Treat bare tokens as hex so lookups line up with queuingStructure.json.
    """
    s = tok.strip()
    if not s:
        return None
    try:
        if s.lower().startswith("0x"):
            return int(s, 16)
        return int(s, 16)
    except Exception:
        return None


def _parse_tc_rate_to_mbps(token: str, unit: Optional[str]) -> float:
    try:
        val = float(token)
    except Exception:
        return -1.0
    mult = 1.0
    if unit is None or unit == "":
        # tc typically prints explicit units; be conservative and treat as mbit
        mult = 1.0
    else:
        u = unit.upper()
        if u == 'G':
            mult = 1000.0
        elif u == 'M':
            mult = 1.0
        elif u == 'K':
            mult = 0.001
        else:
            mult = 1.0
    return val * mult


def _read_htb_rate_ceil_mbps(iface: str, major: int, minor: int) -> Optional[Tuple[float, float, str]]:
    rc, out, err = _tc(["class", "show", "dev", iface])
    if rc != 0:
        return None
    # Match the class line for this major/minor, accepting decimal or hex forms.
    cls_pat = re.compile(r"^class\s+htb\s+([0-9A-Fa-fx]+):([0-9A-Fa-fx]+)\b.*$", re.MULTILINE)

    line = None
    for m in cls_pat.finditer(out):
        mj_tok, mn_tok = m.group(1), m.group(2)
        mj = _tc_id_token_to_int(mj_tok)
        mn = _tc_id_token_to_int(mn_tok)
        if mj == major and mn == minor:
            line = m.group(0)
            break
    if line is None:
        return None
    # Extract rate and ceil tokens
    rpat = re.compile(r"rate\s+([0-9]+(?:\.[0-9]+)?)([GMK])?bit.*?ceil\s+([0-9]+(?:\.[0-9]+)?)([GMK])?bit")
    mm = rpat.search(line)
    if not mm:
        return None
    rate_mbps = _parse_tc_rate_to_mbps(mm.group(1), mm.group(2))
    ceil_mbps = _parse_tc_rate_to_mbps(mm.group(3), mm.group(4))
    return rate_mbps, ceil_mbps, line

def _qdisc_show(iface: str) -> Tuple[int, str, str]:
    return _tc(["qdisc", "show", "dev", iface])


def _read_text(path: str) -> str:
    with open(path, "r", encoding="utf-8", errors="replace") as handle:
        return handle.read()


def _write_text(path: str, text: str) -> None:
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)
        handle.flush()
        os.fsync(handle.fileno())


def _set_queue_mode_in_lqos_conf_text(config_text: str, mode: str) -> str:
    if mode not in {"shape", "observe"}:
        raise ValueError(f"unsupported queue mode: {mode}")

    section_pat = re.compile(r"(?ms)^(\[queues\]\n)(.*?)(?=^\[|\Z)")
    match = section_pat.search(config_text)
    if not match:
        raise RuntimeError("queue-mode suite: [queues] section not found in /etc/lqos.conf")

    header = match.group(1)
    body = match.group(2)
    queue_mode_line = f'queue_mode = "{mode}"\n'
    if re.search(r'(?m)^queue_mode\s*=\s*".*?"\s*$', body):
        new_body = re.sub(
            r'(?m)^queue_mode\s*=\s*".*?"\s*$',
            queue_mode_line.rstrip("\n"),
            body,
            count=1,
        )
        if not new_body.endswith("\n"):
            new_body += "\n"
    else:
        new_body = queue_mode_line + body
    return config_text[:match.start()] + header + new_body + config_text[match.end():]


@dataclass
class TcShapeSummary:
    iface: str
    root_mq: bool
    clsact: bool
    root_htb_qdiscs: int
    htb_classes: int
    child_leaf_qdiscs: int


def _summarize_iface_tc(iface: str) -> TcShapeSummary:
    qdisc_rc, qdisc_out, qdisc_err = _qdisc_show(iface)
    if qdisc_rc != 0:
        raise RuntimeError(f"queue-mode suite: tc qdisc show dev {iface} failed: {qdisc_err.strip()}")

    class_rc, class_out, class_err = _tc(["class", "show", "dev", iface])
    if class_rc != 0:
        raise RuntimeError(f"queue-mode suite: tc class show dev {iface} failed: {class_err.strip()}")

    root_mq = bool(re.search(r"(?m)^qdisc\s+mq\s+7fff:\s+root\b", qdisc_out))
    clsact = bool(re.search(r"(?m)^qdisc\s+clsact\s+ffff:\s+parent\s+ffff:fff1\b", qdisc_out))
    root_htb_qdiscs = len(
        re.findall(r"(?m)^qdisc\s+htb\s+[0-9A-Fa-fx]+:\s+parent\s+7fff:[0-9A-Fa-fx]+\b", qdisc_out)
    )
    htb_classes = len(re.findall(r"(?m)^class\s+htb\s+[0-9A-Fa-fx]+:[0-9A-Fa-fx]+\b", class_out))
    child_leaf_qdiscs = len(
        re.findall(
            r"(?m)^qdisc\s+(?!mq\b)(?!htb\b)(?!clsact\b)\S+\s+\S+:\s+parent\s+(?!ffff:)[0-9A-Fa-fx]+:[0-9A-Fa-fx]+\b",
            qdisc_out,
        )
    )

    return TcShapeSummary(
        iface=iface,
        root_mq=root_mq,
        clsact=clsact,
        root_htb_qdiscs=root_htb_qdiscs,
        htb_classes=htb_classes,
        child_leaf_qdiscs=child_leaf_qdiscs,
    )


def _shape_summary_message(prefix: str, summary: TcShapeSummary) -> str:
    return (
        f"{prefix}: {summary.iface}: "
        f"root_mq={summary.root_mq}, clsact={summary.clsact}, "
        f"root_htb_qdiscs={summary.root_htb_qdiscs}, "
        f"htb_classes={summary.htb_classes}, "
        f"child_leaf_qdiscs={summary.child_leaf_qdiscs}"
    )


def check_no_site_circuit_minor_collisions() -> Tuple[bool, List[str]]:
    """Verify that no circuit under any node shares the same classMinor as its parent node.

    This guards against regressions where a circuit could be assigned the site/PN's minor,
    causing qdiscs to attach at the site handle instead of the circuit handle.
    """
    msgs: List[str] = []
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["minor collision check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["minor collision check: queuingStructure has no 'Network'"]

    def hx(v) -> Optional[int]:
        try:
            s = str(v)
            return int(s, 16) if s.startswith("0x") else int(s)
        except Exception:
            return None

    ok = True

    def walk(node_map: Dict[str, dict]):
        nonlocal ok
        for name, node in node_map.items():
            if not isinstance(node, dict):
                continue
            site_minor = hx(node.get("classMinor"))
            if isinstance(node.get("circuits"), list) and site_minor is not None:
                for c in node.get("circuits", []):
                    try:
                        cmin = hx(c.get("classMinor"))
                        cid = str(c.get("circuitID", ""))
                        if cmin is not None and cmin == site_minor:
                            msgs.append(f"minor collision check: circuit {cid} under '{name}' shares site minor 0x{site_minor:x}")
                            ok = False
                    except Exception:
                        pass
            ch = node.get("children")
            if isinstance(ch, dict):
                walk(ch)

    walk(net)
    if ok:
        msgs.append("minor collision check: no site/circuit minor collisions detected")
    return ok, msgs


def snapshot_site_class_assignments() -> Tuple[bool, str, Dict[str, Tuple[str, str, str, str]]]:
    """Capture current site class assignment state from queuingStructure.json."""
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, "site snapshot: queuingStructure.json not found", {}
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, "site snapshot: queuingStructure has no 'Network'", {}

    snapshot: Dict[str, Tuple[str, str, str, str]] = {}

    def walk(node_map: Dict[str, dict], trail: Tuple[str, ...] = ()) -> None:
        for name, node in node_map.items():
            if not isinstance(node, dict):
                continue
            path = "/".join(trail + (name,))
            snapshot[path] = (
                str(node.get("classMinor", "")),
                str(node.get("classid", "")),
                str(node.get("parentClassID", "")),
                str(node.get("cpuNum", "")),
            )
            ch = node.get("children")
            if isinstance(ch, dict):
                walk(ch, trail + (name,))

    walk(net)
    return True, "", snapshot


def check_site_class_assignments_unchanged(
    before: Dict[str, Tuple[str, str, str, str]]
) -> Tuple[bool, List[str]]:
    """Verify that site class assignments did not drift between two snapshots."""
    ok, err, after = snapshot_site_class_assignments()
    if not ok:
        return False, [err]

    msgs: List[str] = []
    changed = []
    for path in sorted(set(before.keys()) | set(after.keys())):
        if before.get(path) != after.get(path):
            changed.append((path, before.get(path), after.get(path)))

    if changed:
        for path, old_value, new_value in changed[:20]:
            msgs.append(
                f"site assignment stability: {path} changed {old_value} -> {new_value}"
            )
        if len(changed) > 20:
            msgs.append(
                f"site assignment stability: additional changed sites omitted ({len(changed) - 20})"
            )
        return False, msgs

    msgs.append("site assignment stability: all site class assignments unchanged")
    return True, msgs


# -----------------------
# IP mapping checks
# -----------------------

def _list_ip_mappings() -> Tuple[bool, str, List[Tuple[str, int, str]]]:
    """Return (ok, message, entries) where entries are (ip, cpu, tc) from the mapper.
    Uses ./bin/xdp_iphash_to_cpu_cmdline list. When unavailable, returns ok=False with reason.
    """
    tool = os.path.join(".", "bin", "xdp_iphash_to_cpu_cmdline")
    if not os.path.exists(tool):
        return False, "ipmap check: mapping tool not found", []
    try:
        proc = subprocess.run([tool, "list"], capture_output=True, text=True, check=False)
    except Exception as e:
        return False, f"ipmap check: failed to run mapping tool: {e}", []
    out = (proc.stdout or "")
    err = (proc.stderr or "").strip()
    if "Socket" in out or "Socket" in err:
        return False, "ipmap check: lqosd bus socket not found; skipping", []
    entries: List[Tuple[str, int, str]] = []
    # Lines look like:
    # "<ip/prefix>    CPU: <cpu>  TC: <a:b>"
    # or newer output with trailing identity fields:
    # "<ip/prefix>    CPU: <cpu>  TC: <a:b> CIRCUIT: <id> DEVICE: <id>"
    pat = re.compile(
        r"^\s*([0-9A-Fa-f:.]+)/\d+\s+CPU:\s+(\d+)\s+TC:\s+([0-9A-Fa-f]+:[0-9A-Fa-f]+)(?:\s+.*)?$"
    )
    for line in out.splitlines():
        m = pat.match(line)
        if m:
            ip = m.group(1)
            cpu = int(m.group(2))
            tc = m.group(3).lower()
            entries.append((ip, cpu, tc))
    return True, "", entries


def _expect_handles_for_circuit(parent_node: dict, circuit: dict) -> Tuple[str, Optional[str], Optional[int]]:
    """Compute expected down/up tc handle strings (hex without 0x) and CPU from structure."""
    def hx(v):
        try:
            return int(str(v), 16)
        except Exception:
            return None
    maj = hx(circuit.get("classMajor"))
    mnr = hx(circuit.get("classMinor"))
    upm = hx(circuit.get("up_classMajor"))
    cpu = hx(parent_node.get("cpuNum"))
    down_tc = f"{maj:x}:{mnr:x}" if maj is not None and mnr is not None else ""
    up_tc = f"{upm:x}:{mnr:x}" if upm is not None and mnr is not None else None
    return down_tc, up_tc, cpu


def _find_parent_node(net: Dict[str, dict], name: str) -> Optional[dict]:
    for k, v in net.items():
        if k == name and isinstance(v, dict):
            return v
        ch = v.get("children") if isinstance(v, dict) else None
        if isinstance(ch, dict):
            got = _find_parent_node(ch, name)
            if got is not None:
                return got
    return None


def _find_node_path(node_map: Dict[str, dict], target_name: str, prefix: Optional[List[str]] = None) -> Optional[List[str]]:
    current_prefix = prefix or []
    for name, node in node_map.items():
        if not isinstance(node, dict):
            continue
        node_path = current_prefix + [name]
        if name == target_name:
            return node_path
        children = node.get("children")
        if isinstance(children, dict):
            found = _find_node_path(children, target_name, node_path)
            if found is not None:
                return found
    return None


def _walk_nodes_for_circuit_path(
    node_map: Dict[str, dict],
    circuit_id: str,
    prefix: Optional[List[str]] = None,
) -> Optional[Tuple[List[str], dict]]:
    current_prefix = prefix or []
    for name, node in node_map.items():
        if not isinstance(node, dict):
            continue
        node_path = current_prefix + [name]
        circuits = node.get("circuits")
        if isinstance(circuits, list):
            for circuit in circuits:
                try:
                    if str(circuit.get("circuitID", "")) == str(circuit_id):
                        return node_path, circuit
                except Exception:
                    pass
        children = node.get("children")
        if isinstance(children, dict):
            found = _walk_nodes_for_circuit_path(children, circuit_id, node_path)
            if found:
                return found
    return None


def check_virtualized_node_branch_promotion(
    virtual_node_name: str,
    promoted_child_name: str,
    promoted_child_expected_path: List[str],
    circuit_expectations: List[Tuple[str, str, List[str]]],
    *,
    expect_virtual_node_listed: bool = True,
) -> Tuple[bool, List[str]]:
    msgs: List[str] = []
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["virtualized node check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["virtualized node check: queuingStructure has no 'Network'"]

    ok = True
    if expect_virtual_node_listed:
        virtual_nodes = qs.get("virtual_nodes")
        if not isinstance(virtual_nodes, list) or virtual_node_name not in virtual_nodes:
            msgs.append(f"virtualized node check: '{virtual_node_name}' missing from queuingStructure virtual_nodes")
            ok = False
        else:
            msgs.append(f"virtualized node check: '{virtual_node_name}' listed in queuingStructure virtual_nodes")

    virtual_node = _find_parent_node(net, virtual_node_name)
    if virtual_node is not None:
        msgs.append(f"virtualized node check: '{virtual_node_name}' still exists in physical queue tree")
        ok = False
    else:
        msgs.append(f"virtualized node check: '{virtual_node_name}' absent from physical queue tree")

    promoted_child_path = _find_node_path(net, promoted_child_name)
    if promoted_child_path is None:
        msgs.append(f"virtualized node check: promoted child '{promoted_child_name}' not found")
        ok = False
    elif promoted_child_path != promoted_child_expected_path:
        msgs.append(
            "virtualized node check: promoted child "
            f"'{promoted_child_name}' found at {'/'.join(promoted_child_path)}, "
            f"expected {'/'.join(promoted_child_expected_path)}"
        )
        ok = False
    else:
        msgs.append(
            f"virtualized node check: promoted child '{promoted_child_name}' found at {'/'.join(promoted_child_path)}"
        )

    for label, circuit_id, expected_parent_path in circuit_expectations:
        found = _walk_nodes_for_circuit_path(net, circuit_id)
        if not found:
            msgs.append(f"virtualized node check: {label} circuit {circuit_id} not found in structure")
            ok = False
            continue
        actual_parent_path, _ = found
        if actual_parent_path != expected_parent_path:
            msgs.append(
                f"virtualized node check: {label} circuit {circuit_id} attached to "
                f"{'/'.join(actual_parent_path)}, expected {'/'.join(expected_parent_path)}"
            )
            ok = False
        else:
            msgs.append(
                f"virtualized node check: {label} circuit {circuit_id} attached to {'/'.join(actual_parent_path)}"
            )

    return ok, msgs


def check_node_present_in_physical_tree(node_name: str) -> Tuple[bool, List[str]]:
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["treeguard node presence check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["treeguard node presence check: queuingStructure has no 'Network'"]
    node = _find_parent_node(net, node_name)
    if node is None:
        return False, [f"treeguard node presence check: '{node_name}' absent from physical queue tree"]
    return True, [f"treeguard node presence check: '{node_name}' still present in physical queue tree"]


def _log_contains_unexpected_full_reload(lines: List[str]) -> bool:
    full_reload = any(FULL_RELOAD_PAT.search(line) or FULL_RELOAD_SUMMARY_PAT.search(line) for line in lines)
    mq_init = any(MQ_INIT_PAT.search(line) for line in lines)
    return full_reload and not mq_init


def wait_for_runtime_virtualized_node(
    virtual_node_name: str,
    promoted_child_name: str,
    promoted_child_expected_path: List[str],
    circuit_expectations: List[Tuple[str, str, List[str]]],
    timeout_s: float,
    poll_s: float = 1.0,
) -> Tuple[bool, List[str]]:
    deadline = time.time() + timeout_s
    last_msgs: List[str] = []
    while time.time() < deadline:
        passed, msgs = check_virtualized_node_branch_promotion(
            virtual_node_name,
            promoted_child_name,
            promoted_child_expected_path,
            circuit_expectations,
            expect_virtual_node_listed=False,
        )
        last_msgs = msgs
        if passed:
            last_msgs.append(
                f"treeguard runtime check: '{virtual_node_name}' virtualized live within {timeout_s:.1f}s"
            )
            return True, last_msgs
        time.sleep(poll_s)
    last_msgs.append(
        f"treeguard runtime check: timed out waiting {timeout_s:.1f}s for '{virtual_node_name}' runtime virtualization"
    )
    return False, last_msgs


def get_treeguard_cpu_mode(config_path: str = "/etc/lqos.conf") -> Optional[str]:
    current_section: Optional[str] = None
    mode_re = re.compile(r'^mode\s*=\s*"([^"]+)"')
    try:
        with open(config_path, "r", errors="replace") as f:
            for raw in f:
                line = raw.strip()
                if not line or line.startswith("#"):
                    continue
                if line.startswith("[") and line.endswith("]"):
                    current_section = line.strip("[]").strip()
                    continue
                if current_section != "treeguard.cpu":
                    continue
                m = mode_re.match(line)
                if m:
                    return m.group(1).strip()
    except Exception:
        return None
    return None


def wait_for_circuit_direction_ceil(
    circuit_id: str,
    timeout_s: float = 5.0,
    poll_s: float = 0.5,
) -> Tuple[bool, List[str]]:
    deadline = time.time() + timeout_s
    last_msgs: List[str] = []
    while time.time() < deadline:
        passed, msgs = check_circuit_direction_ceil(circuit_id)
        last_msgs = msgs
        if passed:
            return True, msgs
        time.sleep(poll_s)
    last_msgs.append(
        f"direction check: timed out waiting {timeout_s:.1f}s for circuit {circuit_id} direction mapping to settle"
    )
    return False, last_msgs


def check_ip_mappings_for_circuit(circuit_id: str) -> Tuple[bool, List[str]]:
    """Ensure that all IPv4s for this circuit are mapped to the correct down (and up if on-a-stick) tc handles."""
    ok_tool, msg, entries = _list_ip_mappings()
    msgs: List[str] = []
    if not ok_tool:
        return True, [msg]  # soft-skip
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["ipmap check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["ipmap check: queuingStructure has no 'Network'"]
    found = _walk_nodes_for_circuit(net, circuit_id)
    if not found:
        return False, [f"ipmap check: circuit {circuit_id} not found in structure"]
    parent_name, circuit = found
    parent_node = _find_parent_node(net, parent_name)
    if not isinstance(parent_node, dict):
        return False, [f"ipmap check: parent node '{parent_name}' not found"]
    down_tc, up_tc, cpu = _expect_handles_for_circuit(parent_node, circuit)
    # Collect expected IPv4s
    ips: List[str] = []
    for dev in circuit.get("devices", []):
        try:
            for ip in dev.get("ipv4s", []):
                ips.append(str(ip))
        except Exception:
            pass
    if not ips:
        return True, [f"ipmap check: circuit {circuit_id} has no IPv4s; skipping"]
    # Evaluate mappings
    ok = True
    on_stick = False
    try:
        on_stick = bool(on_a_stick()) if callable(on_a_stick) else False  # type: ignore[misc]
    except Exception:
        on_stick = False
    for ip in ips:
        # Find all entries matching this IP
        ents = [(i, c, t) for (i, c, t) in entries if i == ip]
        if not ents:
            msgs.append(f"ipmap check: missing mapping for {ip}")
            ok = False
            continue
        # Down must exist
        if any(t == down_tc for (_i, _c, t) in ents):
            msgs.append(f"ipmap check: {ip} -> {down_tc} present")
        else:
            msgs.append(f"ipmap check: {ip} missing down tc {down_tc}")
            ok = False
        # Up mapping (only if on-a-stick)
        if on_stick and up_tc:
            if any(t == up_tc for (_i, _c, t) in ents):
                msgs.append(f"ipmap check: {ip} -> {up_tc} present (upload)")
            else:
                msgs.append(f"ipmap check: {ip} missing up tc {up_tc}")
                ok = False
    return ok, msgs


def check_ip_mapping_absent(ip: str) -> Tuple[bool, List[str]]:
    ok_tool, msg, entries = _list_ip_mappings()
    if not ok_tool:
        return True, [msg]  # soft-skip
    present = any(i == ip for (i, _c, _t) in entries)
    if present:
        return False, [f"ipmap check: unexpected mapping remains for {ip}"]
    return True, [f"ipmap check: {ip} successfully absent"]


def _expected_ips_for_circuit(circuit: dict) -> List[str]:
    ips: List[str] = []
    for dev in circuit.get("devices", []):
        try:
            for ip in dev.get("ipv4s", []):
                ips.append(str(ip))
        except Exception:
            pass
    return ips


def check_exact_ip_mappings_for_circuit(
    circuit_id: str,
    forbidden_tcs: Optional[Set[str]] = None,
) -> Tuple[bool, List[str]]:
    ok_tool, msg, entries = _list_ip_mappings()
    msgs: List[str] = []
    if not ok_tool:
        return True, [msg]  # soft-skip

    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["ipmap exact check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["ipmap exact check: queuingStructure has no 'Network'"]
    found = _walk_nodes_for_circuit(net, circuit_id)
    if not found:
        return False, [f"ipmap exact check: circuit {circuit_id} not found in structure"]
    parent_name, circuit = found
    parent_node = _find_parent_node(net, parent_name)
    if not isinstance(parent_node, dict):
        return False, [f"ipmap exact check: parent node '{parent_name}' not found"]

    down_tc, up_tc, _cpu = _expect_handles_for_circuit(parent_node, circuit)
    allowed_tcs = {down_tc.lower()}
    if up_tc:
        allowed_tcs.add(up_tc.lower())
    forbidden = {tc.lower() for tc in (forbidden_tcs or set())}

    ips = _expected_ips_for_circuit(circuit)
    if not ips:
        return True, [f"ipmap exact check: circuit {circuit_id} has no IPv4s; skipping"]

    ok = True
    for ip in ips:
        ip_entries = [(cpu, tc.lower()) for (mapped_ip, cpu, tc) in entries if mapped_ip == ip]
        if not ip_entries:
            msgs.append(f"ipmap exact check: missing mapping for {ip}")
            ok = False
            continue
        actual_tcs = {tc for (_cpu, tc) in ip_entries}
        if not actual_tcs.issubset(allowed_tcs):
            msgs.append(
                f"ipmap exact check: {ip} has unexpected handles {sorted(actual_tcs - allowed_tcs)} (allowed={sorted(allowed_tcs)})"
            )
            ok = False
        if forbidden and actual_tcs.intersection(forbidden):
            msgs.append(
                f"ipmap exact check: {ip} still mapped to forbidden handles {sorted(actual_tcs.intersection(forbidden))}"
            )
            ok = False
        if ok:
            msgs.append(f"ipmap exact check: {ip} only mapped to expected handles {sorted(actual_tcs)}")

    return ok, msgs


def check_exact_ip_mappings_for_circuits(
    circuit_ids: List[str],
    forbidden_by_circuit: Optional[Dict[str, Set[str]]] = None,
) -> Tuple[bool, List[str]]:
    ok_tool, msg, entries = _list_ip_mappings()
    msgs: List[str] = []
    if not ok_tool:
        return True, [msg]  # soft-skip

    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["ipmap batch exact check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["ipmap batch exact check: queuingStructure has no 'Network'"]

    ok = True
    expected_ip_count = 0
    for circuit_id in circuit_ids:
        found = _walk_nodes_for_circuit(net, circuit_id)
        if not found:
            msgs.append(f"ipmap batch exact check: circuit {circuit_id} not found in structure")
            ok = False
            continue
        parent_name, circuit = found
        parent_node = _find_parent_node(net, parent_name)
        if not isinstance(parent_node, dict):
            msgs.append(f"ipmap batch exact check: parent node '{parent_name}' not found for circuit {circuit_id}")
            ok = False
            continue

        down_tc, up_tc, _cpu = _expect_handles_for_circuit(parent_node, circuit)
        allowed_tcs = {down_tc.lower()}
        if up_tc:
            allowed_tcs.add(up_tc.lower())
        forbidden = {tc.lower() for tc in (forbidden_by_circuit or {}).get(circuit_id, set())}

        ips = _expected_ips_for_circuit(circuit)
        if not ips:
            msgs.append(f"ipmap batch exact check: circuit {circuit_id} has no IPv4s; skipping")
            continue
        expected_ip_count += len(ips)

        for ip in ips:
            ip_entries = [(cpu, tc.lower()) for (mapped_ip, cpu, tc) in entries if mapped_ip == ip]
            if not ip_entries:
                msgs.append(f"ipmap batch exact check: missing mapping for {ip} (circuit {circuit_id})")
                ok = False
                continue
            actual_tcs = {tc for (_cpu, tc) in ip_entries}
            if not actual_tcs.issubset(allowed_tcs):
                msgs.append(
                    f"ipmap batch exact check: {ip} (circuit {circuit_id}) has unexpected handles {sorted(actual_tcs - allowed_tcs)} (allowed={sorted(allowed_tcs)})"
                )
                ok = False
            if forbidden and actual_tcs.intersection(forbidden):
                msgs.append(
                    f"ipmap batch exact check: {ip} (circuit {circuit_id}) still mapped to forbidden handles {sorted(actual_tcs.intersection(forbidden))}"
                )
                ok = False
            if actual_tcs.issubset(allowed_tcs) and not (forbidden and actual_tcs.intersection(forbidden)):
                msgs.append(f"ipmap batch exact check: {ip} (circuit {circuit_id}) only mapped to expected handles {sorted(actual_tcs)}")

    if ok:
        msgs.append(
            f"ipmap batch exact check: verified {expected_ip_count} expected IPv4 mappings across {len(circuit_ids)} circuits"
        )
    return ok, msgs


def check_generated_pn_tc_bandwidths() -> Tuple[bool, List[str]]:
    msgs: List[str] = []
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["PN tc check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["PN tc check: queuingStructure has no 'Network'"]
    # Resolve PN config
    if not generated_pn_download_mbps or not generated_pn_upload_mbps:
        return False, ["PN tc check: PN defaults not available via bindings"]
    try:
        expect_dl = float(generated_pn_download_mbps())
        expect_ul = float(generated_pn_upload_mbps())
    except Exception as e:
        return False, [f"PN tc check: failed reading PN defaults: {e}"]
    # Enumerate PN nodes
    pn_nodes = _find_pn_nodes(net)
    if not pn_nodes:
        return False, ["PN tc check: no Generated_PN_* nodes found in structure"]
    # Interfaces
    if not interface_a or not callable(interface_a):
        return False, ["PN tc check: interface_a() binding unavailable"]
    try:
        ifa = interface_a()
    except Exception as e:
        return False, [f"PN tc check: failed to read interface_a(): {e}"]
    ifb = None
    if interface_b and callable(interface_b):
        try:
            ifb = interface_b()
        except Exception:
            ifb = None
    # Check each PN on downlink side (interface_a)
    ok_all = True
    for name, node in pn_nodes:
        maj_hex = node.get("classMajor")
        min_hex = node.get("classMinor")
        # Fallback to parse from classid if needed
        if (maj_hex is None or min_hex is None) and isinstance(node.get("classid"), str):
            try:
                maj_s, min_s = str(node["classid"]).split(":", 1)
                maj_hex = maj_hex or maj_s
                min_hex = min_hex or min_s
            except Exception:
                pass
        maj = _hex_to_int(maj_hex) if isinstance(maj_hex, str) else _hex_to_int(str(maj_hex))
        mnr = _hex_to_int(min_hex) if isinstance(min_hex, str) else _hex_to_int(str(min_hex))
        if maj is None or mnr is None:
            msgs.append(f"PN tc check: {name} missing classMajor/classMinor")
            ok_all = False
            continue
        res = _read_htb_rate_ceil_mbps(ifa, maj, mnr)
        if not res:
            msgs.append(f"PN tc check: {name} class {maj}:{mnr} not found on {ifa}")
            ok_all = False
            continue
        rate_mbps, ceil_mbps, line = res
        msgs.append(f"PN {name} {ifa} class {maj}:{mnr} rate={rate_mbps:.1f} ceil={ceil_mbps:.1f} (expected {expect_dl:.1f}) | {line.strip()}")
        # Allow small tolerance due to rounding
        if abs(rate_mbps - expect_dl) > 1.0 or abs(ceil_mbps - expect_dl) > 1.0:
            ok_all = False
    # Check uplink side if available
    if ifb:
        for name, node in pn_nodes:
            up_maj_hex = node.get("up_classMajor")
            min_hex = node.get("classMinor")
            if (up_maj_hex is None or min_hex is None) and isinstance(node.get("up_classid"), str):
                try:
                    upm_s, mins = str(node["up_classid"]).split(":", 1)
                    up_maj_hex = up_maj_hex or upm_s
                    min_hex = min_hex or mins
                except Exception:
                    pass
            upmaj = _hex_to_int(up_maj_hex) if isinstance(up_maj_hex, str) else _hex_to_int(str(up_maj_hex))
            mnr = _hex_to_int(min_hex) if isinstance(min_hex, str) else _hex_to_int(str(min_hex))
            if upmaj is None or mnr is None:
                msgs.append(f"PN tc check: {name} missing up_classMajor/classMinor")
                ok_all = False
                continue
            res = _read_htb_rate_ceil_mbps(ifb, upmaj, mnr)
            if not res:
                msgs.append(f"PN tc check: {name} class {upmaj}:{mnr} not found on {ifb}")
                ok_all = False
                continue
            rate_mbps, ceil_mbps, line = res
            msgs.append(f"PN {name} {ifb} class {upmaj}:{mnr} rate={rate_mbps:.1f} ceil={ceil_mbps:.1f} (expected {expect_ul:.1f}) | {line.strip()}")
            if abs(rate_mbps - expect_ul) > 1.0 or abs(ceil_mbps - expect_ul) > 1.0:
                ok_all = False
    return ok_all, msgs


def _class_has_parent(iface: str, maj: int, mnr: int, pmaj: int, pmnr: int) -> Tuple[bool, Optional[str]]:
    """Robustly verify that class maj:mnr has parent pmaj:pmnr on iface.

    Accepts both decimal and 0x-prefixed hex forms as printed by tc.
    Returns (True, line) when parent matches, else (False, line or None).
    """
    rc, out, err = _tc(["class", "show", "dev", iface])
    if rc != 0:
        return False, f"tc error: {err.strip()}"
    # Find the specific class line first (accept decimal or hex handles)
    cls_pat = re.compile(r"^class\s+htb\s+([0-9A-Fa-fx]+):([0-9A-Fa-fx]+)\b.*$", re.MULTILINE)

    def _to_int(tok: str) -> Optional[int]:
        s = tok.strip()
        # Accept decimal, 0x-prefixed hex, or bare hex (e.g. "f")
        try:
            if s.lower().startswith("0x"):
                return int(s, 16)
            try:
                return int(s)  # decimal
            except Exception:
                return int(s, 16)  # bare hex
        except Exception:
            return None

    line = None
    for m in cls_pat.finditer(out):
        mj_tok, mn_tok = m.group(1), m.group(2)
        mj = _to_int(mj_tok)
        mn = _to_int(mn_tok)
        if mj == maj and mn == mnr:
            line = m.group(0)
            break
    if line is None:
        return False, None
    # Extract parent handle (supports hex or decimal)
    p = re.search(r"parent\s+([0-9A-Fa-fx]+):([0-9A-Fa-fx]+)\b", line)
    if not p:
        return False, line
    pmj = _to_int(p.group(1))
    pmn = _to_int(p.group(2))
    if pmj is None or pmn is None:
        return False, line
    return (pmj == pmaj and pmn == pmnr), line


def _tc_class_occurrences(iface: str, maj: int, mnr: int) -> Tuple[int, List[str]]:
    rc, out, err = _tc(["class", "show", "dev", iface])
    if rc != 0:
        return 0, [f"tc class occurrences: 'tc class show dev {iface}' failed: {err.strip()}"]

    matches: List[str] = []
    cls_pat = re.compile(r"^class\s+htb\s+([0-9A-Fa-fx]+):([0-9A-Fa-fx]+)\b.*$", re.MULTILINE)
    for found in cls_pat.finditer(out):
        mj = _tc_id_token_to_int(found.group(1))
        mn = _tc_id_token_to_int(found.group(2))
        if mj == maj and mn == mnr:
            matches.append(found.group(0))
    return len(matches), matches


def _tc_qdisc_occurrences_for_parent(iface: str, maj: int, mnr: int) -> Tuple[int, List[str]]:
    rc, out, err = _qdisc_show(iface)
    if rc != 0:
        return 0, [f"tc qdisc occurrences: 'tc qdisc show dev {iface}' failed: {err.strip()}"]

    matches: List[str] = []
    pat = re.compile(r"^qdisc\s+\S+\s+\S+:\s+parent\s+([0-9A-Fa-fx]+):([0-9A-Fa-fx]+)\b.*$", re.MULTILINE)
    for found in pat.finditer(out):
        mj = _tc_id_token_to_int(found.group(1))
        mn = _tc_id_token_to_int(found.group(2))
        if mj == maj and mn == mnr:
            matches.append(found.group(0))
    return len(matches), matches


def _handle_to_parts(handle: Optional[str]) -> Optional[Tuple[int, int]]:
    if not handle or ":" not in handle:
        return None
    left, right = handle.split(":", 1)
    major = _tc_id_token_to_int(left)
    minor = _tc_id_token_to_int(right)
    if major is None or minor is None:
        return None
    return major, minor


def _current_circuit_handle_strings(circuit_id: str) -> Tuple[Optional[str], Optional[str], List[str]]:
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return None, None, ["cleanup check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return None, None, ["cleanup check: queuingStructure has no 'Network'"]
    found = _walk_nodes_for_circuit(net, circuit_id)
    if not found:
        return None, None, [f"cleanup check: circuit {circuit_id} not found in structure"]
    _parent_name, circuit = found
    down_tc = str(circuit.get("classid", "")).replace("0x", "").lower()
    up_tc = str(circuit.get("up_classid", "")).replace("0x", "").lower()
    return down_tc or None, up_tc or None, []


def check_circuit_transition_cleanup(
    circuit_id: str,
    *,
    old_down_tc: Optional[str],
    old_up_tc: Optional[str],
) -> Tuple[bool, List[str]]:
    msgs: List[str] = []
    ok = True

    new_down_tc, new_up_tc, handle_msgs = _current_circuit_handle_strings(circuit_id)
    msgs.extend(handle_msgs)
    if handle_msgs:
        return False, msgs

    if not interface_a or not callable(interface_a):
        return False, ["cleanup check: interface_a() binding unavailable"]
    try:
        ifa = interface_a()
    except Exception as e:
        return False, [f"cleanup check: failed to get interface_a(): {e}"]
    ifb = None
    if interface_b and callable(interface_b):
        try:
            ifb = interface_b()
        except Exception:
            ifb = None

    if new_down_tc:
        down_parts = _handle_to_parts(new_down_tc)
        if down_parts is None:
            msgs.append(f"cleanup check: could not parse current down handle {new_down_tc} for circuit {circuit_id}")
            ok = False
        else:
            count, lines = _tc_class_occurrences(ifa, *down_parts)
            if count != 1:
                msgs.append(
                    f"cleanup check: expected exactly one current down class {new_down_tc} for circuit {circuit_id} on {ifa}, found {count}"
                )
                msgs.extend(lines)
                ok = False
            else:
                msgs.append(f"cleanup check: current down class {new_down_tc} present exactly once on {ifa}")
    if new_up_tc and ifb:
        up_parts = _handle_to_parts(new_up_tc)
        if up_parts is None:
            msgs.append(f"cleanup check: could not parse current up handle {new_up_tc} for circuit {circuit_id}")
            ok = False
        else:
            count, lines = _tc_class_occurrences(ifb, *up_parts)
            if count != 1:
                msgs.append(
                    f"cleanup check: expected exactly one current up class {new_up_tc} for circuit {circuit_id} on {ifb}, found {count}"
                )
                msgs.extend(lines)
                ok = False
            else:
                msgs.append(f"cleanup check: current up class {new_up_tc} present exactly once on {ifb}")

    if old_down_tc and old_down_tc.lower() != (new_down_tc or "").lower():
        old_down_parts = _handle_to_parts(old_down_tc)
        if old_down_parts is not None:
            count, lines = _tc_class_occurrences(ifa, *old_down_parts)
            if count != 0:
                msgs.append(
                    f"cleanup check: stale down class {old_down_tc} still present for circuit {circuit_id} on {ifa}"
                )
                msgs.extend(lines)
                ok = False
            else:
                msgs.append(f"cleanup check: stale down class {old_down_tc} absent on {ifa}")

            qcount, qlines = _tc_qdisc_occurrences_for_parent(ifa, *old_down_parts)
            if qcount != 0:
                msgs.append(
                    f"cleanup check: stale down qdisc parent {old_down_tc} still present for circuit {circuit_id} on {ifa}"
                )
                msgs.extend(qlines)
                ok = False
            else:
                msgs.append(f"cleanup check: stale down qdisc parent {old_down_tc} absent on {ifa}")

    if old_up_tc and ifb and old_up_tc.lower() != (new_up_tc or "").lower():
        old_up_parts = _handle_to_parts(old_up_tc)
        if old_up_parts is not None:
            count, lines = _tc_class_occurrences(ifb, *old_up_parts)
            if count != 0:
                msgs.append(
                    f"cleanup check: stale up class {old_up_tc} still present for circuit {circuit_id} on {ifb}"
                )
                msgs.extend(lines)
                ok = False
            else:
                msgs.append(f"cleanup check: stale up class {old_up_tc} absent on {ifb}")

            qcount, qlines = _tc_qdisc_occurrences_for_parent(ifb, *old_up_parts)
            if qcount != 0:
                msgs.append(
                    f"cleanup check: stale up qdisc parent {old_up_tc} still present for circuit {circuit_id} on {ifb}"
                )
                msgs.extend(qlines)
                ok = False
            else:
                msgs.append(f"cleanup check: stale up qdisc parent {old_up_tc} absent on {ifb}")

    forbidden = {tc for tc in [old_down_tc, old_up_tc] if tc}
    passed_ip_exact, ip_msgs = check_exact_ip_mappings_for_circuit(circuit_id, forbidden_tcs=forbidden)
    msgs.extend(ip_msgs)
    ok &= passed_ip_exact

    return ok, msgs


def check_orphan_tc_attachment(orphan_id: str = "99901") -> Tuple[bool, List[str]]:
    msgs: List[str] = []
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["orphan tc check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["orphan tc check: missing 'Network'"]
    found = _walk_nodes_for_circuit(net, orphan_id)
    if not found:
        return False, ["orphan tc check: orphan circuit not found in structure"]
    parent_name, circuit = found
    # Locate PN node object
    def _find_node(node_map: Dict[str, dict], name: str) -> Optional[dict]:
        for k, v in node_map.items():
            if k == name and isinstance(v, dict):
                return v
            ch = v.get("children") if isinstance(v, dict) else None
            if isinstance(ch, dict):
                got = _find_node(ch, name)
                if got is not None:
                    return got
        return None
    pn_node = _find_node(net, parent_name)
    if not isinstance(pn_node, dict):
        return False, [f"orphan tc check: PN node '{parent_name}' not found"]
    # Downlink
    if not interface_a or not callable(interface_a):
        return False, ["orphan tc check: interface_a() unavailable"]
    try:
        ifa = interface_a()
    except Exception as e:
        return False, [f"orphan tc check: failed to get interface_a(): {e}"]
    maj_hex = circuit.get("classMajor")
    cmin_hex = circuit.get("classMinor")
    pmaj_hex = pn_node.get("classMajor")
    pmin_hex = pn_node.get("classMinor")
    maj = _hex_to_int(maj_hex if isinstance(maj_hex, str) else str(maj_hex))
    cmin = _hex_to_int(cmin_hex if isinstance(cmin_hex, str) else str(cmin_hex))
    pmaj = _hex_to_int(pmaj_hex if isinstance(pmaj_hex, str) else str(pmaj_hex))
    pmin = _hex_to_int(pmin_hex if isinstance(pmin_hex, str) else str(pmin_hex))
    if None in (maj, cmin, pmaj, pmin):
        return False, ["orphan tc check: missing classMajor/classMinor for PN or circuit (downlink)"]
    ok_dl, line_dl = _class_has_parent(ifa, maj, cmin, pmaj, pmin)
    if not ok_dl:
        # Diagnostic hook: show the actual class line if present
        diag = _read_htb_rate_ceil_mbps(ifa, maj, cmin)
        if diag:
            _, _, raw = diag
            msgs.append(
                f"orphan tc check: downlink class {maj}:{cmin} not attached to parent {pmaj}:{pmin} on {ifa} | {raw}"
            )
        else:
            msgs.append(
                f"orphan tc check: downlink class {maj}:{cmin} not attached to parent {pmaj}:{pmin} on {ifa} (class not found)"
            )
        return False, msgs
    msgs.append(f"orphan tc check: {ifa} class {maj}:{cmin} parent {pmaj}:{pmin} | {line_dl}")
    # Uplink (optional)
    if interface_b and callable(interface_b):
        try:
            ifb = interface_b()
            upm_hex = circuit.get("up_classMajor")
            upmaj = _hex_to_int(upm_hex if isinstance(upm_hex, str) else str(upm_hex))
            # Circuit minor is same
            up_pmaj_hex = pn_node.get("up_classMajor")
            up_pmaj = _hex_to_int(up_pmaj_hex if isinstance(up_pmaj_hex, str) else str(up_pmaj_hex))
            if None not in (upmaj, cmin, up_pmaj, pmin):
                ok_ul, line_ul = _class_has_parent(ifb, upmaj, cmin, up_pmaj, pmin)
                if ok_ul:
                    msgs.append(f"orphan tc check: {ifb} class {upmaj}:{cmin} parent {up_pmaj}:{pmin} | {line_ul}")
                else:
                    # Diagnostic hook: show the actual class line if present
                    diag_ul = _read_htb_rate_ceil_mbps(ifb, upmaj, cmin)
                    if diag_ul:
                        _, _, raw_ul = diag_ul
                        msgs.append(
                            f"orphan tc check: uplink class {upmaj}:{cmin} not attached to parent {up_pmaj}:{pmin} on {ifb} | {raw_ul}"
                        )
                    else:
                        msgs.append(
                            f"orphan tc check: uplink class {upmaj}:{cmin} not attached to parent {up_pmaj}:{pmin} on {ifb} (class not found)"
                        )
            else:
                msgs.append("orphan tc check: uplink IDs missing; skipping uplink parent check")
        except Exception:
            msgs.append("orphan tc check: interface_b() failed; skipping uplink parent check")
    return True, msgs


def check_site_direction_ceil(node_name: str, expected_dl: Optional[float] = None, expected_ul: Optional[float] = None) -> Tuple[bool, List[str]]:
    """Verify that a site's HTB ceilings match its configured asymmetric bandwidths.

    - interface_a (downlink): ceil == downloadBandwidthMbps
    - interface_b (uplink): ceil == uploadBandwidthMbps
    """
    msgs: List[str] = []
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["site direction check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["site direction check: queuingStructure has no 'Network'"]
    node = _find_parent_node(net, node_name)
    if not isinstance(node, dict):
        return False, [f"site direction check: node '{node_name}' not found in structure"]

    # Determine expected rates
    try:
        node_dl = float(node.get("downloadBandwidthMbps", -1))
        node_ul = float(node.get("uploadBandwidthMbps", -1))
    except Exception:
        return False, [f"site direction check: node '{node_name}' missing downloadBandwidthMbps/uploadBandwidthMbps"]
    if expected_dl is None:
        expected_dl = node_dl
    if expected_ul is None:
        expected_ul = node_ul

    ok = True
    if abs(node_dl - expected_dl) > 0.1 or abs(node_ul - expected_ul) > 0.1:
        msgs.append(
            f"site direction check: node '{node_name}' bandwidths {node_dl}/{node_ul} Mbps do not match expected {expected_dl}/{expected_ul} Mbps"
        )
        ok = False

    # Interfaces
    if not interface_a or not callable(interface_a):
        return False, ["site direction check: interface_a() binding unavailable"]
    try:
        ifa = interface_a()
    except Exception as e:
        return False, [f"site direction check: failed to get interface_a(): {e}"]
    ifb = None
    if interface_b and callable(interface_b):
        try:
            ifb = interface_b()
        except Exception:
            ifb = None

    # IDs
    maj_hex = node.get("classMajor")
    mnr_hex = node.get("classMinor")
    upm_hex = node.get("up_classMajor")
    maj = _hex_to_int(maj_hex if isinstance(maj_hex, str) else str(maj_hex))
    mnr = _hex_to_int(mnr_hex if isinstance(mnr_hex, str) else str(mnr_hex))
    upm = _hex_to_int(upm_hex if isinstance(upm_hex, str) else str(upm_hex))
    if None in (maj, mnr):
        return False, [f"site direction check: node '{node_name}' missing down IDs"]

    # Downlink
    res_dl = _read_htb_rate_ceil_mbps(ifa, maj, mnr)
    if not res_dl:
        msgs.append(f"site direction check: {ifa} class {maj}:{mnr} not found for node '{node_name}'")
        ok = False
    else:
        rate_mbps, ceil_mbps, line = res_dl
        msgs.append(
            f"site direction check: {node_name} down {ifa} ceil={ceil_mbps:.1f} expect={expected_dl:.1f} | {line}"
        )
        if abs(ceil_mbps - expected_dl) > 0.6:
            ok = False

    # Uplink
    if ifb and upm is not None:
        res_ul = _read_htb_rate_ceil_mbps(ifb, upm, mnr)
        if not res_ul:
            msgs.append(f"site direction check: {ifb} class {upm}:{mnr} not found for node '{node_name}'")
            ok = False
        else:
            rate_mbps, ceil_mbps, line = res_ul
            msgs.append(
                f"site direction check: {node_name} up {ifb} ceil={ceil_mbps:.1f} expect={expected_ul:.1f} | {line}"
            )
            if abs(ceil_mbps - expected_ul) > 0.6:
                ok = False

    return ok, msgs


def check_circuit_direction_ceil(circuit_id: str) -> Tuple[bool, List[str]]:
    """Verify that the circuit's ceil values are applied to the correct interfaces:
    - interface_a (downlink): ceil == maxDownload
    - interface_b (uplink): ceil == maxUpload
    Tolerance of 0.6 Mbps to account for rounding.
    """
    msgs: List[str] = []
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["direction check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["direction check: queuingStructure has no 'Network'"]
    found = _walk_nodes_for_circuit(net, circuit_id)
    if not found:
        return False, [f"direction check: circuit {circuit_id} not found in structure"]
    parent_name, circuit = found
    # Interfaces
    if not interface_a or not callable(interface_a):
        return False, ["direction check: interface_a() binding unavailable"]
    try:
        ifa = interface_a()
    except Exception as e:
        return False, [f"direction check: failed to get interface_a(): {e}"]
    ifb = None
    if interface_b and callable(interface_b):
        try:
            ifb = interface_b()
        except Exception:
            ifb = None
    # IDs
    maj_hex = circuit.get("classMajor")
    mnr_hex = circuit.get("classMinor")
    upm_hex = circuit.get("up_classMajor")
    maj = _hex_to_int(maj_hex if isinstance(maj_hex, str) else str(maj_hex))
    mnr = _hex_to_int(mnr_hex if isinstance(mnr_hex, str) else str(mnr_hex))
    upm = _hex_to_int(upm_hex if isinstance(upm_hex, str) else str(upm_hex))
    if None in (maj, mnr):
        return False, [f"direction check: circuit {circuit_id} missing down IDs"]
    # Expectations
    try:
        max_dl = float(circuit.get("maxDownload", -1))
        max_ul = float(circuit.get("maxUpload", -1))
    except Exception:
        return False, ["direction check: circuit missing maxDownload/maxUpload"]
    # Read downlink
    ok = True
    res_dl = _read_htb_rate_ceil_mbps(ifa, maj, mnr)
    if not res_dl:
        msgs.append(f"direction check: {ifa} class {maj}:{mnr} not found for circuit {circuit_id}")
        ok = False
    else:
        rate_mbps, ceil_mbps, line = res_dl
        msgs.append(
            f"direction check: {circuit_id} down {ifa} ceil={ceil_mbps:.1f} expect={max_dl:.1f} | {line}"
        )
        if abs(ceil_mbps - max_dl) > 0.6:
            ok = False
    # Read uplink
    if ifb and upm is not None:
        res_ul = _read_htb_rate_ceil_mbps(ifb, upm, mnr)
        if not res_ul:
            msgs.append(f"direction check: {ifb} class {upm}:{mnr} not found for circuit {circuit_id}")
            ok = False
        else:
            rate_mbps, ceil_mbps, line = res_ul
            msgs.append(
                f"direction check: {circuit_id} up {ifb} ceil={ceil_mbps:.1f} expect={max_ul:.1f} | {line}"
            )
            if abs(ceil_mbps - max_ul) > 0.6:
                ok = False
    return ok, msgs


def check_circuit_qdisc_kind(
    circuit_id: str,
    expected_down_kind: Optional[str] = None,
    expected_up_kind: Optional[str] = None,
) -> Tuple[bool, List[str]]:
    msgs: List[str] = []
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["qdisc kind check: queuingStructure.json not found"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["qdisc kind check: queuingStructure has no 'Network'"]
    found = _walk_nodes_for_circuit(net, circuit_id)
    if not found:
        return False, [f"qdisc kind check: circuit {circuit_id} not found in structure"]
    _parent_name, circuit = found

    maj = _hex_to_int(str(circuit.get("classMajor")))
    mnr = _hex_to_int(str(circuit.get("classMinor")))
    upm = _hex_to_int(str(circuit.get("up_classMajor")))
    if maj is None or mnr is None:
        return False, [f"qdisc kind check: circuit {circuit_id} missing downlink class IDs"]

    def _check_iface(iface: str, parent_major: int, parent_minor: int, expected_kind: Optional[str], label: str) -> Tuple[bool, str]:
        rc, out, err = _qdisc_show(iface)
        if rc != 0:
            return False, f"qdisc kind check: 'tc qdisc show dev {iface}' failed: {err.strip()}"

        pat = re.compile(r"^qdisc\s+(\S+)\s+\S+:\s+parent\s+([0-9A-Fa-fx]+):([0-9A-Fa-fx]+)\b.*$", re.MULTILINE)
        for match in pat.finditer(out):
            kind = match.group(1).lower()
            mj = _tc_id_token_to_int(match.group(2))
            mn = _tc_id_token_to_int(match.group(3))
            if mj == parent_major and mn == parent_minor:
                if expected_kind is not None and kind != expected_kind.lower():
                    return False, f"qdisc kind check: {label} {iface} expected {expected_kind}, found {kind} for circuit {circuit_id} | {match.group(0)}"
                return True, f"qdisc kind check: {label} {iface} found {kind} for circuit {circuit_id} | {match.group(0)}"

        return False, f"qdisc kind check: {label} {iface} no qdisc found for parent {parent_major}:{parent_minor} (circuit {circuit_id})"

    ok = True
    if expected_down_kind is not None:
        if not interface_a or not callable(interface_a):
            return False, ["qdisc kind check: interface_a() binding unavailable"]
        try:
            ifa = interface_a()
        except Exception as e:
            return False, [f"qdisc kind check: failed to get interface_a(): {e}"]
        passed, message = _check_iface(ifa, maj, mnr, expected_down_kind, "down")
        msgs.append(message)
        ok &= passed

    if expected_up_kind is not None and interface_b and callable(interface_b) and upm is not None:
        try:
            ifb = interface_b()
        except Exception as e:
            return False, [f"qdisc kind check: failed to get interface_b(): {e}"]
        passed, message = _check_iface(ifb, upm, mnr, expected_up_kind, "up")
        msgs.append(message)
        ok &= passed

    return ok, msgs


def _walk_nodes_for_circuit(node_map: Dict[str, dict], circuit_id: str) -> Optional[Tuple[str, dict]]:
    for name, node in node_map.items():
        if not isinstance(node, dict):
            continue
        if "circuits" in node and isinstance(node["circuits"], list):
            for c in node["circuits"]:
                try:
                    if str(c.get("circuitID", "")) == str(circuit_id):
                        return name, c
                except Exception:
                    pass
        ch = node.get("children") if isinstance(node, dict) else None
        if isinstance(ch, dict):
            found = _walk_nodes_for_circuit(ch, circuit_id)
            if found:
                return found
    return None


def test_orphan_circuit_assignment(csv_rows: List[Dict[str, object]]) -> Tuple[bool, List[str]]:
    """Append an orphan circuit (ParentNode=none) with max speeds below PN defaults,
    refresh, and assert it landed under a Generated_PN_* with its original max speeds.
    """
    msgs: List[str] = []
    # Determine PN defaults; require the bindings
    if not generated_pn_download_mbps or not generated_pn_upload_mbps:  # type: ignore[name-defined]
        return False, ["PN default speeds unavailable via bindings"]
    try:
        pn_dl = int(generated_pn_download_mbps())  # type: ignore[misc]
        pn_ul = int(generated_pn_upload_mbps())    # type: ignore[misc]
    except Exception as e:
        return False, [f"Failed to read PN defaults: {e}"]
    # Choose asymmetric orphan speeds strictly below PN defaults to verify direction mapping clearly.
    # Target 50/20 Mbps, but cap to PN defaults minus 1 to stay valid on smaller configs.
    orphan_dl = max(1, min(50, pn_dl - 1))
    orphan_ul = max(1, min(20, pn_ul - 1))
    orphan_id = "99901"
    orphan_row: Dict[str, object] = {
        "Circuit ID": orphan_id,
        "Circuit Name": "ORPHAN_TEST",
        "Device ID": "99901",
        "Device Name": "OD1",
        "Parent Node": "none",
        "MAC": "",
        "IPv4": "100.64.250.1",
        "IPv6": "",
        "Download Min Mbps": 1,
        "Upload Min Mbps": 1,
        "Download Max Mbps": orphan_dl,
        "Upload Max Mbps": orphan_ul,
        "Comment": "",
        "sqm": "",
    }
    rows2 = list(csv_rows) + [orphan_row]
    write_circuits(rows2)
    # Trigger refresh
    LibreQoS.refreshShapers()
    time.sleep(0.5)
    # Load structure
    qs = _load_queuing_structure()
    if not isinstance(qs, dict):
        return False, ["queuingStructure.json not found after orphan test"]
    net = qs.get("Network")
    if not isinstance(net, dict):
        return False, ["queuingStructure.json missing 'Network' after orphan test"]
    found = _walk_nodes_for_circuit(net, orphan_id)
    if not found:
        return False, ["Orphan circuit not found in queuing structure"]
    parent_name, circuit = found
    if not str(parent_name).startswith("Generated_PN_"):
        return False, [f"Orphan circuit parent is not a Generated_PN_*: {parent_name}"]
    try:
        md = float(circuit.get("maxDownload", -1))
        mu = float(circuit.get("maxUpload", -1))
    except Exception:
        return False, ["Orphan circuit missing maxDownload/maxUpload in structure"]
    if int(md) != orphan_dl or int(mu) != orphan_ul:
        return False, [f"Orphan circuit speeds mismatch (got {md}/{mu}, expected {orphan_dl}/{orphan_ul})"]
    msgs.append(f"orphan circuit assigned to {parent_name} with speeds {orphan_dl}/{orphan_ul} Mbps")
    return True, msgs


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


def _journal_since_lines(since_ts: float) -> List[str]:
    proc = subprocess.run(
        [
            "journalctl",
            "-u",
            "lqosd",
            "--since",
            f"@{since_ts:.6f}",
            "--no-pager",
            "-o",
            "short-unix",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        return []
    return [line.rstrip("\n") for line in proc.stdout.splitlines() if line.strip()]


def check_no_hidden_incremental_failures(step_started_at: float) -> Tuple[bool, List[str]]:
    lines = _journal_since_lines(step_started_at)
    patterns = [
        re.compile(r"reload required", re.IGNORECASE),
        re.compile(r"\bRTNETLINK\b", re.IGNORECASE),
        re.compile(r"\bDirty\b", re.IGNORECASE),
        re.compile(r"Command error for \(", re.IGNORECASE),
        re.compile(r"Bakery wrote numbered command failure", re.IGNORECASE),
        re.compile(r"full reload is now required", re.IGNORECASE),
    ]

    matches = [line for line in lines if any(p.search(line) for p in patterns)]

    last_error_path = "/tmp/lqos_bakery_last_error.txt"
    if os.path.exists(last_error_path):
        try:
            if os.path.getmtime(last_error_path) >= step_started_at:
                matches.append(f"last error file updated: {last_error_path}")
        except Exception:
            pass

    if matches:
        return False, [
            "incremental health: unexpected error/desync indicators detected after step",
            *matches[-20:],
        ]

    return True, ["incremental health: no hidden Bakery desync/error indicators detected after step"]


def check_fault_injection_observed(step_started_at: float) -> Tuple[bool, List[str]]:
    lines = _journal_since_lines(step_started_at)
    fault_patterns = [
        re.compile(r"synthetic Bakery test fault", re.IGNORECASE),
        re.compile(r"Bakery test fault injected", re.IGNORECASE),
    ]

    fault_matches = [line for line in lines if any(p.search(line) for p in fault_patterns)]
    msgs: List[str] = []

    if os.path.exists(TEST_FAULT_ONCE_PATH):
        msgs.append(f"fault injection file still present and was not consumed: {TEST_FAULT_ONCE_PATH}")
        return False, msgs

    if not fault_matches:
        return False, [
            "fault reload check: synthetic Bakery fault was not observed in recent logs",
            *lines[-20:],
        ]

    msgs.append("fault reload check: Bakery observed and consumed the injected synthetic fault")
    msgs.extend(fault_matches[-5:])
    return True, msgs


def check_reload_required_since(step_started_at: float) -> Tuple[bool, List[str]]:
    lines = _journal_since_lines(step_started_at)
    reload_patterns = [
        re.compile(r"reload required", re.IGNORECASE),
        re.compile(r"full reload is now required", re.IGNORECASE),
        re.compile(r"full reload required before further incremental topology mutation", re.IGNORECASE),
    ]

    reload_matches = [line for line in lines if any(p.search(line) for p in reload_patterns)]
    if not reload_matches:
        return False, [
            "fault reload check: Bakery did not report reload-required state after injected fault",
            *lines[-20:],
        ]

    msgs: List[str] = []
    msgs.append("fault reload check: Bakery entered reload-required state after the injected fault")
    msgs.extend(reload_matches[-5:])
    return True, msgs


def collect_failure_diagnostics(
    tag: str,
    *,
    step_started_at: Optional[float] = None,
    circuit_ids: Optional[List[str]] = None,
    node_names: Optional[List[str]] = None,
) -> List[str]:
    circuit_ids = circuit_ids or []
    node_names = node_names or []
    msgs = [f"diagnostics: begin for {tag}"]

    if step_started_at is not None:
        journal_lines = _journal_since_lines(step_started_at)
        msgs.append("diagnostics: recent lqosd journal")
        msgs.extend(journal_lines[-20:] if journal_lines else ["diagnostics: no recent journal lines found"])

    planner = None
    try:
        with open("planner_state.json", "r") as f:
            planner = json.load(f)
    except Exception as e:
        msgs.append(f"diagnostics: failed to read planner_state.json: {e}")

    qs = _load_queuing_structure()
    net = qs.get("Network") if isinstance(qs, dict) else None

    if isinstance(planner, dict):
        for circuit_id in circuit_ids:
            entry = planner.get("circuits", {}).get(str(circuit_id))
            msgs.append(f"diagnostics: planner circuit {circuit_id} -> {json.dumps(entry, sort_keys=True)}")

    if isinstance(net, dict):
        for circuit_id in circuit_ids:
            found = _walk_nodes_for_circuit(net, circuit_id)
            if found:
                parent_name, circuit = found
                msgs.append(
                    f"diagnostics: structure circuit {circuit_id} under {parent_name} -> {json.dumps(circuit, sort_keys=True)}"
                )
            else:
                msgs.append(f"diagnostics: structure circuit {circuit_id} not found")
        for node_name in node_names:
            node = _find_parent_node(net, node_name)
            msgs.append(
                f"diagnostics: structure node {node_name} -> {json.dumps(node, sort_keys=True) if node is not None else 'missing'}"
            )

    if interface_a and callable(interface_a):
        try:
            ifa = interface_a()
            rc, out, err = _tc(["class", "show", "dev", ifa])
            msgs.append(f"diagnostics: tc class show dev {ifa} rc={rc}")
            msgs.extend(out.splitlines()[-20:] if rc == 0 else [err.strip()])
            rc, out, err = _qdisc_show(ifa)
            msgs.append(f"diagnostics: tc qdisc show dev {ifa} rc={rc}")
            msgs.extend(out.splitlines()[-20:] if rc == 0 else [err.strip()])
        except Exception as e:
            msgs.append(f"diagnostics: failed tc dump for interface_a: {e}")

    if interface_b and callable(interface_b):
        try:
            ifb = interface_b()
            rc, out, err = _tc(["class", "show", "dev", ifb])
            msgs.append(f"diagnostics: tc class show dev {ifb} rc={rc}")
            msgs.extend(out.splitlines()[-20:] if rc == 0 else [err.strip()])
            rc, out, err = _qdisc_show(ifb)
            msgs.append(f"diagnostics: tc qdisc show dev {ifb} rc={rc}")
            msgs.extend(out.splitlines()[-20:] if rc == 0 else [err.strip()])
        except Exception as e:
            msgs.append(f"diagnostics: failed tc dump for interface_b: {e}")

    last_error_path = "/tmp/lqos_bakery_last_error.txt"
    if os.path.exists(last_error_path):
        try:
            with open(last_error_path, "r", errors="replace") as f:
                lines = f.read().splitlines()
            msgs.append(f"diagnostics: {last_error_path}")
            msgs.extend(lines[-20:] if lines else ["diagnostics: last error file empty"])
        except Exception as e:
            msgs.append(f"diagnostics: failed reading {last_error_path}: {e}")

    msgs.append(f"diagnostics: end for {tag}")
    return msgs


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


def run_queue_mode_suite(log: AnyLogReader, timeout_s: float, results: List[str]) -> bool:
    if os.geteuid() != 0:
        results.append("queue-mode: must run as root to edit /etc/lqos.conf")
        return False
    if not queue_mode or not callable(queue_mode):  # type: ignore[name-defined]
        results.append("queue-mode: queue_mode() binding unavailable")
        return False
    if not sync_lqosd_config_from_disk or not callable(sync_lqosd_config_from_disk):  # type: ignore[name-defined]
        results.append("queue-mode: sync_lqosd_config_from_disk() binding unavailable")
        return False
    if not interface_a or not callable(interface_a):  # type: ignore[name-defined]
        results.append("queue-mode: interface_a() binding unavailable")
        return False

    iface_names: List[str] = []
    try:
        iface_names.append(interface_a())  # type: ignore[misc]
    except Exception as exc:
        results.append(f"queue-mode: failed to resolve interface_a(): {exc}")
        return False
    if interface_b and callable(interface_b):  # type: ignore[name-defined]
        try:
            iface_b_name = interface_b()  # type: ignore[misc]
            if iface_b_name and iface_b_name not in iface_names:
                iface_names.append(iface_b_name)
        except Exception as exc:
            results.append(f"queue-mode: failed to resolve interface_b(): {exc}")
            return False

    config_path = "/etc/lqos.conf"
    original_config_text = _read_text(config_path)
    try:
        original_mode = queue_mode()  # type: ignore[misc]
    except Exception as exc:
        results.append(f"queue-mode: failed to read current queue_mode(): {exc}")
        return False

    last_applied_mode = original_mode

    def _set_mode_and_refresh(mode: str, step_name: str) -> Tuple[bool, LogResult]:
        nonlocal last_applied_mode
        _mark_step(step_name)
        _write_text(config_path, _set_queue_mode_in_lqos_conf_text(_read_text(config_path), mode))
        sync_lqosd_config_from_disk()  # type: ignore[misc]
        step_started_at = time.time()
        res = run_refresh_subprocess_and_wait(log, timeout_s)
        last_applied_mode = mode
        results.append(
            f"{step_name}: bakery outcome full_reload={res.full_reload} mq_init={res.mq_init} incremental={res.incremental_event or 'none'}"
        )
        if not res.raw_lines:
            results.append(f"{step_name}: no Bakery logs observed after refresh (started {step_started_at:.6f})")
            return False, res
        return True, res

    def _check_shape_state(tag: str, baseline: Optional[Dict[str, TcShapeSummary]] = None) -> Tuple[bool, Dict[str, TcShapeSummary]]:
        ok = True
        summaries: Dict[str, TcShapeSummary] = {}
        for iface in iface_names:
            summary = _summarize_iface_tc(iface)
            summaries[iface] = summary
            results.append(_shape_summary_message(tag, summary))
            if not summary.root_mq:
                results.append(f"{tag}: {iface}: expected root mq to remain present")
                ok = False
            if summary.root_htb_qdiscs <= 0:
                results.append(f"{tag}: {iface}: expected HTB qdiscs in shape mode")
                ok = False
            if summary.htb_classes <= 0:
                results.append(f"{tag}: {iface}: expected HTB classes in shape mode")
                ok = False
            if summary.child_leaf_qdiscs <= 0:
                results.append(f"{tag}: {iface}: expected leaf qdiscs in shape mode")
                ok = False
            if baseline is not None:
                prior = baseline.get(iface)
                if prior is None:
                    results.append(f"{tag}: {iface}: missing baseline summary for comparison")
                    ok = False
                elif summary != prior:
                    results.append(
                        f"{tag}: {iface}: note restored shape counts differ from baseline "
                        f"(baseline root_htb_qdiscs={prior.root_htb_qdiscs}, htb_classes={prior.htb_classes}, "
                        f"child_leaf_qdiscs={prior.child_leaf_qdiscs}); "
                        "shape rebuild appears to have normalized pre-existing stale TC state"
                    )
        return ok, summaries

    def _check_observe_state(tag: str) -> Tuple[bool, Dict[str, TcShapeSummary]]:
        ok = True
        summaries: Dict[str, TcShapeSummary] = {}
        for iface in iface_names:
            summary = _summarize_iface_tc(iface)
            summaries[iface] = summary
            results.append(_shape_summary_message(tag, summary))
            if not summary.root_mq:
                results.append(f"{tag}: {iface}: expected root mq to remain present")
                ok = False
            if summary.root_htb_qdiscs != 0:
                results.append(f"{tag}: {iface}: expected no root HTB qdiscs in observe mode")
                ok = False
            if summary.htb_classes != 0:
                results.append(f"{tag}: {iface}: expected no HTB classes in observe mode")
                ok = False
            if summary.child_leaf_qdiscs != 0:
                results.append(f"{tag}: {iface}: expected no child leaf qdiscs in observe mode")
                ok = False
        return ok, summaries

    overall_ok = True
    baseline_shape: Optional[Dict[str, TcShapeSummary]] = None

    try:
        mode_ok, _ = _set_mode_and_refresh("shape", "queue-mode: baseline shape sync")
        overall_ok &= mode_ok

        shape_ok, baseline_shape = _check_shape_state("queue-mode: baseline shape")
        overall_ok &= shape_ok

        mode_ok, _ = _set_mode_and_refresh("observe", "queue-mode: switch to observe")
        overall_ok &= mode_ok
        observe_ok, _ = _check_observe_state("queue-mode: observe state")
        overall_ok &= observe_ok

        mode_ok, _ = _set_mode_and_refresh("shape", "queue-mode: switch back to shape")
        overall_ok &= mode_ok
        restore_ok, _ = _check_shape_state("queue-mode: restored shape", baseline_shape)
        overall_ok &= restore_ok
    finally:
        current_config_text = _read_text(config_path)
        if current_config_text != original_config_text:
            _write_text(config_path, original_config_text)
        if last_applied_mode != original_mode:
            try:
                _mark_step("queue-mode: restore original runtime mode")
                sync_lqosd_config_from_disk()  # type: ignore[misc]
                _ = run_refresh_subprocess_and_wait(log, timeout_s)
                results.append(f"queue-mode: restored original runtime mode {original_mode}")
            except Exception as exc:
                results.append(f"queue-mode: failed to restore original runtime mode {original_mode}: {exc}")
                overall_ok = False
        else:
            results.append("queue-mode: restored /etc/lqos.conf to original text")

    return overall_ok


def run_tiered_suite(log: AnyLogReader, timeout_s: float, results: List[str]) -> bool:
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

    # Sanity: verify defaults, PN presence, and orphan assignment behavior on tiered setup
    _mark_step("sanity: default classes present on interface_a")
    passed, msg = check_default_classes_present_on_iface()
    results.append(msg)
    ok &= passed
    # Also validate interface_b when available
    passed_b, msg_b = check_default_classes_present_on_iface_b()
    results.append(msg_b)
    ok &= passed_b

    _mark_step("sanity: generated PN items present")
    passed2, msgs = assert_generated_pns_present()
    results.extend(msgs)
    ok &= passed2

    _mark_step("sanity: no site/circuit minor collisions")
    passed2b, msgs2b = check_no_site_circuit_minor_collisions()
    results.extend(msgs2b)
    ok &= passed2b

    _mark_step("sanity: orphan circuit assigned under Generated_PN with original speeds")
    passed3, msgs3 = test_orphan_circuit_assignment(rows)
    results.extend(msgs3)
    ok &= passed3
    # Mapping for orphan circuit
    _mark_step("sanity: orphan circuit ip mappings")
    passed3b, msgs3b = check_ip_mappings_for_circuit("99901")
    results.extend(msgs3b)
    ok &= passed3b

    _mark_step("sanity: PN HTB rates match PN defaults")
    passed4, msgs4 = check_generated_pn_tc_bandwidths()
    results.extend(msgs4)
    ok &= passed4
    _mark_step("sanity: no site/circuit minor collisions (post-orphan)")
    passed4b, msgs4b = check_no_site_circuit_minor_collisions()
    results.extend(msgs4b)
    ok &= passed4b
    _mark_step("sanity: orphan circuit HTB attached to PN parent in tc")
    passed5, msgs5 = check_orphan_tc_attachment("99901")
    results.extend(msgs5)
    ok &= passed5

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
    # Verify direction mapping for Circuit ID 1 (max DL=20, UL=15)
    _mark_step("sanity: circuit direction mapping (ceil)")
    passed_dir, msgs_dir = check_circuit_direction_ceil("1")
    results.extend(msgs_dir)
    ok &= passed_dir
    # IP mappings persist for circuit 1
    _mark_step("sanity: circuit ip mappings (no change)")
    passed_map1, msgs_map1 = check_ip_mappings_for_circuit("1")
    results.extend(msgs_map1)
    ok &= passed_map1

    # Circuit IP change (should not trigger full reload); verify mapping moves
    _mark_step("tiered: circuit IP change (no full reload)")
    rows_ip = json.loads(json.dumps(rows2))
    old_ip = rows_ip[0]["IPv4"]
    rows_ip[0]["IPv4"] = "100.64.0.11"
    new_ip = rows_ip[0]["IPv4"]
    write_circuits(rows_ip)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("tiered: circuit IP change", res, step_t0)
    results.append(msg)
    ok &= passed
    # Verify mapping updated
    passed_map_new, msgs_map_new = check_ip_mappings_for_circuit("1")
    results.extend(msgs_map_new)
    ok &= passed_map_new
    passed_map_old, msgs_map_old = check_ip_mapping_absent(str(old_ip))
    results.extend(msgs_map_old)
    ok &= passed_map_old

    # Add a low-rate circuit to trigger cake RTT tokens and verify units (e.g., 180ms)
    _mark_step("sanity: add low-rate circuit for RTT check")
    low_id = "99"
    rows_lr = json.loads(json.dumps(rows2))
    rows_lr.append({
        "Circuit ID": low_id,
        "Circuit Name": "LOWRATE",
        "Device ID": "99",
        "Device Name": "LD1",
        "Parent Node": "AP_A",
        "MAC": "",
        "IPv4": "100.64.0.99",
        "IPv6": "",
        "Download Min Mbps": 1,
        "Upload Min Mbps": 1,
        "Download Max Mbps": 2,
        "Upload Max Mbps": 2,
        "Comment": "",
        "sqm": "",
    })
    write_circuits(rows_lr)
    _ = run_refresh_and_wait(log, timeout_s)
    # Check RTT tokens on cake qdisc for low-rate circuit
    def _check_lowrate_rtt(cid: str) -> Tuple[bool, List[str]]:
        msgs: List[str] = []
        qs = _load_queuing_structure()
        if not isinstance(qs, dict):
            return False, ["low-rate rtt: queuingStructure.json not found"]
        net = qs.get("Network")
        if not isinstance(net, dict):
            return False, ["low-rate rtt: queuingStructure has no 'Network'"]
        found = _walk_nodes_for_circuit(net, cid)
        if not found:
            return False, ["low-rate rtt: circuit not found in structure"]
        parent_name, circuit = found
        # Resolve interfaces
        try:
            ifa = interface_a()
        except Exception as e:
            return False, [f"low-rate rtt: failed to get interface_a(): {e}"]
        ifb = None
        if interface_b and callable(interface_b):
            try:
                ifb = interface_b()
            except Exception:
                ifb = None
        # IDs
        maj = _hex_to_int(str(circuit.get("classMajor")))
        mnr = _hex_to_int(str(circuit.get("classMinor")))
        upm = _hex_to_int(str(circuit.get("up_classMajor")))
        ok_local = True
        # Downlink
        rc, out, err = _qdisc_show(ifa)
        if rc != 0:
            msgs.append(f"low-rate rtt: 'tc qdisc show dev {ifa}' failed: {err.strip()}")
            ok_local = False
        else:
            pat = re.compile(r"^qdisc\s+\S+\s+\S+:\s+parent\s+([0-9A-Fa-fx]+):([0-9A-Fa-fx]+)\b.*$", re.MULTILINE)

            line = None
            for m in pat.finditer(out):
                mj_tok, mn_tok = m.group(1), m.group(2)
                mj = _tc_id_token_to_int(mj_tok)
                mn = _tc_id_token_to_int(mn_tok)
                if mj == maj and mn == mnr:
                    line = m.group(0)
                    break
            if line is not None:
                if "cake" in line.lower():
                    if re.search(r"\brtt\s+\d+\s*ms\b", line, re.IGNORECASE):
                        msgs.append(f"low-rate rtt: down {ifa} OK | {line}")
                    else:
                        msgs.append(f"low-rate rtt: down {ifa} missing 'rtt <n>ms' in cake line | {line}")
                        ok_local = False
                else:
                    msgs.append(f"low-rate rtt: down {ifa} non-cake qdisc (RTT N/A) | {line}")
            else:
                msgs.append(f"low-rate rtt: {ifa} no qdisc for parent {maj}:{mnr}")
                ok_local = False
        # Uplink
        if ifb and upm is not None:
            rc2, out2, err2 = _qdisc_show(ifb)
            if rc2 != 0:
                msgs.append(f"low-rate rtt: 'tc qdisc show dev {ifb}' failed: {err2.strip()}")
                ok_local = False
            else:
                pat2 = re.compile(r"^qdisc\s+\S+\s+\S+:\s+parent\s+([0-9A-Fa-fx]+):([0-9A-Fa-fx]+)\b.*$", re.MULTILINE)

                line2 = None
                for m2 in pat2.finditer(out2):
                    mj_tok, mn_tok = m2.group(1), m2.group(2)
                    mj = _tc_id_token_to_int(mj_tok)
                    mn = _tc_id_token_to_int(mn_tok)
                    if mj == upm and mn == mnr:
                        line2 = m2.group(0)
                        break
                if line2 is not None:
                    if "cake" in line2.lower():
                        if re.search(r"\brtt\s+\d+\s*ms\b", line2, re.IGNORECASE):
                            msgs.append(f"low-rate rtt: up {ifb} OK | {line2}")
                        else:
                            msgs.append(f"low-rate rtt: up {ifb} missing 'rtt <n>ms' in cake line | {line2}")
                            ok_local = False
                    else:
                        msgs.append(f"low-rate rtt: up {ifb} non-cake qdisc (RTT N/A) | {line2}")
                else:
                    msgs.append(f"low-rate rtt: {ifb} no qdisc for parent {upm}:{mnr}")
                    ok_local = False
        return ok_local, msgs

    passed_lr, msgs_lr = _check_lowrate_rtt(low_id)
    results.extend(msgs_lr)
    ok &= passed_lr
    # Remove the low-rate circuit
    rows3 = [r for r in rows_lr if r["Circuit ID"] != low_id]
    write_circuits(rows3)
    _ = run_refresh_and_wait(log, timeout_s)

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
    # Mapping for added circuit C3
    _mark_step("sanity: added circuit ip mappings")
    passed_map_add, msgs_map_add = check_ip_mappings_for_circuit("3")
    results.extend(msgs_map_add)
    ok &= passed_map_add

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
    # Mapping removed
    _mark_step("sanity: removed circuit ip unmapped")
    passed_unmap, msgs_unmap = check_ip_mapping_absent("100.64.0.3")
    results.extend(msgs_unmap)
    ok &= passed_unmap

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


def run_virtualized_tiered_suite(log: AnyLogReader, timeout_s: float, results: List[str]) -> bool:
    ok = True

    write_network_json(VIRTUALIZED_TIERED_NETWORK_BASE)
    rows = virtualized_tiered_circuits_base()
    write_circuits(rows)
    _ = run_refresh_and_wait(log, timeout_s)
    settle_initial_bakery_logs(log)
    results.append("virtualized: baseline: ok (initial run)")

    _mark_step("virtualized: operator-authored virtual node promotion")
    passed, msgs = check_virtualized_node_branch_promotion(
        "Town_V",
        "AP_V",
        ["Site_A", "AP_V"],
        [
            ("direct", "501", ["Site_A"]),
            ("promoted-child", "502", ["Site_A", "AP_V"]),
        ],
    )
    results.extend(msgs)
    ok &= passed

    _mark_step("virtualized: direct circuit ip mappings")
    passed_map_direct, msgs_map_direct = check_ip_mappings_for_circuit("501")
    results.extend(msgs_map_direct)
    ok &= passed_map_direct

    _mark_step("virtualized: promoted child circuit ip mappings")
    passed_map_child, msgs_map_child = check_ip_mappings_for_circuit("502")
    results.extend(msgs_map_child)
    ok &= passed_map_child

    _mark_step("virtualized: promoted child circuit rate change")
    rows2 = json.loads(json.dumps(rows))
    rows2[1]["Download Max Mbps"] = 22
    rows2[1]["Upload Max Mbps"] = 11
    write_circuits(rows2)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("virtualized: promoted child circuit rate change", res, step_t0)
    results.append(msg)
    ok &= passed

    _mark_step("virtualized: promoted child circuit direction mapping")
    passed_dir, msgs_dir = check_circuit_direction_ceil("502")
    results.extend(msgs_dir)
    ok &= passed_dir

    _mark_step("virtualized: promoted child circuit IP change")
    rows3 = json.loads(json.dumps(rows2))
    old_ip = str(rows3[1]["IPv4"])
    rows3[1]["IPv4"] = "100.64.2.22"
    write_circuits(rows3)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("virtualized: promoted child circuit IP change", res, step_t0)
    results.append(msg)
    ok &= passed

    passed_map_new, msgs_map_new = check_ip_mappings_for_circuit("502")
    results.extend(msgs_map_new)
    ok &= passed_map_new

    passed_map_old, msgs_map_old = check_ip_mapping_absent(old_ip)
    results.extend(msgs_map_old)
    ok &= passed_map_old

    _mark_step("virtualized: promoted child circuit parent move")
    rows4 = json.loads(json.dumps(rows3))
    rows4[1]["Parent Node"] = "AP_A"
    write_circuits(rows4)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("virtualized: promoted child circuit parent move", res, step_t0)
    results.append(msg)
    ok &= passed

    _mark_step("virtualized: parent-moved circuit direction mapping")
    passed_parent_dir, msgs_parent_dir = check_circuit_direction_ceil("502")
    results.extend(msgs_parent_dir)
    ok &= passed_parent_dir

    _mark_step("virtualized: parent-moved circuit ip mappings")
    passed_parent_map, msgs_parent_map = check_ip_mappings_for_circuit("502")
    results.extend(msgs_parent_map)
    ok &= passed_parent_map

    return ok


def run_realistic_tiered_suite(log: AnyLogReader, timeout_s: float, results: List[str]) -> bool:
    ok = True

    def _first_row_index(
        rows_local: List[Dict[str, str | int | float]],
        parent_name: str,
        exclude_ids: Optional[set[str]] = None,
    ) -> Optional[int]:
        excluded = exclude_ids or set()
        for idx, row in enumerate(rows_local):
            if str(row.get("Parent Node")) == parent_name and str(row.get("Circuit ID")) not in excluded:
                return idx
        return None

    def _record_check(
        tag: str,
        passed: bool,
        msgs: List[str],
        *,
        step_started_at: Optional[float] = None,
        circuit_ids: Optional[List[str]] = None,
        node_names: Optional[List[str]] = None,
    ) -> None:
        nonlocal ok
        results.extend(msgs)
        ok &= passed
        if not passed:
            results.extend(
                collect_failure_diagnostics(
                    tag,
                    step_started_at=step_started_at,
                    circuit_ids=circuit_ids,
                    node_names=node_names,
                )
            )

    def _record_hidden_health(
        tag: str,
        step_started_at: float,
        *,
        circuit_ids: Optional[List[str]] = None,
        node_names: Optional[List[str]] = None,
    ) -> None:
        passed, msgs = check_no_hidden_incremental_failures(step_started_at)
        _record_check(
            f"{tag} health",
            passed,
            msgs,
            step_started_at=step_started_at,
            circuit_ids=circuit_ids,
            node_names=node_names,
        )

    # Baseline: write realistic network and circuits (including orphans)
    _mark_step("realistic: baseline")
    write_network_json(REALISTIC_TIERED_NETWORK)
    rows = realistic_tiered_circuits_base()
    write_circuits(rows)
    res = run_refresh_and_wait(log, timeout_s)
    # First run may include MQ init/full reload; do not assert here
    results.append("realistic: baseline: ok (initial run)")

    # Sanity: generated PNs and orphan behavior
    _mark_step("realistic: generated PN items present")
    passed_gpns, msgs_gpns = assert_generated_pns_present()
    results.extend(msgs_gpns)
    ok &= passed_gpns

    # Use one orphan circuit from the base set (last 10 entries)
    orphan_candidate: Optional[str] = None
    for r in reversed(rows):
        if str(r.get("Parent Node")) == "none":
            orphan_candidate = str(r.get("Circuit ID"))
            break
    if orphan_candidate is not None:
        _mark_step("realistic: orphan circuit HTB attached to PN parent")
        passed_orphan_tc, msgs_orphan_tc = check_orphan_tc_attachment(orphan_candidate)
        results.extend(msgs_orphan_tc)
        ok &= passed_orphan_tc

        _mark_step("realistic: orphan circuit ip mappings")
        passed_orphan_map, msgs_orphan_map = check_ip_mappings_for_circuit(orphan_candidate)
        results.extend(msgs_orphan_map)
        ok &= passed_orphan_map

    _mark_step("realistic: no site/circuit minor collisions")
    passed_coll, msgs_coll = check_no_site_circuit_minor_collisions()
    results.extend(msgs_coll)
    ok &= passed_coll

    # Site speed change on a mid-tree site (NET2-1)
    _mark_step("realistic: site speed change")
    net2 = json.loads(json.dumps(REALISTIC_TIERED_NETWORK))
    try:
        net2["NET2"]["children"]["NET2-1"]["downloadBandwidthMbps"] = 360
        net2["NET2"]["children"]["NET2-1"]["uploadBandwidthMbps"] = 240
    except Exception:
        results.append("realistic: site speed change: failed to modify NET2-1 speeds in fixture")
        return False
    write_network_json(net2)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("realistic: site speed change", res, step_t0)
    results.append(msg)
    ok &= passed
    _record_hidden_health("realistic: site speed change", step_t0, node_names=["NET2-1"])

    _mark_step("realistic: site direction mapping (NET2-1)")
    passed_site, msgs_site = check_site_direction_ceil("NET2-1", expected_dl=360.0, expected_ul=240.0)
    _record_check("realistic: site direction mapping (NET2-1)", passed_site, msgs_site, circuit_ids=[], node_names=["NET2-1"])

    # Circuit speed change for a deep leaf circuit (under NET1-1-1)
    _mark_step("realistic: circuit speed change")
    rows2 = json.loads(json.dumps(rows))
    target_idx: Optional[int] = None
    for idx, r in enumerate(rows2):
        if r.get("Parent Node") == "NET1-1-1":
            target_idx = idx
            break
    if target_idx is None:
        results.append("realistic: circuit speed change: no circuit found under NET1-1-1")
        return False
    target_id = str(rows2[target_idx]["Circuit ID"])
    try:
        old_dl = float(rows2[target_idx]["Download Max Mbps"])
        old_ul = float(rows2[target_idx]["Upload Max Mbps"])
    except Exception:
        old_dl = 10.0
        old_ul = 5.0
    rows2[target_idx]["Download Max Mbps"] = old_dl + 30.0
    rows2[target_idx]["Upload Max Mbps"] = old_ul + 10.0
    write_circuits(rows2)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("realistic: circuit speed change", res, step_t0)
    results.append(msg)
    ok &= passed
    _record_hidden_health("realistic: circuit speed change", step_t0, circuit_ids=[target_id])

    _mark_step("realistic: circuit direction mapping (ceil)")
    passed_dir, msgs_dir = check_circuit_direction_ceil(target_id)
    _record_check("realistic: circuit direction mapping (ceil)", passed_dir, msgs_dir, circuit_ids=[target_id])

    _mark_step("realistic: circuit ip mappings (no change)")
    passed_map, msgs_map = check_ip_mappings_for_circuit(target_id)
    _record_check("realistic: circuit ip mappings (no change)", passed_map, msgs_map, circuit_ids=[target_id])

    passed_site_snapshot, site_snapshot_msg, site_snapshot = snapshot_site_class_assignments()
    if not passed_site_snapshot:
        results.append(site_snapshot_msg)
        ok = False

    # Circuit parent-node move between existing nodes (should not trigger full reload)
    _mark_step("realistic: circuit parent move")
    rows_parent = json.loads(json.dumps(rows2))
    old_down_tc, old_up_tc, _old_handle_msgs = _current_circuit_handle_strings(target_id)
    old_parent = str(rows_parent[target_idx]["Parent Node"])
    new_parent = "NET1-1-2" if old_parent != "NET1-1-2" else "NET1-2"
    rows_parent[target_idx]["Parent Node"] = new_parent
    write_circuits(rows_parent)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("realistic: circuit parent move", res, step_t0)
    results.append(msg)
    ok &= passed
    _record_hidden_health("realistic: circuit parent move", step_t0, circuit_ids=[target_id])

    _mark_step("realistic: site class assignment stability after parent move")
    passed_sites_stable, msgs_sites_stable = check_site_class_assignments_unchanged(site_snapshot)
    _record_check("realistic: site class assignment stability after parent move", passed_sites_stable, msgs_sites_stable, circuit_ids=[target_id])

    _mark_step("realistic: parent-moved circuit direction mapping (ceil)")
    passed_parent_dir, msgs_parent_dir = wait_for_circuit_direction_ceil(target_id)
    _record_check("realistic: parent-moved circuit direction mapping (ceil)", passed_parent_dir, msgs_parent_dir, circuit_ids=[target_id], step_started_at=step_t0)

    _mark_step("realistic: parent-moved circuit ip mappings")
    passed_parent_map, msgs_parent_map = check_ip_mappings_for_circuit(target_id)
    _record_check("realistic: parent-moved circuit ip mappings", passed_parent_map, msgs_parent_map, circuit_ids=[target_id])

    _mark_step("realistic: parent-moved circuit cleanup")
    passed_parent_cleanup, msgs_parent_cleanup = check_circuit_transition_cleanup(
        target_id,
        old_down_tc=old_down_tc,
        old_up_tc=old_up_tc,
    )
    _record_check("realistic: parent-moved circuit cleanup", passed_parent_cleanup, msgs_parent_cleanup, circuit_ids=[target_id])

    _mark_step("realistic: parent-moved circuit batch exact IP mappings")
    passed_parent_batch_ip, msgs_parent_batch_ip = check_exact_ip_mappings_for_circuits(
        [target_id],
        forbidden_by_circuit={target_id: {tc for tc in [old_down_tc, old_up_tc] if tc}},
    )
    _record_check(
        "realistic: parent-moved circuit batch exact IP mappings",
        passed_parent_batch_ip,
        msgs_parent_batch_ip,
        circuit_ids=[target_id],
    )

    # Multiple simultaneous parent-node moves in one incremental commit
    _mark_step("realistic: multiple circuit parent moves")
    rows_multi = json.loads(json.dumps(rows_parent))
    multi_specs = [
        ("NET2-1-1", "NET2-2"),
        ("NET3-1", "NET3-2"),
        ("NET1-2", "NET1-1-1"),
    ]
    moved_ids: List[str] = []
    old_handles_by_circuit: Dict[str, Tuple[Optional[str], Optional[str]]] = {}
    excluded_ids = {target_id}
    for old_parent_name, new_parent_name in multi_specs:
        idx = _first_row_index(rows_multi, old_parent_name, excluded_ids)
        if idx is None:
            results.append(
                f"realistic: multiple circuit parent moves: no circuit found under {old_parent_name}"
            )
            return False
        moved_id = str(rows_multi[idx]["Circuit ID"])
        old_handles_by_circuit[moved_id] = _current_circuit_handle_strings(moved_id)[:2]
        rows_multi[idx]["Parent Node"] = new_parent_name
        moved_ids.append(moved_id)
        excluded_ids.add(moved_id)
    write_circuits(rows_multi)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("realistic: multiple circuit parent moves", res, step_t0)
    results.append(msg)
    ok &= passed
    _record_hidden_health("realistic: multiple circuit parent moves", step_t0, circuit_ids=moved_ids)

    _mark_step("realistic: site class assignment stability after multiple parent moves")
    passed_sites_stable_multi, msgs_sites_stable_multi = check_site_class_assignments_unchanged(site_snapshot)
    _record_check("realistic: site class assignment stability after multiple parent moves", passed_sites_stable_multi, msgs_sites_stable_multi, circuit_ids=moved_ids)

    for moved_id in moved_ids:
        _mark_step(f"realistic: moved circuit {moved_id} direction mapping")
        passed_move_dir, msgs_move_dir = wait_for_circuit_direction_ceil(moved_id)
        _record_check(f"realistic: moved circuit {moved_id} direction mapping", passed_move_dir, msgs_move_dir, circuit_ids=[moved_id], step_started_at=step_t0)

        _mark_step(f"realistic: moved circuit {moved_id} ip mappings")
        passed_move_map, msgs_move_map = check_ip_mappings_for_circuit(moved_id)
        _record_check(f"realistic: moved circuit {moved_id} ip mappings", passed_move_map, msgs_move_map, circuit_ids=[moved_id])

        _mark_step(f"realistic: moved circuit {moved_id} cleanup")
        old_down_tc_multi, old_up_tc_multi = old_handles_by_circuit.get(moved_id, (None, None))
        passed_move_cleanup, msgs_move_cleanup = check_circuit_transition_cleanup(
            moved_id,
            old_down_tc=old_down_tc_multi,
            old_up_tc=old_up_tc_multi,
        )
        _record_check(f"realistic: moved circuit {moved_id} cleanup", passed_move_cleanup, msgs_move_cleanup, circuit_ids=[moved_id])

    _mark_step("realistic: multiple parent moves batch exact IP mappings")
    forbidden_multi = {
        circuit_id: {tc for tc in old_handles_by_circuit.get(circuit_id, (None, None)) if tc}
        for circuit_id in moved_ids
    }
    passed_multi_batch_ip, msgs_multi_batch_ip = check_exact_ip_mappings_for_circuits(
        moved_ids,
        forbidden_by_circuit=forbidden_multi,
    )
    _record_check(
        "realistic: multiple parent moves batch exact IP mappings",
        passed_multi_batch_ip,
        msgs_multi_batch_ip,
        circuit_ids=moved_ids,
    )

    # Same-circuit mixed change: parent move + rate + IP + SQM in one incremental commit
    _mark_step("realistic: same-circuit mixed change")
    rows_same = json.loads(json.dumps(rows_multi))
    same_idx = _first_row_index(rows_same, "NET3-2", {target_id, *moved_ids})
    if same_idx is None:
        results.append("realistic: same-circuit mixed change: no candidate circuit found under NET3-2")
        return False
    same_id = str(rows_same[same_idx]["Circuit ID"])
    same_old_ip = str(rows_same[same_idx]["IPv4"])
    same_old_down_tc, same_old_up_tc, _ = _current_circuit_handle_strings(same_id)
    rows_same[same_idx]["Parent Node"] = "NET3-1"
    rows_same[same_idx]["Download Max Mbps"] = float(rows_same[same_idx]["Download Max Mbps"]) + 19.0
    rows_same[same_idx]["Upload Max Mbps"] = float(rows_same[same_idx]["Upload Max Mbps"]) + 7.0
    rows_same[same_idx]["IPv4"] = "100.64.220.1"
    rows_same[same_idx]["sqm"] = "cake/fq_codel"
    write_circuits(rows_same)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("realistic: same-circuit mixed change", res, step_t0)
    results.append(msg)
    ok &= passed
    _record_hidden_health("realistic: same-circuit mixed change", step_t0, circuit_ids=[same_id])

    _mark_step("realistic: same-circuit mixed direction mapping")
    passed_same_dir, msgs_same_dir = wait_for_circuit_direction_ceil(same_id)
    _record_check("realistic: same-circuit mixed direction mapping", passed_same_dir, msgs_same_dir, circuit_ids=[same_id], step_started_at=step_t0)

    _mark_step("realistic: same-circuit mixed IP mappings")
    passed_same_ip, msgs_same_ip = check_ip_mappings_for_circuit(same_id)
    _record_check("realistic: same-circuit mixed IP mappings", passed_same_ip, msgs_same_ip, circuit_ids=[same_id])
    passed_same_old_ip, msgs_same_old_ip = check_ip_mapping_absent(same_old_ip)
    _record_check("realistic: same-circuit mixed old IP absent", passed_same_old_ip, msgs_same_old_ip, circuit_ids=[same_id])

    _mark_step("realistic: same-circuit mixed exact IP mappings")
    passed_same_exact_ip, msgs_same_exact_ip = check_exact_ip_mappings_for_circuit(
        same_id,
        forbidden_tcs={tc for tc in [same_old_down_tc, same_old_up_tc] if tc},
    )
    _record_check("realistic: same-circuit mixed exact IP mappings", passed_same_exact_ip, msgs_same_exact_ip, circuit_ids=[same_id])

    _mark_step("realistic: same-circuit mixed SQM kind")
    passed_same_sqm, msgs_same_sqm = check_circuit_qdisc_kind(
        same_id,
        expected_down_kind="cake",
        expected_up_kind="fq_codel",
    )
    _record_check("realistic: same-circuit mixed SQM kind", passed_same_sqm, msgs_same_sqm, circuit_ids=[same_id])

    _mark_step("realistic: same-circuit mixed cleanup")
    passed_same_cleanup, msgs_same_cleanup = check_circuit_transition_cleanup(
        same_id,
        old_down_tc=same_old_down_tc,
        old_up_tc=same_old_up_tc,
    )
    _record_check("realistic: same-circuit mixed cleanup", passed_same_cleanup, msgs_same_cleanup, circuit_ids=[same_id])

    _mark_step("realistic: same-circuit mixed batch exact IP mappings")
    passed_same_batch_ip, msgs_same_batch_ip = check_exact_ip_mappings_for_circuits(
        [same_id],
        forbidden_by_circuit={same_id: {tc for tc in [same_old_down_tc, same_old_up_tc] if tc}},
    )
    _record_check(
        "realistic: same-circuit mixed batch exact IP mappings",
        passed_same_batch_ip,
        msgs_same_batch_ip,
        circuit_ids=[same_id],
    )

    # Mixed incremental batch: site speed, circuit speed, IP, SQM, and parent move together
    _mark_step("realistic: mixed incremental batch")
    net_mixed = json.loads(json.dumps(net2))
    try:
        net_mixed["NET3"]["children"]["NET3-1"]["downloadBandwidthMbps"] = 275
        net_mixed["NET3"]["children"]["NET3-1"]["uploadBandwidthMbps"] = 155
    except Exception:
        results.append("realistic: mixed incremental batch: failed to modify NET3-1 speeds in fixture")
        return False

    rows_mixed = json.loads(json.dumps(rows_same))
    excluded_mixed = {target_id, *moved_ids, same_id}

    speed_idx = _first_row_index(rows_mixed, "NET2-2", excluded_mixed)
    if speed_idx is None:
        results.append("realistic: mixed incremental batch: no speed-change circuit found under NET2-2")
        return False
    speed_id = str(rows_mixed[speed_idx]["Circuit ID"])
    rows_mixed[speed_idx]["Download Max Mbps"] = float(rows_mixed[speed_idx]["Download Max Mbps"]) + 17.0
    rows_mixed[speed_idx]["Upload Max Mbps"] = float(rows_mixed[speed_idx]["Upload Max Mbps"]) + 6.0
    excluded_mixed.add(speed_id)

    ip_idx = _first_row_index(rows_mixed, "NET3-2", excluded_mixed)
    if ip_idx is None:
        results.append("realistic: mixed incremental batch: no IP-change circuit found under NET3-2")
        return False
    ip_id = str(rows_mixed[ip_idx]["Circuit ID"])
    old_ip = str(rows_mixed[ip_idx]["IPv4"])
    rows_mixed[ip_idx]["IPv4"] = "100.64.210.1"
    excluded_mixed.add(ip_id)

    sqm_idx = _first_row_index(rows_mixed, "NET1-1-2", excluded_mixed)
    if sqm_idx is None:
        results.append("realistic: mixed incremental batch: no SQM-change circuit found under NET1-1-2")
        return False
    sqm_id = str(rows_mixed[sqm_idx]["Circuit ID"])
    rows_mixed[sqm_idx]["sqm"] = "fq_codel/cake"
    excluded_mixed.add(sqm_id)

    mixed_move_idx: Optional[int] = None
    mixed_move_target: Optional[str] = None
    for source_parent, target_parent in [
        ("NET2-2", "NET2-1-1"),
        ("NET3-2", "NET3-1"),
        ("NET1-1-2", "NET1-2"),
    ]:
        idx = _first_row_index(rows_mixed, source_parent, excluded_mixed)
        if idx is not None:
            mixed_move_idx = idx
            mixed_move_target = target_parent
            break
    if mixed_move_idx is None or mixed_move_target is None:
        results.append("realistic: mixed incremental batch: no parent-move circuit found in fallback candidate set")
        return False
    mixed_move_id = str(rows_mixed[mixed_move_idx]["Circuit ID"])
    mixed_old_down_tc, mixed_old_up_tc, _ = _current_circuit_handle_strings(mixed_move_id)
    rows_mixed[mixed_move_idx]["Parent Node"] = mixed_move_target

    write_network_json(net_mixed)
    write_circuits(rows_mixed)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    passed, msg = assert_no_full_reload("realistic: mixed incremental batch", res, step_t0)
    results.append(msg)
    ok &= passed
    _record_hidden_health(
        "realistic: mixed incremental batch",
        step_t0,
        circuit_ids=[speed_id, ip_id, sqm_id, mixed_move_id],
        node_names=["NET3-1"],
    )

    _mark_step("realistic: site class assignment stability after mixed batch")
    passed_sites_stable_mixed, msgs_sites_stable_mixed = check_site_class_assignments_unchanged(site_snapshot)
    _record_check("realistic: site class assignment stability after mixed batch", passed_sites_stable_mixed, msgs_sites_stable_mixed, circuit_ids=[speed_id, ip_id, sqm_id, mixed_move_id], node_names=["NET3-1"])

    _mark_step("realistic: mixed batch site direction mapping (NET3-1)")
    passed_site_mixed, msgs_site_mixed = check_site_direction_ceil("NET3-1", expected_dl=275.0, expected_ul=155.0)
    _record_check("realistic: mixed batch site direction mapping (NET3-1)", passed_site_mixed, msgs_site_mixed, node_names=["NET3-1"])

    _mark_step("realistic: mixed batch circuit speed direction mapping")
    passed_speed_dir, msgs_speed_dir = wait_for_circuit_direction_ceil(speed_id)
    _record_check("realistic: mixed batch circuit speed direction mapping", passed_speed_dir, msgs_speed_dir, circuit_ids=[speed_id], step_started_at=step_t0)

    _mark_step("realistic: mixed batch circuit IP mappings")
    passed_ip_map, msgs_ip_map = check_ip_mappings_for_circuit(ip_id)
    _record_check("realistic: mixed batch circuit IP mappings", passed_ip_map, msgs_ip_map, circuit_ids=[ip_id])
    passed_old_ip_absent, msgs_old_ip_absent = check_ip_mapping_absent(old_ip)
    _record_check("realistic: mixed batch old IP absent", passed_old_ip_absent, msgs_old_ip_absent, circuit_ids=[ip_id])

    _mark_step("realistic: mixed batch exact IP mappings")
    passed_ip_exact, msgs_ip_exact = check_exact_ip_mappings_for_circuit(ip_id)
    _record_check("realistic: mixed batch exact IP mappings", passed_ip_exact, msgs_ip_exact, circuit_ids=[ip_id])

    _mark_step("realistic: mixed batch circuit SQM kind")
    passed_sqm_kind, msgs_sqm_kind = check_circuit_qdisc_kind(
        sqm_id,
        expected_down_kind="fq_codel",
        expected_up_kind="cake",
    )
    _record_check("realistic: mixed batch circuit SQM kind", passed_sqm_kind, msgs_sqm_kind, circuit_ids=[sqm_id])

    _mark_step("realistic: mixed batch parent-moved circuit direction mapping")
    passed_mixed_move_dir, msgs_mixed_move_dir = wait_for_circuit_direction_ceil(mixed_move_id)
    _record_check("realistic: mixed batch parent-moved circuit direction mapping", passed_mixed_move_dir, msgs_mixed_move_dir, circuit_ids=[mixed_move_id], step_started_at=step_t0)

    _mark_step("realistic: mixed batch parent-moved circuit ip mappings")
    passed_mixed_move_map, msgs_mixed_move_map = check_ip_mappings_for_circuit(mixed_move_id)
    _record_check("realistic: mixed batch parent-moved circuit ip mappings", passed_mixed_move_map, msgs_mixed_move_map, circuit_ids=[mixed_move_id])

    _mark_step("realistic: mixed batch parent-moved circuit cleanup")
    passed_mixed_move_cleanup, msgs_mixed_move_cleanup = check_circuit_transition_cleanup(
        mixed_move_id,
        old_down_tc=mixed_old_down_tc,
        old_up_tc=mixed_old_up_tc,
    )
    _record_check("realistic: mixed batch parent-moved circuit cleanup", passed_mixed_move_cleanup, msgs_mixed_move_cleanup, circuit_ids=[mixed_move_id])

    _mark_step("realistic: mixed batch exact IP mappings")
    passed_mixed_batch_ip, msgs_mixed_batch_ip = check_exact_ip_mappings_for_circuits(
        [speed_id, ip_id, sqm_id, mixed_move_id],
        forbidden_by_circuit={
            mixed_move_id: {tc for tc in [mixed_old_down_tc, mixed_old_up_tc] if tc},
        },
    )
    _record_check(
        "realistic: mixed batch exact IP mappings",
        passed_mixed_batch_ip,
        msgs_mixed_batch_ip,
        circuit_ids=[speed_id, ip_id, sqm_id, mixed_move_id],
    )

    # Add circuit under a different leaf (NET3-2)
    _mark_step("realistic: add circuit")
    rows3 = json.loads(json.dumps(rows_mixed))
    new_circuit_id = "99901"
    new_circuit_ip = "100.64.200.1"
    rows3.append({
        "Circuit ID": new_circuit_id,
        "Circuit Name": "CIRCUIT_ADDED_99901",
        "Device ID": new_circuit_id,
        "Device Name": "DEV_ADDED_99901",
        "Parent Node": "NET3-2",
        "MAC": "",
        "IPv4": new_circuit_ip,
        "IPv6": "",
        "Download Min Mbps": 1,
        "Upload Min Mbps": 1,
        "Download Max Mbps": 90,
        "Upload Max Mbps": 30,
        "Comment": "",
        "sqm": "",
    })
    write_circuits(rows3)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    if res.full_reload and not res.mq_init:
        results.append("realistic: add circuit: full reload (allowed for larger tree)")
    else:
        results.append("realistic: add circuit: OK (no full reload)")
    _record_hidden_health("realistic: add circuit", step_t0, circuit_ids=[new_circuit_id])

    _mark_step("realistic: added circuit ip mappings")
    passed_add_map, msgs_add_map = check_ip_mappings_for_circuit(new_circuit_id)
    _record_check("realistic: added circuit ip mappings", passed_add_map, msgs_add_map, circuit_ids=[new_circuit_id])

    # Remove the added circuit
    _mark_step("realistic: remove circuit")
    rows4 = [r for r in rows3 if str(r.get("Circuit ID")) != new_circuit_id]
    write_circuits(rows4)
    step_t0 = time.time()
    res = run_refresh_and_wait(log, timeout_s)
    if res.full_reload and not res.mq_init:
        results.append("realistic: remove circuit: full reload (allowed for larger tree)")
    else:
        results.append("realistic: remove circuit: OK (no full reload)")
    _record_hidden_health("realistic: remove circuit", step_t0, circuit_ids=[new_circuit_id])

    _mark_step("realistic: removed circuit ip unmapped")
    passed_unmap, msgs_unmap = check_ip_mapping_absent(new_circuit_ip)
    _record_check("realistic: removed circuit ip unmapped", passed_unmap, msgs_unmap, circuit_ids=[new_circuit_id])

    return ok


def run_fault_reload_suite(log: AnyLogReader, timeout_s: float, results: List[str]) -> bool:
    ok = True

    def _record_check(
        tag: str,
        passed: bool,
        msgs: List[str],
        *,
        step_started_at: Optional[float] = None,
        circuit_ids: Optional[List[str]] = None,
        node_names: Optional[List[str]] = None,
    ) -> None:
        nonlocal ok
        results.extend(msgs)
        ok &= passed
        if not passed:
            results.extend(
                collect_failure_diagnostics(
                    tag,
                    step_started_at=step_started_at,
                    circuit_ids=circuit_ids,
                    node_names=node_names,
                )
            )

    clear_bakery_fault_once()
    try:
        _mark_step("fault-reload: baseline")
        write_network_json(REALISTIC_TIERED_NETWORK)
        rows = realistic_tiered_circuits_base()
        write_circuits(rows)
        _ = run_refresh_and_wait(log, timeout_s)
        results.append("fault-reload: baseline: ok (initial run)")

        target_idx: Optional[int] = None
        for idx, row in enumerate(rows):
            if str(row.get("Parent Node")) == "NET1-1-1":
                target_idx = idx
                break
        if target_idx is None:
            results.append("fault-reload: no candidate circuit found under NET1-1-1")
            return False

        target_id = str(rows[target_idx]["Circuit ID"])
        old_down_tc, old_up_tc, _ = _current_circuit_handle_strings(target_id)

        rows_fault = json.loads(json.dumps(rows))
        rows_fault[target_idx]["Parent Node"] = "NET1-1-2"
        arm_bakery_fault_once("migrating circuits between parent nodes")

        _mark_step("fault-reload: injected parent-move failure")
        write_circuits(rows_fault)
        step_t0 = time.time()
        res = run_refresh_and_wait(log, timeout_s)
        if res.full_reload and not res.mq_init:
            results.append("fault-reload: injected failure triggered an immediate full reload")
        else:
            results.append("fault-reload: injected failure commit completed without immediate full reload")

        passed_reload_required, msgs_reload_required = check_fault_injection_observed(step_t0)
        _record_check(
            "fault-reload: fault injection observed",
            passed_reload_required,
            msgs_reload_required,
            step_started_at=step_t0,
            circuit_ids=[target_id],
        )

        rows_rebuild = json.loads(json.dumps(rows_fault))
        rows_rebuild[target_idx]["Download Max Mbps"] = float(rows_rebuild[target_idx]["Download Max Mbps"]) + 7.0
        rows_rebuild[target_idx]["Upload Max Mbps"] = float(rows_rebuild[target_idx]["Upload Max Mbps"]) + 3.0

        _mark_step("fault-reload: follow-up commit forces full reload")
        write_circuits(rows_rebuild)
        step_t1 = time.time()
        res = run_refresh_and_wait(log, timeout_s)
        passed_full, msg_full = assert_full_reload(
            "fault-reload: follow-up commit forces full reload",
            res,
            step_started_at=step_t1,
        )
        results.append(msg_full)
        ok &= passed_full
        if not passed_full:
            results.extend(
                collect_failure_diagnostics(
                    "fault-reload: follow-up commit forces full reload",
                    step_started_at=step_t1,
                    circuit_ids=[target_id],
                )
            )

        passed_reload_gate, msgs_reload_gate = check_reload_required_since(step_t0)
        _record_check(
            "fault-reload: reload-required gate observed",
            passed_reload_gate,
            msgs_reload_gate,
            step_started_at=step_t0,
            circuit_ids=[target_id],
        )

        _mark_step("fault-reload: post-reload circuit direction mapping")
        passed_dir, msgs_dir = wait_for_circuit_direction_ceil(target_id)
        _record_check(
            "fault-reload: post-reload circuit direction mapping",
            passed_dir,
            msgs_dir,
            circuit_ids=[target_id],
            step_started_at=step_t1,
        )

        _mark_step("fault-reload: post-reload exact IP mappings")
        passed_ip, msgs_ip = check_exact_ip_mappings_for_circuit(
            target_id,
            forbidden_tcs={tc for tc in [old_down_tc, old_up_tc] if tc},
        )
        _record_check(
            "fault-reload: post-reload exact IP mappings",
            passed_ip,
            msgs_ip,
            circuit_ids=[target_id],
        )

        _mark_step("fault-reload: post-reload cleanup")
        passed_cleanup, msgs_cleanup = check_circuit_transition_cleanup(
            target_id,
            old_down_tc=old_down_tc,
            old_up_tc=old_up_tc,
        )
        _record_check(
            "fault-reload: post-reload cleanup",
            passed_cleanup,
            msgs_cleanup,
            circuit_ids=[target_id],
        )

        rows_recovered = json.loads(json.dumps(rows_rebuild))
        rows_recovered[target_idx]["Download Max Mbps"] = float(rows_recovered[target_idx]["Download Max Mbps"]) + 5.0
        rows_recovered[target_idx]["Upload Max Mbps"] = float(rows_recovered[target_idx]["Upload Max Mbps"]) + 2.0

        _mark_step("fault-reload: post-reload incremental recovery")
        write_circuits(rows_recovered)
        step_t2 = time.time()
        res = run_refresh_and_wait(log, timeout_s)
        passed_recovery, msg_recovery = assert_no_full_reload(
            "fault-reload: post-reload incremental recovery",
            res,
            step_started_at=step_t2,
        )
        results.append(msg_recovery)
        ok &= passed_recovery
        if not passed_recovery:
            results.extend(
                collect_failure_diagnostics(
                    "fault-reload: post-reload incremental recovery",
                    step_started_at=step_t2,
                    circuit_ids=[target_id],
                )
            )

        passed_health, msgs_health = check_no_hidden_incremental_failures(step_t2)
        _record_check(
            "fault-reload: post-reload incremental recovery health",
            passed_health,
            msgs_health,
            step_started_at=step_t2,
            circuit_ids=[target_id],
        )

        return ok
    finally:
        clear_bakery_fault_once()


def run_treeguard_runtime_suite(
    log: AnyLogReader, timeout_s: float, treeguard_timeout_s: float, results: List[str]
) -> bool:
    ok = True
    cpu_mode = get_treeguard_cpu_mode()
    fixture = treeguard_runtime_fixture_metadata()

    _mark_step("treeguard: baseline")
    write_network_json(treeguard_runtime_network_base())
    rows = treeguard_runtime_circuits_base()
    write_circuits(rows)
    _ = run_refresh_and_wait(log, timeout_s)
    results.append(
        "treeguard: baseline: ok "
        f"({fixture['total_circuits']} circuits across {TREEGUARD_RUNTIME_REGION_COUNT} top-level nodes)"
    )

    after_baseline_offset = log.snapshot()

    _mark_step("treeguard: wait for runtime virtualization")
    passed, msgs = wait_for_runtime_virtualized_node(
        str(fixture["virtualized_region"]),
        str(fixture["promoted_pop"]),
        [str(fixture["promoted_pop"])],
        [
            (
                "promoted-branch",
                str(fixture["promoted_leaf_circuit"]),
                [str(fixture["promoted_pop"]), str(fixture["promoted_leaf"])],
            ),
            (
                "sibling-branch",
                str(fixture["sibling_leaf_circuit"]),
                [str(fixture["sibling_pop"]), str(fixture["sibling_leaf"])],
            ),
        ],
        treeguard_timeout_s,
    )
    if passed:
        results.extend(msgs)
        ok &= passed
    elif cpu_mode == "cpu_aware":
        still_physical, still_physical_msgs = check_node_present_in_physical_tree(
            str(fixture["virtualized_region"])
        )
        results.extend(still_physical_msgs)
        results.append(
            "treeguard runtime check: SKIP under cpu_aware because no runtime virtualization occurred without induced CPU pressure"
        )
        ok &= still_physical
    else:
        results.extend(msgs)
        ok &= passed

    _, new_lines = log.read_since(after_baseline_offset)
    if _log_contains_unexpected_full_reload(new_lines):
        results.append("treeguard: runtime virtualization: unexpected full reload detected")
        ok = False
    else:
        results.append("treeguard: runtime virtualization: OK (no full reload)")

    _mark_step("treeguard: promoted branch circuit ip mappings")
    passed_map_promoted, msgs_map_promoted = check_ip_mappings_for_circuit(
        str(fixture["promoted_leaf_circuit"])
    )
    results.extend(msgs_map_promoted)
    ok &= passed_map_promoted

    _mark_step("treeguard: sibling branch circuit ip mappings")
    passed_map_sibling, msgs_map_sibling = check_ip_mappings_for_circuit(
        str(fixture["sibling_leaf_circuit"])
    )
    results.extend(msgs_map_sibling)
    ok &= passed_map_sibling

    _mark_step("treeguard: low-value sibling remains physical")
    passed_small, msgs_small = check_node_present_in_physical_tree(str(fixture["low_value_sibling"]))
    results.extend(msgs_small)
    ok &= passed_small

    return ok


def run_flat_suite(log: AnyLogReader, timeout_s: float, results: List[str]) -> bool:
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


def write_json_report(path: str, payload: dict) -> None:
    report_path = Path(path)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")


# -----------------------
# Main
# -----------------------


def main() -> int:
    ap = argparse.ArgumentParser(description="LibreQoS/Bakery integration tests")
    ap.add_argument(
        "--log-file",
        help="Path to lqosd log file (stdout/tee capture). When omitted, the harness reads journalctl -u lqosd.",
    )
    ap.add_argument("--timeout", type=float, default=8.0, help="Wait time per step (seconds)")
    ap.add_argument(
        "--treeguard-timeout",
        type=float,
        default=90.0,
        help="Max wait time for TreeGuard runtime virtualization in the dedicated TreeGuard suite",
    )
    g = ap.add_mutually_exclusive_group()
    g.add_argument("--full-suite", action="store_true", help="Run the full harness (slower)")
    g.add_argument("--tiered-only", action="store_true", help="Run tiered cases only")
    g.add_argument("--flat-only", action="store_true", help="Run flat cases only")
    g.add_argument("--realistic-only", action="store_true", help="Run realistic tiered cases only")
    g.add_argument("--virtualized-only", action="store_true", help="Run virtualized-node tiered cases only")
    g.add_argument("--treeguard-only", action="store_true", help="Run live TreeGuard runtime cases only")
    g.add_argument("--queue-mode-only", action="store_true", help="Run live Observe/Shape queue-mode toggle checks only")
    g.add_argument(
        "--fault-reload-only",
        action="store_true",
        help="Run the opt-in Bakery fault-injection reload-escalation suite only",
    )
    ap.add_argument("--no-restore", action="store_true", help="Do not restore original files (debugging)")
    ap.add_argument(
        "--json-report",
        help="Write a machine-readable summary JSON report to this path",
    )

    args = ap.parse_args()

    if not is_lqosd_alive():
        print("WARNING: lqosd does not appear to be running. This test will not observe Bakery logs.")
        print("Start lqosd and re-run, or continue if already started but lib binding cannot detect it.")

    log: AnyLogReader
    if args.log_file:
        if not os.path.exists(args.log_file):
            print(f"ERROR: Log file not found: {args.log_file}")
            print("Either point --log-file at an existing tee'd log, or omit it to use journalctl -u lqosd.")
            return 1
        print(f"Using tee'd lqosd log file: {args.log_file}")
        log = LogReader(args.log_file)
    else:
        print("Using journalctl -u lqosd for Bakery event detection")
        log = JournalctlLogReader("lqosd")
    results: List[str] = []
    overall_ok = True
    selected_suite = "tiered-only"

    files = ["network.json", "ShapedDevices.csv"]
    ctx = contextlib.nullcontext()
    if not args.no_restore:
        ctx = with_backups(files)

    with ctx:
        if args.tiered_only:
            selected_suite = "tiered-only"
            ok = run_tiered_suite(log, args.timeout, results)
            overall_ok &= ok
        elif args.virtualized_only:
            selected_suite = "virtualized-only"
            ok = run_virtualized_tiered_suite(log, args.timeout, results)
            overall_ok &= ok
        elif args.flat_only:
            selected_suite = "flat-only"
            ok = run_flat_suite(log, args.timeout, results)
            overall_ok &= ok
        elif args.realistic_only:
            selected_suite = "realistic-only"
            ok = run_realistic_tiered_suite(log, args.timeout, results)
            overall_ok &= ok
        elif args.treeguard_only:
            selected_suite = "treeguard-only"
            ok = run_treeguard_runtime_suite(
                log, args.timeout, args.treeguard_timeout, results
            )
            overall_ok &= ok
        elif args.queue_mode_only:
            selected_suite = "queue-mode-only"
            ok = run_queue_mode_suite(log, args.timeout, results)
            overall_ok &= ok
        elif args.fault_reload_only:
            selected_suite = "fault-reload-only"
            ok = run_fault_reload_suite(log, args.timeout, results)
            overall_ok &= ok
        elif args.full_suite:
            selected_suite = "full-suite"
            ok = run_tiered_suite(log, args.timeout, results)
            overall_ok &= ok

            ok = run_virtualized_tiered_suite(log, args.timeout, results)
            overall_ok &= ok

            ok = run_flat_suite(log, args.timeout, results)
            overall_ok &= ok

            ok = run_realistic_tiered_suite(log, args.timeout, results)
            overall_ok &= ok

        else:
            selected_suite = "tiered-only"
            ok = run_tiered_suite(log, args.timeout, results)
            overall_ok &= ok

    report_payload = {
        "suite": selected_suite,
        "log_source": args.log_file or "journalctl -u lqosd",
        "timeout_seconds": args.timeout,
        "treeguard_timeout_seconds": args.treeguard_timeout,
        "results": results,
        "overall_ok": overall_ok,
    }
    if args.json_report:
        write_json_report(args.json_report, report_payload)

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
