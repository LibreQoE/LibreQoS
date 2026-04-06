import time
import datetime
import csv
import io
import json
import atexit
import chardet
from LibreQoS import refreshShapers, refreshShapersUpdateOnly
import subprocess
import sys
import tempfile
from io import StringIO
from liblqos_python import automatic_import_uisp, automatic_import_splynx, queue_refresh_interval_mins, \
    automatic_import_powercode, automatic_import_sonar, influx_db_enabled, get_libreqos_directory, \
    blackboard_finish, blackboard_submit, automatic_import_wispgate, enable_insight_topology, insight_topology_role, \
    automatic_import_netzur, automatic_import_visp, calculate_shaping_runtime_hash, efficiency_core_ids, scheduler_alive, scheduler_error, \
    scheduler_progress, overrides_persistent_devices_materialized, overrides_circuit_adjustments_materialized, \
    overrides_network_adjustments_materialized, \
    scheduler_output, wait_for_bus_ready

from apscheduler.schedulers.background import BlockingScheduler
from apscheduler.executors.pool import ThreadPoolExecutor
import os.path
import os

ads = BlockingScheduler(executors={'default': ThreadPoolExecutor(1)})
shaping_runtime_hash = 0
topology_runtime_process = None
topology_runtime_missing_reported = False
INTEGRATION_FAILURE_PREVIEW_LINES = 30
INTEGRATION_FAILURE_PREVIEW_CHARS = 4000
TOPOLOGY_RUNTIME_REFRESH_SECONDS = 3
SCHEDULER_STARTUP_STEP_COUNT = 5
SCHEDULER_REFRESH_STEP_COUNT = 4


def clear_scheduler_error():
    """Clear the scheduler error status shown in the Web UI."""
    scheduler_error("")


def clear_scheduler_output():
    """Clear the scheduler output shown in the Web UI."""
    scheduler_output("")


def _scheduler_progress_percent(step_index: int, step_count: int, *, active: bool) -> int:
    if step_count <= 0:
        return 0
    bounded_step = max(1, min(int(step_index), int(step_count)))
    completed_steps = bounded_step - 1 if active else bounded_step
    return max(0, min(100, int(round((completed_steps / step_count) * 100))))


def publish_scheduler_progress(active: bool, phase: str, phase_label: str, step_index: int, step_count: int, *, percent=None):
    try:
        resolved_percent = _scheduler_progress_percent(step_index, step_count, active=active) if percent is None else int(percent)
        resolved_percent = max(0, min(100, resolved_percent))
        scheduler_progress(
            bool(active),
            str(phase),
            str(phase_label),
            int(step_index),
            int(step_count),
            resolved_percent,
        )
    except Exception as e:
        print(f"Failed to publish scheduler progress: {e}")


def _integration_output_lines(output):
    normalized = (output or "").replace("\r\n", "\n").strip()
    if not normalized:
        return []
    return normalized.split("\n")


def _summarize_output_preview(output, *, max_lines, max_chars):
    lines = _integration_output_lines(output)
    if not lines:
        return ""

    preview_lines = lines[:max_lines]
    preview = "\n".join(preview_lines)
    truncated = len(lines) > max_lines
    if len(preview) > max_chars:
        preview = preview[:max_chars].rstrip()
        truncated = True
    if truncated:
        preview += "\n..."
    return preview


def _sanitize_label_for_filename(label):
    token = "".join(ch.lower() if ch.isalnum() else "_" for ch in (label or "integration"))
    token = token.strip("_")
    return token or "integration"


def _write_integration_output_artifact(label, output):
    normalized = (output or "").replace("\r\n", "\n").strip()
    if not normalized:
        return None
    timestamp = datetime.datetime.now().strftime("%Y%m%d_%H%M%S")
    artifact = os.path.join(
        tempfile.gettempdir(),
        f"lqos_scheduler_{_sanitize_label_for_filename(label)}_{timestamp}.log",
    )
    try:
        with open(artifact, "w", encoding="utf-8") as handle:
            handle.write(normalized)
            handle.write("\n")
        return artifact
    except Exception as e:
        print(f"Failed to write {label} output artifact: {e}")
        return None


