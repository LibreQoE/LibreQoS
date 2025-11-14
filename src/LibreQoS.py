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

from liblqos_python import is_lqosd_alive, clear_ip_mappings, delete_ip_mapping, validate_shaped_devices, \
    is_libre_already_running, create_lock_file, free_lock_file, add_ip_mapping, BatchedCommands, \
    check_config, sqm, upstream_bandwidth_capacity_download_mbps, upstream_bandwidth_capacity_upload_mbps, \
    interface_a, interface_b, enable_actual_shell_commands, use_bin_packing_to_balance_cpu, monitor_mode_only, \
    run_shell_commands_as_sudo, generated_pn_download_mbps, generated_pn_upload_mbps, queues_available_override, \
    on_a_stick, get_tree_weights, get_weights, is_network_flat, get_libreqos_directory, enable_insight_topology, \
    fast_queues_fq_codel, hash_to_i64, fetch_planner_remote, store_planner_remote, \
    Bakery

# Optional: urgent issue submission (available in newer liblqos_python)
try:
    from liblqos_python import submit_urgent_issue  # type: ignore
except Exception:
    def submit_urgent_issue(*_args, **_kwargs):
        return False

# Optional: check if Insight is enabled (available in newer liblqos_python)
try:
    from liblqos_python import is_insight_enabled  # type: ignore
except Exception:
    def is_insight_enabled(*_args, **_kwargs):
        return False

class Planner:
    """
    Planner: lightweight state manager to reduce churn between runs.

    PUBLIC methods:
    - getState(): dict — current planner state (default or loaded)
    - persist(): None — persist planner state via Insight when enabled
    """

    # Print the Insight planning banner only once per process
    _insight_banner_printed = False

    def __init__(self, queuesAvailable: int, onAStick: bool, siteNamesSet, siteCount: int):
        now_ts = time.time()

        # Informational banner when Insight-enhanced planning is in use
        try:
            if is_insight_enabled() and not Planner._insight_banner_printed:
                logging.info("Using Insight enhanced planning (remote planner state enabled).")
                Planner._insight_banner_printed = True
        except Exception:
            pass

        loaded = self._load_state(queuesAvailable, onAStick, siteCount)
        # Normalize site names by hashing to i64 for storage (no strings retained)
        normalized_site_names = []
        try:
            normalized_site_names = sorted([int(hash_to_i64(str(x))) for x in list(siteNamesSet)])
        except Exception:
            normalized_site_names = []

        self._is_default = False
        if loaded is None:
            # Missing/invalid planner state: build a fresh default and record why.
            try:
                if is_insight_enabled():
                    logging.warning("Planner: no existing plan available from Insight; using default plan.")
                else:
                    logging.warning("Planner: no existing plan available (Insight disabled); using default plan.")
            except Exception:
                pass
            self.state = self._default_state(normalized_site_names, siteCount, queuesAvailable, onAStick, now_ts)
            self._is_default = True
            return

        # Validate loaded state and decide if we should rebuild
        should_rebuild, reason = self._should_rebuild(
            loaded, normalized_site_names, siteCount, queuesAvailable, onAStick, now_ts
        )
        if should_rebuild:
            # Existing planner state is not compatible with the current configuration.
            try:
                logging.warning(f"Planner: discarding existing plan; reason={reason}. Falling back to default plan.")
            except Exception:
                pass
            self.state = self._default_state(normalized_site_names, siteCount, queuesAvailable, onAStick, now_ts)
            self._is_default = True
        else:
            # Accept loaded state, convert legacy string keys to hashed if needed, rebuild ephemeral fields, and update dynamic fields
            st = dict(loaded)
            st['updated_at'] = now_ts
            # Convert site_names (list) to hashed ints if necessary
            try:
                raw_names = st.get('site_names', [])
                hashed = []
                for x in raw_names:
                    try:
                        hashed.append(int(x))
                    except Exception:
                        hashed.append(int(hash_to_i64(str(x))))
                st['site_names'] = sorted(list(set(hashed)))
            except Exception:
                st['site_names'] = []
            # Convert site_map keys to ints (hash) when possible
            try:
                sm0 = st.get('site_map') if isinstance(st, dict) else None
                sm: dict = {}
                if isinstance(sm0, dict):
                    for k, v in sm0.items():
                        try:
                            key_int = int(k)
                        except Exception:
                            key_int = int(hash_to_i64(str(k)))
                        sm[key_int] = v
                st['site_map'] = sm
            except Exception:
                st['site_map'] = {}
            # Convert circuit_map keys to ints (hash) when possible
            try:
                cm0 = st.get('circuit_map') if isinstance(st, dict) else None
                cm: dict = {}
                if isinstance(cm0, dict):
                    for k, v in cm0.items():
                        try:
                            key_int = int(k)
                        except Exception:
                            key_int = int(hash_to_i64(str(k)))
                        cm[key_int] = v
                st['circuit_map'] = cm
            except Exception:
                st['circuit_map'] = {}
            # Rebuild free_minors from hashed maps
            try:
                q = int(st.get('queuesAvailable', 0))
                sm = st.get('site_map') if isinstance(st, dict) else None
                cm = st.get('circuit_map') if isinstance(st, dict) else None
                st['free_minors'] = self._rebuild_free_minors(q, sm if isinstance(sm, dict) else {}, cm if isinstance(cm, dict) else {})
            except Exception:
                st['free_minors'] = [[] for _ in range(int(st.get('queuesAvailable', 0)))]
            self.state = st
            self._is_default = False
            

    def getState(self):
        return self.state

    def persist(self):
        """Persist planner state.

        Insight-only feature: when Insight is enabled, planner state is
        persisted remotely via `store_planner_remote`. Ephemeral fields such
        as `free_minors` are never persisted, and no local CBOR files are used.
        """
        try:
            to_save = dict(self.state)
            if 'free_minors' in to_save:
                try:
                    to_save.pop('free_minors')
                except Exception:
                    pass
            # Insight-only feature: when Insight is disabled, skip persistence
            # entirely so planner reuse remains an Insight capability.
            use_remote = False
            try:
                use_remote = bool(is_insight_enabled())
            except Exception:
                use_remote = False
            if use_remote:
                store_planner_remote(to_save)
            else:
                # No local fallback; skip persistence entirely when Insight is
                # not enabled so that planner reuse remains an Insight feature.
                pass
        except Exception as e:
            warnings.warn(f"Failed to persist planner state to Insight: {e}", stacklevel=2)

    def _default_state(self, site_names_sorted, site_count, queues_available, on_a_stick_flag, now_ts):
        """Build a default planner state.

        Notes
        - Minor IDs 1..3 are globally reserved:
          1 = root qdisc, 2 = default class, 3 = per-CPU Generated_PN_* site classes
        - All other sites, circuits, and any ephemeral classes use 4+.
        - `free_minors` is per-CPU and rebuilt on load; it is not persisted.
        """
        try:
            # Reserve minors 1..3 globally:
            #  - 1 is used by root qdisc
            #  - 2 is used by the default class under root
            #  - 3 is used for per-CPU Generated_PN_* site classes
            # All other site and circuit classes must use 4+
            free_minors = [list(range(4, 0x10000)) for _ in range(int(queues_available))]
        except Exception:
            free_minors = []
        return {
            'algo_version': 'v1',
            'updated_at': float(now_ts),
            'queuesAvailable': int(queues_available),
            'on_a_stick': bool(on_a_stick_flag),
            'site_count': int(site_count),
            'site_names': list(site_names_sorted),  # list of i64 hashes
            'site_map': {},            # site_hash(i64) -> { cpu:int, major:int, minor:int }
            'circuit_map': {},         # circuit_hash(i64) -> { cpu:int, minor:int }
            'free_minors': free_minors,  # ephemeral: rebuilt on load; not persisted to disk
        }

    def _load_state(self, queues_available, on_a_stick_flag, site_count):
        """Load planner state.

        Insight-only feature: if Insight is enabled, fetch planner state from
        the remote API. When Insight is disabled, no planner state is used.

        - Validates that minimum required keys exist.
        - Normalizes `updated_at` when remote to avoid needless rebuilds based
          on age alone.
        """
        try:
            # Try Insight first if enabled; otherwise treat as no state
            use_remote = False
            try:
                use_remote = bool(is_insight_enabled())
            except Exception:
                use_remote = False
            if use_remote:
                try:
                    data = fetch_planner_remote(int(queues_available), bool(on_a_stick_flag), int(site_count))
                    if isinstance(data, dict):
                        for k in ['queuesAvailable', 'site_count', 'site_names']:
                            if k not in data:
                                return None
                        # Normalize dynamic fields for rebuild check
                        try:
                            data['updated_at'] = float(time.time())
                        except Exception:
                            pass
                        return data
                except Exception:
                    pass
                return None
            else:
                # Insight is disabled or unavailable: no planner state is used.
                return None
        except Exception:
            return None

    def _rebuild_free_minors(self, queues_available, site_map, circuit_map):
        """
        Recompute free_minors per CPU from persisted mappings.
        - Start with full range [4..0xFFFF] for each CPU (1..3 reserved)
        - Remove any minors referenced by site_map or circuit_map on the corresponding CPU
        Returns: list[list[int]] sized by queues_available
        """
        try:
            q = max(0, int(queues_available))
        except Exception:
            q = 0
        # See rationale above; reserve minors 1..3.
        free = [set(range(4, 0x10000)) for _ in range(q)]
        # Remove site minors
        try:
            for name, entry in site_map.items():
                try:
                    cpu = int(entry.get('cpu', -1))
                    minor = int(entry.get('minor', -1))
                    if 0 <= cpu < q and 4 <= minor <= 0xFFFF:
                        if minor in free[cpu]:
                            free[cpu].discard(minor)
                except Exception:
                    continue
        except Exception:
            pass
        # Remove circuit minors
        try:
            for cid, entry in circuit_map.items():
                try:
                    cpu = int(entry.get('cpu', -1))
                    minor = int(entry.get('minor', -1))
                    if 0 <= cpu < q and 4 <= minor <= 0xFFFF:
                        if minor in free[cpu]:
                            free[cpu].discard(minor)
                except Exception:
                    continue
        except Exception:
            pass
        # Convert to sorted lists
        return [sorted(list(s)) for s in free]

    def _should_rebuild(self, loaded, site_names_sorted, site_count, queues_available, on_a_stick_flag, now_ts):
        # Age check
        try:
            updated_at = float(loaded.get('updated_at', 0.0))
        except Exception:
            updated_at = 0.0
        # Consider planner state stale only after 7 days to avoid churn when
        # remote planner objects are otherwise valid but old.
        if (float(now_ts) - updated_at) > (7 * 24 * 3600):
            return True, 'stale_planner_state'

        # queuesAvailable change
        try:
            if int(loaded.get('queuesAvailable', -1)) != int(queues_available):
                return True, 'queues_available_changed'
        except Exception:
            return True, 'queues_available_invalid'

        # on-a-stick changed
        try:
            if bool(loaded.get('on_a_stick', False)) != bool(on_a_stick_flag):
                return True, 'on_a_stick_changed'
        except Exception:
            return True, 'on_a_stick_invalid'

        # Site count changed
        try:
            if int(loaded.get('site_count', -1)) != int(site_count):
                return True, 'site_count_changed'
        except Exception:
            return True, 'site_count_invalid'

        # Site set changed (order ignored). Compare hashed ints
        try:
            raw_loaded = loaded.get('site_names', [])
            loaded_sites = set()
            for x in raw_loaded:
                try:
                    loaded_sites.add(int(x))
                except Exception:
                    loaded_sites.add(int(hash_to_i64(str(x))))
            current_sites = set([int(x) for x in site_names_sorted])
            if loaded_sites != current_sites:
                return True, 'site_names_changed'
        except Exception:
            return True, 'site_names_invalid'

        return False, None

