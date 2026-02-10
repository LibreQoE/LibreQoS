import time
import datetime
import csv
import io
import json
import chardet
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
import subprocess
import sys
from io import StringIO
from liblqos_python import automatic_import_uisp, automatic_import_splynx, queue_refresh_interval_mins, \
    automatic_import_powercode, automatic_import_sonar, influx_db_enabled, get_libreqos_directory, \
    blackboard_finish, blackboard_submit, automatic_import_wispgate, enable_insight_topology, insight_topology_role, \
    automatic_import_netzur, calculate_hash, scheduler_alive, scheduler_error, overrides_persistent_devices, overrides_circuit_adjustments, overrides_network_adjustments

from apscheduler.schedulers.background import BlockingScheduler
from apscheduler.executors.pool import ThreadPoolExecutor
import os.path
import os

ads = BlockingScheduler(executors={'default': ThreadPoolExecutor(1)})
network_hash = 0


def capture_output_and_run(func):
    """Capture stdout/stderr from a callable and ensure failures are non-fatal."""
    old_stdout = sys.stdout
    old_stderr = sys.stderr
    captured_output = StringIO()
    try:
        sys.stdout = captured_output
        sys.stderr = captured_output
        func()
    except BaseException as e:
        # Catch BaseException to also handle SystemExit/KeyboardInterrupt from integrations
        error_msg = f"Failed to execute function: {str(e)}"
        try:
            print(error_msg)
        finally:
            # Ensure scheduler gets error details even if printing fails
            scheduler_error(error_msg)
    finally:
        # Always restore stdio and flush captured output
        sys.stdout = old_stdout
        sys.stderr = old_stderr
        output = captured_output.getvalue()
        if output:
            print(output)
            scheduler_error(output)


def run_python_integration(module_name: str, func_name: str, label: str = ""):
    """
    Run a Python integration in a subprocess so failures cannot terminate the scheduler.
    Captures stdout/stderr, logs them, and continues regardless of exit code.
    """
    try:
        code = f"from {module_name} import {func_name} as f; f()"
        cmd = [sys.executable, "-c", code]
        result = subprocess.run(cmd, capture_output=True, text=True)
        output = (result.stdout or "") + (result.stderr or "")
        if output:
            print(output)
            scheduler_error(output)
        if result.returncode != 0:
            # Non-zero exit shouldn't stop scheduling; log and continue
            friendly = label or f"{module_name}.{func_name}"
            msg = f"Integration {friendly} exited with code {result.returncode}. Continuing."
            print(msg)
            scheduler_error(msg)
    except Exception as e:
        err = f"Failed to invoke integration {label or (module_name + '.' + func_name)}: {e}"
        print(err)
        scheduler_error(err)

def importFromCRM():
    # CRM Hooks
    if automatic_import_uisp():
        try:
            # Execute UISP integration in a subprocess and keep going on failure
            path = get_libreqos_directory() + "/bin/uisp_integration"
            result = subprocess.run([path], capture_output=True, text=True)
            output = (result.stdout or "") + (result.stderr or "")
            if output:
                print(output)
                # Report UISP output to error channel regardless of return code.
                scheduler_error(output)
            if result.returncode != 0:
                msg = f"UISP integration exited with code {result.returncode}. Continuing."
                print(msg)
                scheduler_error(msg)
            blackboard_finish()
        except Exception as e:
            error_msg = f"Failed to run UISP integration: {str(e)}"
            print(error_msg)
            scheduler_error(error_msg)
    elif automatic_import_splynx():
        run_python_integration("integrationSplynx", "importFromSplynx", label="Splynx")
    elif automatic_import_netzur():
        run_python_integration("integrationNetzur", "importFromNetzur", label="Netzur")
    elif automatic_import_powercode():
        run_python_integration("integrationPowercode", "importFromPowercode", label="Powercode")
    elif automatic_import_sonar():
        run_python_integration("integrationSonar", "importFromSonar", label="Sonar")
    elif automatic_import_wispgate():
        run_python_integration("integrationWISPGate", "importFromWISPGate", label="WISPGate")
    # Post-CRM Hooks
    path = get_libreqos_directory() + "/bin/post_integration_hook.sh"
    binPath = get_libreqos_directory() + "/bin"
    if os.path.isfile(path):
        try:
            subprocess.Popen(path, cwd=binPath)
        except Exception as e:
            msg = f"post_integration_hook.sh failed to launch: {e}"
            print(msg)
            scheduler_error(msg)
    # Handle lqos_overrides
    try:
        apply_lqos_overrides()
    except Exception as e:
        scheduler_error(f"Failed to apply lqos_overrides: {e}")
        print(f"Failed to apply lqos_overrides: {e}")