def _publish_integration_result(label, result):
    output = ((result.stdout or "") + (result.stderr or "")).replace("\r\n", "\n").strip()
    if result.returncode == 0:
        line_count = len(_integration_output_lines(output))
        summary = (
            f"{label} completed successfully."
            if line_count == 0
            else f"{label} completed successfully. Captured {line_count} line(s) of output."
        )
        print(summary)
        scheduler_output(summary)
        return

    preview = _summarize_output_preview(
        output,
        max_lines=INTEGRATION_FAILURE_PREVIEW_LINES,
        max_chars=INTEGRATION_FAILURE_PREVIEW_CHARS,
    )
    artifact = _write_integration_output_artifact(label, output)
    message = f"{label} exited with code {result.returncode}. Continuing."
    if preview:
        message += f"\nOutput preview:\n{preview}"
    if artifact is not None:
        message += f"\nFull output saved to {artifact}"
    print(message)
    scheduler_error(message)


def get_integration_affinity_cpus():
    """Return efficiency-core CPU IDs to prefer for integration subprocesses."""
    try:
        cpus = efficiency_core_ids()
    except Exception as e:
        msg = f"Failed to determine efficiency cores for integrations: {e}"
        print(msg)
        scheduler_error(msg)
        return []

    normalized = []
    for cpu in cpus or []:
        try:
            normalized.append(int(cpu))
        except (TypeError, ValueError):
            continue
    return sorted(set(cpu for cpu in normalized if cpu >= 0))


def _affinity_preexec(cpu_ids):
    cpu_set = set(cpu_ids)

    def apply_affinity():
        os.sched_setaffinity(0, cpu_set)

    return apply_affinity


def run_integration_subprocess(cmd, *, capture_output=True, text=True, cwd=None, label="integration"):
    """
    Launch a scheduler-managed integration subprocess.
    Prefer detected efficiency cores when available, but retry unpinned on failure.
    """
    kwargs = {
        "capture_output": capture_output,
        "text": text,
    }
    if cwd is not None:
        kwargs["cwd"] = cwd

    cpu_ids = get_integration_affinity_cpus()
    used_affinity = False
    if cpu_ids and hasattr(os, "sched_setaffinity"):
        kwargs["preexec_fn"] = _affinity_preexec(cpu_ids)
        used_affinity = True

    try:
        return subprocess.run(cmd, **kwargs)
    except Exception as e:
        if not used_affinity:
            raise
        msg = (
            f"Failed to pin {label} to efficiency cores {cpu_ids}: {e}. "
            "Retrying without affinity."
        )
        print(msg)
        scheduler_error(msg)
        kwargs.pop("preexec_fn", None)
        return subprocess.run(cmd, **kwargs)


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
            preview = _summarize_output_preview(
                output,
                max_lines=INTEGRATION_FAILURE_PREVIEW_LINES,
                max_chars=INTEGRATION_FAILURE_PREVIEW_CHARS,
            )
            artifact = _write_integration_output_artifact("captured_integration", output)
            message = "Captured integration output."
            if preview:
                message += f"\nOutput preview:\n{preview}"
            if artifact is not None:
                message += f"\nFull output saved to {artifact}"
            print(message)
            scheduler_output(message)


def run_python_integration(module_name: str, func_name: str, label: str = ""):
    """
    Run a Python integration in a subprocess so failures cannot terminate the scheduler.
    Captures stdout/stderr, logs them, and continues regardless of exit code.
    """
    try:
        code = f"from {module_name} import {func_name} as f; f()"
        cmd = [sys.executable, "-c", code]
        friendly = label or f"{module_name}.{func_name}"
        result = run_integration_subprocess(
            cmd,
            capture_output=True,
            text=True,
            label=friendly,
        )
        _publish_integration_result(friendly, result)
    except Exception as e:
        err = f"Failed to invoke integration {label or (module_name + '.' + func_name)}: {e}"
        print(err)
        scheduler_error(err)

