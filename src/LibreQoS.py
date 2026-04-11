#!/usr/bin/python3
from pythonCheck import checkPythonVersion
checkPythonVersion()
import csv
import io
import chardet
import ipaddress
import json
import math
import os
import os.path
import subprocess
from subprocess import PIPE, STDOUT
from datetime import datetime, timedelta
import multiprocessing
import warnings
import psutil
import argparse
import logging
import shutil
import time
from deepdiff import DeepDiff

from virtual_tree_nodes import (
    build_logical_to_physical_node_map,
    build_physical_network,
    collect_physical_parent_node_aliases,
    collect_physical_parent_node_ids,
    is_virtual_node,
)
from shaping_skip_report import (
    build_unshaped_device_report,
    collect_parent_node_names,
    device_shaping_key,
    format_unshaped_device_line,
)

from liblqos_python import is_lqosd_alive, clear_ip_mappings, delete_ip_mapping, validate_shaped_devices, \
    is_libre_already_running, create_lock_file, free_lock_file, add_ip_mapping, BatchedCommands, \
    check_config, sqm, upstream_bandwidth_capacity_download_mbps, upstream_bandwidth_capacity_upload_mbps, \
    interface_a, interface_b, enable_actual_shell_commands, use_bin_packing_to_balance_cpu, queue_mode, \
    run_shell_commands_as_sudo, generated_pn_download_mbps, generated_pn_upload_mbps, queues_available_override, \
    on_a_stick, get_tree_weights, get_weights, is_network_flat, get_libreqos_directory, enable_insight_topology, \
    is_insight_enabled, scheduler_error, xdp_ip_mapping_capacity, \
    overrides_circuit_adjustments_effective, \
    automatic_import_uisp, automatic_import_splynx, automatic_import_powercode, automatic_import_sonar, \
    automatic_import_wispgate, automatic_import_netzur, automatic_import_visp, \
    plan_top_level_cpu_bins, topology_import_ingress_enabled, \
    plan_class_identities, \
    fast_queues_fq_codel, \
    shaping_cpu_count, \
    Bakery

# Optional: urgent issue submission (available in newer liblqos_python)
try:
    from liblqos_python import submit_urgent_issue  # type: ignore
except Exception:
    def submit_urgent_issue(*_args, **_kwargs):
        return False

try:
    from liblqos_python import calculate_topology_source_generation  # type: ignore
except Exception:
    def calculate_topology_source_generation():
        return None

try:
    from liblqos_python import get_libreqos_state_directory as _get_state_dir_native
except Exception:
    _get_state_dir_native = None


class RefreshFailure(Exception):
    pass


def report_refresh_failure(code, message, context=None, dedupe_key=None):
    logging.error(message)
    print("ERROR: " + message)
    try:
        scheduler_error(message)
    except Exception:
        pass
    try:
        submit_urgent_issue(
            "LibreQoS",
            "Error",
            code,
            message,
            json.dumps(context) if context is not None else None,
            dedupe_key,
        )
    except Exception:
        pass
    raise RefreshFailure(message)

R2Q = 10
#MAX_R2Q = 200_000
MAX_R2Q = 60_000 # See https://lartc.vger.kernel.narkive.com/NKaH1ZNG/htb-quantum-of-class-100001-is-small-consider-r2q-change
MIN_QUANTUM = 1522

# Gap after each node's circuits for future additions
# Can be overridden by setting CIRCUIT_PADDING in ispConfig.py
# Setting to 0 disables padding (not recommended for production)
# Higher values provide more room for growth but reduce total capacity
try:
    from ispConfig import CIRCUIT_PADDING
except ImportError:
    CIRCUIT_PADDING = 8  # Default value if not configured

def get_shaped_devices_path():
    base_dir = get_libreqos_directory()

    if enable_insight_topology():
        insight_path = os.path.join(base_dir, "ShapedDevices.insight.csv")
        if os.path.exists(insight_path):
            return insight_path

    # Either insight not enabled, or file doesn't exist
    return os.path.join(base_dir, "ShapedDevices.csv")

def get_state_directory():
    if _get_state_dir_native is not None:
        return _get_state_dir_native()
    base_dir = get_libreqos_directory()
    if os.path.basename(base_dir.rstrip("/")) == "src":
        parent = os.path.dirname(base_dir.rstrip("/"))
        if parent:
            return os.path.join(parent, "state")
    return os.path.join(base_dir, "state")


def get_state_path(category, filename):
    return os.path.join(get_state_directory(), category, filename)


def get_existing_state_path(category, filename):
    preferred = get_state_path(category, filename)
    if os.path.exists(preferred):
        return preferred
    return os.path.join(get_libreqos_directory(), filename)


def ensure_parent_dir(path):
    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)


def get_network_json_path():
    base_dir = get_libreqos_directory()
    effective_path = get_existing_state_path("topology", "network.effective.json")

    if os.path.exists(effective_path):
        return effective_path

    if topology_import_ingress_enabled():
        return effective_path

    if enable_insight_topology():
        insight_path = os.path.join(base_dir, "network.insight.json")
        if os.path.exists(insight_path):
            return insight_path

    # Either insight not enabled, or file doesn't exist
    return os.path.join(base_dir, "network.json")


def get_shaping_inputs_path():
    return get_existing_state_path("shaping", "shaping_inputs.json")


def get_circuit_anchors_path():
    return get_existing_state_path("topology", "circuit_anchors.json")


def get_planner_state_path():
    return get_existing_state_path("shaping", "planner_state.json")


def get_topology_runtime_status_path():
    return get_existing_state_path("topology", "topology_runtime_status.json")


def get_queuing_structure_path():
    return get_existing_state_path("shaping", "queuingStructure.json")


def get_last_run_path():
    return get_existing_state_path("stats", "lastRun.txt")


def get_last_good_config_json_path():
    return get_existing_state_path("shaping", "lastGoodConfig.json")


def get_last_good_config_csv_path():
    return get_existing_state_path("shaping", "lastGoodConfig.csv")


def get_last_loaded_shaped_devices_path():
    return get_existing_state_path("shaping", "ShapedDevices.lastLoaded.csv")


def get_stats_by_circuit_path():
    return get_existing_state_path("stats", "statsByCircuit.json")


def get_stats_by_parent_node_path():
    return get_existing_state_path("stats", "statsByParentNode.json")


def get_linux_tc_path():
    return get_state_path("debug", "linux_tc.txt")


def observe_mode_enabled():
    return queue_mode() == "observe"


def _load_json_dict(path):
    try:
        with open(path, "r") as infile:
            data = json.load(infile)
        if isinstance(data, dict):
            return data
    except Exception:
        pass
    return {}


def loaded_network_is_flat(network):
    """Evaluate flat-network mode from one in-memory topology snapshot.

    refreshShapers() should make all topology decisions from the same loaded
    network tree. Re-reading the preferred network path mid-run can observe a
    different runtime-effective snapshot and produce internally inconsistent
    queue builds.
    """
    return not isinstance(network, dict) or len(network) == 0


def _normalize_shaped_devices_header(header_value):
    return ''.join(ch for ch in str(header_value).lower() if ch.isalnum())


_SHAPED_DEVICES_LEGACY_LAYOUT = {
    'circuitID': 0,
    'circuitName': 1,
    'deviceID': 2,
    'deviceName': 3,
    'ParentNode': 4,
    'mac': 5,
    'ipv4_input': 6,
    'ipv6_input': 7,
    'downloadMin': 8,
    'uploadMin': 9,
    'downloadMax': 10,
    'uploadMax': 11,
    'comment': 12,
    'sqm': 13,
}

_SHAPED_DEVICES_HEADER_ALIASES = {
    'circuitID': {'circuitid'},
    'circuitName': {'circuitname'},
    'deviceID': {'deviceid'},
    'deviceName': {'devicename'},
    'ParentNode': {'parentnode'},
    'ParentNodeID': {'parentnodeid'},
    'AnchorNodeID': {'anchornodeid', 'id'},
    'mac': {'mac'},
    'ipv4_input': {'ipv4'},
    'ipv6_input': {'ipv6'},
    'downloadMin': {'downloadmin', 'downloadminmbps'},
    'uploadMin': {'uploadmin', 'uploadminmbps'},
    'downloadMax': {'downloadmax', 'downloadmaxmbps'},
    'uploadMax': {'uploadmax', 'uploadmaxmbps'},
    'comment': {'comment'},
    'sqm': {'sqm'},
}


def _build_shaped_devices_layout(header_row):
    layout = dict(_SHAPED_DEVICES_LEGACY_LAYOUT)
    layout['ParentNodeID'] = None
    layout['AnchorNodeID'] = None
    for idx, header in enumerate(header_row):
        normalized = _normalize_shaped_devices_header(header)
        for field, aliases in _SHAPED_DEVICES_HEADER_ALIASES.items():
            if normalized in aliases:
                layout[field] = idx
                break
    return layout


def _shaped_devices_row_value(row, layout, field):
    idx = layout.get(field)
    if idx is None or idx >= len(row):
        return ''
    return row[idx]


def _resolve_effective_parent_node(circuit, parent_node_ids, parent_node_aliases):
    for candidate_id in (
        circuit.get('ParentNodeID', ''),
        circuit.get('effectiveAttachmentID', ''),
    ):
        parent_node_id = str(candidate_id or '').strip()
        if not parent_node_id:
            continue
        resolved_parent = parent_node_ids.get(parent_node_id)
        if resolved_parent:
            return resolved_parent, parent_node_id

    resolved_parent_id = ''
    for candidate_name in (
        circuit.get('ParentNode', ''),
        circuit.get('effectiveAttachmentName', ''),
        circuit.get('logicalParentNode', ''),
    ):
        parent_node = str(candidate_name or '').strip()
        if not parent_node or parent_node == 'none':
            continue
        resolved_parent = parent_node_aliases.get(parent_node)
        if not resolved_parent:
            continue
        for node_id, node_name in parent_node_ids.items():
            if node_name == resolved_parent:
                resolved_parent_id = node_id
                break
        return resolved_parent, resolved_parent_id

    parent_node = str(circuit.get('ParentNode', '') or '').strip()
    parent_node_id = str(circuit.get('ParentNodeID', '') or '').strip()
    if not parent_node or parent_node == 'none':
        return parent_node, ''
    return parent_node, parent_node_id


def _attachment_lookup_candidates(node_key, node_data):
    candidates = []
    node_id = str(node_data.get('id', '') or '').strip()
    node_name = str(node_data.get('name', '') or '').strip()
    for candidate in (node_id, str(node_key).strip(), node_name):
        if candidate and candidate not in candidates:
            candidates.append(candidate)
    return candidates


def _current_topology_source_generation():
    try:
        generation = calculate_topology_source_generation()
    except Exception:
        return None
    if generation is None:
        return None
    generation = str(generation).strip()
    return generation if generation != '' else None


def _topology_runtime_status_supports_shaping_inputs(shaping_inputs_path):
    if not topology_import_ingress_enabled():
        return False

    current_generation = _current_topology_source_generation()
    if current_generation is None:
        return False

    status_path = get_topology_runtime_status_path()
    try:
        with open(status_path, 'r', encoding='utf-8') as handle:
            status = json.load(handle)
    except Exception:
        return False

    if not isinstance(status, dict):
        return False
    if not bool(status.get('ready')):
        return False

    status_generation = str(status.get('source_generation', '') or '').strip()
    if status_generation != current_generation:
        return False

    status_shaping_inputs_path = str(status.get('shaping_inputs_path', '') or '').strip()
    if status_shaping_inputs_path != '':
        try:
            if os.path.abspath(status_shaping_inputs_path) != os.path.abspath(shaping_inputs_path):
                return False
        except Exception:
            return False

    return True


def _shaping_inputs_are_fresh(shaping_inputs_path, shaped_devices_file, network_json_file, circuit_anchors_file=None):
    if not os.path.isfile(shaping_inputs_path):
        return False
    if _topology_runtime_status_supports_shaping_inputs(shaping_inputs_path):
        return True
    try:
        shaping_inputs_mtime = os.path.getmtime(shaping_inputs_path)
        if os.path.isfile(shaped_devices_file) and shaping_inputs_mtime < os.path.getmtime(shaped_devices_file):
            return False
        if os.path.isfile(network_json_file) and shaping_inputs_mtime < os.path.getmtime(network_json_file):
            return False
        if circuit_anchors_file and os.path.isfile(circuit_anchors_file) and shaping_inputs_mtime < os.path.getmtime(circuit_anchors_file):
            return False
        return True
    except OSError:
        return False


def loadSubscriberCircuitsFromShapingInputs(shapingInputsPath):
    payload = _load_json_dict(shapingInputsPath)
    circuits = payload.get('circuits', [])
    if not isinstance(circuits, list):
        raise ValueError(f"Invalid shaping_inputs.json at {shapingInputsPath}: circuits must be a list")

    subscriberCircuits = []
    dictForCircuitsWithoutParentNodes = {}
    counterForCircuitsWithoutParentNodes = 0
    warnings_from_file = payload.get('warnings', [])
    if isinstance(warnings_from_file, list):
        for warning_text in warnings_from_file:
            if isinstance(warning_text, str) and warning_text.strip():
                warnings.warn(f"shaping_inputs.json: {warning_text}", stacklevel=2)

    for circuit in circuits:
        if not isinstance(circuit, dict):
            continue
        circuitID = str(circuit.get('circuit_id', '') or '').strip()
        if circuitID == '':
            raise ValueError("Missing circuit_id in shaping_inputs.json")
        parent_node = str(circuit.get('effective_parent_node_name', '') or '').strip()
        parent_node_id = str(circuit.get('effective_parent_node_id', '') or '').strip()
        logical_parent_node = str(circuit.get('logical_parent_node_name', '') or '').strip()
        logical_parent_node_id = str(circuit.get('logical_parent_node_id', '') or '').strip()
        anchor_node_id = str(circuit.get('anchor_node_id', '') or '').strip()
        devices = []
        for device in circuit.get('devices', []):
            if not isinstance(device, dict):
                continue
            devices.append(
                {
                    "deviceID": str(device.get('device_id', '') or '').strip(),
                    "deviceName": str(device.get('device_name', '') or '').strip(),
                    "mac": str(device.get('mac', '') or '').strip(),
                    "ipv4s": [str(entry).strip() for entry in device.get('ipv4', []) if str(entry).strip() != ''],
                    "ipv6s": [str(entry).strip() for entry in device.get('ipv6', []) if str(entry).strip() != ''],
                    "comment": str(device.get('comment', '') or ''),
                }
            )
        thisCircuit = {
            "circuitID": circuitID,
            "circuitName": str(circuit.get('circuit_name', '') or ''),
            "ParentNode": parent_node if parent_node != '' else 'none',
            "ParentNodeID": parent_node_id,
            "AnchorNodeID": anchor_node_id,
            "devices": devices,
            "minDownload": float(circuit.get('download_min_mbps', 0.0) or 0.0),
            "minUpload": float(circuit.get('upload_min_mbps', 0.0) or 0.0),
            "maxDownload": float(circuit.get('download_max_mbps', 0.0) or 0.0),
            "maxUpload": float(circuit.get('upload_max_mbps', 0.0) or 0.0),
            "classid": '',
            "comment": str(circuit.get('comment', '') or ''),
            "logicalParentNode": logical_parent_node if logical_parent_node != '' else (parent_node if parent_node != '' else 'none'),
            "logicalParentNodeID": logical_parent_node_id,
            "effectiveAttachmentID": str(circuit.get('effective_attachment_id', '') or '').strip(),
            "effectiveAttachmentName": str(circuit.get('effective_attachment_name', '') or '').strip(),
            "parentResolutionSource": str(circuit.get('resolution_source', '') or '').strip(),
            "parentResolvedByShapingInputs": True,
        }
        sqm_override = normalize_sqm_override_token(str(circuit.get('sqm_override', '') or ''))
        if sqm_override != '':
            thisCircuit['sqm'] = sqm_override
        if thisCircuit['ParentNode'] == 'none':
            thisCircuit['idForCircuitsWithoutParentNodes'] = counterForCircuitsWithoutParentNodes
            dictForCircuitsWithoutParentNodes[counterForCircuitsWithoutParentNodes] = (
                thisCircuit['maxDownload'] + thisCircuit['maxUpload']
            )
            counterForCircuitsWithoutParentNodes += 1
        subscriberCircuits.append(thisCircuit)

    return subscriberCircuits, dictForCircuitsWithoutParentNodes