# --------------- Overrides Handling ---------------

SHAPED_DEVICES_HEADER = [
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


def shaped_devices_csv_path() -> str:
    base_dir = get_libreqos_directory()
    return base_dir + "/ShapedDevices.csv"


def read_shaped_devices_csv(path: str):
    """Read CSV with comment stripping and header handling. Returns (header, rows)."""
    if not os.path.isfile(path):
        return SHAPED_DEVICES_HEADER, []

    with open(path, 'rb') as f:
        raw_bytes = f.read()

    # Handle BOMs and encoding similar to LibreQoS.py
    if raw_bytes.startswith(b'\xef\xbb\xbf'):
        raw_bytes = raw_bytes[3:]
        text_content = raw_bytes.decode('utf-8')
    elif raw_bytes.startswith(b'\xff\xfe') or raw_bytes.startswith(b'\xfe\xff'):
        text_content = raw_bytes.decode('utf-16')
    else:
        try:
            text_content = raw_bytes.decode('utf-8')
        except UnicodeDecodeError:
            detected = chardet.detect(raw_bytes)
            encoding = detected['encoding'] or 'utf-8'
            text_content = raw_bytes.decode(encoding, errors='replace')

    with io.StringIO(text_content) as csv_file:
        reader = csv.reader(csv_file, delimiter=',')
        rows = [row for row in reader if row and not row[0].startswith('#')]
        if not rows:
            return SHAPED_DEVICES_HEADER, []
        header = rows[0]
        data_rows = rows[1:]
        if len(header) < len(SHAPED_DEVICES_HEADER):
            header = list(header) + SHAPED_DEVICES_HEADER[len(header):]
        target_len = len(header)
        if target_len > 0:
            for row in data_rows:
                if len(row) < target_len:
                    row.extend([""] * (target_len - len(row)))
        return header, data_rows


def override_devices_to_rows(devices):
    """Convert override device dicts to CSV rows (14 columns, optional sqm)."""
    rows = []
    for d in devices:
        ipv4s = d.get('ipv4s', [])
        ipv6s = d.get('ipv6s', [])
        sqm = d.get('sqm', '') or d.get('sqm_override', '')
        sqm = sqm or ""
        row = [
            d.get('circuitID', ''),
            d.get('circuitName', ''),
            d.get('deviceID', ''),
            d.get('deviceName', ''),
            d.get('ParentNode', ''),
            d.get('mac', ''),
            ','.join(ipv4s),
            ','.join(ipv6s),
            str(d.get('minDownload', '')),
            str(d.get('minUpload', '')),
            str(d.get('maxDownload', '')),
            str(d.get('maxUpload', '')),
            d.get('comment', ''),
            str(sqm),
        ]
        rows.append(row)
    return rows


def merge_rows_replace_by_device_id(existing_rows, override_rows):
    """Replace existing rows by device_id if present, else append."""
    index_by_device = {}
    for idx, row in enumerate(existing_rows):
        if len(row) >= 3:
            index_by_device[row[2]] = idx
    merged = list(existing_rows)
    changed = False
    for o in override_rows:
        device_id = o[2] if len(o) >= 3 else ''
        if device_id in index_by_device:
            idx = index_by_device[device_id]
            if merged[idx] != o:
                merged[idx] = o
                changed = True
        else:
            merged.append(o)
            changed = True
    return merged, changed


def write_shaped_devices_csv(path: str, header, rows):
    with open(path, 'w', encoding='utf-8', newline='') as f:
        writer = csv.writer(f)
        writer.writerow(header)
        writer.writerows(rows)


def apply_lqos_overrides():
    """Load ShapedDevices.csv, apply persistent devices and circuit adjustments, and save back."""
    path = shaped_devices_csv_path()
    header, rows = read_shaped_devices_csv(path)

    # 1) Persistent devices: replace by device_id or append
    extra = overrides_persistent_devices()
    override_rows = override_devices_to_rows(extra or [])
    merged_rows, changed = merge_rows_replace_by_device_id(rows, override_rows)

    # 2) Circuit adjustments: speed changes, removals, reparenting
    try:
        adjustments = overrides_circuit_adjustments()
    except Exception as e:
        print(f"Failed to read circuit adjustments: {e}")
        adjustments = []

    def set_if_some(value_opt, current_str):
        if value_opt is None:
            return current_str
        try:
            return str(float(value_opt))
        except Exception:
            return current_str

    if adjustments:
        for adj in adjustments:
            t = adj.get('type')
            if t == 'circuit_adjust_speed':
                cid = adj.get('circuit_id', '')
                for r in merged_rows:
                    if len(r) >= 12 and r[0] == cid:
                        r[8] = set_if_some(adj.get('min_download_bandwidth'), r[8] if len(r) > 8 else '')
                        r[10] = set_if_some(adj.get('max_download_bandwidth'), r[10] if len(r) > 10 else '')
                        r[9] = set_if_some(adj.get('min_upload_bandwidth'), r[9] if len(r) > 9 else '')
                        r[11] = set_if_some(adj.get('max_upload_bandwidth'), r[11] if len(r) > 11 else '')
                        changed = True
            elif t == 'device_adjust_speed':
                did = adj.get('device_id', '')
                for r in merged_rows:
                    if len(r) >= 12 and r[2] == did:
                        r[8] = set_if_some(adj.get('min_download_bandwidth'), r[8] if len(r) > 8 else '')
                        r[10] = set_if_some(adj.get('max_download_bandwidth'), r[10] if len(r) > 10 else '')
                        r[9] = set_if_some(adj.get('min_upload_bandwidth'), r[9] if len(r) > 9 else '')
                        r[11] = set_if_some(adj.get('max_upload_bandwidth'), r[11] if len(r) > 11 else '')
                        changed = True
            elif t == 'remove_circuit':
                cid = adj.get('circuit_id', '')
                before = len(merged_rows)
                merged_rows = [r for r in merged_rows if len(r) < 1 or r[0] != cid]
                if len(merged_rows) != before:
                    changed = True
            elif t == 'remove_device':
                did = adj.get('device_id', '')
                before = len(merged_rows)
                merged_rows = [r for r in merged_rows if len(r) < 3 or r[2] != did]
                if len(merged_rows) != before:
                    changed = True
            elif t == 'reparent_circuit':
                cid = adj.get('circuit_id', '')
                parent_node = adj.get('parent_node', '')
                for r in merged_rows:
                    if len(r) >= 5 and r[0] == cid:
                        r[4] = parent_node
                        changed = True

    if changed:
        write_shaped_devices_csv(path, header if header else SHAPED_DEVICES_HEADER, merged_rows)
        print("Updated ShapedDevices.csv with overrides")

    # 3) Load, adjust, and optionally save network.json
    nj_path = network_json_path()
    network = load_network_json(nj_path)
    net_changed = apply_network_adjustments(network)
    if net_changed:
        write_network_json(nj_path, network)
        print("Updated network.json with overrides")


# --------------- Network JSON handling ---------------

def network_json_path() -> str:
    base_dir = get_libreqos_directory()
    if enable_insight_topology():
        insight_path = os.path.join(base_dir, "network.insight.json")
        if os.path.exists(insight_path):
            return insight_path
    return os.path.join(base_dir, "network.json")


def load_network_json(path: str):
    if not os.path.isfile(path):
        return {}
    with open(path, 'r', encoding='utf-8') as f:
        try:
            return json.loads(f.read())
        except Exception:
            return {}


def apply_network_adjustments(network: dict) -> bool:
    """Apply network adjustments from overrides to the network JSON structure.

    Currently supports: adjust_site_speed (by site_name) updating
    downloadBandwidthMbps and uploadBandwidthMbps at the matching node, and
    set_node_virtual (by node_name) updating the boolean 'virtual' flag.
    Returns True if any changes were applied.
    """
    try:
        adjustments = overrides_network_adjustments()
    except Exception as e:
        print(f"Failed to read network adjustments: {e}")
        return False

    if not adjustments:
        return False

    def adjust_node(tree: dict, site: str, dl_opt, ul_opt) -> bool:
        changed_local = False
        for key in list(tree.keys()):
            if key == 'children':
                child = tree.get('children')
                if isinstance(child, dict):
                    if adjust_node(child, site, dl_opt, ul_opt):
                        changed_local = True
                continue
            node = tree.get(key)
            if isinstance(node, dict):
                if key == site:
                    if dl_opt is not None:
                        node['downloadBandwidthMbps'] = int(dl_opt)
                        changed_local = True
                    if ul_opt is not None:
                        node['uploadBandwidthMbps'] = int(ul_opt)
                        changed_local = True
                # Recurse into children
                if 'children' in node and isinstance(node['children'], dict):
                    if adjust_node(node['children'], site, dl_opt, ul_opt):
                        changed_local = True
        return changed_local

    def set_virtual_flag(tree: dict, node_name: str, virtual_val) -> bool:
        changed_local = False
        v = bool(virtual_val)
        for key in list(tree.keys()):
            if key == 'children':
                child = tree.get('children')
                if isinstance(child, dict):
                    if set_virtual_flag(child, node_name, v):
                        changed_local = True
                continue
            node = tree.get(key)
            if isinstance(node, dict):
                if key == node_name:
                    prev = bool(node.get('virtual', False))
                    if prev != v:
                        node['virtual'] = v
                        changed_local = True
                if 'children' in node and isinstance(node['children'], dict):
                    if set_virtual_flag(node['children'], node_name, v):
                        changed_local = True
        return changed_local

    net_changed = False
    for adj in adjustments:
        if adj.get('type') == 'adjust_site_speed':
            site = adj.get('site_name', '')
            dl = adj.get('download_bandwidth_mbps', None)
            ul = adj.get('upload_bandwidth_mbps', None)
            if site:
                if adjust_node(network, site, dl, ul):
                    net_changed = True
        elif adj.get('type') == 'set_node_virtual':
            node_name = adj.get('node_name', '')
            v = adj.get('virtual', None)
            if node_name and v is not None:
                if set_virtual_flag(network, node_name, v):
                    net_changed = True

    return net_changed


def write_network_json(path: str, network: dict):
    with open(path, 'w', encoding='utf-8') as f:
        f.write(json.dumps(network, indent=4))


def importAndShapeFullReload():
    importFromCRM()
    if not enable_insight_topology():
        refreshShapers()


def importAndShapePartialReload():
    global network_hash

    importFromCRM()
    # Calculate if the network.json or ShapedDevices.csv has changed and reload only if it has.
    new_hash = calculate_hash()
    if new_hash != network_hash:
        refreshShapersUpdateOnly()
        network_hash = new_hash
    else:
        print("No changes detected in network.json or ShapedDevices.csv, skipping shaper refresh.")


def not_dead_yet():
    #print(f"Scheduler alive at {datetime.datetime.now()}")
    scheduler_alive()

if __name__ == '__main__':
    try:
        importAndShapeFullReload()
        network_hash = calculate_hash()

        print("Starting scheduler with jobs:")
        print(f"- not_dead_yet every 1 minute")
        refresh_interval = queue_refresh_interval_mins()
        print(f"- importAndShapePartialReload every {refresh_interval} minutes")
        
        not_dead_yet()
        ads.add_job(not_dead_yet, 'interval', minutes=1, max_instances=1)
        ads.add_job(importAndShapePartialReload, 'interval', minutes=refresh_interval, max_instances=1)

        print("Scheduler starting...")
        ads.start()
    except Exception as e:
        print(f"Error starting scheduler: {e}")
        import traceback
        traceback.print_exc()