def importFromCRM(
    *,
    integration_phase="running_integration",
    integration_label="Running integration sync",
    integration_step=2,
    progress_step_count=SCHEDULER_STARTUP_STEP_COUNT,
    overrides_phase="applying_overrides",
    overrides_label="Applying overrides",
    overrides_step=3,
):
    clear_scheduler_error()
    clear_scheduler_output()
    publish_scheduler_progress(
        True,
        integration_phase,
        integration_label,
        integration_step,
        progress_step_count,
    )
    # CRM Hooks
    if automatic_import_uisp():
        try:
            # Execute UISP integration in a subprocess and keep going on failure
            path = get_libreqos_directory() + "/bin/uisp_integration"
            result = run_integration_subprocess(
                [path],
                capture_output=True,
                text=True,
                label="UISP integration",
            )
            _publish_integration_result("UISP integration", result)
            blackboard_finish()
        except Exception as e:
            error_msg = f"Failed to run UISP integration: {str(e)}"
            print(error_msg)
            scheduler_error(error_msg)
    elif automatic_import_splynx():
        run_python_integration("integrationSplynx", "importFromSplynx", label="Splynx")
    elif automatic_import_visp():
        run_python_integration("integrationVISP", "importFromVISP", label="VISP")
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
        publish_scheduler_progress(
            True,
            overrides_phase,
            overrides_label,
            overrides_step,
            progress_step_count,
        )
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
]

SHAPED_DEVICES_HEADER_WITH_SQM = [
    *SHAPED_DEVICES_HEADER,
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


def override_devices_to_rows(devices, include_sqm=False):
    """Convert override device dicts to CSV rows, preserving the existing CSV shape by default."""
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
        ]
        if include_sqm:
            row.append(str(sqm))
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


def header_has_sqm(header):
    return len(header) > 13 and header[13].strip().lower() == "sqm"


def operator_requires_sqm_column(devices, adjustments):
    if any(((d.get('sqm', '') or d.get('sqm_override', '') or '').strip()) for d in devices or []):
        return True
    return any(
        adj.get('type') == 'device_adjust_sqm' and (adj.get('sqm_override') or '').strip()
        for adj in adjustments or []
    )


def apply_lqos_overrides():
    """Load ShapedDevices.csv, apply persistent devices and circuit adjustments, and save back."""
    path = shaped_devices_csv_path()
    header, rows = read_shaped_devices_csv(path)

    # 1) Persistent devices: replace by device_id or append
    try:
        extra = overrides_persistent_devices_materialized()
    except Exception as e:
        # Persistent device overrides are optional. Keep the scheduler healthy
        # and continue applying the rest of the override sources if this loader
        # is unavailable or temporarily broken.
        print(f"Skipping persistent device overrides: {e}")
        extra = []

    # 2) Circuit adjustments: speed changes, removals, reparenting
    try:
        adjustments = overrides_circuit_adjustments_materialized()
    except Exception as e:
        print(f"Failed to read circuit adjustments: {e}")
        adjustments = []

    need_sqm_column = header_has_sqm(header) or operator_requires_sqm_column(extra, adjustments)
    if need_sqm_column and not header_has_sqm(header):
        header = list(header) + ["sqm"]
        for row in rows:
            if len(row) < len(header):
                row.extend([""] * (len(header) - len(row)))

    override_rows = override_devices_to_rows(extra or [], include_sqm=need_sqm_column)
    merged_rows, changed = merge_rows_replace_by_device_id(rows, override_rows)

    def set_if_some(value_opt, current_str):
        if value_opt is None:
            return current_str
        try:
            return str(float(value_opt))
        except Exception:
            return current_str

    def set_row_value(row, index, value):
        while len(row) <= index:
            row.append('')
        row[index] = value

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
            elif t == 'device_adjust_sqm':
                if not need_sqm_column:
                    continue
                did = adj.get('device_id', '')
                sqm_override = (adj.get('sqm_override') or '').strip()
                for r in merged_rows:
                    if len(r) >= 3 and r[2] == did:
                        current_sqm = r[13] if len(r) > 13 else ''
                        if current_sqm != sqm_override:
                            set_row_value(r, 13, sqm_override)
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
        final_header = header if header else (SHAPED_DEVICES_HEADER_WITH_SQM if need_sqm_column else SHAPED_DEVICES_HEADER)
        write_shaped_devices_csv(path, final_header, merged_rows)
        print("Updated ShapedDevices.csv with overrides")

    # 3) Load, adjust, and optionally save network.json
    nj_path = network_json_path()
    network = load_network_json(nj_path)
    try:
        adjustments = overrides_network_adjustments_materialized()
    except Exception as e:
        print(f"Failed to read network adjustments: {e}")
        adjustments = []
    net_changed = apply_network_adjustments(network, adjustments)
    if net_changed:
        write_network_json(nj_path, network)
        print("Updated network.json with overrides")
        canonical_path = topology_canonical_state_path()
        canonical_state = load_topology_canonical_state(canonical_path)
        if canonical_state and apply_network_adjustments_to_canonical_state(canonical_state, adjustments):
            write_topology_canonical_state(canonical_path, canonical_state)
            print("Updated topology_canonical_state.json with overrides")


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


