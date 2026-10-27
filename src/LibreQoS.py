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

from shaping_skip_report import (
    build_unshaped_device_report,
    collect_parent_node_names,
    device_shaping_key,
    format_unshaped_device_line,
)

from liblqos_python import is_lqosd_alive, validate_shaped_devices, \
    is_libre_already_running, create_lock_file, free_lock_file, \
    check_config, sqm, upstream_bandwidth_capacity_download_mbps, upstream_bandwidth_capacity_upload_mbps, \
    interface_a, interface_b, enable_actual_shell_commands, use_bin_packing_to_balance_cpu, queue_mode, \
    run_shell_commands_as_sudo, generated_pn_download_mbps, generated_pn_upload_mbps, queues_available_override, \
    on_a_stick, get_tree_weights, get_weights, is_network_flat, get_libreqos_directory, enable_insight_topology, \
    is_insight_enabled, scheduler_error, \
    overrides_circuit_adjustments_effective, \
    automatic_import_uisp, automatic_import_splynx, automatic_import_powercode, automatic_import_sonar, \
    automatic_import_wispgate, automatic_import_netzur, automatic_import_visp, \
    plan_top_level_cpu_bins, \
    plan_class_identities, \
    shaping_cpu_count

# K3-A: the kernel XDP/TC datapath (and its Bakery) was removed; LibreQoS is
# bridge-only. The kernel-only liblqos_python bindings no longer exist. These
# shims keep this module importable (flake8 F82-safe) and are INERT on the
# bridge path (the kernel path is dead), so refreshShapers() runs to completion
# without applying kernel mappings. The full python repoint rides 3a.
_K3A_WARNED = set()


def _k3a_removed(name):
    def _removed(*_args, **_kwargs):
        if name not in _K3A_WARNED:
            _K3A_WARNED.add(name)
            logging.warning(
                "%s is a no-op: the kernel datapath was removed (K3); the bridge is the shaper.",
                name,
            )
        return 0
    return _removed


add_ip_mapping = _k3a_removed("add_ip_mapping")
clear_ip_mappings = _k3a_removed("clear_ip_mappings")
delete_ip_mapping = _k3a_removed("delete_ip_mapping")
xdp_ip_mapping_capacity = _k3a_removed("xdp_ip_mapping_capacity")


class _K3aRemovedClass:
    def __init__(self, *_args, **_kwargs):
        raise RuntimeError(
            f"{type(self).__name__} is unavailable: the kernel datapath was removed (K3-A)"
        )


class BatchedCommands(_K3aRemovedClass):
    pass


class Bakery(_K3aRemovedClass):
    pass


# K3-B: the XDP/TC datapath that these objects drive is gone, so the kernel
# path in refreshShapers() is dead. This predicate mode-branches the two
# construction sites only; the full python repoint rides 3a.
KERNEL_DATAPATH_AVAILABLE = False  # K3: the XDP/TC datapath was removed; LibreQoS is bridge-only.


class _K3bInertBatchedCommands:
    """Inert stand-in for BatchedCommands(); exists only until the 3a repoint.

    Lets the (dead) kernel-only calls later in refreshShapers() run harmlessly
    on the bridge path without constructing the removed kernel binding.
    """

    def add_ip_mapping(self, *_args, **_kwargs):
        return None

    def length(self):
        return 0

    def log(self, *_args, **_kwargs):
        return None

    def finish_ip_mappings(self, *_args, **_kwargs):
        return None

    def submit(self, *_args, **_kwargs):
        return None


class _K3bInertBakery:
    """Inert stand-in for Bakery(); exists only until the 3a repoint."""

    def start_batch(self, *_args, **_kwargs):
        return self

    def setup_mq(self, *_args, **_kwargs):
        return self

    def add_site(self, *_args, **_kwargs):
        return self

    def add_circuit(self, *_args, **_kwargs):
        return self

    def estimate_qdisc_budget(self, *_args, **_kwargs):
        return {"ok": True}

    def commit(self, *_args, **_kwargs):
        return self


# Optional: urgent issue helpers (available in newer liblqos_python)
try:
    from liblqos_python import submit_urgent_issue  # type: ignore
except Exception:
    def submit_urgent_issue(*_args, **_kwargs):
        return False

try:
    from liblqos_python import clear_urgent_issue_by_identity  # type: ignore
