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
    is_virtual_node,
)

from liblqos_python import is_lqosd_alive, clear_ip_mappings, delete_ip_mapping, validate_shaped_devices, \
    is_libre_already_running, create_lock_file, free_lock_file, add_ip_mapping, BatchedCommands, \
    check_config, sqm, upstream_bandwidth_capacity_download_mbps, upstream_bandwidth_capacity_upload_mbps, \
    interface_a, interface_b, enable_actual_shell_commands, use_bin_packing_to_balance_cpu, monitor_mode_only, \
    run_shell_commands_as_sudo, generated_pn_download_mbps, generated_pn_upload_mbps, queues_available_override, \
    on_a_stick, get_tree_weights, get_weights, is_network_flat, get_libreqos_directory, enable_insight_topology, \
    is_insight_enabled, \
    fast_queues_fq_codel, \
    Bakery

# Optional: urgent issue submission (available in newer liblqos_python)
try:
    from liblqos_python import submit_urgent_issue  # type: ignore
except Exception:
    def submit_urgent_issue(*_args, **_kwargs):
        return False

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

        # Preserve the logical parent (as configured in ShapedDevices.csv) before any shaping-time rewrites.
        for circuit in subscriberCircuits:
            circuit['logicalParentNode'] = circuit.get('ParentNode')

        # Load network hierarchy
        with open(networkJSONfile, 'r') as j:
            network = json.loads(j.read())

        # Flat networks ({}) don't require ParentNode entries. Treat every circuit as
        # unparented so they can be distributed across generated parent nodes / CPUs.
        flat_network = (len(network) == 0)
        try:
            flat_network = flat_network or is_network_flat()
        except Exception:
            pass

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

        # If in monitorOnlyMode, override network.json bandwidth rates to where no shaping will actually occur
        if monitor_mode_only() == True:
            def overrideNetworkBandwidths(data):
                for elem in data:
                    if 'children' in data[elem]:
                        overrideNetworkBandwidths(data[elem]['children'])
                    data[elem]['downloadBandwidthMbpsMin'] = 100000
                    data[elem]['uploadBandwidthMbpsMin'] = 100000
            overrideNetworkBandwidths(network)

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
            state_path = os.path.join(get_libreqos_directory(), "planner_state.json")
            state = {}
            if bin_planner is not None:
                state = bin_planner.load_state(state_path)
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
            if monitor_mode_only() == True:
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
                    bin_planner.save_state(state_path, state)
                except Exception as e:
                    warnings.warn(f"Failed to save planner state at {state_path}: {e}", stacklevel=2)

            print("Finished planning generated parent nodes")
        else:
            genPNcounter = 0
            for circuit in subscriberCircuits:
                if circuit['ParentNode'] == 'none':
                    circuit['ParentNode'] = generatedPNs[genPNcounter]
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
        minorByCPUpreloaded = {}
        knownClassIDs = []
        nodes_requiring_min_squashing = {}
        # Persisted circuit minor assignments for stability across reloads
        circuit_state_from_disk = {}
        circuit_state_updated = {}
        used_minors_by_queue = {cpu + 1: set() for cpu in range(queuesAvailable)}
        # Track minor counter by CPU. This way we can have > 32000 hosts (htb has u16 limit to minor handle)
        # Minor numbers start at 3 to reserve 1 for root qdisc and 2 for default class
        # With CIRCUIT_PADDING, we leave gaps between nodes to allow future circuit additions
        # without disrupting existing ClassID assignments. This maintains stability across reloads.
        for x in range(queuesAvailable):
            minorByCPUpreloaded[x+1] = 3
        def traverseNetwork(data, depth, major, minorByCPU, queue, parentClassID, upParentClassID, parentMaxDL, parentMaxUL, parentMinDL, parentMinUL):
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
                circuitsForThisNetworkNode = []
                nodeClassID = hex(major) + ':' + hex(minorByCPU[queue])
                upNodeClassID = hex(major+stickOffset) + ':' + hex(minorByCPU[queue])
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
                data[node]['classMinor'] = hex(minorByCPU[queue])
                data[node]['cpuNum'] = hex(queue-1)
                data[node]['up_cpuNum'] = hex(queue-1+stickOffset)
                used_minors_by_queue.setdefault(queue, set()).add(minorByCPU[queue])
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
                # If a device from ShapedDevices.csv lists this node as its Parent Node, attach it as a leaf to this node HTB
                if node in circuits_by_parent_node:
                    # If mins of circuits combined exceed min of parent node - set to 1
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
                    # Sort circuits by name for stable ordering
                    sorted_circuits = sorted(circuits_by_parent_node[node],
                               key=lambda c: c.get('circuitName', c.get('circuitID', '')))
                    for circuit in sorted_circuits:
                        if node == circuit['ParentNode']:
                            if monitor_mode_only() == False:
                                if circuit['maxDownload'] > data[node]['downloadBandwidthMbps']:
                                    logging.info("downloadMax of Circuit ID [" + circuit['circuitID'] + "] exceeded that of its parent node. Reducing to that of its parent node now.", stacklevel=2)
                                if circuit['maxUpload'] > data[node]['uploadBandwidthMbps']:
                                    logging.info("uploadMax of Circuit ID [" + circuit['circuitID'] + "] exceeded that of its parent node. Reducing to that of its parent node now.", stacklevel=2)
                            parentString = hex(major) + ':'
                            # Attempt to reuse a stored minor for stability (only if parent/bin unchanged and not colliding)
                            reuse_minor = None
                            try:
                                circuit_id_str = str(circuit.get('circuitID'))
                                stored_entry = circuit_state_from_disk.get(circuit_id_str, {}) if isinstance(circuit_state_from_disk, dict) else {}
                                stored_minor = stored_entry.get("class_minor")
                                stored_queue = stored_entry.get("queue")
                                stored_parent = stored_entry.get("parent_node")
                                if stored_minor is not None and stored_queue is not None and stored_parent is not None:
                                    try:
                                        stored_minor = int(stored_minor)
                                    except Exception:
                                        stored_minor = None
                                    try:
                                        stored_queue = int(stored_queue)
                                    except Exception:
                                        stored_queue = None
                                if stored_minor is not None and stored_queue == queue and stored_parent == circuit['ParentNode']:
                                    if stored_minor not in used_minors_by_queue.setdefault(queue, set()):
                                        reuse_minor = stored_minor
                            except Exception:
                                pass

                            candidate_minor = reuse_minor if reuse_minor is not None else minorByCPU[queue]
                            reserved = used_minors_by_queue.setdefault(queue, set())
                            while candidate_minor in reserved:
                                candidate_minor += 1

                            flowIDstring = hex(major) + ':' + hex(candidate_minor)
                            upFlowIDstring = hex(major + stickOffset) + ':' + hex(candidate_minor)
                            circuit['classid'] = flowIDstring
                            circuit['up_classid'] = upFlowIDstring
                            logging.info("Added up_classid to circuit: " + circuit['up_classid'])
                            # Create circuit dictionary to be added to network structure, eventually output as queuingStructure.json
                            maxDownload = min(circuit['maxDownload'],data[node]['downloadBandwidthMbps'])
                            maxUpload = min(circuit['maxUpload'],data[node]['uploadBandwidthMbps'])
                            if override_min_down:
                                circuit['minDownload'] = 1
                            if override_min_up:
                                circuit['minUpload'] = 1
                            minDownload = min(circuit['minDownload'],maxDownload)
                            minUpload = min(circuit['minUpload'],maxUpload)
                            thisNewCircuitItemForNetwork = {
                                'maxDownload' : maxDownload,
                                'maxUpload' : maxUpload,
                                'minDownload' : minDownload,
                                'minUpload' : minUpload,
                                "circuitID": circuit['circuitID'],
                                "circuitName": circuit['circuitName'],
                                "ParentNode": circuit['ParentNode'],
                                "logicalParentNode": circuit.get('logicalParentNode', circuit['ParentNode']),
                                "devices": circuit['devices'],
                                "classid": flowIDstring,
                                "up_classid" : upFlowIDstring,
                                "classMajor": hex(major),
                                "up_classMajor" : hex(major + stickOffset),
                                "classMinor": hex(candidate_minor),
                                "comment": circuit['comment']
                            }
                            # Attach the planner weight used by the planner/UI summary
                            # Priority: explicit weight -> fallback to maxDownload
                            try:
                                cid = str(circuit.get('circuitID',''))
                                w = None
                                if cid in weight_by_circuit_id:
                                    w = float(weight_by_circuit_id[cid])
                                if w is None:
                                    w = float(maxDownload)
                                # Treat 1000 as a sentinel default from Insight; use maxDownload instead
                                if abs(w - 1000.0) < 1e-6:
                                    w = float(maxDownload)
                                # If the circuit's configured rate equals the generated PN default,
                                # ignore it for weight purposes.
                                try:
                                    default_rate = float(generated_pn_download_mbps())
                                    if abs(float(maxDownload) - default_rate) < 1e-6:
                                        w = 0.0
                                except Exception:
                                    pass
                                thisNewCircuitItemForNetwork['planner_weight'] = w
                            except Exception:
                                pass
                            # Preserve optional per-circuit SQM override for downstream bakery call
                            if 'sqm' in circuit and circuit['sqm']:
                                thisNewCircuitItemForNetwork['sqm'] = circuit['sqm']
                            # Generate TC commands to be executed later
                            thisNewCircuitItemForNetwork['devices'] = circuit['devices']
                            circuitsForThisNetworkNode.append(thisNewCircuitItemForNetwork)
                            reserved.add(candidate_minor)
                            minorByCPU[queue] = max(minorByCPU[queue], candidate_minor) + 1
                            try:
                                # Record the assigned minor for future stability
                                circuit_state_updated[str(circuit.get('circuitID'))] = {
                                    "class_minor": candidate_minor,
                                    "queue": queue,
                                    "parent_node": circuit['ParentNode'],
                                    "class_major": major,
                                    "up_class_major": major + stickOffset,
                                }
                            except Exception:
                                pass
                if len(circuitsForThisNetworkNode) > 0:
                    data[node]['circuits'] = circuitsForThisNetworkNode

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

            def round_robin_assign(items_list, bins):
                if not bins:
                    return {}, []
                names = [str(it["id"]) for it in items_list]
                names.sort()
                assignment_local = {}
                for idx, name in enumerate(names):
                    assignment_local[name] = bins[idx % len(bins)]
                return assignment_local, list(assignment_local.keys())

            for node in network:
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

            def greedy_assign(items_list, bins):
                bin_loads = {key: 0.0 for key in bins}
                pairs = [(str(it["id"]), float(it["weight"])) for it in items_list]
                pairs.sort(key=lambda nw: (-nw[1], nw[0]))
                assignment_local = {}
                for name, wgt in pairs:
                    target = min(bin_loads.items(), key=lambda kv: (kv[1], kv[0]))[0]
                    assignment_local[name] = target
                    bin_loads[target] += wgt
                return assignment_local, list(assignment_local.keys())

            # Prepare bins and capacities
            cpu_keys = ["CpueQueue" + str(cpu) for cpu in range(queuesAvailable)]
            bins_list = [{"id": key} for key in cpu_keys]
            capacities = {key: 1.0 for key in cpu_keys}
            valid_bins = set(capacities.keys())

            planner_used = False
            state_path = os.path.join(get_libreqos_directory(), "planner_state.json")
            state = {}
            now_ts = time.time()
            assignment = {}
            changed = []
            bin_planner = None

            # Load planner state and generate assignments when Insight is enabled/licensed
            if insight_enabled:
                try:
                    import bin_planner as bin_planner  # type: ignore
                except ImportError:
                    bin_planner = None

                if bin_planner is not None:
                    try:
                        state = bin_planner.load_state(state_path)
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
                    if monitor_mode_only() == True:
                        params["move_budget_per_run"] = 0

                    try:
                        assignment, changed = bin_planner.plan_assignments(
                            items, bins_list, capacities, prev_assign, now_ts, params
                        )
                        planner_used = True
                    except Exception as e:
                        warnings.warn(
                            f"Planner failed ({e}); falling back to greedy distribution.",
                            stacklevel=2,
                        )
                        assignment, changed = greedy_assign(items, cpu_keys)
                        planner_used = False
                else:
                    warnings.warn(
                        "Binpacking requested, but planner module is unavailable; using greedy distribution.",
                        stacklevel=2,
                    )
                    assignment, changed = greedy_assign(items, cpu_keys)
                    planner_used = False
            else:
                # Insight is unavailable; use deterministic round-robin distribution.
                assignment, changed = round_robin_assign(items, cpu_keys)
                planner_used = False

            # Validate assignment covers all items and uses known bins (planner may return partial results)
            try:
                if not isinstance(assignment, dict):
                    assignment, changed = greedy_assign(items, cpu_keys)
            except Exception:
                assignment, changed = greedy_assign(items, cpu_keys)
            try:
                item_ids = {str(it["id"]) for it in items}
                rr_fallback, _ = round_robin_assign(items, cpu_keys)
                for iid in item_ids:
                    tgt = assignment.get(iid)
                    if tgt not in valid_bins:
                        assignment[iid] = rr_fallback.get(
                            iid, cpu_keys[-1] if cpu_keys else "CpueQueue0"
                        )
            except Exception:
                pass

            # Sanity check: if planner returns a degenerate assignment (everything in one bin),
            # fall back to a simple greedy distribution so the physical tree can spread.
            if planner_used:
                try:
                    # Exclude Generated_PN_* placeholders from the degeneracy test: they may not
                    # appear in the UI tree and can mask a collapsed real topology.
                    interesting_ids = {
                        str(it["id"])
                        for it in items
                        if not str(it["id"]).startswith("Generated_PN_")
                    }
                    used = {
                        tgt
                        for iid, tgt in assignment.items()
                        if iid in interesting_ids and tgt in valid_bins
                    }
                    if queuesAvailable > 1 and len(interesting_ids) > 1 and len(used) <= 1:
                        warnings.warn(
                            "Planner produced degenerate CPU binning (single-bin result); falling back to greedy distribution.",
                            stacklevel=2,
                        )
                        assignment, changed = greedy_assign(items, cpu_keys)
                        planner_used = False
                except Exception:
                    pass

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
                tgt = assignment.get(node)
                if tgt is None:
                    tgt = "CpueQueue" + str(queuesAvailable - 1)
                binnedNetwork[tgt]['children'][node] = network[node]
            network = binnedNetwork

            # Update and save state
            if planner_used and bin_planner is not None and isinstance(state, dict):
                if state.get("salt") is None:
                    state["salt"] = "default_salt"
                if "assignments" not in state or not isinstance(state["assignments"], dict):
                    state["assignments"] = {}
                if "last_change_ts" not in state or not isinstance(state["last_change_ts"], dict):
                    state["last_change_ts"] = {}
                for iid, b in assignment.items():
                    if iid in changed:
                        state["last_change_ts"][iid] = now_ts
                    state["assignments"][iid] = b
                try:
                    print(f"Saving planner state to {state_path} (top-level CPU binning)")
                    bin_planner.save_state(state_path, state)
                except Exception as e:
                    warnings.warn(
                        f"Failed to save planner state at {state_path}: {e}", stacklevel=2
                    )

        # Seed persisted circuit minor assignments (TTL already enforced in bin_planner.load_state()).
        try:
            state  # noqa: B018
        except NameError:
            state = {}
        try:
            import bin_planner  # noqa: F401
        except ImportError:
            bin_planner = None
        if bin_planner is not None and (not isinstance(state, dict) or len(state.keys()) == 0):
            try:
                state_path = os.path.join(get_libreqos_directory(), "planner_state.json")
                state = bin_planner.load_state(state_path)
            except Exception:
                state = {}
        try:
            circuit_state_from_disk = state.get("circuits", {}) if isinstance(state, dict) else {}
        except Exception:
            circuit_state_from_disk = {}
        circuit_state_updated = {}
        used_minors_by_queue = {cpu + 1: set() for cpu in range(queuesAvailable)}

        # Here is the actual call to the recursive traverseNetwork() function. finalMinor is not used.
        minorByCPU = traverseNetwork(network, 0, major=1, minorByCPU=minorByCPUpreloaded, queue=1, parentClassID=None, upParentClassID=None, parentMaxDL=upstream_bandwidth_capacity_download_mbps(), parentMaxUL=upstream_bandwidth_capacity_upload_mbps(), parentMinDL=upstream_bandwidth_capacity_download_mbps(), parentMinUL=upstream_bandwidth_capacity_upload_mbps())

        # Persist the updated circuit minor map for future stability
        if 'bin_planner' not in locals():
            bin_planner = None
        if bin_planner is not None and isinstance(state, dict):
            if 'circuits' not in state or not isinstance(state.get('circuits'), dict):
                state['circuits'] = {}
            state['circuits'] = circuit_state_updated
            try:
                state_path = os.path.join(get_libreqos_directory(), "planner_state.json")
            except Exception:
                state_path = None
            if state_path:
                try:
                    print(f"Saving planner state to {state_path} (circuit minors)")
                    bin_planner.save_state(state_path, state)
                except Exception as e:
                    warnings.warn(f"Failed to save planner circuit state at {state_path}: {e}", stacklevel=2)

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


        # Parse network structure. For each tier, generate commands to create corresponding HTB and leaf classes. Prepare commands for execution later
        # Define lists for hash filters
        print("Preparing TC commands")
        def traverseNetwork(data):

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
                        # Only add CAKE / fq_codel qdisc if monitorOnlyMode is Off
                        if monitor_mode_only() == False:
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
                        # Only add CAKE / fq_codel qdisc if monitorOnlyMode is Off
                        if monitor_mode_only() == False:
                            # SQM Fixup for lower rates (and per-circuit override)
                            sqm_override = circuit['sqm'] if 'sqm' in circuit else None
                            useSqm = effective_sqm_str(circuit['maxUpload'], sqm_override, 'up')
                            if useSqm != '':
                                command = 'qdisc add dev ' + interface_b() + ' parent ' + circuit['up_classMajor'] + ':' + circuit['classMinor'] + ' ' + useSqm
                                linuxTCcommands.append(command)
                        for device in circuit['devices']:
                            if device['ipv4s']:
                                for ipv4 in device['ipv4s']:
                                    ipMapBatch.add_ip_mapping(str(ipv4), circuit['classid'], data[node]['cpuNum'], False)
                                    #xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv4) + ' --cpu ' + data[node]['cpuNum'] + ' --classid ' + circuit['classid'])
                                    if on_a_stick():
                                        ipMapBatch.add_ip_mapping(str(ipv4), circuit['up_classid'], data[node]['up_cpuNum'], True)
                                        #xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv4) + ' --cpu ' + data[node]['up_cpuNum'] + ' --classid ' + circuit['up_classid'] + ' --upload 1')
                            if device['ipv6s']:
                                for ipv6 in device['ipv6s']:
                                    ipMapBatch.add_ip_mapping(str(ipv6), circuit['classid'], data[node]['cpuNum'], False)
                                    #xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv6) + ' --cpu ' + data[node]['cpuNum'] + ' --classid ' + circuit['classid'])
                                    if on_a_stick():
                                        ipMapBatch.add_ip_mapping(str(ipv6), circuit['up_classid'], data[node]['up_cpuNum'], True)
                                        #xdpCPUmapCommands.append('./bin/xdp_iphash_to_cpu_cmdline add --ip ' + str(ipv6) + ' --cpu ' + data[node]['up_cpuNum'] + ' --classid ' + circuit['up_classid'] + ' --upload 1')
                            if device['deviceName'] not in devicesShaped:
                                devicesShaped.append(device['deviceName'])
                # Recursive call this function for children nodes attached to this node
                if 'children' in data[node]:
                    # Sort children to ensure consistent traversal order
                    sorted_children = dict(sorted(data[node]['children'].items()))
                    traverseNetwork(sorted_children)
        # Here is the actual call to the recursive traverseNetwork() function.
        traverseNetwork(network)

        # Save queuingStructure
        queuingStructure = {}
        queuingStructure['Network'] = network
        queuingStructure['lastUsedClassIDCounterByCPU'] = minorByCPU
        queuingStructure['generatedPNs'] = generatedPNs
        queuingStructure['logical_to_physical_node'] = logical_to_physical_node
        queuingStructure['virtual_nodes'] = virtual_nodes
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
            else:
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
            state_path = os.path.join(get_libreqos_directory(), "planner_state.json")
            if os.path.exists(state_path):
                os.remove(state_path)
                print(f"Removed planner state: {state_path}")
        except Exception as e:
            print(f"Warning: could not remove planner state: {e}")

    if args.validate:
        status = validateNetworkAndDevices()
    elif args.clearrules:
        tearDown(interface_a(), interface_b())
    elif args.updateonly:
        # Single-interface updates don't work at all right now.
        if on_a_stick():
            print("--updateonly is not supported for single-interface configurations")
            os._exit(-1)
        refreshShapersUpdateOnly()
    else:
        # Refresh and/or set up queues
        refreshShapers()

    # Free the lock file
    free_lock_file()