def topology_canonical_state_path() -> str:
    return os.path.join(get_libreqos_directory(), "topology_canonical_state.json")


def load_topology_canonical_state(path: str):
    if not os.path.isfile(path):
        return None
    with open(path, 'r', encoding='utf-8') as f:
        try:
            return json.loads(f.read())
        except Exception:
            return None


def write_topology_canonical_state(path: str, canonical_state: dict):
    with open(path, 'w', encoding='utf-8') as f:
        f.write(json.dumps(canonical_state, indent=4))


def apply_network_adjustments(network: dict, adjustments=None) -> bool:
    """Apply network adjustments from overrides to the network JSON structure.

    Currently supports: adjust_site_speed (preferring node_id, with legacy
    site_name fallback) updating downloadBandwidthMbps and
    uploadBandwidthMbps at the matching node, and set_node_virtual (by
    node_name) updating the boolean 'virtual' flag for operator-authored
    topology overrides.

    This path intentionally excludes runtime automation changes such as
    StormGuard adaptive site-speed overrides and TreeGuard virtual-node
    decisions so they do not overwrite the operator-authored `network.json`
    source of truth.
    Returns True if any changes were applied.
    """
    if adjustments is None:
        try:
            adjustments = overrides_network_adjustments_materialized()
        except Exception as e:
            print(f"Failed to read network adjustments: {e}")
            return False

    if not adjustments:
        return False

    def normalize_bandwidth_value(value):
        numeric = float(value)
        if numeric.is_integer():
            return int(numeric)
        return numeric

    def adjust_node(tree: dict, site: str, node_id, dl_opt, ul_opt) -> bool:
        changed_local = False
        for key in list(tree.keys()):
            if key == 'children':
                child = tree.get('children')
                if isinstance(child, dict):
                    if adjust_node(child, site, node_id, dl_opt, ul_opt):
                        changed_local = True
                continue
            node = tree.get(key)
            if isinstance(node, dict):
                current_node_id = node.get('id')
                matches_target = False
                if node_id:
                    matches_target = current_node_id == node_id
                elif key == site:
                    matches_target = True

                if matches_target:
                    if dl_opt is not None:
                        node['downloadBandwidthMbps'] = normalize_bandwidth_value(dl_opt)
                        changed_local = True
                    if ul_opt is not None:
                        node['uploadBandwidthMbps'] = normalize_bandwidth_value(ul_opt)
                        changed_local = True
                # Recurse into children
                if 'children' in node and isinstance(node['children'], dict):
                    if adjust_node(node['children'], site, node_id, dl_opt, ul_opt):
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
            node_id = adj.get('node_id', None)
            dl = adj.get('download_bandwidth_mbps', None)
            ul = adj.get('upload_bandwidth_mbps', None)
            if site or node_id:
                if adjust_node(network, site, node_id, dl, ul):
                    net_changed = True
        elif adj.get('type') == 'set_node_virtual':
            node_name = adj.get('node_name', '')
            v = adj.get('virtual', None)
            if node_name and v is not None:
                if set_virtual_flag(network, node_name, v):
                    net_changed = True

    return net_changed