except ImportError:
    _missing_clear_urgent_issue_by_identity_warned = False

    def clear_urgent_issue_by_identity(*_args, **_kwargs):
        global _missing_clear_urgent_issue_by_identity_warned
        if not _missing_clear_urgent_issue_by_identity_warned:
            logging.warning("Unable to clear recovered urgent issues: liblqos_python helper is unavailable")
            _missing_clear_urgent_issue_by_identity_warned = True
        return False

try:
    from liblqos_python import xdp_ip_mapping_ready  # type: ignore
except ImportError:
    _missing_xdp_ip_mapping_ready_warned = False

    def xdp_ip_mapping_ready():
        global _missing_xdp_ip_mapping_ready_warned
        if not _missing_xdp_ip_mapping_ready_warned:
            logging.warning("Unable to check XDP IP mapping readiness: liblqos_python helper is unavailable")
            _missing_xdp_ip_mapping_ready_warned = True
        return True

try:
    from liblqos_python import validated_runtime_shaping_inputs_path  # type: ignore
except ImportError:
    def validated_runtime_shaping_inputs_path():
        return None

try:
    from liblqos_python import verify_published_shaping_generation  # type: ignore
except ImportError:
    def verify_published_shaping_generation():
        return None

try:
    from liblqos_python import get_libreqos_state_directory as _get_state_dir_native
except Exception:
    _get_state_dir_native = None


class RefreshFailure(Exception):
    pass


class ValidationFailure(RefreshFailure):
    pass


XDP_IP_MAPPING_APPLY_FAILED = "XDP_IP_MAPPING_APPLY_FAILED"
XDP_MAPPING_READY_TIMEOUT_SECONDS = 5.0
XDP_MAPPING_READY_INTERVAL_SECONDS = 0.1




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


def get_runtime_state_path(category, filename):
    return get_state_path(category, filename)



def get_network_json_path():
    base_dir = get_libreqos_directory()
    if enable_insight_topology():
        insight_path = os.path.join(base_dir, "network.insight.json")
        if os.path.exists(insight_path):
            return insight_path

    # Either insight not enabled, or file doesn't exist
    return os.path.join(base_dir, "network.json")



def get_planner_state_path():
    return get_runtime_state_path("shaping", "planner_state.json")













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

def clearPriorSettings(interfaceA, interfaceB):
    if enable_actual_shell_commands():
        if 'mq' in shellReturn('tc qdisc show dev ' + interfaceA + ' root'):
            print('MQ detected. Will delete and recreate mq qdisc.')
            # Clear tc filter
            shell('tc qdisc delete dev ' + interfaceA + ' root')
            shell('tc qdisc delete dev ' + interfaceB + ' root')

def tearDown(interfaceA, interfaceB):
    # Full teardown of everything for exiting LibreQoS
    if enable_actual_shell_commands():
        # Clear IP filters and remove xdp program from interfaces
        # The bakery tracks and prunes mappings; avoid clearing everything here.
        clearPriorSettings(interfaceA, interfaceB)

def validateNetworkAndDevices():
    # Verify Network.json is valid json
    networkValidatedOrNot = True
    # Verify ShapedDevices.csv is valid
    devicesValidatedOrNot = True # True by default, switches to false if ANY entry in ShapedDevices.csv fails validation
    # Phase C S4: the compiled import is the ONE source.
    integration_ingress = True

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
    network_json_path = get_network_json_path()
    if not os.path.isfile(network_json_path):
        if integration_ingress:
            warnings.warn(
                f"Integration topology has not published an effective network tree yet: {network_json_path}",
                stacklevel=2,
            )
        else:
            warnings.warn(f"network.json is missing: {network_json_path}", stacklevel=2)
        networkValidatedOrNot = False
    else:
        with open(network_json_path) as file:
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



def refreshShapers():
    # Phase C S5: the topology runtime/bridge is the shaper; the legacy
    # file/apply refresh engine retired whole.
    print(
        "refreshShapers: the topology runtime/bridge is the shaper; skipping the "
        "legacy file chain."
    )
    return True

def refreshShapersUpdateOnly():
    # Phase C S5: the runtime/bridge is the shaper; the legacy file chain
    # retired whole.
    print("refreshShapersUpdateOnly: skipping the legacy file chain.")
    return True

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