def loadSubscriberCircuitsForShaping(shapedDevicesFile, networkJSONfile):
    shaping_inputs_path = get_shaping_inputs_path()
    circuit_anchors_path = get_circuit_anchors_path()
    if _shaping_inputs_are_fresh(shaping_inputs_path, shapedDevicesFile, networkJSONfile, circuit_anchors_path):
        try:
            subscriberCircuits, dictForCircuitsWithoutParentNodes = loadSubscriberCircuitsFromShapingInputs(shaping_inputs_path)
            print("Loaded shaping inputs from " + shaping_inputs_path)
            return subscriberCircuits, dictForCircuitsWithoutParentNodes
        except Exception as e:
            raise RefreshFailure(
                f"Unable to load required shaping_inputs.json at {shaping_inputs_path}: {e}"
            ) from e
    raise RefreshFailure(
        "Missing or stale shaping_inputs.json. Run topology runtime before shaping."
    )


def load_planner_state(state_path=None, planner_module=None):
    if state_path is None:
        state_path = get_planner_state_path()
    if planner_module is not None:
        try:
            state = planner_module.load_state(state_path)
            if isinstance(state, dict):
                return sanitize_planner_state(state)
        except Exception:
            pass
    return sanitize_planner_state(_load_json_dict(state_path))


def save_planner_state(state, state_path=None, planner_module=None):
    if state_path is None:
        state_path = get_planner_state_path()
    if planner_module is not None:
        planner_module.save_state(state_path, state)
        return

    parent = os.path.dirname(state_path)
    if parent:
        os.makedirs(parent, exist_ok=True)
    temp_path = state_path + ".tmp"
    with open(temp_path, "w") as outfile:
        json.dump(state, outfile, indent=2, sort_keys=True)
    os.replace(temp_path, state_path)


def sanitize_planner_state(state):
    if not isinstance(state, dict):
        return {}

    sanitized = False

    def _sanitize_entry(entry):
        nonlocal sanitized
        if not isinstance(entry, dict):
            return

        raw_minor = None
        for key in ('class_minor', 'minor', 'classMinor'):
            if key in entry:
                raw_minor = _parse_int_token(entry.get(key))
                if raw_minor is not None:
                    break

        # TC minor 0xffff is not safe for leaf classes here. If stale planner
        # state reuses it, Bakery later emits qdisc parents like 1:ffff and the
        # kernel rejects them. Drop the stored identity and let the planner
        # allocate a new minor on this run.
        if raw_minor is not None and (raw_minor < 3 or raw_minor >= 0xFFFF):
            for key in ('class_minor', 'minor', 'classMinor', 'class_major', 'up_class_major'):
                if key in entry:
                    del entry[key]
            sanitized = True

        for value in entry.values():
            if isinstance(value, dict):
                _sanitize_entry(value)
            elif isinstance(value, list):
                for item in value:
                    if isinstance(item, dict):
                        _sanitize_entry(item)

    state_copy = json.loads(json.dumps(state))
    for section in ('sites', 'circuits'):
        section_value = state_copy.get(section)
        if isinstance(section_value, dict):
            _sanitize_entry(section_value)

    if sanitized:
        logging.warning(
            "Sanitized invalid planner state identities; removed reserved or out-of-range minor assignments before planning."
        )

    return state_copy


def _parse_int_token(value):
    if value is None:
        return None
    try:
        if isinstance(value, int):
            return value
        token = str(value).strip()
        if not token:
            return None
        if token.lower().startswith("0x"):
            return int(token, 16)
        return int(token)
    except Exception:
        return None


def is_generated_parent_node_name(node_name):
    return isinstance(node_name, str) and node_name.startswith("Generated_PN_")


def generated_parent_node_queue_key(node_name, queues_available):
    if not is_generated_parent_node_name(node_name) or queues_available <= 0:
        return None
    suffix = _parse_int_token(str(node_name).rsplit("_", 1)[-1])
    if suffix is None or suffix <= 0:
        return None
    return "CpueQueue" + str((suffix - 1) % queues_available)


def planner_circuit_identity_key(circuit):
    circuit_id = str(circuit.get("circuitID", "") or "").strip()
    if not circuit_id:
        raise ValueError("Missing circuitID is unsupported for planner identity")
    return circuit_id


def load_minor_state_from_queuing_structure(path=None):
    if path is None:
        path = get_queuing_structure_path()
    data = _load_json_dict(path)
    network = data.get("Network")
    if not isinstance(network, dict):
        return {"sites": {}, "circuits": {}}

    sites = {}
    circuits = {}

    def walk(node_map, trail=()):
        for node_name, node in sorted(node_map.items()):
            if not isinstance(node, dict):
                continue
            node_path = trail + (node_name,)
            site_key = "/".join(node_path)
            parent_path = "/".join(trail)
            queue = _parse_int_token(node.get("cpuNum"))
            class_minor = _parse_int_token(node.get("classMinor"))
            class_major = _parse_int_token(node.get("classMajor"))
            up_class_major = _parse_int_token(node.get("up_classMajor"))
            if queue is not None and class_minor is not None:
                sites[site_key] = {
                    "class_minor": class_minor,
                    "queue": queue + 1,
                    "parent_path": parent_path,
                    "class_major": class_major,
                    "up_class_major": up_class_major,
                }

            if isinstance(node.get("circuits"), list):
                for circuit in node.get("circuits", []):
                    if not isinstance(circuit, dict):
                        continue
                    circuit_id = circuit.get("circuitID")
                    circuit_minor = _parse_int_token(circuit.get("classMinor"))
                    if queue is None or circuit_id is None or circuit_minor is None:
                        continue
                    circuits[str(circuit_id)] = {
                        "class_minor": circuit_minor,
                        "queue": queue + 1,
                        "parent_node": circuit.get("ParentNode", node_name),
                        "class_major": _parse_int_token(circuit.get("classMajor")),
                        "up_class_major": _parse_int_token(circuit.get("up_classMajor")),
                    }

            children = node.get("children")
            if isinstance(children, dict):
                walk(children, node_path)

    walk(network)
    return {"sites": sites, "circuits": circuits}

def calculateR2q(maxRateInMbps):
    # So we've learned that r2q defaults to 10, and is used to calculate quantum. Quantum is rateInBytes/r2q by
    # default. This default gives errors at high rates, and tc clamps the quantum to 200000. Setting a high quantum
    # directly gives no errors. So we want to calculate r2q to default to 10, but not exceed 200000 for the highest
    # specified rate (which will be the available bandwidth rate).
    maxRateInBytesPerSecond = maxRateInMbps * 125000
    r2q = 10
    quantum = maxRateInBytesPerSecond / r2q
    while quantum > MAX_R2Q:
        r2q += 1
        quantum = maxRateInBytesPerSecond / r2q
    global R2Q
    R2Q = r2q

def quantum(rateInMbps):
    # Attempt to calculate an appropriate quantum for an HTB queue, given
    # that `mq` does not appear to carry a valid `r2q` value to individual
    # root nodes.
    rateInBytesPerSecond = rateInMbps * 125000
    quantum = max(MIN_QUANTUM, int(rateInBytesPerSecond / R2Q))
    #print("R2Q=" + str(R2Q) + ", quantum: " + str(quantum))
    quantrumString = " quantum " + str(quantum)
    return quantrumString

def format_rate_for_tc(rate_mbps):
    """
    Format a rate in Mbps for TC commands with smart unit selection.
    - Rates >= 1000 Mbps use 'gbit'
    - Rates >= 1 Mbps use 'mbit'
    - Rates < 1 Mbps use 'kbit'
    """
    if rate_mbps >= 1000:
        return f"{rate_mbps/1000:.1f}gbit"
    elif rate_mbps >= 1:
        return f"{rate_mbps:.1f}mbit"
    else:
        return f"{rate_mbps*1000:.0f}kbit"

def shell(command):
    if enable_actual_shell_commands():
        if run_shell_commands_as_sudo():
            command = 'sudo ' + command
        logging.info(command)
        commands = command.split(' ')
        proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
        for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
            if logging.DEBUG <= logging.root.level:
                print(line)
            if ("RTNETLINK answers" in line) or ("We have an error talking to the kernel" in line):
                warnings.warn("Command: '" + command + "' resulted in " + line, stacklevel=2)
    else:
        logging.info(command)

def shellReturn(command):
    returnableString = ''
    if enable_actual_shell_commands():
        commands = command.split(' ')
        proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
        for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
            returnableString = returnableString + line + '\n'
    return returnableString

def checkIfFirstRunSinceBoot():
    last_run_path = get_last_run_path()
    if os.path.isfile(last_run_path):
        with open(last_run_path, 'r') as file:
            lastRun = datetime.strptime(file.read(), "%d-%b-%Y (%H:%M:%S.%f)")
        systemRunningSince = datetime.fromtimestamp(psutil.boot_time())
        if systemRunningSince > lastRun:
            print("First time run since system boot.")
            return True
        else:
            print("Not first time run since system boot.")
            return False
    else:
        print("First time run since system boot.")
        return True

def clearPriorSettings(interfaceA, interfaceB):
    if enable_actual_shell_commands():
        if 'mq' in shellReturn('tc qdisc show dev ' + interfaceA + ' root'):
            print('MQ detected. Will delete and recreate mq qdisc.')
            # Clear tc filter
            if on_a_stick() == True:
                shell('tc qdisc delete dev ' + interfaceA + ' root')
            else:
                shell('tc qdisc delete dev ' + interfaceA + ' root')
                shell('tc qdisc delete dev ' + interfaceB + ' root')

def tearDown(interfaceA, interfaceB):
    # Full teardown of everything for exiting LibreQoS
    if enable_actual_shell_commands():
        # Clear IP filters and remove xdp program from interfaces
        # The bakery tracks and prunes mappings; avoid clearing everything here.
        clearPriorSettings(interfaceA, interfaceB)

def findQueuesAvailable(interfaceName):
    # Find queues and CPU cores available. Use min between those two as queuesAvailable
    if enable_actual_shell_commands():
        if queues_available_override() == 0:
            queuesAvailable = 0
            path = '/sys/class/net/' + interfaceName + '/queues/'
            directory_contents = os.listdir(path)
            for item in directory_contents:
                if "tx-" in str(item):
                    queuesAvailable += 1
            print(f"Interface {interfaceName} NIC queues:\t\t\t" + str(queuesAvailable))
        else:
            queuesAvailable = queues_available_override()
            print(f"Interface {interfaceName} NIC queues (Override):\t\t\t" + str(queuesAvailable))
        try:
            cpuCount = shaping_cpu_count()
        except Exception:
            cpuCount = multiprocessing.cpu_count()
        print("CPU cores:\t\t\t" + str(cpuCount))
        if queuesAvailable < 2:
            raise SystemError(f'Only 1 NIC rx/tx queue available for interface {interfaceName}. You will need to use a NIC with 2 or more rx/tx queues available.')
        if cpuCount < 2:
            raise SystemError('Only 1 CPU core available. You will need to use a CPU with 2 or more CPU cores.')
        queuesAvailable = min(queuesAvailable,cpuCount)
        print(f"queuesAvailable for interface {interfaceName} set to:\t" + str(queuesAvailable))
    else:
        print("As enableActualShellCommands is False, CPU core / queue count has been set to 16")
        logging.info(f"Interface {interfaceName} NIC queues:\t\t\t" + str(16))
        cpuCount = multiprocessing.cpu_count()
        logging.info("CPU cores:\t\t\t" + str(16))
        logging.info(f"queuesAvailable for interface {interfaceName} set to:\t" + str(16))
        queuesAvailable = 16
    return queuesAvailable