def apply_network_adjustments_to_canonical_state(canonical_state: dict, adjustments) -> bool:
    if not isinstance(canonical_state, dict):
        return False

    compatibility_network = canonical_state.get('compatibility_network_json')
    nodes = canonical_state.get('nodes')
    if not isinstance(compatibility_network, dict) or not isinstance(nodes, list):
        return False

    compatibility_changed = apply_network_adjustments(compatibility_network, adjustments)

    def normalize_bandwidth_value(value):
        numeric = float(value)
        if numeric.is_integer():
            return int(numeric)
        return numeric

    nodes_changed = False
    for adj in adjustments:
        adj_type = adj.get('type')
        if adj_type == 'adjust_site_speed':
            target_node_id = adj.get('node_id', None)
            target_name = adj.get('site_name', '')
            download = adj.get('download_bandwidth_mbps', None)
            upload = adj.get('upload_bandwidth_mbps', None)
            for node in nodes:
                if not isinstance(node, dict):
                    continue
                matches_target = False
                if target_node_id:
                    matches_target = node.get('node_id') == target_node_id
                elif target_name:
                    matches_target = node.get('node_name') == target_name
                if not matches_target:
                    continue
                rate_input = node.get('rate_input')
                if not isinstance(rate_input, dict):
                    rate_input = {}
                    node['rate_input'] = rate_input
                if download is not None:
                    normalized_download = normalize_bandwidth_value(download)
                    rate_input['intrinsic_download_mbps'] = normalized_download
                    rate_input['legacy_imported_download_mbps'] = normalized_download
                    nodes_changed = True
                if upload is not None:
                    normalized_upload = normalize_bandwidth_value(upload)
                    rate_input['intrinsic_upload_mbps'] = normalized_upload
                    rate_input['legacy_imported_upload_mbps'] = normalized_upload
                    nodes_changed = True
                if download is not None or upload is not None:
                    rate_input['source'] = 'compatibility_export'
        elif adj_type == 'set_node_virtual':
            target_name = adj.get('node_name', '')
            virtual_val = adj.get('virtual', None)
            if not target_name or virtual_val is None:
                continue
            normalized_virtual = bool(virtual_val)
            for node in nodes:
                if not isinstance(node, dict) or node.get('node_name') != target_name:
                    continue
                if bool(node.get('is_virtual', False)) != normalized_virtual:
                    node['is_virtual'] = normalized_virtual
                    nodes_changed = True

    return compatibility_changed or nodes_changed


def write_network_json(path: str, network: dict):
    with open(path, 'w', encoding='utf-8') as f:
        f.write(json.dumps(network, indent=4))


def importAndShapeFullReload():
    global shaping_runtime_hash
    importFromCRM()
    publish_scheduler_progress(True, "starting_topology_runtime", "Starting topology runtime", 4, SCHEDULER_STARTUP_STEP_COUNT)
    ensure_topology_runtime_process(wait_for_outputs=True)
    publish_scheduler_progress(True, "initial_shaping_reload", "Refreshing shaper state", 5, SCHEDULER_STARTUP_STEP_COUNT)
    if not enable_insight_topology():
        refreshShapers()
        shaping_runtime_hash = calculate_shaping_runtime_hash()
    else:
        shaping_runtime_hash = calculate_shaping_runtime_hash()


