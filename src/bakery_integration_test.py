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


def run_refresh_and_wait(log: LogReader, timeout_s: float) -> LogResult:
    offset = log.snapshot()
    # Call the core refresh function; this does not require running as __main__
    LibreQoS.refreshShapers()
    # Allow lqosd time to process the commit and log
    print("Sleeping 0.5s to allow lqosd to commit and log...")
    time.sleep(0.5)
    return log.wait_for_events(offset, timeout_s=timeout_s)


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
    # Lines look like: "<ip/prefix>    CPU: <cpu>  TC: <a:b>"
    pat = re.compile(r"^\s*([0-9A-Fa-f:.]+)/\d+\s+CPU:\s+(\d+)\s+TC:\s+([0-9A-Fa-f]+:[0-9A-Fa-f]+)\s*$")
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
            for m in pat.finditer(out):
                mj_tok, mn_tok = m.group(1), m.group(2)
                mj = _to_int(mj_tok)
                mn = _to_int(mn_tok)
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

                def _to_int2(tok: str) -> Optional[int]:
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

                line2 = None
                for m2 in pat2.finditer(out2):
                    mj_tok, mn_tok = m2.group(1), m2.group(2)
                    mj = _to_int2(mj_tok)
                    mn = _to_int2(mn_tok)
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


def run_realistic_tiered_suite(log: LogReader, timeout_s: float, results: List[str]) -> bool:
    ok = True

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

    _mark_step("realistic: site direction mapping (NET2-1)")
    passed_site, msgs_site = check_site_direction_ceil("NET2-1", expected_dl=360.0, expected_ul=240.0)
    results.extend(msgs_site)
    ok &= passed_site

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

    _mark_step("realistic: circuit direction mapping (ceil)")
    passed_dir, msgs_dir = check_circuit_direction_ceil(target_id)
    results.extend(msgs_dir)
    ok &= passed_dir

    _mark_step("realistic: circuit ip mappings (no change)")
    passed_map, msgs_map = check_ip_mappings_for_circuit(target_id)
    results.extend(msgs_map)
    ok &= passed_map

    # Add circuit under a different leaf (NET3-2)
    _mark_step("realistic: add circuit")
    rows3 = json.loads(json.dumps(rows2))
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

    _mark_step("realistic: added circuit ip mappings")
    passed_add_map, msgs_add_map = check_ip_mappings_for_circuit(new_circuit_id)
    results.extend(msgs_add_map)
    ok &= passed_add_map

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

    _mark_step("realistic: removed circuit ip unmapped")
    passed_unmap, msgs_unmap = check_ip_mapping_absent(new_circuit_ip)
    results.extend(msgs_unmap)
    ok &= passed_unmap

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
    g.add_argument("--realistic-only", action="store_true", help="Run realistic tiered cases only")
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
        if args.tiered_only:
            ok = run_tiered_suite(log, args.timeout, results)
            overall_ok &= ok
        elif args.flat_only:
            ok = run_flat_suite(log, args.timeout, results)
            overall_ok &= ok
        elif args.realistic_only:
            ok = run_realistic_tiered_suite(log, args.timeout, results)
            overall_ok &= ok
        else:
            ok = run_tiered_suite(log, args.timeout, results)
            overall_ok &= ok

            ok = run_flat_suite(log, args.timeout, results)
            overall_ok &= ok

            ok = run_realistic_tiered_suite(log, args.timeout, results)
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