def _greedy_binpack(items, bin_ids, capacities):
    """
    Simple greedy binpacking without state:
    - items: list of dicts with keys 'id' and 'weight'
    - bin_ids: list of bin identifiers
    - capacities: dict mapping bin_id -> capacity (float)
    Returns: dict mapping item_id -> bin_id
    """
    # Normalize inputs
    cap = {str(b): float(capacities.get(b, 1.0)) for b in bin_ids}
    loads = {str(b): 0.0 for b in bin_ids}
    # Sort items: heavier first, deterministic by id
    sorted_items = sorted(
        [(str(it.get('id')), float(it.get('weight', 1.0))) for it in items],
        key=lambda t: (-t[1], t[0])
    )
    assignment = {}
    for it_id, w in sorted_items:
        # Choose the bin with lowest load/cap ratio; tie-breaker: lexicographic bin id
        best_bin = min(bin_ids, key=lambda b: (loads[str(b)]/max(cap[str(b)], 1e-9), str(b)))
        assignment[it_id] = str(best_bin)
        loads[str(best_bin)] += w
    return assignment

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

def get_network_json_path():
    base_dir = get_libreqos_directory()

    if enable_insight_topology():
        insight_path = os.path.join(base_dir, "network.insight.json")
        if os.path.exists(insight_path):
            return insight_path

    # Either insight not enabled, or file doesn't exist
    return os.path.join(base_dir, "network.json")

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
    if os.path.isfile("lastRun.txt"):
        with open("lastRun.txt", 'r') as file:
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

    # Verify that the Rust side of things can read the CSV file
    rustValid = validate_shaped_devices()
    if rustValid == "OK":
        print("Rust validated ShapedDevices.csv")
    else:
        warnings.warn("Rust failed to validate ShapedDevices.csv", stacklevel=2)
        warnings.warn(rustValid, stacklevel=2)
        devicesValidatedOrNot = False
    with open(get_network_json_path()) as file:
        try:
            data = json.load(file) # put JSON-data to a variable
            if data != {}:
                #Traverse
                observedNodes = {} # Will not be used later
                def traverseToVerifyValidity(data):
                    for elem in data:
                        if isinstance(elem, str):
                            if (isinstance(data[elem], dict)) and (elem != 'children'):
                                if elem not in observedNodes:
                                    observedNodes[elem] = {'downloadBandwidthMbps': data[elem]['uploadBandwidthMbps'], 'downloadBandwidthMbps': data[elem]['uploadBandwidthMbps']}
                                    if 'children' in data[elem]:
                                        traverseToVerifyValidity(data[elem]['children'])
                                else:
                                    warnings.warn("Non-unique Node name in network.json: " + elem, stacklevel=2)
                                    networkValidatedOrNot = False
                traverseToVerifyValidity(data)
                if len(observedNodes) < 1:
                    warnings.warn("network.json had 0 valid nodes. Only {} is accepted for that scenario.", stacklevel=2)
                    networkValidatedOrNot = False
        except json.decoder.JSONDecodeError:
            warnings.warn("network.json is an invalid JSON file", stacklevel=2) # in case json is invalid
            networkValidatedOrNot = False
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
        #Remove comments if any
        commentsRemoved = []
        for row in csv_reader:
            if not row[0].startswith('#'):
                commentsRemoved.append(row)
        #Remove header
        commentsRemoved.pop(0)
        seenTheseIPsAlready = []
        for row in commentsRemoved:
            # Accept optional 14th column 'sqm' but ignore here (validation focuses on core fields)
            circuitID, circuitName, deviceID, deviceName, ParentNode, mac, ipv4_input, ipv6_input, downloadMin, uploadMin, downloadMax, uploadMax, comment = row[0:13]
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
                            seenTheseIPsAlready.append(ipEntry)
                        else:
                            if (type(ipaddress.ip_network(ipEntry)) is ipaddress.IPv4Network) or (type(ipaddress.ip_address(ipEntry)) is ipaddress.IPv4Address):
                                ipv4_subnets_and_hosts.extend(ipEntry)
                            else:
                                warnings.warn("Provided IPv4 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
                                devicesValidatedOrNot = False
                            seenTheseIPsAlready.append(ipEntry)
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
                            seenTheseIPsAlready.append(ipEntry)
                        else:
                            if (type(ipaddress.ip_network(ipEntry)) is ipaddress.IPv6Network) or (type(ipaddress.ip_address(ipEntry)) is ipaddress.IPv6Address):
                                ipv6_subnets_and_hosts.extend(ipEntry)
                            else:
                                warnings.warn("Provided IPv6 '" + ipEntry + "' in ShapedDevices.csv at row " + str(rowNum) + " is not valid.", stacklevel=2)
                                devicesValidatedOrNot = False
                            seenTheseIPsAlready.append(ipEntry)
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
    knownCircuitIDs = []
    counterForCircuitsWithoutParentNodes = 0
    dictForCircuitsWithoutParentNodes = {}
    # If the network is flat, treat any explicit parent_site in CSV as 'none'
    # so circuits are assigned to Generated_PN_* bins automatically when switching to flat.
    try:
        flat_mode_csv = bool(is_network_flat())
    except Exception:
        flat_mode_csv = False
    with open(shapedDevicesFile) as csv_file:
        csv_reader = csv.reader(csv_file, delimiter=',')
        # Remove comments if any
        commentsRemoved = []
        for row in csv_reader:
            if not row[0].startswith('#'):
                commentsRemoved.append(row)
        # Remove header
        commentsRemoved.pop(0)
        for row in commentsRemoved:
            # Optional per-circuit SQM override in last column
            sqm_override_token = ''
            if len(row) > 13:
                # Normalize: lowercase, trim, collapse spaces around '/'
                raw_token = row[13]
                token = raw_token.strip().lower()
                if '/' in token:
                    parts = token.split('/', 1)
                    left = parts[0].strip()
                    right = parts[1].strip()
                    token = left + '/' + right
                sqm_override_token = token
            circuitID, circuitName, deviceID, deviceName, ParentNode, mac, ipv4_input, ipv6_input, downloadMin, uploadMin, downloadMax, uploadMax, comment = row[0:13]
            # In flat mode, override any non-empty, non-Generated_PN_* ParentNode to 'none'
            try:
                if flat_mode_csv:
                    pn = (ParentNode or '').strip()
                    pn_lower = pn.lower()
                    if (pn != ''
                        and pn_lower != 'none'
                        and not pn.startswith('Generated_PN_')
                        and not pn_lower.startswith('generated_pn_')):
                        ParentNode = 'none'
            except Exception:
                pass
            # If in monitorOnlyMode, override bandwidth rates to where no shaping will actually occur
            if monitor_mode_only() == True:
                downloadMin = 10000
                uploadMin = 10000
                downloadMax = 10000
                uploadMax = 10000
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
                if circuitID in knownCircuitIDs:
                    for circuit in subscriberCircuits:
                        if circuit['circuitID'] == circuitID:
                            if circuit['ParentNode'] != "none":
                                if circuit['ParentNode'] != ParentNode:
                                    errorMessageString = "Device " + deviceName + " with deviceID " + deviceID + " had different Parent Node from other devices of circuit ID #" + circuitID
                                    raise ValueError(errorMessageString)
                            # Check if bandwidth parameters match other cdevices of this same circuit ID, but only check if monitorOnlyMode is Off
                            if monitor_mode_only() == False:
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
                    knownCircuitIDs.append(circuitID)
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
            # If there is nothing in the circuit ID field
            else:
                # Copy deviceName to circuitName if none defined already
                if circuitName == "":
                    circuitName = deviceName
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
                                }
                deviceListForCircuit.append(thisDevice)
                thisCircuit = {
                  "circuitID": circuitID,
                  "circuitName": circuitName,
                  "ParentNode": ParentNode,
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
    return (subscriberCircuits,	dictForCircuitsWithoutParentNodes)

def refreshShapers():

    # Starting
    print("refreshShapers starting at " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))
    # Create a single batch of xdp update commands to execute together
    ipMapBatch = BatchedCommands()

    # Warn user if enableActualShellCommands is False, because that would mean no actual commands are executing
    if enable_actual_shell_commands() == False:
        warnings.warn("enableActualShellCommands is set to False. None of the commands below will actually be executed. Simulated run.", stacklevel=2)
    # Warn user if monitorOnlyMode is True, because that would mean no actual shaping is happening
    if monitor_mode_only() == True:
        warnings.warn("monitorOnlyMode is set to True. Shaping will not occur.", stacklevel=2)


    # Check if first run since boot
    isThisFirstRunSinceBoot = checkIfFirstRunSinceBoot()


    # Files
    shapedDevicesFile = get_shaped_devices_path()
    networkJSONfile = get_network_json_path()


    # Check validation
    safeToRunRefresh = False
    print("Validating input files '" + shapedDevicesFile + "' and '" + networkJSONfile + "'")
    if (validateNetworkAndDevices() == True):
        shutil.copyfile('ShapedDevices.csv', 'lastGoodConfig.csv')
        shutil.copyfile('network.json', 'lastGoodConfig.json')
        print("Backed up good config as lastGoodConfig.csv and lastGoodConfig.json")
        safeToRunRefresh = True
    else:
        if (isThisFirstRunSinceBoot == False):
            warnings.warn("Validation failed. Because this is not the first run since boot (queues already set up) - will now exit.", stacklevel=2)
            safeToRunRefresh = False
        else:
            warnings.warn("Validation failed. However - because this is the first run since boot - will load queues from last good config", stacklevel=2)
            shapedDevicesFile = 'lastGoodConfig.csv'
            networkJSONfile = 'lastGoodConfig.json'
            safeToRunRefresh = True

    if safeToRunRefresh == True:

        # Load Subscriber Circuits & Devices
        subscriberCircuits,	dictForCircuitsWithoutParentNodes = loadSubscriberCircuits(shapedDevicesFile)


        # Load network hierarchy
        with open(networkJSONfile, 'r') as j:
            network = json.loads(j.read())

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

        # Detect flat mode once for reuse in traversal (network.json contains only implicit Root)
        try:
            flat_mode = bool(is_network_flat())
        except Exception:
            flat_mode = False


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

        # If in monitorOnlyMode, override network.json bandwidth rates to where no shaping will actually occur
        if monitor_mode_only() == True:
            def overrideNetworkBandwidths(data):
                for elem in data:
                    if 'children' in data[elem]:
                        overrideNetworkBandwidths(data[elem]['children'])
                    data[elem]['downloadBandwidthMbpsMin'] = 100000
                    data[elem]['uploadBandwidthMbpsMin'] = 100000
            overrideNetworkBandwidths(network)

        # Initialize planner (state is read/validated here; behavior not yet changed)
        try:
            # Use the same notion of "sites" that Planner persists to site_map:
            # - For tiered networks, these are depth-1 nodes (children of top-level nodes)
            # - For flat networks, fallback to top-level keys (Generated_PN_* are created later)
            def collect_site_names_tiered(n):
                """Collect depth-1 site names for tiered networks.

                Falls back to top-level keys when no children are present.
                Returns a set of string site names.
                """
                names = set()
                try:
                    for k, v in n.items():
                        if not isinstance(v, dict):
                            continue
                        ch = v.get('children')
                        if isinstance(ch, dict):
                            names.update(str(ck) for ck in ch.keys())
                except Exception:
                    names = set()
                if not names:
                    try:
                        names = {str(k) for k, v in n.items() if isinstance(v, dict)}
                    except Exception:
                        names = set()
                return names

            current_site_names = set()
            try:
                if isinstance(network, dict):
                    current_site_names = collect_site_names_tiered(network) if not flat_mode else set(
                        [str(k) for k, v in network.items() if isinstance(v, dict)]
                    )
            except Exception:
                current_site_names = set()
            planner = Planner(queuesAvailable, on_a_stick(), current_site_names, len(current_site_names))
        except Exception as e:
            warnings.warn(f"Planner initialization failed: {e}", stacklevel=2)

        # Generate Parent Nodes. Spread ShapedDevices.csv which lack defined ParentNode across these (balance across CPUs)
        print("Generating parent nodes")
        generatedPNs = []
        numberOfGeneratedPNs = queuesAvailable
        # If in monitorOnlyMode, override bandwidth rates to where no shaping will actually occur
        if monitor_mode_only() == True:
            chosenDownloadMbps = 10000
            chosenUploadMbps = 10000
        else:
            chosenDownloadMbps = generated_pn_download_mbps()
            chosenUploadMbps = generated_pn_upload_mbps()
        for x in range(numberOfGeneratedPNs):
            genPNname = "Generated_PN_" + str(x+1)
            network[genPNname] =	{
                                        "downloadBandwidthMbps": chosenDownloadMbps,
                                        "uploadBandwidthMbps": chosenUploadMbps
                                    }
            generatedPNs.append(genPNname)
        if use_bin_packing_to_balance_cpu():
            if is_insight_enabled():
                print("Using greedy binpacking to sort circuits by CPU core")
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
                    # Treat blank or 'none' (any case) as unassigned parent in flat mode handling
                    pn_val = str(circuit.get('ParentNode', '')).strip().lower()
                    if (pn_val in ('', 'none')) and ('idForCircuitsWithoutParentNodes' in circuit):
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

                # Prepare capacities (equal split by default)
                capacities = {pn: 1.0 for pn in generatedPNs}

                # Compute greedy assignments and apply
                assignments = _greedy_binpack(items, generatedPNs, capacities)
                for circuit in subscriberCircuits:
                    pn_val = str(circuit.get('ParentNode', '')).strip().lower()
                    if (pn_val in ('', 'none')) and ('idForCircuitsWithoutParentNodes' in circuit):
                        item_id = circuit['idForCircuitsWithoutParentNodes']
                        if item_id in assignments:
                            circuit['ParentNode'] = assignments[item_id]

                print("Finished binpacking generated parent nodes")
            else:
                warn_msg = "Binpacking is enabled, but requires an Insight subscription."
                print("Warning: " + warn_msg)
                try:
                    submit_urgent_issue("LibreQoS", "Warning", "BINPACKING_REQUIRES_INSIGHT", warn_msg, "{}", "BINPACKING_REQUIRES_INSIGHT_PARENT_NODES")
                except Exception:
                    pass
                # Fallback: round-robin assignment when Insight is not enabled
                genPNcounter = 0
                for circuit in subscriberCircuits:
                    pn_val = str(circuit.get('ParentNode', '')).strip().lower()
                    if pn_val in ('', 'none'):
                        circuit['ParentNode'] = generatedPNs[genPNcounter]
                        genPNcounter += 1
                        if genPNcounter >= queuesAvailable:
                            genPNcounter = 0
        else:
            genPNcounter = 0
            for circuit in subscriberCircuits:
                pn_val = str(circuit.get('ParentNode', '')).strip().lower()
                if pn_val in ('', 'none'):
                    circuit['ParentNode'] = generatedPNs[genPNcounter]
                    genPNcounter += 1
                    if genPNcounter >= queuesAvailable:
                        genPNcounter = 0
        print("Generated parent nodes created")

        # Safety pass: ensure every circuit has a valid ParentNode key present in the current network
        try:
            available_nodes = set([str(k) for k in network.keys()]) if isinstance(network, dict) else set()
        except Exception:
            available_nodes = set()
        if len(available_nodes) > 0 and len(generatedPNs) > 0:
            rr = 0
            for circuit in subscriberCircuits:
                try:
                    pn_raw = str(circuit.get('ParentNode', '')).strip()
                    pn_lower = pn_raw.lower()
                    if pn_raw == '' or pn_lower == 'none' or pn_raw not in available_nodes:
                        circuit['ParentNode'] = generatedPNs[rr]
                        rr = (rr + 1) % len(generatedPNs)
                except Exception:
                    pass

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

        # Populate transient site speeds for planner reuse (not persisted)
        try:
            if 'planner' in locals() and isinstance(planner, Planner):
                speeds = {}
                if isinstance(network, dict):
                    for key, val in network.items():
                        if isinstance(val, dict):
                            try:
                                speeds[str(key)] = {
                                    'dl_max': float(val.get('downloadBandwidthMbps', 0)),
                                    'ul_max': float(val.get('uploadBandwidthMbps', 0)),
                                    'dl_min': float(val.get('downloadBandwidthMbpsMin', val.get('downloadBandwidthMbps', 0))),
                                    'ul_min': float(val.get('uploadBandwidthMbpsMin', val.get('uploadBandwidthMbps', 0))),
                                }
                            except Exception:
                                pass
                planner._site_speeds = speeds
        except Exception:
            pass

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

        # Prepare planner weights only when Insight is enabled; otherwise avoid LTS chatter
        weight_by_circuit_id = {}
        if is_insight_enabled():
            try:
                weights = get_weights()
                for w in weights:
                    try:
                        weight_by_circuit_id[str(w.circuit_id)] = float(w.weight)
                    except Exception:
                        pass
            except Exception:
                # If we can't get weights, leave mapping empty; UI will fall back
                weight_by_circuit_id = {}

        # Group circuits by parent node. Reduces runtime for section below this one.
        circuits_by_parent_node = {}
        circuit_min_down_combined_by_parent_node = {}
        circuit_min_up_combined_by_parent_node = {}
        for circuit in subscriberCircuits:
            #If a device from ShapedDevices.csv lists this node as its Parent Node, attach it as a leaf to this node HTB
            if circuit['ParentNode'] not in  circuits_by_parent_node:
                circuits_by_parent_node[circuit['ParentNode']] = []
            temp = circuits_by_parent_node[circuit['ParentNode']]
            temp.append(circuit)
            circuits_by_parent_node[circuit['ParentNode']] = temp
            if circuit['ParentNode'] not in  circuit_min_down_combined_by_parent_node:
                circuit_min_down_combined_by_parent_node[circuit['ParentNode']] = 0
            circuit_min_down_combined_by_parent_node[circuit['ParentNode']] += circuit['minDownload']
            if circuit['ParentNode'] not in  circuit_min_up_combined_by_parent_node:
                circuit_min_up_combined_by_parent_node[circuit['ParentNode']] = 0
            circuit_min_up_combined_by_parent_node[circuit['ParentNode']] += circuit['minUpload']

        # Parse network structure and add devices from ShapedDevices.csv
        print("Parsing network structure and tallying devices")
        parentNodes = []
        site_insert_counter = 0
        minorByCPUpreloaded = {}
        node_refs = {}
        circuits_start_minor = {}
        knownClassIDs = []
        nodes_requiring_min_squashing = {}
        # Track minor counter by CPU. This way we can have > 32000 hosts (htb has u16 limit to minor handle)
        # Minor numbers start at 4 to reserve:
        #   1 = root qdisc, 2 = default class, 3 = per-CPU Generated_PN_* sites
        # With CIRCUIT_PADDING, we leave gaps between nodes to allow future circuit additions
        # without disrupting existing ClassID assignments. This maintains stability across reloads.
        for x in range(queuesAvailable):
            minorByCPUpreloaded[x+1] = 4
        def traverseNetwork(data, depth, major, minorByCPU, queue, parentClassID, upParentClassID, parentMaxDL, parentMaxUL, parentMinDL, parentMinUL):
            nonlocal site_insert_counter
            # ClassID Assignment Strategy:
            # - Nodes and circuits are processed in alphabetical order for stability
            # - Each node gets a unique minor number that increments sequentially
            # - After processing all circuits for a node, we add CIRCUIT_PADDING to the minor counter
            # - This creates gaps that allow adding new circuits without affecting other ClassIDs
            # - Children are also sorted before recursive processing to ensure deterministic traversal
            # For top-level binpacked keys (CpueQueueN), enforce numeric ordering of N
            keys = list(data.keys())
            if depth == 0 and len(keys) > 0 and all(k.startswith("CpueQueue") for k in keys):
                try:
                    keys.sort(key=lambda k: int(k.replace("CpueQueue", "")))
                except Exception:
                    keys = sorted(keys)
            else:
                keys = sorted(keys)
            for node in keys:
                #if data[node]['type'] == "virtual":
                #	print(node + " is a virtual node. Skipping.")
                #	if depth == 0:
                #		parentClassID = hex(major) + ':'
                #		upParentClassID = hex(major+stickOffset) + ':'
                #	data[node]['parentClassID'] = parentClassID
                #	data[node]['up_parentClassID'] = upParentClassID
                #	data[node]['classMajor'] = hex(major)
                #	data[node]['up_classMajor'] = hex(major + stickOffset)
                #	data[node]['classMinor'] = hex(minorByCPU[queue])
                #	data[node]['cpuNum'] = hex(queue-1)
                #	data[node]['up_cpuNum'] = hex(queue-1+stickOffset)
                #	traverseNetwork(data[node]['children'], depth, major, minorByCPU, queue, parentClassID, upParentClassID, parentMaxDL, parentMaxUL)
                #	continue
                # Prefer fixed, low minors for Generated_PN_* per CPU so they are
                # visually obvious and stable. For all other nodes, prefer reusing
                # previously-assigned site minors from planner.site_map; else try
                # planner free_minors for this CPU; else fallback to sequential
                # counter starting at 4.
                name_s = str(node)
                is_generated_pn = False
                try:
                    is_generated_pn = name_s.startswith('Generated_PN_')
                except Exception:
                    is_generated_pn = False

                chosen_minor = None
                # Fixed minor for Generated_PN_*: always 3 per CPU, independent of
                # planner state and traversal order.
                if is_generated_pn:
                    chosen_minor = 3
                else:
                    try:
                        if ('planner' in locals() or 'planner' in globals()) and isinstance(planner, Planner) and (not getattr(planner, '_is_default', False)):
                            st = planner.getState()
                            cpu_index = int(queue - 1)
                            # 1) Attempt reuse from site_map when CPU/major match
                            try:
                                site_map = st.get('site_map') if isinstance(st, dict) else None
                                if isinstance(site_map, dict):
                                    key_hash = int(hash_to_i64(str(node)))
                                    entry = site_map.get(key_hash)
                                    if isinstance(entry, dict):
                                        saved_cpu = int(entry.get('cpu', -1))
                                        saved_major = int(entry.get('major', -1))
                                        saved_minor = int(entry.get('minor', -1))
                                        if saved_cpu == cpu_index and saved_major == int(major) and saved_minor >= 4:
                                            chosen_minor = saved_minor
                                            # Remove from free list if present
                                            try:
                                                fm = st.get('free_minors')
                                                if isinstance(fm, list) and 0 <= cpu_index < len(fm) and isinstance(fm[cpu_index], list) and saved_minor in fm[cpu_index]:
                                                    fm[cpu_index].remove(saved_minor)
                                            except Exception:
                                                pass
                            except Exception:
                                pass
                            # 2) If no reuse, allocate from free list for this CPU (depth >= 1 only)
                            if chosen_minor is None and depth >= 1:
                                try:
                                    fm = st.get('free_minors') if isinstance(st, dict) else None
                                    if isinstance(fm, list) and 0 <= cpu_index < len(fm) and isinstance(fm[cpu_index], list) and len(fm[cpu_index]) > 0:
                                        chosen_minor = int(fm[cpu_index].pop(0))
                                except Exception:
                                    chosen_minor = None
                    except Exception:
                        chosen_minor = None
                    if chosen_minor is None:
                        try:
                            chosen_minor = int(minorByCPU[queue])
                        except Exception:
                            chosen_minor = 4

                nodeClassID = hex(major) + ':' + hex(chosen_minor)
                upNodeClassID = hex(major+stickOffset) + ':' + hex(chosen_minor)
                data[node]['classid'] = nodeClassID
                data[node]['up_classid'] = upNodeClassID
                if depth == 0:
                    parentClassID = hex(major) + ':'
                    upParentClassID = hex(major+stickOffset) + ':'
                data[node]['parentClassID'] = parentClassID
                data[node]['up_parentClassID'] = upParentClassID
                # If in monitorOnlyMode, override bandwidth rates to where no shaping will actually occur
                if monitor_mode_only() == True:
                    data[node]['downloadBandwidthMbps'] = 100000
                    data[node]['uploadBandwidthMbps'] = 100000
                    data[node]['downloadBandwidthMbpsMin'] = 100000
                    data[node]['uploadBandwidthMbpsMin'] = 100000
                # If not in monitorOnlyMode
                else:
                    # Cap based on this node's max bandwidth, or parent node's max bandwidth, whichever is lower
                    data[node]['downloadBandwidthMbps'] = min(data[node]['downloadBandwidthMbps'],parentMaxDL)
                    data[node]['uploadBandwidthMbps'] = min(data[node]['uploadBandwidthMbps'],parentMaxUL)
                    data[node]['downloadBandwidthMbpsMin'] = min(data[node]['downloadBandwidthMbpsMin'], data[node]['downloadBandwidthMbps'], parentMinDL)
                    data[node]['uploadBandwidthMbpsMin'] = min(data[node]['uploadBandwidthMbpsMin'], data[node]['uploadBandwidthMbps'], parentMinUL)
                # Calculations are done in findBandwidthMins() to determine optimal HTB rates (mins) and ceils (maxs)
                data[node]['classMajor'] = hex(major)
                data[node]['up_classMajor'] = hex(major + stickOffset)
                data[node]['classMinor'] = hex(chosen_minor)
                data[node]['cpuNum'] = hex(queue-1)
                data[node]['up_cpuNum'] = hex(queue-1+stickOffset)
                # Index node by name for later circuit attachment
                try:
                    node_refs[str(node)] = data[node]
                except Exception:
                    pass

                # Notify planner of site details where applicable (insert-only)
                try:
                    if ('planner' in locals() or 'planner' in globals()) and isinstance(planner, Planner):
                        name_s = str(node)
                        synthetic = (not flat_mode) and (name_s.startswith('Generated_PN_') or name_s.startswith('CpueQueue'))
                        # Capture site mapping for non-synthetic nodes at depth 0/1 (tiered),
                        # or depth 0 in flat mode.
                        if (not synthetic) and ((depth in (0, 1)) or (flat_mode and depth == 0)):
                            st = planner.getState()
                            if 'site_map' not in st or not isinstance(st['site_map'], dict):
                                st['site_map'] = {}
                            # Store as integers for ease of later use
                            cpu_int = int(queue - 1)
                            major_int = int(major)
                            minor_int = int(chosen_minor)
                            site_key = int(hash_to_i64(str(node)))
                            st['site_map'][site_key] = {
                                'cpu': cpu_int,
                                'major': major_int,
                                'minor': minor_int,
                                'insertion_order': int(site_insert_counter),
                            }
                            site_insert_counter += 1
                        # Always reserve the site's minor in the free list, even for synthetic nodes
                        try:
                            st = planner.getState()
                            fm = st.get('free_minors')
                            cpu_int = int(queue - 1)
                            minor_int = int(chosen_minor)
                            if isinstance(fm, list) and 0 <= cpu_int < len(fm) and isinstance(fm[cpu_int], list):
                                if int(minor_int) in fm[cpu_int]:
                                    fm[cpu_int].remove(int(minor_int))
                        except Exception:
                            pass
                except Exception:
                    # Planner is optional; ignore errors during capture
                    pass
                thisParentNode =	{
                                    "parentNodeName": node,
                                    "classID": nodeClassID,
                                    "maxDownload": data[node]['downloadBandwidthMbps'],
                                    "maxUpload": data[node]['uploadBandwidthMbps'],
                                    }
                parentNodes.append(thisParentNode)
                minorByCPU[queue] = minorByCPU[queue] + 1
                # Check for overflow - TC uses u16 for minor class ID (max 65535)
                if minorByCPU[queue] > 0xFFFF:
                    msg = f"Minor class ID overflow on CPU {queue}: {minorByCPU[queue]} exceeds TC's u16 limit (65535). Consider increasing queue count or restructuring network hierarchy."
                    logging.error(msg)
                    try:
                        ctx = json.dumps({"cpu": queue, "minor": minorByCPU[queue]})
                        submit_urgent_issue("LibreQoS", "Error", "TC_U16_OVERFLOW", msg, ctx, f"TC_U16_OVERFLOW_CPU_{queue}")
                    except Exception:
                        pass
                    raise ValueError(f"Minor class ID overflow on CPU {queue}: {minorByCPU[queue]} exceeds limit of 65535")
                # If a device from ShapedDevices.csv lists this node as its Parent Node,
                # record starting minor and reserve enough minors to keep classid sequence stable.
                if node in circuits_by_parent_node:
                    # Determine if we need to squash mins at TC time (record only)
                    override_min_down = None
                    override_min_up = None
                    if monitor_mode_only() == False:
                        if (circuit_min_down_combined_by_parent_node[node] > data[node]['downloadBandwidthMbpsMin']) or (circuit_min_up_combined_by_parent_node[node] > data[node]['uploadBandwidthMbpsMin']):
                            override_min_down = 1
                            override_min_up = 1
                            logging.info("The combined minimums of circuits in Parent Node [" + node + "] exceeded that of the parent node. Reducing these circuits' minimums to 1 now.", stacklevel=2)
                            if ((override_min_down * len(circuits_by_parent_node[node])) > data[node]['downloadBandwidthMbpsMin']) or ((override_min_up * len(circuits_by_parent_node[node])) > data[node]['uploadBandwidthMbpsMin']):
                                logging.info("Even with this change, minimums will exceed the min rate of the parent node. Using 10 kbps as the minimum for these circuits instead.", stacklevel=2)
                                nodes_requiring_min_squashing[node] = True
                    # Record starting minor for circuits under this node
                    circuits_start_minor[str(node)] = int(minorByCPU[queue])
                    # Increment minor counter by number of circuits to keep classid allocation stable
                    try:
                        minorByCPU[queue] = minorByCPU[queue] + len(circuits_by_parent_node[node])
                    except Exception:
                        pass

                # Add padding for future circuit additions (applies to all nodes)
                # This ensures space is reserved even for nodes without circuits
                minorByCPU[queue] = minorByCPU[queue] + CIRCUIT_PADDING

                # Recursive call this function for children nodes attached to this node
                if 'children' in data[node]:
                    # Sort children to ensure consistent traversal order
                    sorted_children = dict(sorted(data[node]['children'].items()))
                    # We need to keep tabs on the minor counter, because we can't have repeating class IDs. Here, we bring back the minor counter from the recursive function
                    minorByCPU[queue] = minorByCPU[queue] + 1
                    # Check for overflow - TC uses u16 for minor class ID (max 65535)
                    if minorByCPU[queue] > 0xFFFF:
                        msg = f"Minor class ID overflow on CPU {queue}: {minorByCPU[queue]} exceeds TC's u16 limit (65535). Consider increasing queue count or restructuring network hierarchy."
                        logging.error(msg)
                        try:
                            ctx = json.dumps({"cpu": queue, "minor": minorByCPU[queue]})
                            submit_urgent_issue("LibreQoS", "Error", "TC_U16_OVERFLOW", msg, ctx, f"TC_U16_OVERFLOW_CPU_{queue}")
                        except Exception:
                            pass
                        raise ValueError(f"Minor class ID overflow on CPU {queue}: {minorByCPU[queue]} exceeds limit of 65535")
                    minorByCPU = traverseNetwork(sorted_children, depth+1, major, minorByCPU, queue, nodeClassID, upNodeClassID, data[node]['downloadBandwidthMbps'], data[node]['uploadBandwidthMbps'], data[node]['downloadBandwidthMbpsMin'], data[node]['uploadBandwidthMbpsMin'])
                # If top level node, increment to next queue / cpu core
                if depth == 0:
                    if queue >= queuesAvailable:
                        queue = 1
                        major = queue
                    else:
                        queue += 1
                        major += 1
            return minorByCPU

        # If we're in binpacking mode, we need to sort the network structure a bit
        if use_bin_packing_to_balance_cpu() and not is_network_flat():
            if not is_insight_enabled():
                warn_msg2 = "Binpacking is enabled, but requires an Insight subscription."
                print("Warning: " + warn_msg2)
                try:
                    submit_urgent_issue("LibreQoS", "Warning", "BINPACKING_REQUIRES_INSIGHT", warn_msg2, "{}", "BINPACKING_REQUIRES_INSIGHT_SITE_DISTRIBUTION")
                except Exception:
                    pass
            else:
                # Decide reuse vs greedy
                reuse_planner = False
                try:
                    if 'planner' in locals() and isinstance(planner, Planner):
                        st = planner.getState()
                        if isinstance(st, dict) and isinstance(st.get('site_map', None), dict) and (not getattr(planner, '_is_default', False)):
                            reuse_planner = True
                except Exception:
                    reuse_planner = False

                if reuse_planner:
                    print("Rebuilding CPU bins from planner map (stable).")
                    cpu_keys = ["CpueQueue" + str(cpu) for cpu in range(queuesAvailable)]
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
                    # Build name->node map from current (flattened) network
                    def map_all_nodes(data, out):
                        if isinstance(data, dict):
                            for key, val in data.items():
                                if key == 'children':
                                    continue
                                if isinstance(val, dict):
                                    try:
                                        out[str(key)] = val
                                    except Exception:
                                        pass
                                    if 'children' in val and isinstance(val['children'], dict):
                                        map_all_nodes(val['children'], out)
                    all_nodes = {}
                    map_all_nodes(network, all_nodes)
                    # Hash map for current names
                    name_hash_to_name = {}
                    try:
                        for nm in list(all_nodes.keys()):
                            try:
                                name_hash_to_name[int(hash_to_i64(str(nm)))] = str(nm)
                            except Exception:
                                pass
                    except Exception:
                        pass
                    # Group sites by CPU and order by insertion_order
                    st = planner.getState()
                    cpu_to_sites = {cpu: [] for cpu in range(queuesAvailable)}
                    for name, entry in st['site_map'].items():
                        try:
                            cpu = int(entry.get('cpu', 0))
                            ins = int(entry.get('insertion_order', 0))
                            # name may be hashed int or legacy string; coerce to hash int
                            try:
                                name_hash = int(name)
                            except Exception:
                                name_hash = int(hash_to_i64(str(name)))
                            cpu_to_sites.setdefault(cpu, []).append((ins, name_hash))
                        except Exception:
                            pass
                    for cpu, lst in cpu_to_sites.items():
                        lst.sort(key=lambda t: t[0])
                        cpuKey = "CpueQueue" + str(cpu)
                        for _, site_hash in lst:
                            site_name = name_hash_to_name.get(int(site_hash))
                            if site_name is None:
                                continue
                            # Skip synthetic default PN containers in site binning
                            try:
                                if str(site_name).startswith('Generated_PN_'):
                                    continue
                            except Exception:
                                pass
                            node_obj = all_nodes.get(site_name)
                            if node_obj is None:
                                continue
                            # Ensure speeds reflect current network.json
                            try:
                                speeds = getattr(planner, '_site_speeds', {}).get(site_name)
                                if isinstance(speeds, dict):
                                    node_obj['downloadBandwidthMbps'] = speeds.get('dl_max', node_obj.get('downloadBandwidthMbps'))
                                    node_obj['uploadBandwidthMbps'] = speeds.get('ul_max', node_obj.get('uploadBandwidthMbps'))
                                    node_obj['downloadBandwidthMbpsMin'] = speeds.get('dl_min', node_obj.get('downloadBandwidthMbpsMin', node_obj.get('downloadBandwidthMbps')))
                                    node_obj['uploadBandwidthMbpsMin'] = speeds.get('ul_min', node_obj.get('uploadBandwidthMbpsMin', node_obj.get('uploadBandwidthMbps')))
                            except Exception:
                                pass
                            binnedNetwork[cpuKey]['children'][site_name] = node_obj
                    # After placing real sites, guarantee one Generated_PN per CPU
                    try:
                        for idx, pn in enumerate(generatedPNs):
                            cpuKey = "CpueQueue" + str(idx if idx < queuesAvailable else queuesAvailable-1)
                            node_obj = network.get(pn) if isinstance(network, dict) else None
                            if node_obj is None:
                                node_obj = {
                                    'downloadBandwidthMbps': generated_pn_download_mbps(),
                                    'uploadBandwidthMbps': generated_pn_upload_mbps(),
                                    'type': 'site',
                                    'downloadBandwidthMbpsMin': generated_pn_download_mbps(),
                                    'uploadBandwidthMbpsMin': generated_pn_upload_mbps(),
                                }
                            binnedNetwork[cpuKey]['children'][pn] = node_obj
                    except Exception:
                        pass
                    network = binnedNetwork
                else:
                    print("Using greedy binpacking to distribute sites across CPU queues.")
                    # Build items from top-level nodes with weights
                    items = []
                    try:
                        weights = get_tree_weights()
                    except Exception as e:
                        warnings.warn("get_tree_weights() failed; defaulting to equal weights (" + str(e) + ")", stacklevel=2)
                        weights = None
                    weight_by_name = {}
                    if weights is not None:
                        try:
                            for w in weights:
                                weight_by_name[str(w.name)] = float(w.weight)
                        except Exception:
                            pass
                    for node in network:
                        # Exclude Generated_PN_* from site binpacking entirely
                        try:
                            if str(node).startswith('Generated_PN_'):
                                continue
                        except Exception:
                            pass
                        w = weight_by_name.get(str(node), 1.0)
                        items.append({"id": str(node), "weight": float(w)})

                    # Prepare bins and capacities
                    cpu_keys = ["CpueQueue" + str(cpu) for cpu in range(queuesAvailable)]
                    capacities = {key: 1.0 for key in cpu_keys}

                    # Greedy assignment of sites to CPU bins
                    assignment = _greedy_binpack(items, cpu_keys, capacities)
                    for x in range(queuesAvailable):
                        key = "CpueQueue" + str(x)
                        assigned = [name for name, tgt in assignment.items() if tgt == key]
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
                        try:
                            if str(node).startswith('Generated_PN_'):
                                continue
                        except Exception:
                            pass
                        tgt = assignment.get(node)
                        if tgt is None:
                            tgt = "CpueQueue" + str(queuesAvailable - 1)
                        binnedNetwork[tgt]['children'][node] = network[node]
                    # After placing real sites, guarantee one Generated_PN per CPU
                    try:
                        for idx, pn in enumerate(generatedPNs):
                            cpuKey = "CpueQueue" + str(idx if idx < queuesAvailable else queuesAvailable-1)
                            node_obj = network.get(pn) if isinstance(network, dict) else None
                            if node_obj is None:
                                node_obj = {
                                    'downloadBandwidthMbps': generated_pn_download_mbps(),
                                    'uploadBandwidthMbps': generated_pn_upload_mbps(),
                                    'type': 'site',
                                    'downloadBandwidthMbpsMin': generated_pn_download_mbps(),
                                    'uploadBandwidthMbpsMin': generated_pn_upload_mbps(),
                                }
                            binnedNetwork[cpuKey]['children'][pn] = node_obj
                    except Exception:
                        pass
                    network = binnedNetwork

        # Here is the actual call to the recursive traverseNetwork() function. finalMinor is not used.
        minorByCPU = traverseNetwork(network, 0, major=1, minorByCPU=minorByCPUpreloaded, queue=1, parentClassID=None, upParentClassID=None, parentMaxDL=upstream_bandwidth_capacity_download_mbps(), parentMaxUL=upstream_bandwidth_capacity_upload_mbps(), parentMinDL=upstream_bandwidth_capacity_download_mbps(), parentMinUL=upstream_bandwidth_capacity_upload_mbps())

        # Hoisted pass: attach circuits to built site tree using reserved minors
        # Prepare a stale set of prior circuits to detect removals
        stale_circuits = set()
        try:
            if 'planner' in locals() and isinstance(planner, Planner):
                st0 = planner.getState()
                cmap = st0.get('circuit_map') if isinstance(st0, dict) else None
                if isinstance(cmap, dict):
                    # Keep keys as stored (may be ints or legacy strings)
                    stale_circuits = set(cmap.keys())
        except Exception:
            stale_circuits = set()
        for node_name, circuit_list in circuits_by_parent_node.items():
            if node_name not in node_refs:
                continue
            node = node_refs[node_name]
            # Determine major/cpu
            try:
                major_int = int(node.get('classMajor', '0x0'), 16)
            except Exception:
                major_int = 0
            try:
                if node_name in circuits_start_minor:
                    start_minor = int(circuits_start_minor.get(node_name))
                else:
                    # Fallback: never start circuits at the site's own minor; advance by at least +1
                    site_minor = int(node.get('classMinor', '0x0'), 16)
                    start_minor = max(4, site_minor + 1)
            except Exception:
                start_minor = 4
            # Precompute override flags (same logic as before)
            override_min_down = None
            override_min_up = None
            if monitor_mode_only() == False:
                try:
                    if (circuit_min_down_combined_by_parent_node[node_name] > node['downloadBandwidthMbpsMin']) or (circuit_min_up_combined_by_parent_node[node_name] > node['uploadBandwidthMbpsMin']):
                        override_min_down = 1
                        override_min_up = 1
                except Exception:
                    pass
            # Sort circuits by name for stable ordering
            sorted_circuits = sorted(circuit_list, key=lambda c: c.get('circuitName', c.get('circuitID', '')))
            circuits_for_node = []
            current_minor = start_minor
            # Planner reuse toggle for circuits
            use_planner_circuits = False
            st = {}
            cpu_int = 0
            try:
                if 'planner' in locals() and isinstance(planner, Planner) and (not getattr(planner, '_is_default', False)):
                    st = planner.getState()
                    use_planner_circuits = isinstance(st, dict) and isinstance(st.get('circuit_map', None), dict)
            except Exception:
                use_planner_circuits = False
            # CPU index for this node (from hex string)
            try:
                cpu_int = int(node.get('cpuNum', '0x0'), 16)
            except Exception:
                cpu_int = 0
            for circuit in sorted_circuits:
                if node_name != circuit.get('ParentNode'):
                    continue
                # Mark as present this run (not stale)
                try:
                    cid_raw = circuit.get('circuitID', '')
                    # Discard both hashed and string forms to be robust across versions
                    try:
                        stale_circuits.discard(int(hash_to_i64(str(cid_raw))))
                    except Exception:
                        pass
                    try:
                        stale_circuits.discard(str(cid_raw))
                    except Exception:
                        pass
                except Exception:
                    pass
                # Bound to parent's maxima
                maxDownload = min(circuit['maxDownload'], node['downloadBandwidthMbps'])
                maxUpload = min(circuit['maxUpload'], node['uploadBandwidthMbps'])
                # Apply override min=1 if needed (further squashing to 0.01 happens at TC emission)
                if override_min_down:
                    circuit['minDownload'] = 1
                if override_min_up:
                    circuit['minUpload'] = 1
                minDownload = min(circuit['minDownload'], maxDownload)
                minUpload = min(circuit['minUpload'], maxUpload)
                # Choose minor: reuse from planner if available, else take from free list, else fall back to sequential
                chosen_minor = None
                if use_planner_circuits:
                    try:
                        cid_raw = circuit['circuitID']
                        entry = None
                        # Prefer hashed lookup
                        try:
                            entry = st['circuit_map'].get(int(hash_to_i64(str(cid_raw))))
                        except Exception:
                            entry = None
                        if entry is None:
                            # Fallback to legacy string key (pre-hash)
                            entry = st['circuit_map'].get(str(cid_raw))
                        if isinstance(entry, dict):
                            saved_major = int(entry.get('major', major_int))
                            saved_minor = int(entry.get('minor', -1))
                            saved_parent = str(entry.get('parent_site', node_name))
                            # Reuse only if this circuit still belongs to this node and major matches
                            if saved_parent == str(node_name) and saved_major == int(major_int) and saved_minor >= 4:
                                chosen_minor = saved_minor
                                # Remove from free list if present
                                try:
                                    fm = st.get('free_minors')
                                    if isinstance(fm, list) and 0 <= cpu_int < len(fm) and isinstance(fm[cpu_int], list) and saved_minor in fm[cpu_int]:
                                        fm[cpu_int].remove(saved_minor)
                                except Exception:
                                    pass
                    except Exception:
                        pass
                if chosen_minor is None:
                    # Allocate from free list for this CPU
                    try:
                        fm = st.get('free_minors') if isinstance(st, dict) else None
                        if isinstance(fm, list) and 0 <= cpu_int < len(fm) and isinstance(fm[cpu_int], list) and len(fm[cpu_int]) > 0:
                            chosen_minor = int(fm[cpu_int].pop(0))
                    except Exception:
                        chosen_minor = None
                if chosen_minor is None:
                    # Fallback to reserved sequential range; ensure we never collide with the site's minor (<3 reserved)
                    chosen_minor = max(4, int(current_minor))
                    # Guard against collision with the parent site's own minor (e.g., 0xd)
                    try:
                        site_minor = int(node.get('classMinor', '0x0'), 16)
                    except Exception:
                        site_minor = -1
                    if chosen_minor == site_minor:
                        chosen_minor += 1
                    # Also guard against collisions within this node's already-assigned circuits
                    try:
                        used_minors = set()
                        for c in circuits_for_node:
                            try:
                                used_minors.add(int(str(c.get('classMinor', '0x0')), 16))
                            except Exception:
                                pass
                        while chosen_minor in used_minors:
                            chosen_minor += 1
                    except Exception:
                        pass
                    current_minor = chosen_minor + 1
                flowIDstring = hex(major_int) + ':' + hex(chosen_minor)
                upFlowIDstring = hex(major_int + stickOffset) + ':' + hex(chosen_minor)
                item = {
                    'maxDownload': maxDownload,
                    'maxUpload': maxUpload,
                    'minDownload': minDownload,
                    'minUpload': minUpload,
                    'circuitID': circuit['circuitID'],
                    'circuitName': circuit['circuitName'],
                    'ParentNode': circuit['ParentNode'],
                    'devices': circuit['devices'],
                    'classid': flowIDstring,
                    'up_classid': upFlowIDstring,
                    'classMajor': hex(major_int),
                    'up_classMajor': hex(major_int + stickOffset),
                    'classMinor': hex(chosen_minor),
                    'comment': circuit['comment']
                }
                # Planner/UI weight
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
                    item['planner_weight'] = w
                except Exception:
                    pass
                # SQM override copy-through
                if 'sqm' in circuit and circuit['sqm']:
                    item['sqm'] = circuit['sqm']
                circuits_for_node.append(item)
                # Insert circuit details into planner's circuit_map for future reuse
                try:
                    if 'planner' in locals() and isinstance(planner, Planner):
                        st = planner.getState()
                        if 'circuit_map' not in st or not isinstance(st['circuit_map'], dict):
                            st['circuit_map'] = {}
                        st['circuit_map'][int(hash_to_i64(str(circuit['circuitID'])))] = {
                            'cpu': int(cpu_int),
                            'major': int(major_int),
                            'minor': int(chosen_minor),
                            'parent_site': str(node_name),
                            'sqm': str(circuit['sqm']) if 'sqm' in circuit and circuit['sqm'] else '',
                        }
                        # Remove used minor from the planner's free list for this CPU, if present
                        try:
                            fm = st.get('free_minors')
                            if isinstance(fm, list) and 0 <= cpu_int < len(fm) and isinstance(fm[cpu_int], list):
                                if int(chosen_minor) in fm[cpu_int]:
                                    fm[cpu_int].remove(int(chosen_minor))
                        except Exception:
                            pass
                except Exception:
                    pass
                if current_minor > 0xFFFF:
                    msg = f"Minor class ID overflow while attaching circuits on node {node_name}: {current_minor} exceeds TC u16 limit."
                    logging.error(msg)
                    try:
                        ctx = json.dumps({"node": str(node_name), "minor": int(current_minor)})
                        submit_urgent_issue("LibreQoS", "Error", "TC_U16_OVERFLOW", msg, ctx, f"TC_U16_OVERFLOW_NODE_{str(node_name)}")
                    except Exception:
                        pass
                    raise ValueError(msg)
            if len(circuits_for_node) > 0:
                node['circuits'] = circuits_for_node

        # Any circuits left in stale_circuits are removed this run: reclaim their minors
        try:
            if 'planner' in locals() and isinstance(planner, Planner) and len(stale_circuits) > 0:
                st_final = planner.getState()
                cmap = st_final.get('circuit_map') if isinstance(st_final, dict) else None
                fm = st_final.get('free_minors') if isinstance(st_final, dict) else None
                if isinstance(cmap, dict):
                    for cid in list(stale_circuits):
                        try:
                            entry = cmap.get(cid)
                            if entry is None:
                                # Attempt both string/int conversions for robustness
                                try:
                                    entry = cmap.get(str(cid))
                                except Exception:
                                    entry = None
                                if entry is None:
                                    try:
                                        entry = cmap.get(int(cid))
                                    except Exception:
                                        entry = None
                            if isinstance(entry, dict):
                                cpu = int(entry.get('cpu', 0))
                                minor = int(entry.get('minor', -1))
                                if isinstance(fm, list) and 0 <= cpu < len(fm) and isinstance(fm[cpu], list):
                                    # Reserve 1..3 globally; only return minors >=4 to the free list
                                    if minor >= 4 and minor <= 0xFFFF and (minor not in fm[cpu]):
                                        fm[cpu].append(minor)
                                # Remove from circuit_map
                                try:
                                    cmap.pop(cid, None)
                                except Exception:
                                    try:
                                        cmap.pop(str(cid), None)
                                    except Exception:
                                        pass
                        except Exception:
                            pass
        except Exception:
            pass

        bakery = Bakery()
        bakery.start_batch() # Initializes the bakery transaction
        linuxTCcommands = []
        devicesShaped = []
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


        # Parse network structure. Emit all site HTB classes before any circuits for clarity.
        print("Preparing TC commands")

        def emit_sites(data):
            for node in sorted(data.keys()):
                site_name = data[node]['name'] if 'name' in data[node] else node
                # Avoid creating Bakery sites for CpueQueue* (CPU bin containers) when not in flat mode.
                # Generated_PN_* must be real HTB parents (one per CPU) so circuits under them have valid parents;
                # therefore, do NOT skip Generated_PN_* here.
                try:
                    name_s = str(site_name)
                    if (not is_network_flat()) and (name_s.startswith('CpueQueue')):
                        # Recurse into children to ensure real sites are still emitted
                        if 'children' in data[node]:
                            sorted_children = dict(sorted(data[node]['children'].items()))
                            emit_sites(sorted_children)
                        continue
                except Exception:
                    pass
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
                # Legacy TC output for reference
                command = 'class add dev ' + interface_a() + ' parent ' + data[node]['parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ format_rate_for_tc(data[node]['downloadBandwidthMbpsMin']) + ' ceil '+ format_rate_for_tc(data[node]['downloadBandwidthMbps']) + ' prio 3' + quantum(data[node]['downloadBandwidthMbps'])
                linuxTCcommands.append(command)
                command = 'class add dev ' + interface_b() + ' parent ' + data[node]['up_parentClassID'] + ' classid ' + data[node]['classMinor'] + ' htb rate '+ format_rate_for_tc(data[node]['uploadBandwidthMbpsMin']) + ' ceil '+ format_rate_for_tc(data[node]['uploadBandwidthMbps']) + ' prio 3' + quantum(data[node]['uploadBandwidthMbps'])
                linuxTCcommands.append(command)
                if 'children' in data[node]:
                    sorted_children = dict(sorted(data[node]['children'].items()))
                    emit_sites(sorted_children)

        def emit_circuits(data):
            # Cake needs help handling rates lower than 5 Mbps
            def sqmFixupRate(rate:int, sqm:str) -> str:
                # If we aren't using cake, just return the sqm string
                if not sqm.startswith("cake") or "rtt" in sqm:
                    return sqm
                match rate:
                    case 1: return sqm + " rtt 300"
                    case 2: return sqm + " rtt 180"
                    case 3: return sqm + " rtt 140"
                    case 4: return sqm + " rtt 120"
                    case _: return sqm

            for node in sorted(data.keys()):
                if 'circuits' in data[node]:
                    sorted_circuits = sorted(
                        data[node]['circuits'],
                        key=lambda c: c.get('circuitName', c.get('circuitID', ''))
                    )
                    for circuit in sorted_circuits:
                        # If circuit mins exceed node mins - handle low min rates of 1 to mean 10 kbps.
                        min_down = circuit['minDownload']
                        min_up = circuit['minUpload']
                        if node in nodes_requiring_min_squashing:
                            if min_down == 1:
                                min_down = 0.01
                            if min_up == 1:
                                min_up = 0.01
                        # Ensure min < max
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
                            logging.debug(f"Circuit '{circuit.get('circuitID','unknown')}' min download ({md}) >= max ({max_down}); lowering min to {new_md}")
                            min_down = new_md
                        if mu >= max_up:
                            new_mu = (max_up - 1.0) if max_up >= 1.0 else max(0.01, max_up - 0.01)
                            logging.debug(f"Circuit '{circuit.get('circuitID','unknown')}' min upload ({mu}) >= max ({max_up}); lowering min to {new_mu}")
                            min_up = new_mu

                        # Comment and IP aggregation (for legacy output and mappings)
                        tcComment = " # CircuitID: " + circuit['circuitID'] + " DeviceIDs: "
                        tcComment = tcComment.replace("\n", "")
                        circuit_name = circuit['circuitID'] if 'circuitID' in circuit else "unknown"
                        ip_list = []
                        for device in circuit['devices']:
                            if device['ipv4s']:
                                ip_list.extend(device['ipv4s'])
                            if device['ipv6s']:
                                ip_list.extend(device['ipv6s'])
                        ip_addresses_str = ','.join(ip_list)

                        sqm_override = circuit['sqm'] if 'sqm' in circuit else None
                        bakery.add_circuit(
                            circuit_name,
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
                        # Legacy TC output for reference
                        command = 'class add dev ' + interface_a() + ' parent ' + data[node]['classid'] + ' classid ' + circuit['classMinor'] + ' htb rate '+ format_rate_for_tc(min_down) + ' ceil '+ format_rate_for_tc(circuit['maxDownload']) + ' prio 3' + quantum(circuit['maxDownload']) + tcComment
                        linuxTCcommands.append(command)
                        if monitor_mode_only() == False:
                            def effective_sqm_str(rate, override, direction):
                                base = sqm()
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
                        if monitor_mode_only() == False:
                            sqm_override = circuit['sqm'] if 'sqm' in circuit else None
                            useSqm = effective_sqm_str(circuit['maxUpload'], sqm_override, 'up')
                            if useSqm != '':
                                command = 'qdisc add dev ' + interface_b() + ' parent ' + circuit['up_classMajor'] + ':' + circuit['classMinor'] + ' ' + useSqm
                                linuxTCcommands.append(command)
                        for device in circuit['devices']:
                            if device['ipv4s']:
                                for ipv4 in device['ipv4s']:
                                    ipMapBatch.add_ip_mapping(str(ipv4), circuit['classid'], data[node]['cpuNum'], False)
                                    if on_a_stick():
                                        ipMapBatch.add_ip_mapping(str(ipv4), circuit['up_classid'], data[node]['up_cpuNum'], True)
                            if device['ipv6s']:
                                for ipv6 in device['ipv6s']:
                                    ipMapBatch.add_ip_mapping(str(ipv6), circuit['classid'], data[node]['cpuNum'], False)
                                    if on_a_stick():
                                        ipMapBatch.add_ip_mapping(str(ipv6), circuit['up_classid'], data[node]['up_cpuNum'], True)
                            if device['deviceName'] not in devicesShaped:
                                devicesShaped.append(device['deviceName'])
                if 'children' in data[node]:
                    sorted_children = dict(sorted(data[node]['children'].items()))
                    emit_circuits(sorted_children)

        # Emit site HTB classes first, then circuits
        emit_sites(network)
        emit_circuits(network)

        # Save queuingStructure
        queuingStructure = {}
        queuingStructure['Network'] = network
        queuingStructure['lastUsedClassIDCounterByCPU'] = minorByCPU
        queuingStructure['generatedPNs'] = generatedPNs
        with open('queuingStructure.json', 'w') as infile:
            json.dump(queuingStructure, infile, indent=4)


        # Record start time of actual filter reload
        reloadStartTime = datetime.now()


        # Clear Prior Settings
        # We don't want to do this every time, with lqosd managing queues statefully.
        # clearPriorSettings(interface_a(), interface_b())


        # Setup XDP and disable XPS regardless of whether it is first run or not (necessary to handle cases where systemctl stop was used)
        xdpStartTime = datetime.now()
        #if enable_actual_shell_commands():
        # The bakery will handle stale mapping cleanup; avoid clearing mappings here.
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
        with open('linux_tc.txt', 'w') as f:
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
            ipMapBatch.finish_ip_mappings()
            ipMapBatch.submit()
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
        devicesSkipped = []
        for circuit in subscriberCircuits:
            for device in circuit['devices']:
                if device['deviceName'] not in devicesShaped:
                    devicesSkipped.append((device['deviceName'],device['deviceID']))
        if len(devicesSkipped) > 0:
            warnings.warn('Some devices were not shaped. Please check to ensure they have a valid ParentNode listed in ShapedDevices.csv:', stacklevel=2)
            print("Devices not shaped:")
            for entry in devicesSkipped:
                name, idNum = entry
                print('DeviceID: ' + idNum + '\t DeviceName: ' + name)

        # Save ShapedDevices.csv as ShapedDevices.lastLoaded.csv
        shutil.copyfile('ShapedDevices.csv', 'ShapedDevices.lastLoaded.csv')

        # Save for stats
        with open('statsByCircuit.json', 'w') as f:
            f.write(json.dumps(subscriberCircuits, indent=4))
        with open('statsByParentNode.json', 'w') as f:
            f.write(json.dumps(parentNodes, indent=4))


        # Record time this run completed at
        # filename = os.path.join(_here, 'lastRun.txt')
        with open("lastRun.txt", 'w') as file:
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


        # Persist planner state (if initialized)
        try:
            if 'planner' in locals() and isinstance(planner, Planner):
                # Do not mutate 'site_names' here; it must reflect the original invariant
                # used to decide planner rebuild (e.g., depth-1 names for tiered).
                planner.persist()
        except Exception as e:
            warnings.warn(f"Planner persist failed: {e}", stacklevel=2)

        # Done
        print("refreshShapers completed on " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))

def refreshShapersUpdateOnly():
    # Starting
    print("refreshShapers starting at " + datetime.now().strftime("%d/%m/%Y %H:%M:%S"))


    # Warn user if enableActualShellCommands is False, because that would mean no actual commands are executing
    if enable_actual_shell_commands() == False:
        warnings.warn("enableActualShellCommands is set to False. None of the commands below will actually be executed. Simulated run.", stacklevel=2)


    # Files
    shapedDevicesFile = 'ShapedDevices.csv'
    networkJSONfile = 'network.json'


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
        if os.path.isfile('lastGoodConfig.json'):
            with open('lastGoodConfig.json', 'r') as j:
                originalNetwork = json.loads(j.read())
            with open('network.json', 'r') as j:
                newestNetwork = json.loads(j.read())
            ddiff = DeepDiff(originalNetwork, newestNetwork, ignore_order=True)
            if ddiff != {}:
                networkChanged = True

        # Check for changes to ShapedDevices.csv
        newlyUpdatedSubscriberCircuits,	newlyUpdatedDictForCircuitsWithoutParentNodes = loadSubscriberCircuits('ShapedDevices.csv')
        lastLoadedSubscriberCircuits, lastLoadedDictForCircuitsWithoutParentNodes = loadSubscriberCircuits('ShapedDevices.lastLoaded.csv')

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
        for circuitID, circuit in newlyUpdatedSubscriberCircuitsByID.items():
            if (circuitID not in lastLoadedSubscriberCircuitsByID):
                devicesChanged = True


        if devicesChanged or networkChanged:
            print('Observed changes to ShapedDevices.csv or network.json. Applying full reload now')
            refreshShapers()
        else:
            print('Observed no changes to ShapedDevices.csv or network.json. Leaving queues as is.')

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
        os.exit(-1)

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
    args = parser.parse_args()
    logging.basicConfig(level=args.loglevel)

    # Planner reset no longer applicable (stateful planner removed)

    if args.validate:
        status = validateNetworkAndDevices()
    elif args.clearrules:
        tearDown(interface_a(), interface_b())
    elif args.updateonly:
        # Single-interface updates don't work at all right now.
        if on_a_stick():
            print("--updateonly is not supported for single-interface configurations")
            os.exit(-1)
        refreshShapersUpdateOnly()
    else:
        # Refresh and/or set up queues
        refreshShapers()

    # Free the lock file
    free_lock_file()