def importAndShapePartialReload():
    global shaping_runtime_hash

    importFromCRM(
        integration_phase="partial_integration",
        integration_label="Running scheduled integration refresh",
        integration_step=1,
        progress_step_count=SCHEDULER_REFRESH_STEP_COUNT,
        overrides_phase="partial_overrides",
        overrides_label="Applying scheduled overrides",
        overrides_step=2,
    )
    publish_scheduler_progress(True, "partial_topology_runtime", "Refreshing topology runtime", 3, SCHEDULER_REFRESH_STEP_COUNT)
    ensure_topology_runtime_process(wait_for_outputs=True)
    # Rebuild when runtime shaping inputs change, including effective adaptive
    # circuit overrides that do not belong in source-of-truth files.
    publish_scheduler_progress(True, "partial_runtime_hash", "Checking shaping inputs", 4, SCHEDULER_REFRESH_STEP_COUNT)
    new_hash = calculate_shaping_runtime_hash()
    if new_hash != shaping_runtime_hash:
        publish_scheduler_progress(True, "partial_reload", "Applying incremental shaper refresh", 4, SCHEDULER_REFRESH_STEP_COUNT)
        refreshShapers()
        shaping_runtime_hash = calculate_shaping_runtime_hash()
    publish_scheduler_progress(False, "ready", "Scheduler ready", SCHEDULER_REFRESH_STEP_COUNT, SCHEDULER_REFRESH_STEP_COUNT, percent=100)


def topology_runtime_binary_path():
    return os.path.join(get_libreqos_directory(), "bin", "lqos_topology")


def topology_runtime_output_paths():
    base_dir = get_libreqos_directory()
    return [
        os.path.join(base_dir, "topology_attachment_health_state.json"),
        os.path.join(base_dir, "topology_effective_state.json"),
        os.path.join(base_dir, "network.effective.json"),
    ]


def _load_json_field(path, field):
    try:
        with open(path, "r", encoding="utf-8") as handle:
            data = json.load(handle)
    except Exception:
        return None
    value = data.get(field)
    try:
        return int(value) if value is not None else None
    except Exception:
        return None


def _topology_runtime_freshness_target():
    base_dir = get_libreqos_directory()
    canonical_path = os.path.join(base_dir, "topology_canonical_state.json")
    canonical_generated = _load_json_field(canonical_path, "generated_unix")
    if canonical_generated is not None:
        return ("generated_unix", canonical_generated)

    editor_path = os.path.join(base_dir, "topology_editor_state.json")
    editor_generated = _load_json_field(editor_path, "generated_unix")
    if editor_generated is not None:
        return ("generated_unix", editor_generated)

    for path in (canonical_path, editor_path, os.path.join(base_dir, "network.json")):
        if os.path.isfile(path):
            return ("mtime", os.path.getmtime(path))
    return (None, None)


def _topology_runtime_outputs_are_fresh():
    _, effective_state_path, effective_network_path = topology_runtime_output_paths()
    if not (os.path.isfile(effective_state_path) and os.path.isfile(effective_network_path)):
        return False

    target_kind, target_value = _topology_runtime_freshness_target()
    if target_kind is None:
        return True

    if target_kind == "generated_unix":
        effective_canonical_generated = _load_json_field(effective_state_path, "canonical_generated_unix")
        if effective_canonical_generated is not None:
            return effective_canonical_generated >= target_value

    try:
        effective_state_mtime = os.path.getmtime(effective_state_path)
        effective_network_mtime = os.path.getmtime(effective_network_path)
    except OSError:
        return False
    return min(effective_state_mtime, effective_network_mtime) >= target_value


def clear_topology_runtime_outputs():
    for path in topology_runtime_output_paths():
        try:
            os.remove(path)
        except FileNotFoundError:
            continue
        except Exception as e:
            print(f"Failed to remove topology runtime artifact {path}: {e}")