def validateNetworkAndDevices():
    # Verify Network.json is valid json
    networkValidatedOrNot = True
    # Verify ShapedDevices.csv is valid
    devicesValidatedOrNot = True # True by default, switches to false if ANY entry in ShapedDevices.csv fails validation
    integration_ingress = topology_import_ingress_enabled()

    # Verify that the Rust side of things can read the active shaped-device ingress
    rustValid = validate_shaped_devices()
    if rustValid == "OK":
        if integration_ingress:
            print("Rust validated integration shaping ingress")
        else:
            print("Rust validated ShapedDevices.csv")
    else:
        if integration_ingress:
            warnings.warn("Rust failed to validate integration shaping ingress", stacklevel=2)
        else:
            warnings.warn("Rust failed to validate ShapedDevices.csv", stacklevel=2)
        warnings.warn(rustValid, stacklevel=2)
        devicesValidatedOrNot = False
    with open(get_network_json_path()) as file:
        try:
            data = json.load(file) # put JSON-data to a variable
            if data != {}:
                #Traverse
                observedNodes = set()
                duplicateNodes = set()
                def traverseToVerifyValidity(data):
                    for elem in data:
                        if isinstance(elem, str):
                            if (isinstance(data[elem], dict)) and (elem != 'children'):
                                if elem not in observedNodes:
                                    observedNodes.add(elem)
                                    if 'children' in data[elem]:
                                        traverseToVerifyValidity(data[elem]['children'])
                                else:
                                    duplicateNodes.add(elem)
                traverseToVerifyValidity(data)
                if len(duplicateNodes) > 0:
                    for elem in sorted(duplicateNodes):
                        warnings.warn("Non-unique Node name in network.json: " + elem, stacklevel=2)
                    networkValidatedOrNot = False
                if len(observedNodes) < 1:
                    warnings.warn("network.json had 0 valid nodes. Only {} is accepted for that scenario.", stacklevel=2)
                    networkValidatedOrNot = False
        except json.decoder.JSONDecodeError:
            warnings.warn("network.json is an invalid JSON file", stacklevel=2) # in case json is invalid
            networkValidatedOrNot = False
    if integration_ingress:
        if devicesValidatedOrNot == True:
            print("integration shaping ingress passed validation")
        else:
            print("integration shaping ingress failed validation")
        if networkValidatedOrNot == True:
            print("network.json passed validation")
        else:
            print("network.json failed validation")
        return devicesValidatedOrNot and networkValidatedOrNot

    rowNum = 2

    # Handle non-utf8 encoding in ShapedDevices.csv
    with open(get_shaped_devices_path(), 'rb') as f:
        raw_bytes = f.read()

    # Handle BOM if present
    if raw_bytes.startswith(b'\xef\xbb\xbf'):  # UTF-8 BOM
        raw_bytes = raw_bytes[3:]
        text_content = raw_bytes.decode('utf-8')
    elif raw_bytes.startswith(b'\xff\xfe'):  # UTF-16 LE BOM
        text_content = raw_bytes.decode('utf-16')
    elif raw_bytes.startswith(b'\xfe\xff'):  # UTF-16 BE BOM
        text_content = raw_bytes.decode('utf-16')
    else:
        # Try UTF-8 first
        try:
            text_content = raw_bytes.decode('utf-8')
        except UnicodeDecodeError:
            # Detect encoding
            detected = chardet.detect(raw_bytes)
            encoding = detected['encoding'] or 'utf-8'
            text_content = raw_bytes.decode(encoding, errors='replace')

    # Create a StringIO object to mimic a file
    # And read from the sanitized byte stream
    with io.StringIO(text_content) as csv_file:
        csv_reader = csv.reader(csv_file, delimiter=',')
        header_consumed = False
        layout = None
        seenTheseIPsAlready = set()
        for row in csv_reader:
            if not row:
                continue
            if row[0].startswith('#'):
                continue
            if not header_consumed:
                header_consumed = True
                layout = _build_shaped_devices_layout(row)
                continue
            # Accept optional Anchor Node ID / Parent Node ID columns and trailing
            # 'sqm' override columns while validating the core shaping inputs.
            circuitID = _shaped_devices_row_value(row, layout, 'circuitID')
            circuitName = _shaped_devices_row_value(row, layout, 'circuitName')
            deviceID = _shaped_devices_row_value(row, layout, 'deviceID')
            deviceName = _shaped_devices_row_value(row, layout, 'deviceName')
            ParentNode = _shaped_devices_row_value(row, layout, 'ParentNode')
            mac = _shaped_devices_row_value(row, layout, 'mac')
            ipv4_input = _shaped_devices_row_value(row, layout, 'ipv4_input')
            ipv6_input = _shaped_devices_row_value(row, layout, 'ipv6_input')
            downloadMin = _shaped_devices_row_value(row, layout, 'downloadMin')
            uploadMin = _shaped_devices_row_value(row, layout, 'uploadMin')
            downloadMax = _shaped_devices_row_value(row, layout, 'downloadMax')
            uploadMax = _shaped_devices_row_value(row, layout, 'uploadMax')
            comment = _shaped_devices_row_value(row, layout, 'comment')
            # Must have circuitID, it's a unique identifier required for stateful changes to queue structure
            if circuitID == '':
                warnings.warn("No Circuit ID provided in ShapedDevices.csv at row " + str(rowNum), stacklevel=2)
                devicesValidatedOrNot = False
            # Each entry in ShapedDevices.csv can have multiple IPv4s or IPv6s separated by commas. Split them up and parse each to ensure valid
            ipv4_subnets_and_hosts = []
            ipv6_subnets_and_hosts = []
            if ipv4_input != "":
                try:
                    ipv4_input = ipv4_input.replace(' ','')
                    if "," in ipv4_input:
                        ipv4_list = ipv4_input.split(',')
                    else:
                        ipv4_list = [ipv4_input]
                    for ipEntry in ipv4_list:
                        if ipEntry in seenTheseIPsAlready:
                            warnings.warn("Provided IPv4 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is duplicate.", stacklevel=2)
                            #devicesValidatedOrNot = False
                            seenTheseIPsAlready.add(ipEntry)
                        else:
                            if (type(ipaddress.ip_network(ipEntry)) is ipaddress.IPv4Network) or (type(ipaddress.ip_address(ipEntry)) is ipaddress.IPv4Address):
                                ipv4_subnets_and_hosts.extend(ipEntry)
                            else:
                                warnings.warn("Provided IPv4 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
                                devicesValidatedOrNot = False
                            seenTheseIPsAlready.add(ipEntry)
                except:
                        warnings.warn("Provided IPv4 '" + ipv4_input + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
                        devicesValidatedOrNot = False
            if ipv6_input != "":
                try:
                    ipv6_input = ipv6_input.replace(' ','')
                    if "," in ipv6_input:
                        ipv6_list = ipv6_input.split(',')
                    else:
                        ipv6_list = [ipv6_input]
                    for ipEntry in ipv6_list:
                        if ipEntry in seenTheseIPsAlready:
                            warnings.warn("Provided IPv6 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is duplicate.", stacklevel=2)
                            devicesValidatedOrNot = False
                            seenTheseIPsAlready.add(ipEntry)
                        else:
                            if (type(ipaddress.ip_network(ipEntry)) is ipaddress.IPv6Network) or (type(ipaddress.ip_address(ipEntry)) is ipaddress.IPv6Address):
                                ipv6_subnets_and_hosts.extend(ipEntry)
                            else:
                                warnings.warn("Provided IPv6 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
                                devicesValidatedOrNot = False
                            seenTheseIPsAlready.add(ipEntry)
                except:
                        warnings.warn("Provided IPv6 '" + ipv6_input + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
                        devicesValidatedOrNot = False
            try:
                a = float(downloadMin)
                if a < 0.1:
                    warnings.warn("Provided downloadMin '" + downloadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is < 0.1 Mbps.", stacklevel=2)
                    devicesValidatedOrNot = False
            except:
                warnings.warn("Provided downloadMin '" + downloadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is not a valid number.", stacklevel=2)
                devicesValidatedOrNot = False
            try:
                a = float(uploadMin)
                if a < 0.1:
                    warnings.warn("Provided uploadMin '" + uploadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is < 0.1 Mbps.", stacklevel=2)
                    devicesValidatedOrNot = False
            except:
                warnings.warn("Provided uploadMin '" + uploadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is not a valid number.", stacklevel=2)
                devicesValidatedOrNot = False
            try:
                a = float(downloadMax)
                if a < 0.1:
                    warnings.warn("Provided downloadMax '" + downloadMax + "' in ShapedDevices.csv at row " + str(rowNum) + " is < 0.2 Mbps.", stacklevel=2)
                    devicesValidatedOrNot = False
            except:
                warnings.warn("Provided downloadMax '" + downloadMax + "' in ShapedDevices.csv at row " + str(rowNum) + " is not a valid number.", stacklevel=2)
                devicesValidatedOrNot = False
            try:
                a = float(uploadMax)
                if a < 0.1:
                    warnings.warn("Provided uploadMax '" + uploadMax + "' in ShapedDevices.csv at row " + str(rowNum) + " is < 0.2 Mbps.", stacklevel=2)
                    devicesValidatedOrNot = False
            except:
                warnings.warn("Provided uploadMax '" + uploadMax + "' in ShapedDevices.csv at row " + str(rowNum) + " is not a valid number.", stacklevel=2)
                devicesValidatedOrNot = False

            try:
                if float(downloadMin) > float(downloadMax):
                    warnings.warn("Provided downloadMin '" + downloadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is greater than downloadMax", stacklevel=2)
                    devicesValidatedOrNot = False
                if float(uploadMin) > float(uploadMax):
                    warnings.warn("Provided uploadMin '" + uploadMin + "' in ShapedDevices.csv at row " + str(rowNum) + " is greater than uploadMax", stacklevel=2)
                    devicesValidatedOrNot = False
            except:
                devicesValidatedOrNot = False

            rowNum += 1
    if devicesValidatedOrNot == True:
        print("ShapedDevices.csv passed validation")
    else:
        print("ShapedDevices.csv failed validation")
    if networkValidatedOrNot == True:
        print("network.json passed validation")
    else:
        print("network.json failed validation")
    if (devicesValidatedOrNot == True) and (networkValidatedOrNot == True):
        return True
    else:
        return False

def loadSubscriberCircuits(shapedDevicesFile):
    # Load Subscriber Circuits & Devices
    subscriberCircuits = []
    circuitsById = {}
    counterForCircuitsWithoutParentNodes = 0
    dictForCircuitsWithoutParentNodes = {}
    with open(shapedDevicesFile) as csv_file:
        csv_reader = csv.reader(csv_file, delimiter=',')
        header_consumed = False
        layout = None
        for row in csv_reader:
            if not row:
                continue
            if row[0].startswith('#'):
                continue
            if not header_consumed:
                header_consumed = True
                layout = _build_shaped_devices_layout(row)
                continue
            # Optional per-circuit SQM override in last column
            sqm_override_token = ''
            if layout is not None:
                raw_token = _shaped_devices_row_value(row, layout, 'sqm')
            else:
                raw_token = ''
            if raw_token != '':
                # Normalize: lowercase, trim, collapse spaces around '/'
                token = raw_token.strip().lower()
                if '/' in token:
                    parts = token.split('/', 1)
                    left = parts[0].strip()
                    right = parts[1].strip()
                    token = left + '/' + right
                sqm_override_token = token
            circuitID = _shaped_devices_row_value(row, layout, 'circuitID')
            circuitName = _shaped_devices_row_value(row, layout, 'circuitName')
            deviceID = _shaped_devices_row_value(row, layout, 'deviceID')
            deviceName = _shaped_devices_row_value(row, layout, 'deviceName')
            ParentNode = _shaped_devices_row_value(row, layout, 'ParentNode')
            ParentNodeID = _shaped_devices_row_value(row, layout, 'ParentNodeID')
            AnchorNodeID = _shaped_devices_row_value(row, layout, 'AnchorNodeID')
            mac = _shaped_devices_row_value(row, layout, 'mac')
            ipv4_input = _shaped_devices_row_value(row, layout, 'ipv4_input')
            ipv6_input = _shaped_devices_row_value(row, layout, 'ipv6_input')
            downloadMin = _shaped_devices_row_value(row, layout, 'downloadMin')
            uploadMin = _shaped_devices_row_value(row, layout, 'uploadMin')
            downloadMax = _shaped_devices_row_value(row, layout, 'downloadMax')
            uploadMax = _shaped_devices_row_value(row, layout, 'uploadMax')
            comment = _shaped_devices_row_value(row, layout, 'comment')
            ipv4_subnets_and_hosts = []
            # Each entry in ShapedDevices.csv can have multiple IPv4s or IPv6s separated by commas. Split them up and parse each
            if ipv4_input != "":
                ipv4_input = ipv4_input.replace(' ','')
                if "," in ipv4_input:
                    ipv4_list = ipv4_input.split(',')
                else:
                    ipv4_list = [ipv4_input]
                for ipEntry in ipv4_list:
                    ipv4_subnets_and_hosts.append(ipEntry)
            ipv6_subnets_and_hosts = []
            if ipv6_input != "":
                ipv6_input = ipv6_input.replace(' ','')
                if "," in ipv6_input:
                    ipv6_list = ipv6_input.split(',')
                else:
                    ipv6_list = [ipv6_input]
                for ipEntry in ipv6_list:
                    ipv6_subnets_and_hosts.append(ipEntry)
            # If there is something in the circuit ID field
            if circuitID != "":
                # Seen circuit before
                circuit = circuitsById.get(circuitID)
                if circuit is not None:
                    if circuit['ParentNode'] != "none":
                        if circuit['ParentNode'] != ParentNode:
                            errorMessageString = "Device " + deviceName + " with deviceID " + deviceID + " had different Parent Node from other devices of circuit ID #" + circuitID
                            raise ValueError(errorMessageString)
                        existing_parent_node_id = str(circuit.get('ParentNodeID', '') or '').strip()
                        candidate_parent_node_id = str(ParentNodeID or '').strip()
                        if existing_parent_node_id and candidate_parent_node_id and existing_parent_node_id != candidate_parent_node_id:
                            errorMessageString = "Device " + deviceName + " with deviceID " + deviceID + " had different Parent Node ID from other devices of circuit ID #" + circuitID
                            raise ValueError(errorMessageString)
                        if not existing_parent_node_id and candidate_parent_node_id:
                            circuit['ParentNodeID'] = candidate_parent_node_id
                        existing_anchor_node_id = str(circuit.get('AnchorNodeID', '') or '').strip()
                        candidate_anchor_node_id = str(AnchorNodeID or '').strip()
                        if existing_anchor_node_id and candidate_anchor_node_id and existing_anchor_node_id != candidate_anchor_node_id:
                            errorMessageString = "Device " + deviceName + " with deviceID " + deviceID + " had different Anchor Node ID from other devices of circuit ID #" + circuitID
                            raise ValueError(errorMessageString)
                        if not existing_anchor_node_id and candidate_anchor_node_id:
                            circuit['AnchorNodeID'] = candidate_anchor_node_id
                    if ((circuit['minDownload'] != float(downloadMin))
                        or (circuit['minUpload'] != float(uploadMin))
                        or (circuit['maxDownload'] != float(downloadMax))
                        or (circuit['maxUpload'] != float(uploadMax))):
                        warnings.warn("Device " + deviceName + " with ID " + deviceID + " had different bandwidth parameters than other devices on this circuit. Will instead use the bandwidth parameters defined by the first device added to its circuit.", stacklevel=2)
                    # If this row specifies an SQM override, but the circuit already has a different one, warn and keep the first.
                    if sqm_override_token != '':
                        if 'sqm' in circuit:
                            if circuit['sqm'] != sqm_override_token:
                                warnings.warn("Device " + deviceName + " with ID " + deviceID + " had different SQM override than other devices on this circuit. Will instead use the SQM defined by the first device added to its circuit.", stacklevel=2)
                        else:
                            circuit['sqm'] = sqm_override_token
                    devicesListForCircuit = circuit['devices']
                    thisDevice = 	{
                                      "deviceID": deviceID,
                                      "deviceName": deviceName,
                                      "mac": mac,
                                      "ipv4s": ipv4_subnets_and_hosts,
                                      "ipv6s": ipv6_subnets_and_hosts,
                                      "comment": comment
                                    }
                    devicesListForCircuit.append(thisDevice)
                    circuit['devices'] = devicesListForCircuit
                # Have not seen circuit before
                else:
                    if ParentNode == "":
                        ParentNode = "none"
                    #ParentNode = ParentNode.strip()
                    deviceListForCircuit = []
                    thisDevice = 	{
                                      "deviceID": deviceID,
                                      "deviceName": deviceName,
                                      "mac": mac,
                                      "ipv4s": ipv4_subnets_and_hosts,
                                      "ipv6s": ipv6_subnets_and_hosts,
                                      "comment": comment
                                    }
                    deviceListForCircuit.append(thisDevice)
                    thisCircuit = {
                      "circuitID": circuitID,
                      "circuitName": circuitName,
                      "ParentNode": ParentNode,
                      "ParentNodeID": str(ParentNodeID or '').strip(),
                      "AnchorNodeID": str(AnchorNodeID or '').strip(),
                      "devices": deviceListForCircuit,
                      "minDownload": float(downloadMin),
                      "minUpload": float(uploadMin),
                      "maxDownload": float(downloadMax),
                      "maxUpload": float(uploadMax),
                      "classid": '',
                      "comment": comment
                    }
                    if sqm_override_token != '':
                        thisCircuit['sqm'] = sqm_override_token
                    if thisCircuit['ParentNode'] == 'none':
                        thisCircuit['idForCircuitsWithoutParentNodes'] = counterForCircuitsWithoutParentNodes
                        dictForCircuitsWithoutParentNodes[counterForCircuitsWithoutParentNodes] = ((float(downloadMax))+(float(uploadMax)))
                        counterForCircuitsWithoutParentNodes += 1
                    subscriberCircuits.append(thisCircuit)
                    circuitsById[circuitID] = thisCircuit
            else:
                raise ValueError(
                    "Missing circuitID is unsupported in ShapedDevices.csv "
                    f"(deviceID={deviceID}, deviceName={deviceName}, parent={ParentNode})"
                )
    return (subscriberCircuits,	dictForCircuitsWithoutParentNodes)


def normalize_sqm_override_token(raw_token):
    token = (raw_token or '').strip().lower()
    if token == '':
        return ''
    if '/' in token:
        left, right = token.split('/', 1)
        token = left.strip() + '/' + right.strip()
    return token


def apply_effective_runtime_circuit_overrides(subscriberCircuits):
    """
    Overlay adaptive runtime circuit adjustments in memory without mutating
    ShapedDevices.csv, so rebuilds can honor TreeGuard/StormGuard state.
    """
    try:
        adjustments = overrides_circuit_adjustments_effective()
    except Exception as e:
        warnings.warn(f"Unable to load effective runtime circuit overrides: {e}", stacklevel=2)
        return 0

    sqm_by_device_id = {}
    for adj in adjustments:
        if adj.get('type') != 'device_adjust_sqm':
            continue
        device_id = (adj.get('device_id') or '').strip()
        sqm_override = normalize_sqm_override_token(adj.get('sqm_override'))
        if device_id == '' or sqm_override == '':
            continue
        sqm_by_device_id[device_id] = sqm_override

    if not sqm_by_device_id:
        return 0

    overlay_count = 0
    for circuit in subscriberCircuits:
        circuit_override = None
        for device in circuit.get('devices', []):
            override = sqm_by_device_id.get(device.get('deviceID', ''))
            if override is None:
                continue
            if circuit_override is not None and circuit_override != override:
                warnings.warn(
                    "Effective runtime SQM override conflict on circuit "
                    + circuit.get('circuitID', 'unknown')
                    + ". Will instead use the first runtime SQM override discovered.",
                    stacklevel=2,
                )
                continue
            circuit_override = override

        if circuit_override is None:
            continue
        if circuit.get('sqm') != circuit_override:
            overlay_count += 1
        circuit['sqm'] = circuit_override

    return overlay_count

def refreshShapers():

    # Starting
    print("refreshShapers starting at " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))
    observe_mode = observe_mode_enabled()
    # Create a single batch of xdp update commands to execute together
    ipMapBatch = BatchedCommands()
    requiredIpMappings = 0

    # Warn user if enableActualShellCommands is False, because that would mean no actual commands are executing
    if enable_actual_shell_commands() == False:
        warnings.warn("enableActualShellCommands is set to False. None of the commands below will actually be executed. Simulated run.", stacklevel=2)
    if observe_mode:
        warnings.warn(
            "queue_mode is set to observe. LibreQoS will keep the root MQ and remove the shaping tree for a true baseline. "
            "Switching between Observe and Shape may briefly interrupt traffic.",
            stacklevel=2,
        )


    # Check if first run since boot
    isThisFirstRunSinceBoot = checkIfFirstRunSinceBoot()


    # Files
    shapedDevicesFile = get_shaped_devices_path()
    networkJSONfile = get_network_json_path()


    # Check validation
    safeToRunRefresh = False
    print("Validating input files '" + shapedDevicesFile + "' and '" + networkJSONfile + "'")
    if (validateNetworkAndDevices() == True):
        if os.path.isfile(shapedDevicesFile):
            last_good_csv_path = get_state_path("shaping", "lastGoodConfig.csv")
            ensure_parent_dir(last_good_csv_path)
            shutil.copyfile(shapedDevicesFile, last_good_csv_path)
        last_good_json_path = get_state_path("shaping", "lastGoodConfig.json")
        ensure_parent_dir(last_good_json_path)
        shutil.copyfile(networkJSONfile, last_good_json_path)
        if os.path.isfile(shapedDevicesFile):
            print("Backed up good config as lastGoodConfig.csv and lastGoodConfig.json")
        else:
            print("Backed up good config as lastGoodConfig.json")
        safeToRunRefresh = True
    else:
        if topology_import_ingress_enabled():
            warnings.warn("Validation failed for integration ingress/runtime artifacts - will now exit.", stacklevel=2)
            safeToRunRefresh = False
        elif (isThisFirstRunSinceBoot == False):
            warnings.warn("Validation failed. Because this is not the first run since boot (queues already set up) - will now exit.", stacklevel=2)
            safeToRunRefresh = False
        else:
            warnings.warn("Validation failed. However - because this is the first run since boot - will load queues from last good config", stacklevel=2)
            shapedDevicesFile = get_last_good_config_csv_path()
            networkJSONfile = get_last_good_config_json_path()
            safeToRunRefresh = True

    if safeToRunRefresh == True:

        # Load Subscriber Circuits & Devices
        subscriberCircuits,	dictForCircuitsWithoutParentNodes = loadSubscriberCircuitsForShaping(shapedDevicesFile, networkJSONfile)
        runtime_override_count = apply_effective_runtime_circuit_overrides(subscriberCircuits)
        if runtime_override_count > 0:
            print(
                "Applied "
                + str(runtime_override_count)
                + " effective runtime circuit SQM override(s) in memory"
            )

        # Preserve the logical parent (as configured in ShapedDevices.csv) before any shaping-time rewrites.
        for circuit in subscriberCircuits:
            if 'logicalParentNode' not in circuit:
                circuit['logicalParentNode'] = circuit.get('ParentNode')
            if 'logicalParentNodeID' not in circuit:
                circuit['logicalParentNodeID'] = circuit.get('ParentNodeID', '')

        # Load network hierarchy
        with open(networkJSONfile, 'r') as j:
            network = json.loads(j.read())

        # Flat networks ({}) don't require ParentNode entries. Treat every circuit as
        # unparented so they can be distributed across generated parent nodes / CPUs.
        flat_network = loaded_network_is_flat(network)

        # Virtual Nodes (logical-only): build a physical shaping topology that skips them,
        # while leaving ShapedDevices.csv (and monitoring) unchanged.
        logical_to_physical_node = {}
        virtual_nodes = []
        if not flat_network and isinstance(network, dict) and len(network) > 0:
            logical_to_physical_node, virtual_nodes = build_logical_to_physical_node_map(network)
            if len(virtual_nodes) > 0:
                print(
                    f"Detected {len(virtual_nodes)} virtual node(s) in network.json; building physical HTB tree without them."
                )
                network = build_physical_network(network)
                if len(network) == 0:
                    warnings.warn(
                        "All nodes were removed from the physical shaping tree after virtual-node promotion. Treating this as a flat network for shaping.",
                        stacklevel=2,
                    )
                    flat_network = True
            else:
                # Avoid bloating queuingStructure.json when there are no virtual nodes.
                logical_to_physical_node = {}

        # Re-map circuits that are directly parented to a virtual node to the nearest real ancestor (milestone c).
        if not flat_network and len(virtual_nodes) > 0 and isinstance(logical_to_physical_node, dict):
            next_id = max(dictForCircuitsWithoutParentNodes.keys(), default=-1) + 1
            for circuit in subscriberCircuits:
                logical_parent = circuit.get('logicalParentNode', circuit.get('ParentNode'))
                if logical_parent and logical_parent != 'none' and logical_parent in logical_to_physical_node:
                    physical_parent = logical_to_physical_node.get(logical_parent)
                    if physical_parent is None:
                        warnings.warn(
                            f"Circuit '{circuit.get('circuitID','')}' is parented to virtual top-level node '{logical_parent}'. Attaching it as unparented for shaping.",
                            stacklevel=2,
                        )
                        circuit['ParentNode'] = 'none'
                        circuit['ParentNodeID'] = ''
                        circuit['effectiveParentNodeID'] = ''
                    else:
                        circuit['ParentNode'] = physical_parent

                # If virtual-node mapping created new unparented circuits, ensure they have planner IDs.
                if circuit.get('ParentNode') == 'none' and 'idForCircuitsWithoutParentNodes' not in circuit:
                    try:
                        weight = float(circuit.get('maxDownload', 0)) + float(circuit.get('maxUpload', 0))
                    except Exception:
                        weight = 0.0
                    dictForCircuitsWithoutParentNodes[next_id] = weight
                    circuit['idForCircuitsWithoutParentNodes'] = next_id
                    next_id += 1
        if flat_network:
            print("Flat network detected; assigning circuits to generated parent nodes")
            next_id = max(dictForCircuitsWithoutParentNodes.keys(), default=-1) + 1
            for circuit in subscriberCircuits:
                if circuit.get('ParentNode') != 'none':
                    circuit['ParentNode'] = 'none'
                    circuit['ParentNodeID'] = ''
                    circuit['effectiveParentNodeID'] = ''
                if circuit.get('ParentNode') == 'none' and 'idForCircuitsWithoutParentNodes' not in circuit:
                    try:
                        weight = float(circuit.get('maxDownload', 0)) + float(circuit.get('maxUpload', 0))
                    except Exception:
                        weight = 0.0
                    dictForCircuitsWithoutParentNodes[next_id] = weight
                    circuit['idForCircuitsWithoutParentNodes'] = next_id
                    next_id += 1

        # Normalize any zero or missing bandwidths in the network model early
        # Some users may specify 0 for site bandwidths. HTB requires positive
        # rates, so bump zeros to the parent/default capacity and log a warning.
        def fix_zero_bandwidths(data, parentMaxDL, parentMaxUL):
            for node in data:
                if isinstance(node, str):
                    if (isinstance(data[node], dict)) and (node != 'children'):
                        # Ensure max bandwidths are positive. If 0 or missing, use parent's defaults.
                        dl = data[node].get('downloadBandwidthMbps', None)
                        ul = data[node].get('uploadBandwidthMbps', None)

                        if dl is None or (isinstance(dl, (int, float)) and dl <= 0):
                            logging.warning(f"Node '{node}' has downloadBandwidthMbps set to 0 or missing; using parent/default {parentMaxDL} Mbps.")
                            data[node]['downloadBandwidthMbps'] = parentMaxDL
                        if ul is None or (isinstance(ul, (int, float)) and ul <= 0):
                            logging.warning(f"Node '{node}' has uploadBandwidthMbps set to 0 or missing; using parent/default {parentMaxUL} Mbps.")
                            data[node]['uploadBandwidthMbps'] = parentMaxUL

                        # Recurse into children with this node's maxima as the new parent defaults
                        if 'children' in data[node]:
                            fix_zero_bandwidths(
                                data[node]['children'],
                                data[node]['downloadBandwidthMbps'],
                                data[node]['uploadBandwidthMbps'],
                            )

        fix_zero_bandwidths(
            network,
            upstream_bandwidth_capacity_download_mbps(),
            upstream_bandwidth_capacity_upload_mbps(),
        )


        # Pull rx/tx queues / CPU cores available
        # Handling the case when the number of queues for interfaces are different
        InterfaceAQueuesAvailable = findQueuesAvailable(interface_a())
        InterfaceBQueuesAvailable = findQueuesAvailable(interface_b())
        queuesAvailable = min(InterfaceAQueuesAvailable, InterfaceBQueuesAvailable)
        stickOffset = 0
        if on_a_stick():
            print("On-a-stick override dividing queues")
            # The idea here is that download use queues 0 - n/2, upload uses the other half
            queuesAvailable = math.floor(queuesAvailable / 2)
            stickOffset = queuesAvailable

        # Generate Parent Nodes. Spread ShapedDevices.csv which lack defined ParentNode across these (balance across CPUs)
        print("Generating parent nodes")
        generatedPNs = []
        numberOfGeneratedPNs = queuesAvailable
        chosenDownloadMbps = generated_pn_download_mbps()
        chosenUploadMbps = generated_pn_upload_mbps()
        for x in range(numberOfGeneratedPNs):
            genPNname = "Generated_PN_" + str(x+1)
            network[genPNname] =	{
                                        "downloadBandwidthMbps": chosenDownloadMbps,
                                        "uploadBandwidthMbps": chosenUploadMbps
                                    }
            generatedPNs.append(genPNname)
        # Planner/device weights (fetched only when planner/binpacking is enabled).
        # When disabled, we keep this empty and fall back to rate-based weights later.
        weight_by_circuit_id = {}
        if use_bin_packing_to_balance_cpu():
            print("Using internal planner to sort circuits by CPU core")
            # Build item list with weights for circuits lacking a ParentNode
            items = []
            try:
                weights = get_weights()
            except Exception as e:
                warnings.warn("get_weights() failed; defaulting to equal weights (" + str(e) + ")", stacklevel=2)
                weights = None
            weight_by_circuit_id = {}
            if weights is not None:
                try:
                    for w in weights:
                        weight_by_circuit_id[str(w.circuit_id)] = float(w.weight)
                except Exception:
                    pass
            for circuit in subscriberCircuits:
                if circuit.get('ParentNode') == 'none' and 'idForCircuitsWithoutParentNodes' in circuit:
                    item_id = circuit['idForCircuitsWithoutParentNodes']
                    # Prefer provided weights; default to 1.0
                    w = dictForCircuitsWithoutParentNodes.get(item_id, 1.0)
                    # If a specific circuit weight exists, prefer it
                    if 'circuitID' in circuit and str(circuit['circuitID']) in weight_by_circuit_id:
                        w = weight_by_circuit_id[str(circuit['circuitID'])]
                    # Ignore placeholder default rates for weight purposes
                    try:
                        default_rate = float(generated_pn_download_mbps())
                        max_dl = float(circuit.get('maxDownload', 0))
                        if abs(max_dl - default_rate) < 1e-6:
                            w = 0.0
                    except Exception:
                        pass
                    items.append({"id": item_id, "weight": float(w)})

            # Prepare bins and capacities
            bins_list = [{"id": pn} for pn in generatedPNs]
            capacities = {pn: 1.0 for pn in generatedPNs}

            # Load planner state
            try:
                import bin_planner
            except ImportError:
                bin_planner = None
            # Store planner state directly in lqos_directory (no hidden subdirs)
            state_path = get_planner_state_path()
            state = {}
            if bin_planner is not None:
                state = load_planner_state(state_path, bin_planner)
            now_ts = time.time()
            prev_assign = {}
            last_change_ts = {}
            if isinstance(state, dict):
                prev_assign = state.get("assignments", {}) or {}
                last_change_ts = state.get("last_change_ts", {}) or {}
            # Filter previous assignments to only items/bins in this context
            item_ids = {str(it["id"]) for it in items}
            valid_bins = set(capacities.keys())
            prev_assign = {iid: b for iid, b in prev_assign.items() if iid in item_ids and b in valid_bins}
            last_change_ts = {iid: last_change_ts.get(iid, 0.0) for iid in item_ids}

            # Planner parameters
            params = {
                "candidate_set_size": 4,
                "headroom": 0.05,
                "alpha": 0.1,
                "hysteresis_threshold": 0.03,
                "cooldown_seconds": 3600,
                "move_budget_per_run": max(1, min(32, int(0.01 * max(1, len(items))))),
                "salt": state.get("salt", "default_salt") if isinstance(state, dict) else "default_salt",
                "last_change_ts_by_item": last_change_ts,
            }
            if observe_mode:
                params["move_budget_per_run"] = 0

            if bin_planner is not None:
                assignments, changed = bin_planner.plan_assignments(
                    items, bins_list, capacities, prev_assign, now_ts, params
                )
            else:
                # Fallback to simple greedy if planner unavailable
                bin_loads = {pn: 0.0 for pn in generatedPNs}
                pairs = [(str(it["id"]), float(it["weight"])) for it in items]
                pairs.sort(key=lambda iw: (-iw[1], str(iw[0])))
                assignments = {}
                for item_id, w in pairs:
                    target_pn = min(bin_loads.items(), key=lambda kv: (kv[1], kv[0]))[0]
                    assignments[item_id] = target_pn
                    bin_loads[target_pn] += w
                changed = list(assignments.keys())

            # Apply assignments to circuits
            for circuit in subscriberCircuits:
                if circuit.get('ParentNode') == 'none' and 'idForCircuitsWithoutParentNodes' in circuit:
                    item_id = circuit['idForCircuitsWithoutParentNodes']
                    item_key = str(item_id)
                    if item_key in assignments:
                        circuit['ParentNode'] = assignments[item_key]
                        circuit['ParentNodeID'] = ''
                        circuit['effectiveParentNodeID'] = ''

            # Update and save state
            if bin_planner is not None and isinstance(state, dict):
                if state.get("salt") is None:
                    state["salt"] = "default_salt"
                if "assignments" not in state or not isinstance(state["assignments"], dict):
                    state["assignments"] = {}
                if "last_change_ts" not in state or not isinstance(state["last_change_ts"], dict):
                    state["last_change_ts"] = {}
                for iid, b in assignments.items():
                    # record last change time if changed
                    if iid in changed:
                        state["last_change_ts"][iid] = now_ts
                    state["assignments"][iid] = b
                try:
                    print(f"Saving planner state to {state_path} (generated PNs)")
                    save_planner_state(state, state_path, bin_planner)
                except Exception as e:
                    warnings.warn(f"Failed to save planner state at {state_path}: {e}", stacklevel=2)

            print("Finished planning generated parent nodes")
        else:
            genPNcounter = 0
            for circuit in subscriberCircuits:
                if circuit['ParentNode'] == 'none':
                    circuit['ParentNode'] = generatedPNs[genPNcounter]
                    circuit['ParentNodeID'] = ''
                    circuit['effectiveParentNodeID'] = ''
                    genPNcounter += 1
                    if genPNcounter >= queuesAvailable:
                        genPNcounter = 0
        print("Generated parent nodes created")

        # Find the bandwidth minimums for each node by combining mimimums of devices lower in that node's hierarchy
        def findBandwidthMins(data, depth):
            tabs = '   ' * depth
            minDownload = 0
            minUpload = 0
            for elem in data:
                for circuit in subscriberCircuits:
                    if elem == circuit['ParentNode']:
                        minDownload += circuit['minDownload']
                        minUpload += circuit['minUpload']
                if 'children' in data[elem]:
                    minDL, minUL = findBandwidthMins(data[elem]['children'], depth+1)
                    minDownload += minDL
                    minUpload += minUL
                if 'downloadBandwidthMbpsMin' in data[elem]:
                    data[elem]['downloadBandwidthMbpsMin'] = max(data[elem]['downloadBandwidthMbpsMin'], minDownload)
                else:
                    data[elem]['downloadBandwidthMbpsMin'] = max(data[elem]['downloadBandwidthMbps'], minUpload)
                if 'uploadBandwidthMbpsMin' in data[elem]:
                    data[elem]['uploadBandwidthMbpsMin'] = max(data[elem]['uploadBandwidthMbpsMin'], minUpload)
                else:
                    data[elem]['uploadBandwidthMbpsMin'] = max(data[elem]['uploadBandwidthMbps'], minUpload)
            return minDownload, minUpload
        logging.info("Finding the bandwidth minimums for each node")
        minDownload, minUpload = findBandwidthMins(network, 0)
        logging.info("Found the bandwidth minimums for each node")

        # Child nodes inherit bandwidth maximums of parents. We apply this here to avoid bugs when compression is applied with flattenA().
        def inheritBandwidthMaxes(data, parentMaxDL, parentMaxUL, parentMinDL, parentMinUL):
            for node in data:
                if isinstance(node, str):
                    if (isinstance(data[node], dict)) and (node != 'children'):
                        # Cap based on this node's max bandwidth, or parent node's max bandwidth, whichever is lower
                        data[node]['downloadBandwidthMbps'] = min(int(data[node]['downloadBandwidthMbps']),int(parentMaxDL))
                        data[node]['uploadBandwidthMbps'] = min(int(data[node]['uploadBandwidthMbps']),int(parentMaxUL))
                        data[node]['downloadBandwidthMbpsMin'] = min(int(data[node]['downloadBandwidthMbpsMin']),int(data[node]['downloadBandwidthMbps']),int(parentMinDL))
                        data[node]['uploadBandwidthMbpsMin'] = min(int(data[node]['uploadBandwidthMbpsMin']),int(data[node]['uploadBandwidthMbps']),int(parentMinUL))
                        # Recursive call this function for children nodes attached to this node
                        if 'children' in data[node]:
                            # We need to keep tabs on the minor counter, because we can't have repeating class IDs. Here, we bring back the minor counter from the recursive function
                            inheritBandwidthMaxes(data[node]['children'], data[node]['downloadBandwidthMbps'], data[node]['uploadBandwidthMbps'], data[node]['downloadBandwidthMbpsMin'], data[node]['uploadBandwidthMbpsMin'])
            #return data
        # Here is the actual call to the recursive function
        inheritBandwidthMaxes(network, parentMaxDL=upstream_bandwidth_capacity_download_mbps(), parentMaxUL=upstream_bandwidth_capacity_upload_mbps(), parentMinDL=upstream_bandwidth_capacity_download_mbps(), parentMinUL=upstream_bandwidth_capacity_upload_mbps())

        # Ensure site-level minimums are strictly below maximums for HTB classes
        def ensure_min_less_than_max(data):
            for node in data:
                if isinstance(node, str):
                    if (isinstance(data[node], dict)) and (node != 'children'):
                        try:
                            dl_max = float(data[node].get('downloadBandwidthMbps', 0))
                            ul_max = float(data[node].get('uploadBandwidthMbps', 0))
                            dl_min = float(data[node].get('downloadBandwidthMbpsMin', dl_max))
                            ul_min = float(data[node].get('uploadBandwidthMbpsMin', ul_max))
                        except Exception:
                            # If parsing fails, skip adjustment for this node
                            dl_max = data[node].get('downloadBandwidthMbps', 0)
                            ul_max = data[node].get('uploadBandwidthMbps', 0)
                            dl_min = data[node].get('downloadBandwidthMbpsMin', dl_max)
                            ul_min = data[node].get('uploadBandwidthMbpsMin', ul_max)

                        def adjust(min_v, max_v):
                            # Keep min strictly lower than max; support small max with fractional step
                            if min_v >= max_v:
                                if max_v >= 1.0:
                                    return max_v - 1.0
                                else:
                                    return max(0.01, max_v - 0.01)
                            return min_v

                        new_dl_min = adjust(dl_min, dl_max)
                        new_ul_min = adjust(ul_min, ul_max)
                        if new_dl_min != dl_min:
                            # Too noisy in practice; keep as debug for diagnostics
                            logging.debug(f"Node '{node}' min download ({dl_min}) >= max ({dl_max}); lowering min to {new_dl_min}")
                            data[node]['downloadBandwidthMbpsMin'] = new_dl_min
                        if new_ul_min != ul_min:
                            # Too noisy in practice; keep as debug for diagnostics
                            logging.debug(f"Node '{node}' min upload ({ul_min}) >= max ({ul_max}); lowering min to {new_ul_min}")
                            data[node]['uploadBandwidthMbpsMin'] = new_ul_min
                        if 'children' in data[node]:
                            ensure_min_less_than_max(data[node]['children'])

        ensure_min_less_than_max(network)

        # Compress network.json. HTB only supports 8 levels of HTB depth. Compress to 8 layers if beyond 8.
        def flattenB(data):
            newDict = {}
            for node in data:
                if isinstance(node, str):
                    if (isinstance(data[node], dict)) and (node != 'children'):
                        newDict[node] = dict(data[node])
                        if 'children' in data[node]:
                            result = flattenB(data[node]['children'])
                            del newDict[node]['children']
                            newDict.update(result)
            return newDict
        def flattenA(data, depth):
            newDict = {}
            for node in data:
                if isinstance(node, str):
                    if (isinstance(data[node], dict)) and (node != 'children'):
                        newDict[node] = dict(data[node])
                        if 'children' in data[node]:
                            result = flattenA(data[node]['children'], depth+2)
                            del newDict[node]['children']
                            if depth <= 8:
                                newDict[node]['children'] = result
                            else:
                                flattened = flattenB(data[node]['children'])
                                if 'children' in newDict[node]:
                                    newDict[node]['children'].update(flattened)
                                else:
                                    newDict[node]['children'] = flattened
            return newDict
        network = flattenA(network, 1)

        # After flattening, some attachment-style names may only survive as
        # metadata on the physical node that now owns that branch.
        parent_node_ids = collect_physical_parent_node_ids(network)
        parent_node_aliases = collect_physical_parent_node_aliases(network)
        for circuit in subscriberCircuits:
            parent_node = str(circuit.get('ParentNode', '') or '').strip()
            if (
                (not parent_node or parent_node == 'none')
                and not str(circuit.get('effectiveAttachmentName', '') or '').strip()
                and not str(circuit.get('effectiveAttachmentID', '') or '').strip()
            ):
                continue
            resolved_parent, resolved_parent_id = _resolve_effective_parent_node(
                circuit,
                parent_node_ids,
                parent_node_aliases,
            )
            if resolved_parent_id:
                circuit['effectiveParentNodeID'] = resolved_parent_id
            if resolved_parent and resolved_parent != parent_node:
                logging.info(
                    "Resolved circuit parent reference '%s' (%s) to physical queue node '%s' for circuit '%s'",
                    parent_node,
                    str(circuit.get('ParentNodeID', '') or '').strip(),
                    resolved_parent,
                    circuit.get('circuitID', ''),
                )
                circuit['ParentNode'] = resolved_parent
            elif resolved_parent_id and str(circuit.get('ParentNodeID', '') or '').strip() != resolved_parent_id:
                logging.info(
                    "Resolved circuit parent ID '%s' to physical queue node '%s' for circuit '%s'",
                    str(circuit.get('ParentNodeID', '') or '').strip(),
                    resolved_parent,
                    circuit.get('circuitID', ''),
                )
                circuit['ParentNode'] = resolved_parent
                circuit['ParentNodeID'] = resolved_parent_id
            elif resolved_parent_id:
                circuit['ParentNodeID'] = resolved_parent_id

        # Group circuits by stable parent identity first, then by names as fallback.
        circuits_by_parent_id = {}
        circuits_by_parent_name = {}
        for circuit in subscriberCircuits:
            parent_id = str(circuit.get('effectiveParentNodeID', '') or circuit.get('ParentNodeID', '') or '').strip()
            if parent_id:
                circuits_by_parent_id.setdefault(parent_id, []).append(circuit)

            for parent_name in {
                str(circuit.get('ParentNode', '') or '').strip(),
                str(circuit.get('effectiveParentNodeName', '') or '').strip(),
                str(circuit.get('logicalParentNode', '') or '').strip(),
            }:
                if parent_name:
                    circuits_by_parent_name.setdefault(parent_name, []).append(circuit)

        # Parse network structure and add devices from ShapedDevices.csv
        print("Parsing network structure and tallying devices")
        parentNodes = []
        minorByCPUpreloaded = {}
        nodes_requiring_min_squashing = {}
        # Track minor counter by CPU. This way we can have > 32000 hosts (htb has u16 limit to minor handle)
        # Minor numbers start at 3 to reserve 1 for root qdisc and 2 for default class
        # With CIRCUIT_PADDING, we leave gaps between nodes to allow future circuit additions
        # without disrupting existing ClassID assignments. This maintains stability across reloads.
        for x in range(queuesAvailable):
            minorByCPUpreloaded[x+1] = 3
        def report_minor_overflow(queue, minor):
            msg = f"Minor class ID overflow on CPU {queue}: {minor} exceeds TC's u16 limit (65535). Consider increasing queue count or restructuring network hierarchy."
            logging.error(msg)
            try:
                ctx = json.dumps({"cpu": queue, "minor": minor})
                submit_urgent_issue("LibreQoS", "Error", "TC_U16_OVERFLOW", msg, ctx, f"TC_U16_OVERFLOW_CPU_{queue}")
            except Exception:
                pass
            raise ValueError(msg)

        def ensure_minor_capacity(queue, minor):
            if minor >= 0xFFFF:
                report_minor_overflow(queue, minor)

        def next_free_minor(start_minor, reserved):
            candidate = max(3, start_minor)
            while candidate in reserved:
                candidate += 1
            return candidate

        def sorted_node_keys(data, depth):
            keys = list(data.keys())
            if depth == 0 and len(keys) > 0 and all(k.startswith("CpueQueue") for k in keys):
                try:
                    keys.sort(key=lambda k: int(k.replace("CpueQueue", "")))
                except Exception:
                    keys = sorted(keys)
            else:
                keys = sorted(keys)
            return keys

        # If we're in binpacking mode, we need to sort the network structure a bit
        if use_bin_packing_to_balance_cpu() and not flat_network:
            # Binpacking is an Insight feature; if Insight is not enabled/licensed, fall back to
            # deterministic round-robin placement so "virtual node promotion" can still spread
            # the physical tree across CPUs.
            insight_enabled = False
            try:
                insight_enabled = bool(is_insight_enabled())
            except Exception:
                insight_enabled = False

            if insight_enabled:
                print("Planner is enabled, so we're going to sort your network across CPU queues.")
            else:
                warnings.warn(
                    "Binpacking is enabled but Insight is not available; using round-robin CPU distribution.",
                    stacklevel=2,
                )

            # Build items from top-level nodes with weights
            items = []
            try:
                weights = get_tree_weights()
            except Exception as e:
                warnings.warn(
                    "get_tree_weights() failed; defaulting to equal weights (" + str(e) + ")",
                    stacklevel=2,
                )
                weights = None
            weight_by_name = {}
            if weights is not None:
                try:
                    for w in weights:
                        weight_by_name[str(w.name)] = float(w.weight)
                except Exception:
                    pass

            for node in network:
                if is_generated_parent_node_name(node):
                    continue
                w = weight_by_name.get(str(node), 1.0)
                try:
                    w = float(w)
                except Exception:
                    w = 1.0
                # Ensure we always spread items. Zero/negative weights can cause all items
                # to collapse into a single CPU bin in tie cases.
                if not math.isfinite(w) or w <= 0.0:
                    w = 1.0
                items.append({"id": str(node), "weight": w})

            cpu_keys = ["CpueQueue" + str(cpu) for cpu in range(queuesAvailable)]
            valid_bins = set(cpu_keys)

            planner_used = False
            state_path = get_planner_state_path()
            state = {}
            now_ts = time.time()
            assignment = {}
            changed = []
            try:
                state = load_planner_state(state_path, None)
            except Exception:
                state = {}

            prev_assign = {}
            last_change_ts = {}
            if isinstance(state, dict):
                prev_assign = state.get("assignments", {}) or {}
                last_change_ts = state.get("last_change_ts", {}) or {}
            item_ids = {str(it["id"]) for it in items}
            prev_assign = {
                iid: b for iid, b in prev_assign.items() if iid in item_ids and b in valid_bins
            }
            last_change_ts = {iid: last_change_ts.get(iid, 0.0) for iid in item_ids}

            move_budget = max(1, min(32, int(0.01 * max(1, len(items)))))
            if observe_mode:
                move_budget = 0

            planner_mode = "stable_greedy" if insight_enabled else "round_robin"
            try:
                plan_result = plan_top_level_cpu_bins(
                    items,
                    queuesAvailable,
                    prev_assign=prev_assign,
                    last_change_ts=last_change_ts,
                    now_ts=now_ts,
                    mode=planner_mode,
                    move_budget_per_run=move_budget,
                    cooldown_seconds=3600.0,
                    hysteresis_threshold=0.03,
                )
                assignment = dict(plan_result.get("assignment", {}) or {})
                changed = list(plan_result.get("changed", []) or [])
                planner_used = bool(plan_result.get("planner_used", False))
            except Exception as e:
                warnings.warn(
                    f"Shared Rust planner failed ({e}); falling back to deterministic local assignment.",
                    stacklevel=2,
                )
                assignment = {}
                names = sorted(str(it["id"]) for it in items)
                if cpu_keys:
                    for idx, name in enumerate(names):
                        if insight_enabled:
                            assignment[name] = cpu_keys[idx % len(cpu_keys)]
                        else:
                            assignment[name] = cpu_keys[idx % len(cpu_keys)]
                changed = list(assignment.keys())
                planner_used = False

            resolved_assignment = {}
            for node in network:
                tgt = generated_parent_node_queue_key(node, queuesAvailable)
                if tgt is None:
                    tgt = assignment.get(node)
                if tgt is None:
                    tgt = "CpueQueue" + str(queuesAvailable - 1)
                resolved_assignment[str(node)] = tgt

            for x in range(queuesAvailable):
                key = "CpueQueue" + str(x)
                assigned = [name for name, tgt in resolved_assignment.items() if tgt == key]
                print("Bin " + str(x) + " = ", assigned)

            # Build the binned network structure
            binnedNetwork = {}
            for cpu in range(queuesAvailable):
                cpuKey = "CpueQueue" + str(cpu)
                binnedNetwork[cpuKey] = {
                    'downloadBandwidthMbps': generated_pn_download_mbps(),
                    'uploadBandwidthMbps': generated_pn_upload_mbps(),
                    'type': 'site',
                    'downloadBandwidthMbpsMin': generated_pn_download_mbps(),
                    'uploadBandwidthMbpsMin': generated_pn_upload_mbps(),
                    'children': {},
                    'name': cpuKey
                }
            for node in network:
                tgt = resolved_assignment.get(str(node))
                if tgt is None:
                    tgt = "CpueQueue" + str(queuesAvailable - 1)
                binnedNetwork[tgt]['children'][node] = network[node]
            network = binnedNetwork

            # Update and save state
            if planner_used and isinstance(state, dict):
                if state.get("salt") is None:
                    state["salt"] = "default_salt"
                if "assignments" not in state or not isinstance(state["assignments"], dict):
                    state["assignments"] = {}
                if "last_change_ts" not in state or not isinstance(state["last_change_ts"], dict):
                    state["last_change_ts"] = {}
                stale_generated = [
                    iid for iid in list(state["assignments"].keys())
                    if is_generated_parent_node_name(iid)
                ]
                for iid in stale_generated:
                    state["assignments"].pop(iid, None)
                    state["last_change_ts"].pop(iid, None)
                for iid, b in assignment.items():
                    if iid in changed:
                        state["last_change_ts"][iid] = now_ts
                    state["assignments"][iid] = b
                try:
                    print(f"Saving planner state to {state_path} (top-level CPU binning)")
                    save_planner_state(state, state_path, None)
                except Exception as e:
                    warnings.warn(
                        f"Failed to save planner state at {state_path}: {e}", stacklevel=2
                    )

        # Seed persisted site/circuit minor assignments. When planner state is absent,
        # fall back to the previous queuing structure so the first run after an upgrade
        # can preserve existing class IDs.
        try:
            state  # noqa: B018
        except NameError:
            state = {}
        state_path = get_planner_state_path()
        if not isinstance(state, dict) or len(state.keys()) == 0:
            state = load_planner_state(state_path, None)
        try:
            circuit_state_from_disk = state.get("circuits", {}) if isinstance(state, dict) else {}
        except Exception:
            circuit_state_from_disk = {}
        try:
            site_state_from_disk = state.get("sites", {}) if isinstance(state, dict) else {}
        except Exception:
            site_state_from_disk = {}
        if not isinstance(circuit_state_from_disk, dict):
            circuit_state_from_disk = {}
        if not isinstance(site_state_from_disk, dict):
            site_state_from_disk = {}
        if not circuit_state_from_disk or not site_state_from_disk:
            fallback_minor_state = load_minor_state_from_queuing_structure()
            if not site_state_from_disk:
                site_state_from_disk = fallback_minor_state.get("sites", {}) or {}
            if not circuit_state_from_disk:
                circuit_state_from_disk = fallback_minor_state.get("circuits", {}) or {}
        circuit_state_updated = {}
        site_state_updated = {}
        planner_site_inputs = []
        planner_circuit_groups = []

        def collect_identity_planner_inputs(data, depth, queue, path=()):
            for node in sorted_node_keys(data, depth):
                current_queue = queue
                node_path = path + (node,)
                parent_path = '/'.join(path)
                has_children = bool(data[node].get('children'))
                planner_site_inputs.append(
                    {
                        "site_key": '/'.join(node_path),
                        "parent_path": parent_path,
                        "queue": current_queue,
                        "has_children": has_children,
                    }
                )
                node_id = str(data[node].get('id', '') or '').strip()
                node_name = str(data[node].get('name', '') or '').strip()
                selected_circuits = []
                if node_id and node_id in circuits_by_parent_id:
                    selected_circuits = list(circuits_by_parent_id[node_id])
                else:
                    seen_circuit_ids = set()
                    for candidate in (node, node_name):
                        for circuit in circuits_by_parent_name.get(candidate, []):
                            circuit_id = planner_circuit_identity_key(circuit)
                            if circuit_id in seen_circuit_ids:
                                continue
                            selected_circuits.append(circuit)
                            seen_circuit_ids.add(circuit_id)
                if selected_circuits:
                    sorted_circuits = sorted(
                        selected_circuits,
                        key=lambda c: c.get('circuitName', c.get('circuitID', '')),
                    )
                    planner_circuit_groups.append(
                        {
                            "parent_node": node,
                            "queue": current_queue,
                            "circuit_ids": [
                                planner_circuit_identity_key(circuit)
                                for circuit in sorted_circuits
                            ],
                        }
                    )

                if has_children:
                    sorted_children = dict(sorted(data[node]['children'].items()))
                    collect_identity_planner_inputs(
                        sorted_children,
                        depth + 1,
                        current_queue,
                        node_path,
                    )

                if depth == 0:
                    if queue >= queuesAvailable:
                        queue = 1
                    else:
                        queue += 1

        collect_identity_planner_inputs(network, 0, 1)

        identity_plan = plan_class_identities(
            planner_site_inputs,
            planner_circuit_groups,
            site_state=site_state_from_disk,
            circuit_state=circuit_state_from_disk,
            stick_offset=stickOffset,
            circuit_padding=CIRCUIT_PADDING,
        )
        site_assignment_by_key = {
            entry["site_key"]: entry for entry in identity_plan.get("sites", [])
        }
        circuit_assignment_by_key = {
            entry["circuit_id"]: entry for entry in identity_plan.get("circuits", [])
        }
        site_state_updated = identity_plan.get("site_state", {}) or {}
        circuit_state_updated = identity_plan.get("circuit_state", {}) or {}

        def apply_site_assignments(data, depth, queue, parentClassID, upParentClassID, parentMaxDL, parentMaxUL, parentMinDL, parentMinUL, path=()):
            for node in sorted_node_keys(data, depth):
                current_queue = queue
                node_path = path + (node,)
                site_key = '/'.join(node_path)
                assignment = site_assignment_by_key.get(site_key)
                if assignment is None:
                    raise ValueError(f"Missing planned site identity for {site_key}")
                assigned_site_minor = int(assignment["class_minor"])
                major = int(assignment["class_major"])
                up_major = int(assignment["up_class_major"])
                ensure_minor_capacity(current_queue, assigned_site_minor)
                nodeClassID = hex(major) + ':' + hex(assigned_site_minor)
                upNodeClassID = hex(up_major) + ':' + hex(assigned_site_minor)
                data[node]['classid'] = nodeClassID
                data[node]['up_classid'] = upNodeClassID
                current_parent_classid = parentClassID
                current_up_parent_classid = upParentClassID
                if depth == 0:
                    current_parent_classid = hex(major) + ':'
                    current_up_parent_classid = hex(up_major) + ':'
                data[node]['parentClassID'] = current_parent_classid
                data[node]['up_parentClassID'] = current_up_parent_classid
                data[node]['downloadBandwidthMbps'] = min(data[node]['downloadBandwidthMbps'], parentMaxDL)
                data[node]['uploadBandwidthMbps'] = min(data[node]['uploadBandwidthMbps'], parentMaxUL)
                data[node]['downloadBandwidthMbpsMin'] = min(data[node]['downloadBandwidthMbpsMin'], data[node]['downloadBandwidthMbps'], parentMinDL)
                data[node]['uploadBandwidthMbpsMin'] = min(data[node]['uploadBandwidthMbpsMin'], data[node]['uploadBandwidthMbps'], parentMinUL)
                data[node]['classMajor'] = hex(major)
                data[node]['up_classMajor'] = hex(up_major)
                data[node]['classMinor'] = hex(assigned_site_minor)
                data[node]['cpuNum'] = hex(current_queue-1)
                data[node]['up_cpuNum'] = hex(current_queue-1+stickOffset)
                parentNodes.append(
                    {
                        "parentNodeName": node,
                        "classID": nodeClassID,
                        "maxDownload": data[node]['downloadBandwidthMbps'],
                        "maxUpload": data[node]['uploadBandwidthMbps'],
                    }
                )

                if 'children' in data[node]:
                    sorted_children = dict(sorted(data[node]['children'].items()))
                    apply_site_assignments(
                        sorted_children,
                        depth+1,
                        current_queue,
                        nodeClassID,
                        upNodeClassID,
                        data[node]['downloadBandwidthMbps'],
                        data[node]['uploadBandwidthMbps'],
                        data[node]['downloadBandwidthMbpsMin'],
                        data[node]['uploadBandwidthMbpsMin'],
                        node_path,
                    )

                if depth == 0:
                    if queue >= queuesAvailable:
                        queue = 1
                    else:
                        queue += 1

        apply_site_assignments(
            network,
            0,
            queue=1,
            parentClassID=None,
            upParentClassID=None,
            parentMaxDL=upstream_bandwidth_capacity_download_mbps(),
            parentMaxUL=upstream_bandwidth_capacity_upload_mbps(),
            parentMinDL=upstream_bandwidth_capacity_download_mbps(),
            parentMinUL=upstream_bandwidth_capacity_upload_mbps(),
        )

        attached_circuit_ids = set()

        def attach_circuits(data, depth, path=()):
            for node in sorted_node_keys(data, depth):
                node_data = data[node]
                queue_token = _parse_int_token(node_data.get('cpuNum'))
                major = _parse_int_token(node_data.get('classMajor'))
                if queue_token is None or major is None:
                    continue
                queue = queue_token + 1
                circuitsForThisNetworkNode = []

                node_id = str(node_data.get('id', '') or '').strip()
                node_name = str(node_data.get('name', '') or '').strip()
                parent_candidates = _attachment_lookup_candidates(node, node_data)

                selected_circuits = []
                if node_id and node_id in circuits_by_parent_id:
                    selected_circuits = list(circuits_by_parent_id[node_id])
                else:
                    seen_circuit_ids = set()
                    name_candidates = (
                        parent_candidates[1:] if node_id and parent_candidates else parent_candidates
                    )
                    for candidate in name_candidates:
                        for circuit in circuits_by_parent_name.get(candidate, []):
                            circuit_id = str(circuit.get('circuitID', '') or '')
                            if circuit_id in seen_circuit_ids:
                                continue
                            selected_circuits.append(circuit)
                            seen_circuit_ids.add(circuit_id)

                if selected_circuits:
                    override_min_down = None
                    override_min_up = None
                    combined_min_down = sum(float(circuit.get('minDownload', 0) or 0) for circuit in selected_circuits)
                    combined_min_up = sum(float(circuit.get('minUpload', 0) or 0) for circuit in selected_circuits)
                    if (combined_min_down > node_data['downloadBandwidthMbpsMin']) or (combined_min_up > node_data['uploadBandwidthMbpsMin']):
                        override_min_down = 1
                        override_min_up = 1
                        logging.info("The combined minimums of circuits in Parent Node [" + node + "] exceeded that of the parent node. Reducing these circuits' minimums to 1 now.", stacklevel=2)
                        if ((override_min_down * len(selected_circuits)) > node_data['downloadBandwidthMbpsMin']) or ((override_min_up * len(selected_circuits)) > node_data['uploadBandwidthMbpsMin']):
                            logging.info("Even with this change, minimums will exceed the min rate of the parent node. Using 10 kbps as the minimum for these circuits instead.", stacklevel=2)
                            nodes_requiring_min_squashing[node] = True
                    sorted_circuits = sorted(selected_circuits, key=lambda c: c.get('circuitName', c.get('circuitID', '')))
                    for circuit in sorted_circuits:
                        circuit_id = str(circuit.get('circuitID', '') or '')
                        if circuit_id in attached_circuit_ids:
                            continue
                        if circuit['maxDownload'] > node_data['downloadBandwidthMbps']:
                            logging.info("downloadMax of Circuit ID [" + circuit['circuitID'] + "] exceeded that of its parent node. Reducing to that of its parent node now.", stacklevel=2)
                        if circuit['maxUpload'] > node_data['uploadBandwidthMbps']:
                            logging.info("uploadMax of Circuit ID [" + circuit['circuitID'] + "] exceeded that of its parent node. Reducing to that of its parent node now.", stacklevel=2)
                        planner_key = planner_circuit_identity_key(circuit)
                        planned_identity = circuit_assignment_by_key.get(planner_key)
                        if planned_identity is None:
                            raise ValueError(f"Missing planned circuit identity for {planner_key}")
                        candidate_minor = int(planned_identity["class_minor"])
                        major = int(planned_identity["class_major"])
                        up_major = int(planned_identity["up_class_major"])
                        ensure_minor_capacity(queue, candidate_minor)
                        flowIDstring = hex(major) + ':' + hex(candidate_minor)
                        upFlowIDstring = hex(up_major) + ':' + hex(candidate_minor)
                        circuit['classid'] = flowIDstring
                        circuit['up_classid'] = upFlowIDstring
                        attached_circuit_ids.add(circuit_id)
                        logging.info("Added up_classid to circuit: " + circuit['up_classid'])
                        maxDownload = min(circuit['maxDownload'], node_data['downloadBandwidthMbps'])
                        maxUpload = min(circuit['maxUpload'], node_data['uploadBandwidthMbps'])
                        if override_min_down:
                            circuit['minDownload'] = 1
                        if override_min_up:
                            circuit['minUpload'] = 1
                        minDownload = min(circuit['minDownload'], maxDownload)
                        minUpload = min(circuit['minUpload'], maxUpload)
                        thisNewCircuitItemForNetwork = {
                            'maxDownload': maxDownload,
                            'maxUpload': maxUpload,
                            'minDownload': minDownload,
                            'minUpload': minUpload,
                            "circuitID": circuit['circuitID'],
                            "circuitName": circuit['circuitName'],
                            "ParentNode": circuit['ParentNode'],
                            "ParentNodeID": circuit.get('ParentNodeID', ''),
                            "logicalParentNode": circuit.get('logicalParentNode', circuit['ParentNode']),
                            "logicalParentNodeID": circuit.get('logicalParentNodeID', circuit.get('ParentNodeID', '')),
                            "effectiveParentNodeID": circuit.get('effectiveParentNodeID', circuit.get('ParentNodeID', '')),
                            "devices": circuit['devices'],
                            "classid": flowIDstring,
                            "up_classid": upFlowIDstring,
                            "classMajor": hex(major),
                            "up_classMajor": hex(up_major),
                            "classMinor": hex(candidate_minor),
                            "comment": circuit['comment'],
                        }
                        try:
                            cid = str(circuit.get('circuitID', ''))
                            w = None
                            if cid in weight_by_circuit_id:
                                w = float(weight_by_circuit_id[cid])
                            if w is None:
                                w = float(maxDownload)
                            if abs(w - 1000.0) < 1e-6:
                                w = float(maxDownload)
                            try:
                                default_rate = float(generated_pn_download_mbps())
                                if abs(float(maxDownload) - default_rate) < 1e-6:
                                    w = 0.0
                            except Exception:
                                pass
                            thisNewCircuitItemForNetwork['planner_weight'] = w
                        except Exception:
                            pass
                        if 'sqm' in circuit and circuit['sqm']:
                            thisNewCircuitItemForNetwork['sqm'] = circuit['sqm']
                        thisNewCircuitItemForNetwork['devices'] = circuit['devices']
                        circuitsForThisNetworkNode.append(thisNewCircuitItemForNetwork)

                if len(circuitsForThisNetworkNode) > 0:
                    node_data['circuits'] = circuitsForThisNetworkNode
                else:
                    node_data.pop('circuits', None)

                if 'children' in node_data:
                    sorted_children = dict(sorted(node_data['children'].items()))
                    attach_circuits(sorted_children, depth+1, path + (node,))

        attach_circuits(network, 0)

        unattached_circuits = []
        for circuit in subscriberCircuits:
            circuit_id = str(circuit.get('circuitID', '') or '')
            if circuit_id and circuit_id not in attached_circuit_ids:
                unattached_circuits.append(
                    f"{circuit_id} ({circuit.get('circuitName', '')}) parent={circuit.get('ParentNode', '')} parent_id={circuit.get('effectiveParentNodeID', circuit.get('ParentNodeID', ''))}"
                )
        if unattached_circuits:
            warnings.warn(
                "Some shaped circuits did not attach to the live queue tree. First examples: " + "; ".join(unattached_circuits[:20]),
                stacklevel=2,
            )
            logging.warning(
                "Unattached shaped circuits after queue build: %s total. Examples: %s",
                len(unattached_circuits),
                "; ".join(unattached_circuits[:20]),
            )

        minorByCPU = {
            int(queue): int(minor)
            for queue, minor in (identity_plan.get("last_used_minor_by_queue", {}) or {}).items()
        }
        for cpu in range(queuesAvailable):
            minorByCPU.setdefault(cpu + 1, 3)

        if not isinstance(state, dict):
            state = {}
        state['circuits'] = circuit_state_updated
        state['sites'] = site_state_updated
        try:
            print(f"Saving planner state to {state_path} (circuit/site minors)")
            save_planner_state(state, state_path, None)
        except Exception as e:
            warnings.warn(f"Failed to save planner circuit state at {state_path}: {e}", stacklevel=2)

        bakery = Bakery()
        bakery.start_batch() # Initializes the bakery transaction
        linuxTCcommands = []
        shapedDeviceKeys = set()
        # Root HTB Setup
        # Create MQ qdisc for each CPU core / rx-tx queue. Generate commands to create corresponding HTB and leaf classes. Prepare commands for execution later
        thisInterface = interface_a()
        logging.info("# MQ Setup for " + thisInterface)
        command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
        bakery.setup_mq(queuesAvailable, stickOffset)
        linuxTCcommands.append(command)
        maxBandwidth = max(upstream_bandwidth_capacity_upload_mbps(), upstream_bandwidth_capacity_download_mbps())
        calculateR2q(maxBandwidth)
        for queue in range(queuesAvailable):
            command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+1) + ' handle ' + hex(queue+1) + ': htb default 2'
            linuxTCcommands.append(command)
            command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ': classid ' + hex(queue+1) + ':1 htb rate '+ format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()) + quantum(upstream_bandwidth_capacity_download_mbps())
            linuxTCcommands.append(command)
            command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 ' + sqm()
            linuxTCcommands.append(command)
            # Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
            # Technically, that should not even happen. So don't expect much if any traffic in this default class.
            # Only 1/4 of defaultClassCapacity is guaranteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
            command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':1 classid ' + hex(queue+1) + ':2 htb rate ' + format_rate_for_tc(round((upstream_bandwidth_capacity_download_mbps()-1)/4)) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_download_mbps()-1) + ' prio 5' + quantum(upstream_bandwidth_capacity_download_mbps())
            linuxTCcommands.append(command)
            command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+1) + ':2 ' + sqm()
            linuxTCcommands.append(command)

        # Note the use of stickOffset, and not replacing the root queue if we're on a stick
        thisInterface = interface_b()
        logging.info("# MQ Setup for " + thisInterface)
        if not on_a_stick():
            command = 'qdisc replace dev ' + thisInterface + ' root handle 7FFF: mq'
            linuxTCcommands.append(command)
        for queue in range(queuesAvailable):
            command = 'qdisc add dev ' + thisInterface + ' parent 7FFF:' + hex(queue+stickOffset+1) + ' handle ' + hex(queue+stickOffset+1) + ': htb default 2'
            linuxTCcommands.append(command)
            command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ': classid ' + hex(queue+stickOffset+1) + ':1 htb rate '+ format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()) + quantum(upstream_bandwidth_capacity_upload_mbps())
            linuxTCcommands.append(command)
            command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 ' + sqm()
            linuxTCcommands.append(command)
            # Default class - traffic gets passed through this limiter with lower priority if it enters the top HTB without a specific class.
            # Technically, that should not even happen. So don't expect much if any traffic in this default class.
            # Only 1/4 of defaultClassCapacity is guarenteed (to prevent hitting ceiling of upstream), for the most part it serves as an "up to" ceiling.
            command = 'class add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':1 classid ' + hex(queue+stickOffset+1) + ':2 htb rate ' + format_rate_for_tc(round((upstream_bandwidth_capacity_upload_mbps()-1)/4)) + ' ceil ' + format_rate_for_tc(upstream_bandwidth_capacity_upload_mbps()-1) + ' prio 5' + quantum(upstream_bandwidth_capacity_upload_mbps())
            linuxTCcommands.append(command)
            command = 'qdisc add dev ' + thisInterface + ' parent ' + hex(queue+stickOffset+1) + ':2 ' + sqm()
            linuxTCcommands.append(command)


        # Parse network structure. For each tier, generate commands to create corresponding HTB and leaf classes. Prepare commands for execution later
        # Define lists for hash filters
        print("Preparing TC commands")
        def traverseNetwork(data):
            nonlocal requiredIpMappings

            # Cake needs help handling rates lower than 5 Mbps
            def sqmFixupRate(rate:int, sqm:str) -> str:
                # If we aren't using cake, just return the sqm string
                if not sqm.startswith("cake") or "rtt" in sqm:
                    return sqm

                # If we are using cake, we need to fixup the rate
                # Based on: 1 MTU is 1500 bytes, or 12,000 bits.
                # At 1 Mbps, (1,000 bits per ms) transmitting an MTU takes 12ms. Add 3ms for overhead, and we get 15ms.
                #    So 15ms divided by 5 (for 1%) multiplied by 100 yields 300ms.
                #    The same formula gives 180ms at 2Mbps
                #    140ms at 3Mbps
                #    120ms at 4Mbps
                match rate:
                    case 1: return sqm + " rtt 300"
                    case 2: return sqm + " rtt 180"
                    case 3: return sqm + " rtt 140"
                    case 4: return sqm + " rtt 120"
                    case _: return sqm

            for node in sorted(data.keys()):
                site_name = data[node]['name'] if 'name' in data[node] else node
                bakery.add_site(
                    site_name,
                    data[node]['parentClassID'],
                    data[node]['up_parentClassID'],
                    int(data[node]['classMinor'], 16),
                    data[node]['downloadBandwidthMbpsMin'],
                    data[node]['uploadBandwidthMbpsMin'],
                    data[node]['downloadBandwidthMbps'],
                    data[node]['uploadBandwidthMbps'],
                )
                command = 'class add dev ' + interface_a() + ' parent ' + data[node]['parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ format_rate_for_tc(data[node]['downloadBandwidthMbpsMin']) + ' ceil '+ format_rate_for_tc(data[node]['downloadBandwidthMbps']) + ' prio 3' + quantum(data[node]['downloadBandwidthMbps'])
                linuxTCcommands.append(command)
                logging.info("Up ParentClassID: " + data[node]['up_parentClassID'])
                logging.info("ClassMinor: " + data[node]['classMinor'])
                command = 'class add dev ' + interface_b() + ' parent ' + data[node]['up_parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ format_rate_for_tc(data[node]['uploadBandwidthMbpsMin']) + ' ceil '+ format_rate_for_tc(data[node]['uploadBandwidthMbps']) + ' prio 3' + quantum(data[node]['uploadBandwidthMbps'])
                linuxTCcommands.append(command)
                if 'circuits' in data[node]:
                    # Sort circuits by name for stable ordering
                    sorted_circuits = sorted(data[node]['circuits'],
                                           key=lambda c: c.get('circuitName', c.get('circuitID', '')))
                    for circuit in sorted_circuits:
                        # If circuit mins exceed node mins - handle low min rates of 1 to mean 10 kbps.
                        # Avoid changing minDownload or minUpload because they are used in queuingStructure.json, and must remain integers.
                        min_down = circuit['minDownload']
                        min_up = circuit['minUpload']
                        if node in nodes_requiring_min_squashing:
                            if min_down == 1:
                                min_down = 0.01
                            if min_up == 1:
                                min_up = 0.01
                        # Ensure min < max for circuits as well
                        try:
                            max_down = float(circuit['maxDownload'])
                            max_up = float(circuit['maxUpload'])
                            md = float(min_down)
                            mu = float(min_up)
                        except Exception:
                            max_down = circuit['maxDownload']
                            max_up = circuit['maxUpload']
                            md = min_down
                            mu = min_up
                        if md >= max_down:
                            new_md = (max_down - 1.0) if max_down >= 1.0 else max(0.01, max_down - 0.01)
                            # Too noisy in practice; keep as debug for diagnostics
                            logging.debug(f"Circuit '{circuit.get('circuitID','unknown')}' min download ({md}) >= max ({max_down}); lowering min to {new_md}")
                            min_down = new_md
                        if mu >= max_up:
                            new_mu = (max_up - 1.0) if max_up >= 1.0 else max(0.01, max_up - 0.01)
                            # Too noisy in practice; keep as debug for diagnostics
                            logging.debug(f"Circuit '{circuit.get('circuitID','unknown')}' min upload ({mu}) >= max ({max_up}); lowering min to {new_mu}")
                            min_up = new_mu
                        # Generate TC commands to be executed later
                        tcComment = " # CircuitID: " + circuit['circuitID'] + " DeviceIDs: "
                        for device in circuit['devices']:
                            tcComment = '' #tcComment + device['deviceID'] + ', '
                        if 'devices' in circuit:
                            if 'comment' in circuit['devices'][0]:
                                tcComment = '' # tcComment + '| Comment: ' + circuit['devices'][0]['comment']
                        tcComment = tcComment.replace("\n", "")
                        circuit_name = circuit['circuitID'] if 'circuitID' in circuit else "unknown"
                        # Collect all IP addresses for this circuit
                        ip_list = []
                        for device in circuit['devices']:
                            if device['ipv4s']:
                                ip_list.extend(device['ipv4s'])
                            if device['ipv6s']:
                                ip_list.extend(device['ipv6s'])
                        # Concatenate IPs with comma separator
                        ip_addresses_str = ','.join(ip_list)

                        sqm_override = circuit['sqm'] if 'sqm' in circuit else None
                        bakery.add_circuit(
                            circuit_name,
                            node,
                            data[node]['classid'],
                            data[node]['up_classid'],
                            int(circuit['classMinor'], 16),
                            min_down,
                            min_up,
                            circuit['maxDownload'],
                            circuit['maxUpload'],
                            int(circuit['classMajor'], 16),
                            int(circuit['up_classMajor'], 16),
                            ip_addresses_str,
                            sqm_override,
                        )
                        command = 'class add dev ' + interface_a() + ' parent ' + data[node]['classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ format_rate_for_tc(min_down) + ' ceil '+ format_rate_for_tc(circuit['maxDownload']) + ' prio 3' + quantum(circuit['maxDownload']) + tcComment
                        linuxTCcommands.append(command)
                        # SQM Fixup for lower rates (and per-circuit override)
                        def effective_sqm_str(rate, override, direction):
                            base = sqm()
                            # Resolve per-direction token from override string
                            chosen = None
                            if override:
                                try:
                                    ov = str(override).strip().lower()
                                    if '/' in ov:
                                        left, right = ov.split('/', 1)
                                        left = left.strip()
                                        right = right.strip()
                                        chosen = left if direction == 'down' else right
                                    else:
                                        chosen = ov
                                except Exception:
                                    chosen = None
                            # If no explicit token for this direction, use default behavior
                            if not chosen:
                                try:
                                    thresh = fast_queues_fq_codel()
                                except Exception:
                                    thresh = 1000.0
                                if rate >= thresh:
                                    return 'fq_codel'
                                return sqmFixupRate(rate, base)
                            if chosen == 'none':
                                return ''
                            if chosen == 'fq_codel':
                                return 'fq_codel'
                            if chosen == 'cake':
                                cake_base = base if base.startswith('cake') else 'cake diffserv4'
                                return sqmFixupRate(rate, cake_base)
                            return sqmFixupRate(rate, base)
                        sqm_override = circuit['sqm'] if 'sqm' in circuit else None
                        useSqm = effective_sqm_str(circuit['maxDownload'], sqm_override, 'down')
                        if useSqm != '':
                            command = 'qdisc add dev ' + interface_a() + ' parent ' + circuit['classMajor'] + ':' + circuit['classMinor'] + ' ' + useSqm
                            linuxTCcommands.append(command)
                        command = 'class add dev ' + interface_b() + ' parent ' + data[node]['up_classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ format_rate_for_tc(min_up) + ' ceil '+ format_rate_for_tc(circuit['maxUpload']) + ' prio 3' + quantum(circuit['maxUpload'])
                        linuxTCcommands.append(command)
                        sqm_override = circuit['sqm'] if 'sqm' in circuit else None
                        useSqm = effective_sqm_str(circuit['maxUpload'], sqm_override, 'up')
                        if useSqm != '':
                            command = 'qdisc add dev ' + interface_b() + ' parent ' + circuit['up_classMajor'] + ':' + circuit['classMinor'] + ' ' + useSqm
                            linuxTCcommands.append(command)
                        for device in circuit['devices']:
                            if device['ipv4s']:
                                for ipv4 in device['ipv4s']:
                                    ipMapBatch.add_ip_mapping(
                                        str(ipv4),
                                        circuit['classid'],
                                        data[node]['cpuNum'],
                                        False,
                                        circuit.get('circuitID', ''),
                                        device.get('deviceID', ''),
                                    )
                                    requiredIpMappings += 1
                                    #xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv4) + ' --cpu ' + data[node]['cpuNum'] + ' --classid ' + circuit['classid'])
                                    if on_a_stick():
                                        ipMapBatch.add_ip_mapping(
                                            str(ipv4),
                                            circuit['up_classid'],
                                            data[node]['up_cpuNum'],
                                            True,
                                            circuit.get('circuitID', ''),
                                            device.get('deviceID', ''),
                                        )
                                        #xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv4) + ' --cpu ' + data[node]['up_cpuNum'] + ' --classid ' + circuit['up_classid'] + ' --upload 1')
                            if device['ipv6s']:
                                for ipv6 in device['ipv6s']:
                                    ipMapBatch.add_ip_mapping(
                                        str(ipv6),
                                        circuit['classid'],
                                        data[node]['cpuNum'],
                                        False,
                                        circuit.get('circuitID', ''),
                                        device.get('deviceID', ''),
                                    )
                                    requiredIpMappings += 1
                                    #xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv6) + ' --cpu ' + data[node]['cpuNum'] + ' --classid ' + circuit['classid'])
                                    if on_a_stick():
                                        ipMapBatch.add_ip_mapping(
                                            str(ipv6),
                                            circuit['up_classid'],
                                            data[node]['up_cpuNum'],
                                            True,
                                            circuit.get('circuitID', ''),
                                            device.get('deviceID', ''),
                                        )
                                        #xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv6) + ' --cpu ' + data[node]['up_cpuNum'] + ' --classid ' + circuit['up_classid'] + ' --upload 1')
                            shapedDeviceKeys.add(device_shaping_key(circuit, device))
                # Recursive call this function for children nodes attached to this node
                if 'children' in data[node]:
                    # Sort children to ensure consistent traversal order
                    sorted_children = dict(sorted(data[node]['children'].items()))
                    traverseNetwork(sorted_children)
        # Here is the actual call to the recursive traverseNetwork() function.
        traverseNetwork(network)

        if enable_actual_shell_commands():
            ipMappingCapacity = xdp_ip_mapping_capacity()
            print(
                "Prepared "
                + str(requiredIpMappings)
                + " unique XDP IP mappings against capacity "
                + str(ipMappingCapacity)
            )
            if requiredIpMappings > ipMappingCapacity:
                report_refresh_failure(
                    "XDP_IP_MAPPING_CAPACITY",
                    "Required XDP IP mappings ("
                    + str(requiredIpMappings)
                    + ") exceed current kernel map capacity ("
                    + str(ipMappingCapacity)
                    + "). Aborting refresh before apply.",
                    {
                        "required_ip_mappings": requiredIpMappings,
                        "kernel_map_capacity": ipMappingCapacity,
                        "queued_requests": ipMapBatch.length(),
                        "on_a_stick": on_a_stick(),
                        "shaped_devices_file": shapedDevicesFile,
                        "network_json_file": networkJSONfile,
                    },
                    "XDP_IP_MAPPING_CAPACITY",
                )

            qdiscBudgetEstimate = bakery.estimate_qdisc_budget()
            if not qdiscBudgetEstimate["ok"]:
                interfaceCounts = qdiscBudgetEstimate["interfaces"]
                sortedInterfaces = sorted(interfaceCounts.items())
                preflightSummary = qdiscBudgetEstimate.get("summary") or ""
                report_refresh_failure(
                    "TC_QDISC_CAPACITY",
                    (preflightSummary + " Aborting refresh before apply.").strip(),
                    {
                        "interfaces": dict(sortedInterfaces),
                        "interface_details": qdiscBudgetEstimate.get("interface_details", {}),
                        "safe_budget": qdiscBudgetEstimate["safe_budget"],
                        "hard_limit": qdiscBudgetEstimate["hard_limit"],
                        "estimated_total_memory_bytes": qdiscBudgetEstimate.get("estimated_total_memory_bytes"),
                        "memory_ok": qdiscBudgetEstimate.get("memory_ok"),
                        "memory_guard_min_available_bytes": qdiscBudgetEstimate.get("memory_guard_min_available_bytes"),
                        "memory_total_bytes": qdiscBudgetEstimate.get("memory_total_bytes"),
                        "memory_available_bytes": qdiscBudgetEstimate.get("memory_available_bytes"),
                        "on_a_stick": on_a_stick(),
                        "queue_mode": queue_mode(),
                        "shaped_devices_file": shapedDevicesFile,
                        "network_json_file": networkJSONfile,
                    },
                    "TC_QDISC_CAPACITY",
                )

        # Save queuingStructure
        queuingStructure = {}
        queuingStructure['Network'] = network
        queuingStructure['lastUsedClassIDCounterByCPU'] = minorByCPU
        queuingStructure['generatedPNs'] = generatedPNs
        queuingStructure['logical_to_physical_node'] = logical_to_physical_node
        queuingStructure['virtual_nodes'] = virtual_nodes
        queuing_structure_path = get_state_path("shaping", "queuingStructure.json")
        ensure_parent_dir(queuing_structure_path)
        with open(queuing_structure_path, 'w') as infile:
            json.dump(queuingStructure, infile, indent=4)


        # Record start time of actual filter reload
        reloadStartTime = datetime.now()


        # Clear Prior Settings
        # We don't want to do this every time, with lqosd managing queues statefully.
        # clearPriorSettings(interface_a(), interface_b())


        # Setup XDP and disable XPS regardless of whether it is first run or not (necessary to handle cases where systemctl stop was used)
        xdpStartTime = datetime.now()
        #if enable_actual_shell_commands():
        # Observe mode removes the shaping tree entirely, so clear any live
        # per-circuit IP mappings before the commit to avoid pointing packets
        # at handles that are about to disappear.
        if enable_actual_shell_commands() and observe_mode:
            logging.info("# Observe Mode Mapping Quiesce")
            try:
                clear_ip_mappings()
            except Exception as e:
                report_refresh_failure(
                    "XDP_IP_MAPPING_CLEAR_FAILED",
                    "Failed to clear XDP IP mappings before entering Observe mode: " + str(e),
                    {
                        "required_ip_mappings": requiredIpMappings,
                        "queued_requests": ipMapBatch.length(),
                        "on_a_stick": on_a_stick(),
                        "queue_mode": queue_mode(),
                    },
                    "XDP_IP_MAPPING_CLEAR_FAILED",
                )
        # Set up XDP-CPUMAP-TC
        logging.info("# XDP Setup")
        # Commented out - the daemon does this
        #shell('./cpumap-pping/bin/xps_setup.sh -d ' + interfaceA + ' --default --disable')
        #shell('./cpumap-pping/bin/xps_setup.sh -d ' + interfaceB + ' --default --disable')
        #shell('./cpumap-pping/src/xdp_iphash_to_cpu --dev ' + interfaceA + ' --lan')
        #shell('./cpumap-pping/src/xdp_iphash_to_cpu --dev ' + interfaceB + ' --wan')
        #shell('./cpumap-pping/src/tc_classify --dev-egress ' + interfaceA)
        #shell('./cpumap-pping/src/tc_classify --dev-egress ' + interfaceB)
        xdpEndTime = datetime.now()


        # Execute actual Linux TC commands
        tcStartTime = datetime.now()
        # print("Executing linux TC class/qdisc commands")
        if observe_mode:
            linuxTCcommands = []
        linux_tc_path = get_linux_tc_path()
        ensure_parent_dir(linux_tc_path)
        with open(linux_tc_path, 'w') as f:
            for command in linuxTCcommands:
                logging.info(command)
                f.write(f"{command}\n")
        # if logging.DEBUG <= logging.root.level:
        # 	# Do not --force in debug mode, so we can see any errors
        # 	shell("/sbin/tc -b linux_tc.txt")
        # else:
        # 	shell("/sbin/tc -f -b linux_tc.txt")
        bakery.commit()
        tcEndTime = datetime.now()
        # print("Executed " + str(len(linuxTCcommands)) + " linux TC class/qdisc commands")

        # Execute actual XDP-CPUMAP-TC filter commands
        xdpFilterStartTime = datetime.now()
        print("Executing XDP-CPUMAP-TC IP filter commands")
        numXdpCommands = ipMapBatch.length()
        if enable_actual_shell_commands():
            if observe_mode:
                print("Observe mode active; skipping XDP-CPUMAP-TC IP filter apply after clearing mappings")
                numXdpCommands = 0
            else:
                ipMapBatch.finish_ip_mappings()
                try:
                    ipMapBatch.submit()
                except Exception as e:
                    report_refresh_failure(
                        "XDP_IP_MAPPING_APPLY_FAILED",
                        "Failed to apply XDP IP mappings: " + str(e),
                        {
                            "required_ip_mappings": requiredIpMappings,
                            "queued_requests": numXdpCommands,
                            "on_a_stick": on_a_stick(),
                        },
                        "XDP_IP_MAPPING_APPLY_FAILED",
                    )
            #for command in xdpCPUmapCommands:
            #	logging.info(command)
            #	commands = command.split(' ')
            #	proc = subprocess.Popen(commands, stdout=subprocess.DEVNULL)
        else:
            ipMapBatch.log()
            #for command in xdpCPUmapCommands:
            #	logging.info(command)
        print("Executed " + str(numXdpCommands) + " XDP-CPUMAP-TC IP filter commands")
        #print(xdpCPUmapCommands)
        xdpFilterEndTime = datetime.now()


        # Record end time of all reload commands
        reloadEndTime = datetime.now()


        # Recap - warn operator if devices were skipped
        validParentNodes = collect_parent_node_names(network)
        devicesSkipped = build_unshaped_device_report(
            subscriberCircuits,
            shapedDeviceKeys,
            validParentNodes,
            flat_network,
        )
        if len(devicesSkipped) > 0:
            flat_mode_generated_parent_only = flat_network and all(
                entry.get("reasonCode") == "unattached_flat_network" for entry in devicesSkipped
            )
            if flat_mode_generated_parent_only:
                print(
                    f"Flat network mode assigned {len(devicesSkipped)} device(s) to generated parent queues."
                )
            else:
                warnings.warn(
                    str(len(devicesSkipped)) + " device(s) were not shaped. Detailed reasons are listed below.",
                    stacklevel=2,
                )
                print("Devices not shaped:")
                for entry in devicesSkipped:
                    print(format_unshaped_device_line(entry))

        # DIY/manual mode snapshots the active ShapedDevices.csv for update-only diffs.
        # Built-in integrations no longer emit ShapedDevices.csv, so skip the legacy
        # snapshot there instead of failing the whole scheduler refresh.
        if os.path.isfile(shapedDevicesFile):
            last_loaded_path = get_state_path("shaping", "ShapedDevices.lastLoaded.csv")
            ensure_parent_dir(last_loaded_path)
            shutil.copyfile(shapedDevicesFile, last_loaded_path)

        # Save for stats
        stats_by_circuit_path = get_state_path("stats", "statsByCircuit.json")
        ensure_parent_dir(stats_by_circuit_path)
        with open(stats_by_circuit_path, 'w') as f:
            f.write(json.dumps(subscriberCircuits, indent=4))
        stats_by_parent_node_path = get_state_path("stats", "statsByParentNode.json")
        ensure_parent_dir(stats_by_parent_node_path)
        with open(stats_by_parent_node_path, 'w') as f:
            f.write(json.dumps(parentNodes, indent=4))


        # Record time this run completed at
        # filename = os.path.join(_here, 'lastRun.txt')
        last_run_path = get_state_path("stats", "lastRun.txt")
        ensure_parent_dir(last_run_path)
        with open(last_run_path, 'w') as file:
            file.write(datetime.now().strftime("%d-%b-%Y (%H:%M:%S.%f)"))


        # Report reload time
        reloadTimeSeconds = ((reloadEndTime - reloadStartTime).seconds) + (((reloadEndTime - reloadStartTime).microseconds) / 1000000)
        tcTimeSeconds = ((tcEndTime - tcStartTime).seconds) + (((tcEndTime - tcStartTime).microseconds) / 1000000)
        xdpSetupTimeSeconds = ((xdpEndTime - xdpStartTime).seconds) + (((xdpEndTime - xdpStartTime).microseconds) / 1000000)
        xdpFilterTimeSeconds = ((xdpFilterEndTime - xdpFilterStartTime).seconds) + (((xdpFilterEndTime - xdpFilterStartTime).microseconds) / 1000000)
        print("Queue and IP filter reload completed in " + "{:g}".format(round(reloadTimeSeconds,1)) + " seconds")
        print("\tTC commands: \t" + "{:g}".format(round(tcTimeSeconds,1)) + " seconds")
        print("\tXDP setup: \t " + "{:g}".format(round(xdpSetupTimeSeconds,1)) + " seconds")
        print("\tXDP filters: \t " + "{:g}".format(round(xdpFilterTimeSeconds,4)) + " seconds")


        # Done
        print("refreshShapers completed on " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))

def refreshShapersUpdateOnly():
    # Starting
    print("refreshShapers starting at " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))


    # Warn user if enableActualShellCommands is False, because that would mean no actual commands are executing
    if enable_actual_shell_commands() == False:
        warnings.warn("enableActualShellCommands is set to False. None of the commands below will actually be executed. Simulated run.", stacklevel=2)


    # Files
    shapedDevicesFile = get_shaped_devices_path()
    networkJSONfile = get_network_json_path()


    # Check validation
    safeToRunRefresh = False
    if (validateNetworkAndDevices() == True):
        safeToRunRefresh = True
    else:
        warnings.warn("Validation failed. Will now exit.", stacklevel=2)

    if safeToRunRefresh == True:
        networkChanged = False
        devicesChanged = False
        # Check for changes to network.json
        last_good_config_json_path = get_last_good_config_json_path()
        if os.path.isfile(last_good_config_json_path):
            with open(last_good_config_json_path, 'r') as j:
                originalNetwork = json.loads(j.read())
            with open(networkJSONfile, 'r') as j:
                newestNetwork = json.loads(j.read())
            ddiff = DeepDiff(originalNetwork, newestNetwork, ignore_order=True)
            if ddiff != {}:
                networkChanged = True

        # Check for changes to ShapedDevices.csv
        newlyUpdatedSubscriberCircuits,	newlyUpdatedDictForCircuitsWithoutParentNodes = loadSubscriberCircuits(shapedDevicesFile)
        lastLoadedSubscriberCircuits, lastLoadedDictForCircuitsWithoutParentNodes = loadSubscriberCircuits(get_last_loaded_shaped_devices_path())

        newlyUpdatedSubscriberCircuitsByID = {}
        for circuit in newlyUpdatedSubscriberCircuits:
            circuitid = circuit['circuitID']
            newlyUpdatedSubscriberCircuitsByID[circuitid] = circuit

        lastLoadedSubscriberCircuitsByID = {}
        for circuit in lastLoadedSubscriberCircuits:
            circuitid = circuit['circuitID']
            lastLoadedSubscriberCircuitsByID[circuitid] = circuit

        for circuitID, circuit in lastLoadedSubscriberCircuitsByID.items():
            if (circuitID in newlyUpdatedSubscriberCircuitsByID) and (circuitID in lastLoadedSubscriberCircuitsByID):
                if newlyUpdatedSubscriberCircuitsByID[circuitID]['maxDownload'] != lastLoadedSubscriberCircuitsByID[circuitID]['maxDownload']:
                    devicesChanged = True
                if newlyUpdatedSubscriberCircuitsByID[circuitID]['maxUpload'] != lastLoadedSubscriberCircuitsByID[circuitID]['maxUpload']:
                    devicesChanged = True
                if newlyUpdatedSubscriberCircuitsByID[circuitID]['minDownload'] != lastLoadedSubscriberCircuitsByID[circuitID]['minDownload']:
                    devicesChanged = True
                if newlyUpdatedSubscriberCircuitsByID[circuitID]['minUpload'] != lastLoadedSubscriberCircuitsByID[circuitID]['minUpload']:
                    devicesChanged = True
                if newlyUpdatedSubscriberCircuitsByID[circuitID]['devices'] != lastLoadedSubscriberCircuitsByID[circuitID]['devices']:
                    devicesChanged = True
                if newlyUpdatedSubscriberCircuitsByID[circuitID]['ParentNode'] != lastLoadedSubscriberCircuitsByID[circuitID]['ParentNode']:
                    devicesChanged = True
                if newlyUpdatedSubscriberCircuitsByID[circuitID].get('ParentNodeID', '') != lastLoadedSubscriberCircuitsByID[circuitID].get('ParentNodeID', ''):
                    devicesChanged = True
            else:
                devicesChanged = True
        for circuitID, circuit in newlyUpdatedSubscriberCircuitsByID.items():
            if (circuitID not in lastLoadedSubscriberCircuitsByID):
                devicesChanged = True


        if devicesChanged or networkChanged:
            print('Observed changes to runtime shaping inputs. Applying full reload now')
            refreshShapers()
        else:
            print('Observed no changes to runtime shaping inputs. Leaving queues as is.')

        # Done
        print("refreshShapersUpdateOnly completed on " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))

if __name__ == '__main__':
    # Check that the host lqosd is running
    if is_lqosd_alive():
        print("lqosd is running")
    else:
        print("ERROR: lqosd is not running. Aborting")
        os._exit(-1)

    # Check that the configuration file is usable
    if check_config():
        print("Configuration from /etc/lqos.conf is usable")
    else:
        print("ERROR: Unable to load configuration from /etc/lqos.conf")
        os._exit(-1)

    # Check that we aren't running LibreQoS.py more than once at a time
    if is_libre_already_running():
        print("LibreQoS.py is already running in another process. Aborting.")
        os._exit(-1)

    # We've got this far, so create a lock file
    create_lock_file()

    parser = argparse.ArgumentParser()
    parser.add_argument(
        '-d', '--debug',
        help="Print lots of debugging statements",
        action="store_const", dest="loglevel", const=logging.DEBUG,
        default=logging.WARNING,
    )
    parser.add_argument(
        '-v', '--verbose',
        help="Be verbose",
        action="store_const", dest="loglevel", const=logging.INFO,
    )
    parser.add_argument(
        '--updateonly',
        help="Only update to reflect changes in ShapedDevices.csv (partial reload)",
        action=argparse.BooleanOptionalAction,
    )
    parser.add_argument(
        '--validate',
        help="Just validate network.json and ShapedDevices.csv",
        action=argparse.BooleanOptionalAction,
    )
    parser.add_argument(
        '--clearrules',
        help="Clear ip filters, qdiscs, and xdp setup if any",
        action=argparse.BooleanOptionalAction,
    )
    parser.add_argument(
        '--planner-reset',
        help="Delete planner state file before running",
        action=argparse.BooleanOptionalAction,
    )
    args = parser.parse_args()
    logging.basicConfig(level=args.loglevel)

    if getattr(args, 'planner_reset', False):
        try:
            state_path = get_planner_state_path()
            if os.path.exists(state_path):
                os.remove(state_path)
                print(f"Removed planner state: {state_path}")
        except Exception as e:
            print(f"Warning: could not remove planner state: {e}")

    exit_code = 0
    try:
        if args.validate:
            status = validateNetworkAndDevices()
        elif args.clearrules:
            tearDown(interface_a(), interface_b())
        elif args.updateonly:
            # Single-interface updates don't work at all right now.
            if on_a_stick():
                print("--updateonly is not supported for single-interface configurations")
                exit_code = -1
            else:
                refreshShapersUpdateOnly()
        else:
            # Refresh and/or set up queues
            refreshShapers()
    except RefreshFailure:
        exit_code = 1
    finally:
        free_lock_file()

    if exit_code != 0:
        os._exit(exit_code)