def stop_topology_runtime_process():
    global topology_runtime_process
    process = topology_runtime_process
    topology_runtime_process = None
    if process is None:
        return
    if process.poll() is not None:
        return
    try:
        process.terminate()
        process.wait(timeout=5)
    except Exception:
        try:
            process.kill()
        except Exception:
            pass


def wait_for_topology_runtime_outputs(timeout_seconds=8.0):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        if _topology_runtime_outputs_are_fresh():
            return True
        process = topology_runtime_process
        if process is not None and process.poll() is not None:
            return False
        time.sleep(0.1)
    return False


def ensure_topology_runtime_process(wait_for_outputs=False):
    global topology_runtime_process
    global topology_runtime_missing_reported

    binary = topology_runtime_binary_path()
    if not os.path.isfile(binary):
        if not topology_runtime_missing_reported:
            print(f"Topology runtime helper is unavailable at {binary}. Rain suppression is disabled.")
            topology_runtime_missing_reported = True
        clear_topology_runtime_outputs()
        topology_runtime_process = None
        return False

    topology_runtime_missing_reported = False

    if topology_runtime_process is not None:
        code = topology_runtime_process.poll()
        if code is None:
            if wait_for_outputs:
                wait_for_topology_runtime_outputs()
            return True
        print(f"Topology runtime helper exited with code {code}. Restarting it.")
        clear_topology_runtime_outputs()
        topology_runtime_process = None

    try:
        topology_runtime_process = subprocess.Popen(
            [binary],
            cwd=get_libreqos_directory(),
        )
        print("Started topology runtime helper.")
        if wait_for_outputs:
            wait_for_topology_runtime_outputs()
        return True
    except Exception as e:
        print(f"Failed to start topology runtime helper: {e}")
        clear_topology_runtime_outputs()
        topology_runtime_process = None
        return False


def topology_runtime_refresh_tick():
    global shaping_runtime_hash

    ensure_topology_runtime_process()
    new_hash = calculate_shaping_runtime_hash()
    if new_hash == 0 or new_hash == shaping_runtime_hash:
        return

    refreshShapers()
    shaping_runtime_hash = calculate_shaping_runtime_hash()


def not_dead_yet():
    #print(f"Scheduler alive at {datetime.datetime.now()}")
    scheduler_alive()


def ensure_bus_ready():
    """Wait briefly for lqosd to finish binding the local bus socket."""
    wait_for_bus_ready(5000)

if __name__ == '__main__':
    try:
        atexit.register(stop_topology_runtime_process)
        publish_scheduler_progress(True, "waiting_for_bus", "Waiting for lqosd bus", 1, SCHEDULER_STARTUP_STEP_COUNT)
        ensure_bus_ready()
        importAndShapeFullReload()
        shaping_runtime_hash = calculate_shaping_runtime_hash()
        publish_scheduler_progress(False, "ready", "Scheduler ready", SCHEDULER_STARTUP_STEP_COUNT, SCHEDULER_STARTUP_STEP_COUNT, percent=100)

        print("Starting scheduler with jobs:")
        print(f"- not_dead_yet every 1 minute")
        refresh_interval = queue_refresh_interval_mins()
        print(f"- topology_runtime_refresh_tick every {TOPOLOGY_RUNTIME_REFRESH_SECONDS} seconds")
        print(f"- importAndShapePartialReload every {refresh_interval} minutes")
        
        not_dead_yet()
        ads.add_job(not_dead_yet, 'interval', minutes=1, max_instances=1)
        ads.add_job(topology_runtime_refresh_tick, 'interval', seconds=TOPOLOGY_RUNTIME_REFRESH_SECONDS, max_instances=1)
        ads.add_job(importAndShapePartialReload, 'interval', minutes=refresh_interval, max_instances=1)

        print("Scheduler starting...")
        ads.start()
    except Exception as e:
        print(f"Error starting scheduler: {e}")
        import traceback
        traceback.print_exc()
