import time
import datetime
import csv
import io
import json
import chardet
from LibreQoS import RefreshFailure, ValidationFailure, refreshShapers, refreshShapersUpdateOnly
import subprocess
import sys
import tempfile
from io import StringIO
from liblqos_python import (
    automatic_import_uisp,
    automatic_import_splynx,
    queue_refresh_interval_mins,
    automatic_import_powercode,
    automatic_import_sonar,
    influx_db_enabled,
    get_libreqos_directory,
    blackboard_finish,
    blackboard_submit,
    automatic_import_wispgate,
    enable_insight_topology,
    insight_topology_role,
    automatic_import_netzur,
    automatic_import_visp,
    calculate_shaping_runtime_hash,
    efficiency_core_ids,
    scheduler_alive as _scheduler_alive_native,
    scheduler_error as _scheduler_error_native,
    verify_published_shaping_generation,
    repair_runtime_state,
    write_compiled_topology_from_legacy_artifacts,
    scheduler_progress as _scheduler_progress_native,
    overrides_network_adjustments_materialized,
    overrides_materialized,
    scheduler_output as _scheduler_output_native,
    scheduler_integration_status as _scheduler_integration_status_native,
    wait_for_bus_ready,
)

from apscheduler.schedulers.background import BlockingScheduler
from apscheduler.executors.pool import ThreadPoolExecutor
import os.path
import os

try:
    from liblqos_python import get_libreqos_state_directory as _get_state_dir_native
except Exception:
    _get_state_dir_native = None

ads = BlockingScheduler(executors={'default': ThreadPoolExecutor(1)})
shaping_runtime_hash = 0
INTEGRATION_FAILURE_PREVIEW_LINES = 30
INTEGRATION_FAILURE_PREVIEW_CHARS = 4000
TOPOLOGY_RUNTIME_REFRESH_SECONDS = 3
TOPOLOGY_RUNTIME_REFRESH_PHASE_WARN_SECONDS = 1.0
STARTUP_TOPOLOGY_RUNTIME_MAX_WAIT_SECONDS = 120
PARTIAL_TOPOLOGY_RUNTIME_MAX_WAIT_SECONDS = 120
SCHEDULER_STARTUP_STEP_COUNT = 5
SCHEDULER_REFRESH_STEP_COUNT = 4
# `.63.1` (driver GO): TIME-WEIGHTED step boundaries, from the measured reload
# budget (docs/lqos_74t_63_progress_continuity_card.md §3): the integration
# subprocess is ~70 % of the ~14 s reload, so the step that runs it must occupy
# ~70 % of the bar. Each tuple weights the percent AT each step's completion
# (so `percent` during step k = sum(weights[:k-1])). These are ESTIMATES and
# never replace the truthful terminal publish.
SCHEDULER_STEP_WEIGHTS = {
    SCHEDULER_STARTUP_STEP_COUNT: (2, 70, 3, 15, 10),
    SCHEDULER_REFRESH_STEP_COUNT: (5, 70, 15, 10),
}
OVERRIDE_SECTION_READ_ATTEMPTS = 5
OVERRIDE_SECTION_READ_RETRY_SECONDS = 0.25
OVERRIDE_LOCK_ERROR_MARKERS = (
    "lqos_overrides_locked",
    "overrides files are locked by another process",
    "locked by another process",
)
scheduler_status_bus_enabled = True
startup_topology_runtime_pending = False
startup_topology_runtime_generation = None
startup_topology_runtime_started_monotonic = None
startup_topology_runtime_last_report_state = None
partial_topology_runtime_pending = False
partial_topology_runtime_generation = None
partial_topology_runtime_started_monotonic = None
partial_topology_runtime_last_report_state = None
integration_consecutive_failures = 0
last_integration_duration_ms = None
RECENT_INTEGRATION_RUNS_MAX = 5
recent_integration_runs = []

# On-demand reload request (lqosd writes a token; the scheduler consumes it).
RELOAD_REQUEST_TICK_SECONDS = 1
RELOAD_REQUEST_CATEGORY = "scheduler"
RELOAD_REQUEST_FILENAME = "reload_request.json"
scheduler_started_at = 0
last_reload_request_nonce = None


class RequiredOverrideReadError(RuntimeError):
    """Raised when scheduler reloads cannot safely materialize operator overrides."""


def mark_shaping_runtime_hash_applied(applied_hash):
    """
    Record the runtime payload that a completed shaper refresh targeted.

    If topology runtime publishes a newer payload while refreshShapers() is
    running, the next topology tick must still see that newer payload as
    unapplied and run another refresh.
    """
    global shaping_runtime_hash
    shaping_runtime_hash = applied_hash


def configure_scheduler_stdio():
    """
    Enable line-buffered scheduler logs when running under systemd.

    Side effects: reconfigures `sys.stdout` and `sys.stderr` when the active
    stream supports `reconfigure()`. This keeps periodic scheduler messages from
    sitting in Python's block buffer for hours before journald sees them.
    """
    for stream_name in ("stdout", "stderr"):
        stream = getattr(sys, stream_name, None)
        reconfigure = getattr(stream, "reconfigure", None)
        if not callable(reconfigure):
            continue
        try:
            reconfigure(line_buffering=True, write_through=True)
        except Exception as exc:
            print(
                f"Warning: unable to enable line-buffered scheduler {stream_name}: {exc}",
                file=sys.stderr,
            )


def get_state_directory():
    if _get_state_dir_native is not None:
        return _get_state_dir_native()
    base_dir = get_libreqos_directory()
    if os.path.basename(base_dir.rstrip("/")) == "src":
        parent = os.path.dirname(base_dir.rstrip("/"))
        if parent:
            return os.path.join(parent, "state")
    return os.path.join(base_dir, "state")


def get_state_path(category: str, filename: str) -> str:
    return os.path.join(get_state_directory(), category, filename)


def get_reload_request_path() -> str:
    return get_state_path(RELOAD_REQUEST_CATEGORY, RELOAD_REQUEST_FILENAME)


def _clear_reload_request(path: str):
    try:
        os.remove(path)
    except FileNotFoundError:
        pass
    except OSError as exc:
        scheduler_output(f"Unable to clear the reload request {path}: {exc}")


def reload_request_tick(request_path: str = None):
    """Consume an on-demand reload request written by lqosd.

    A token newer than ``scheduler_started_at`` with an unseen nonce triggers
    exactly one ``importAndShapeFullReload``, enqueued through apscheduler's
    single-worker pool so it serializes with the periodic jobs. The token is
    deleted once consumed; a duplicate nonce is a no-op; a token older than the
    scheduler's start is ignored and cleared. The DPDK bridge is never touched.
    """
    global last_reload_request_nonce
    path = request_path or get_reload_request_path()
    try:
        with open(path, "r", encoding="utf-8") as handle:
            token = json.load(handle)
    except FileNotFoundError:
        return
    except (OSError, ValueError) as exc:
        scheduler_output(f"Ignoring malformed reload request {path}: {exc}")
        _clear_reload_request(path)
        return

    if not isinstance(token, dict):
        scheduler_output(f"Ignoring malformed reload request {path}: not an object")
        _clear_reload_request(path)
        return

    try:
        requested_at = int(token.get("requested_at", 0))
    except (TypeError, ValueError):
        requested_at = 0
    nonce = str(token.get("nonce", "")).strip()

    if requested_at < scheduler_started_at:
        # The startup importAndShapeFullReload() already refreshed the source;
        # never run a surprise second reload after a restart.
        _clear_reload_request(path)
        return
    if not nonce or nonce == last_reload_request_nonce:
        # Duplicate/coalesced request: the nonce is the idempotency key.
        _clear_reload_request(path)
        return

    last_reload_request_nonce = nonce
    _clear_reload_request(path)
    requested_by = str(token.get("requested_by", "")).strip() or "unknown"
    print(f"Reload requested by {requested_by}; queuing a full integration refresh")
    publish_scheduler_progress(
        True,
        "reload_requested",
        "Reload requested",
        1,
        SCHEDULER_STARTUP_STEP_COUNT,
    )
    try:
        ads.add_job(_run_full_reload_and_publish, 'date', max_instances=1)
    except Exception as exc:
        scheduler_output(f"Unable to queue the requested reload: {exc}")


def get_runtime_state_path(category: str, filename: str) -> str:
    return get_state_path(category, filename)


def set_scheduler_status_bus_enabled(enabled: bool):
    global scheduler_status_bus_enabled
    scheduler_status_bus_enabled = bool(enabled)


def scheduler_alive():
    if not scheduler_status_bus_enabled:
        return False
    return _scheduler_alive_native()


def scheduler_error(message: str):
    if not scheduler_status_bus_enabled:
        return False
    return _scheduler_error_native(message)


def scheduler_output(message: str):
    if not scheduler_status_bus_enabled:
        return False
    return _scheduler_output_native(message)


def scheduler_progress(active: bool, phase: str, phase_label: str, step_index: int, step_count: int, percent: int):
    if not scheduler_status_bus_enabled:
        return False
    return _scheduler_progress_native(active, phase, phase_label, step_index, step_count, percent)


def clear_scheduler_error():
    """Clear the scheduler error status shown in the Web UI."""
    scheduler_error("")


def clear_scheduler_output():
    """Clear the scheduler output shown in the Web UI."""
    scheduler_output("")


def classify_integration_error(message: str) -> str:
    """Map an integration failure message to a cause class (emission-side).

    Classes: ``redirect_loop`` | ``auth`` | ``unreachable`` |
    ``exited_unknown`` | ``none``.
    """
    text = str(message or "").lower()
    if not text:
        return "none"
    if "too many redirects" in text or "redirect" in text:
        return "redirect_loop"
    if (
        "401" in text
        or "403" in text
        or "unauthorized" in text
        or "forbidden" in text
        or "authentication" in text
        or "invalid token" in text
    ):
        return "auth"
    if (
        "timed out" in text
        or "timeout" in text
        or "name or service not known" in text
        or "temporary failure in name resolution" in text
        or "connection refused" in text
        or "connection reset" in text
        or "unreachable" in text
    ):
        return "unreachable"
    return "exited_unknown"


def _integration_error_detail(message: str, *, max_chars: int = 400) -> str:
    """Return the actionable line from an integration failure message.

    Skips the generic ``"{label} exited with code N. Continuing."`` wrapper and
    the output-preview/artifact trailers, preferring a line that names the
    failure (a class marker) or an ``ERROR``/``Exception`` line.
    """
    lines = [line.strip() for line in _integration_output_lines(message) if line.strip()]

    def _is_wrapper(line: str) -> bool:
        return line.endswith("Continuing.") and "exited with code" in line

    def _is_trailer(line: str) -> bool:
        return line.startswith("Output preview:") or line.startswith("Full output saved to ")

    body = [line for line in lines if not _is_wrapper(line) and not _is_trailer(line)]
    candidates = body or lines
    markers = (
        "redirect",
        "error",
        "exception",
        "unauthorized",
        "forbidden",
        "timed out",
        "timeout",
        "refused",
        "unreachable",
        "failed",
    )
    for line in candidates:
        low = line.lower()
        if any(marker in low for marker in markers):
            return line[:max_chars]
    # An empty/whitespace-only message yields no candidates: return "" rather
    # than IndexError. (Pre-existing red, NOT caused by .63.1 — see the commit.)
    return candidates[0][:max_chars] if candidates else ""


def publish_integration_status(label, *, ok: bool, error_message: str | None = None):
    """Emit the integration health axis; never writes the scheduler-error axis."""
    global integration_consecutive_failures
    if ok:
        integration_consecutive_failures = 0
    else:
        integration_consecutive_failures = max(0, int(integration_consecutive_failures)) + 1
    error_class = "none"
    error_detail = None
    if not ok:
        error_class = classify_integration_error(error_message or "")
        error_detail = _integration_error_detail(error_message or "")
    run_at = int(time.time())
    recent_integration_runs.append((run_at, bool(ok), last_integration_duration_ms))
    del recent_integration_runs[:-RECENT_INTEGRATION_RUNS_MAX]
    if not scheduler_status_bus_enabled:
        return False
    try:
        return _scheduler_integration_status_native(
            integration_name=label,
            last_import_ok=bool(ok),
            last_import_at=run_at,
            last_import_ms=last_integration_duration_ms,
            consecutive_failures=integration_consecutive_failures,
            error_class=error_class,
            error_detail=error_detail,
            recent_runs=list(recent_integration_runs),
        )
    except Exception as e:
        print(f"Failed to publish integration status: {e}")
        return False


def clear_scheduler_error_for_transient_wait():
    """Clear a transient (non-runtime) scheduler error.

    Integration faults no longer flow through the scheduler-error axis, so
    there is no integration gate here.
    """
    clear_scheduler_error()


def publish_ready_progress(active: bool, phase: str, phase_label: str, step_index: int, step_count: int, *, percent=None):
    publish_scheduler_progress(
        active,
        phase,
        phase_label,
        step_index,
        step_count,
        percent=percent,
    )


def _weighted_progress_percent(step_count: int, completed_steps: int) -> int:
    """`.63.1` (1): time-weighted completion percent (uniform fallback)."""
    weights = SCHEDULER_STEP_WEIGHTS.get(int(step_count))
    if not weights:
        if step_count <= 0:
            return 0
        return max(0, min(100, int(round((completed_steps / step_count) * 100))))
    bounded = max(0, min(int(completed_steps), len(weights)))
    return max(0, min(100, int(sum(weights[:bounded]))))


def _scheduler_progress_percent(step_index: int, step_count: int, *, active: bool) -> int:
    if step_count <= 0:
        return 0
    bounded_step = max(1, min(int(step_index), int(step_count)))
    completed_steps = bounded_step - 1 if active else bounded_step
    return _weighted_progress_percent(step_count, completed_steps)


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


def report_scheduler_runtime_failure(context: str, exc: Exception, *, startup: bool = False):
    message = f"{context}: {exc}"
    print(message)
    scheduler_error(message)
    scheduler_output(message)
    if startup:
        publish_scheduler_progress(
            False,
            "degraded",
            "Scheduler running with topology/runtime error",
            SCHEDULER_STARTUP_STEP_COUNT,
            SCHEDULER_STARTUP_STEP_COUNT,
            percent=100,
        )


def report_scheduler_validation_failure(
    context: str,
    exc: Exception,
    *,
    startup: bool = False,
    step_count: int = SCHEDULER_REFRESH_STEP_COUNT,
):
    message = f"{context}: {exc}"
    print(message)
    scheduler_error(message)
    scheduler_output(message)
    publish_scheduler_progress(
        False,
        "validation_failed",
        "Scheduler validation failed",
        step_count,
        step_count,
        percent=100,
    )
    if startup:
        return


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
        publish_integration_status(label, ok=True)
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
    publish_integration_status(label, ok=False, error_message=message)


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

    global last_integration_duration_ms
    started = time.monotonic()
    try:
        result = subprocess.run(cmd, **kwargs)
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
        result = subprocess.run(cmd, **kwargs)
    last_integration_duration_ms = int((time.monotonic() - started) * 1000)
    return result



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
        friendly = label or (module_name + "." + func_name)
        err = f"Failed to invoke integration {friendly}: {e}"
        print(err)
        publish_integration_status(friendly, ok=False, error_message=err)

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
    # Phase D: the scheduler's idempotent repair-before-materialize (lqosd
    # startup is the primary owner; this maintains the invariants per tick).
    repair_runtime_state()
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
            publish_integration_status("UISP integration", ok=False, error_message=error_msg)
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
    else:
        # Phase C/D: no integration is enabled — the permanent manual pair is
        # the producer. Materialize the compiled ingest artifacts from it.
        write_compiled_topology_from_legacy_artifacts("legacy/manual", "full")
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
        message = f"Failed to apply lqos_overrides; preserving last-known-good topology: {e}"
        scheduler_error(message)
        scheduler_output(message)
        publish_scheduler_progress(
            False,
            "degraded",
            "Scheduler blocked by override failure",
            overrides_step,
            progress_step_count,
            percent=100,
        )
        print(message)
        raise


# --------------- Overrides Handling ---------------

SHAPED_DEVICES_HEADER = [
    "Circuit ID",
    "Circuit Name",
    "Device ID",
    "Device Name",
    "Parent Node",
    "Parent Node ID",
    "Anchor Node ID",
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












def apply_lqos_overrides():
    """Apply overrides to source topology files without mutating integration-owned shaping CSV."""
    materialized = read_required_override_section(
        "materialized overrides",
        overrides_materialized,
    )
    network_adjustments = materialized.get("network_adjustments", [])

    # Phase C S4: the override fold is runtime-owned (the compiled import +
    # E1 override layer). The ShapedDevices.csv mutation and the legacy
    # network.json mutation retired with the mode; only the canonical topology
    # state is adjusted here.
    adjustments = network_adjustments
    canonical_path = topology_canonical_state_path()
    canonical_state = load_topology_canonical_state(canonical_path)
    if canonical_state and apply_network_adjustments_to_canonical_state(canonical_state, adjustments):
        write_topology_canonical_state(canonical_path, canonical_state)
        print("Updated topology_canonical_state.json with overrides")


# --------------- Network JSON handling ---------------



def topology_canonical_state_path() -> str:
    return get_runtime_state_path("topology", "topology_canonical_state.json")


def load_topology_canonical_state(path: str):
    if not os.path.isfile(path):
        return None
    with open(path, 'r', encoding='utf-8') as f:
        try:
            return json.loads(f.read())
        except Exception:
            return None


def write_topology_canonical_state(path: str, canonical_state: dict):
    # Phase C S1b: the canonical RUNTIME file is written by the Rust atomic +
    # validating saver, never by a bare Python open() — the bypass was the
    # defect class. The save path is resolved from config on the Rust side
    # (same `<state>/topology/topology_canonical_state.json`); `path` is kept
    # for call-site compatibility.
    from liblqos_python import persist_topology_canonical_state
    persist_topology_canonical_state(json.dumps(canonical_state))


def _format_attempt_count(attempts: int) -> str:
    if attempts == 1:
        return "1 attempt"
    return f"{attempts} attempts"


def _is_override_lock_error(exc: Exception) -> bool:
    message = str(exc or "").lower()
    return any(marker in message for marker in OVERRIDE_LOCK_ERROR_MARKERS)


def read_required_override_section(section_name: str, reader):
    attempts = max(1, int(OVERRIDE_SECTION_READ_ATTEMPTS))
    last_error = None
    for attempt_index in range(attempts):
        try:
            return reader()
        except Exception as exc:
            last_error = exc
            if not _is_override_lock_error(exc):
                raise
            if attempt_index + 1 < attempts:
                time.sleep(OVERRIDE_SECTION_READ_RETRY_SECONDS)

    raise RequiredOverrideReadError(
        f"failed to read {section_name} after {_format_attempt_count(attempts)}: {last_error}"
    ) from last_error


def apply_network_adjustments(network: dict, adjustments=None) -> bool:
    """Apply network adjustments from overrides to the network JSON structure.

    Currently supports: adjust_site_speed (preferring node_id, with legacy
    site_name fallback) updating downloadBandwidthMbps and
    uploadBandwidthMbps at the matching node for operator-authored
    topology overrides.

    This path intentionally excludes runtime automation changes such as
    StormGuard adaptive site-speed overrides so they do not overwrite the
    operator-authored `network.json` source of truth.
    Returns True if any changes were applied.
    """
    if adjustments is None:
        adjustments = read_required_override_section(
            "network adjustments",
            overrides_network_adjustments_materialized,
        )

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
                    rate_input['source'] = 'operator_override'

    return compatibility_changed or nodes_changed



def importAndShapeFullReload():
    importFromCRM()
    publish_scheduler_progress(True, "starting_topology_runtime", "Starting topology runtime", 4, SCHEDULER_STARTUP_STEP_COUNT)
    if not ensure_topology_runtime_process(wait_for_outputs=True):
        ready, detail, generation = topology_runtime_readiness_detail()
        if not ready and _topology_runtime_not_ready_is_transient(detail):
            _begin_startup_topology_runtime_wait(generation)
            return False
        report_topology_runtime_not_ready(
            "Scheduler startup shaping refresh deferred",
            phase_label="Scheduler waiting for topology runtime",
            step_index=4,
            step_count=SCHEDULER_STARTUP_STEP_COUNT,
        )
        return False
    publish_scheduler_progress(True, "initial_shaping_reload", "Refreshing shaper state", 5, SCHEDULER_STARTUP_STEP_COUNT)
    refresh_target_hash = calculate_shaping_runtime_hash()
    if not enable_insight_topology():
        try:
            refreshShapers()
        except Exception as e:
            if _defer_stale_shaping_inputs_failure(e, startup=True):
                return False
            raise
    mark_shaping_runtime_hash_applied(refresh_target_hash)
    if shaping_runtime_hash != 0:
        set_scheduler_status_bus_enabled(True)
        return True
    # `.63.1` #12 (1): the reload exited without a terminal state; publish
    # degraded/100 so the bar cannot stick (the deferred path returns earlier
    # and lets the 3 s topology tick publish the eventual terminal).
    report_scheduler_runtime_failure(
        "Requested reload did not publish a shaping runtime hash",
        RuntimeError("Shaping runtime hash was not published after the reload."),
    )
    publish_scheduler_progress(
        False,
        "degraded",
        "Scheduler running with topology/runtime error",
        SCHEDULER_STARTUP_STEP_COUNT,
        SCHEDULER_STARTUP_STEP_COUNT,
        percent=100,
    )
    return False


def _run_full_reload_and_publish():
    """`.63.1` (#1, driver GO): run the requested full reload and publish EXACTLY
    ONE terminal `active=false` state on every exit.

    The button path (`reload_request_tick`) used to enqueue the bare
    `importAndShapeFullReload`, whose return was ignored, so the bar stayed at
    the last step until the next scheduled partial reload. This wrapper is the
    terminal publisher (ready/100 on a true return; validation_failed/100 or
    degraded/100 on the failure paths). It never double-publishes and never
    asserts a false ready: the *deferred* path returns False and is NOT terminal
    — the 3 s topology tick publishes the eventual ready/100 — while the
    non-transient topology failure and the `shaping_runtime_hash == 0` failure
    publish their degraded/100 inside `importAndShapeFullReload` itself.
    """
    try:
        completed = importAndShapeFullReload()
    except ValidationFailure as exc:
        report_scheduler_validation_failure(
            "Requested reload blocked by validation",
            exc,
            step_count=SCHEDULER_STARTUP_STEP_COUNT,
        )
        return
    except Exception as exc:
        report_scheduler_runtime_failure("Requested reload failed", exc)
        # `report_scheduler_runtime_failure` does NOT publish when
        # `startup=False`, so publish the terminal explicitly here.
        publish_scheduler_progress(
            False,
            "degraded",
            "Scheduler running with topology/runtime error",
            SCHEDULER_STARTUP_STEP_COUNT,
            SCHEDULER_STARTUP_STEP_COUNT,
            percent=100,
        )
        return
    if completed:
        publish_ready_progress(
            False,
            "ready",
            "Scheduler ready",
            SCHEDULER_STARTUP_STEP_COUNT,
            SCHEDULER_STARTUP_STEP_COUNT,
            percent=100,
        )


def importAndShapePartialReload():
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
    if not ensure_topology_runtime_process(wait_for_outputs=True):
        ready, detail, generation = topology_runtime_readiness_detail()
        if not ready and _topology_runtime_not_ready_is_transient(detail):
            _begin_partial_topology_runtime_wait(generation)
            return
        report_topology_runtime_not_ready(
            "Scheduled shaping refresh deferred",
            phase_label="Scheduler waiting for topology runtime",
            step_index=3,
            step_count=SCHEDULER_REFRESH_STEP_COUNT,
        )
        return
    # Rebuild when runtime shaping inputs change, including effective adaptive
    # circuit overrides that do not belong in source-of-truth files.
    publish_scheduler_progress(True, "partial_runtime_hash", "Checking shaping inputs", 4, SCHEDULER_REFRESH_STEP_COUNT)
    new_hash = calculate_shaping_runtime_hash()
    if new_hash != shaping_runtime_hash:
        publish_scheduler_progress(True, "partial_reload", "Applying incremental shaper refresh", 4, SCHEDULER_REFRESH_STEP_COUNT)
        try:
            refreshShapers()
        except ValidationFailure as e:
            report_scheduler_validation_failure("Scheduled shaping refresh blocked by validation", e)
            return
        except Exception as e:
            if _defer_stale_shaping_inputs_failure(e, startup=False):
                return
            report_scheduler_runtime_failure("Scheduled shaping refresh failed", e)
            return
        mark_shaping_runtime_hash_applied(new_hash)
        if shaping_runtime_hash != 0:
            set_scheduler_status_bus_enabled(True)
    publish_ready_progress(False, "ready", "Scheduler ready", SCHEDULER_REFRESH_STEP_COUNT, SCHEDULER_REFRESH_STEP_COUNT, percent=100)


def topology_runtime_status_path():
    return get_runtime_state_path("topology", "topology_runtime_status.json")


def _load_topology_runtime_status():
    path = topology_runtime_status_path()
    try:
        with open(path, "r", encoding="utf-8") as handle:
            data = json.load(handle)
    except FileNotFoundError:
        return None
    except Exception as e:
        print(f"Failed to read topology runtime status from {path}: {e}")
        return None
    return data if isinstance(data, dict) else None


def topology_runtime_readiness_detail():
    # Phase A §1: the custody door mints the current world and returns both the
    # verdict and the current source-generation token. The Python surface may
    # CALL verify, never re-derive the generation.
    try:
        outcome = verify_published_shaping_generation()
    except Exception as e:
        return (
            False,
            f"Topology runtime shaping inputs could not be verified: {e}",
            None,
        )
    if outcome is None:
        return (
            False,
            "Topology runtime source generation is unavailable for current inputs.",
            None,
        )
 # §0b: the custody door now also NAMES the failing leg
    # (3-tuple). Accept both shapes so older mocks/consumers keep working.
    outcome = tuple(outcome)
    verified = outcome[0] if len(outcome) > 0 else None
    current_generation = outcome[1] if len(outcome) > 1 else None
    failed_leg = outcome[2] if len(outcome) > 2 else None
    current_generation = str(current_generation or "").strip() or None
    if not current_generation:
        return (
            False,
            "Topology runtime source generation is unavailable for current inputs.",
            None,
        )

    status = _load_topology_runtime_status()
    if not isinstance(status, dict):
        return (
            False,
            "Topology runtime is still building outputs for the current source generation.",
            current_generation,
        )

    status_generation = str(status.get("source_generation") or "").strip()
    if status_generation != current_generation:
        return (
            False,
            "Topology runtime is still building outputs for the current source generation.",
            current_generation,
        )

    if not bool(status.get("ready")):
        error = status.get("error")
        if isinstance(error, str) and error.strip():
            return (
                False,
                f"Topology runtime failed for the current source generation: {error.strip()}",
                current_generation,
            )
        return (
            False,
            "Topology runtime is still building outputs for the current source generation.",
            current_generation,
        )

    shaping_generation = str(status.get("shaping_generation") or "").strip()
    if not shaping_generation:
        return (
            False,
            "Topology runtime has not published shaping inputs for the current source generation.",
            current_generation,
        )

    shaping_inputs_path = str(status.get("shaping_inputs_path") or "").strip()
    if not shaping_inputs_path:
        return (
            False,
            "Topology runtime did not publish a shaping inputs path for the current source generation.",
            current_generation,
        )
    if not os.path.isfile(shaping_inputs_path):
        return (
            False,
            f"Topology runtime shaping inputs are not available at {shaping_inputs_path}.",
            current_generation,
        )

    effective_generation = str(status.get("effective_generation") or "").strip()
    # Phase A §2 rename: dual-read new-then-old for one release.
    effective_network_path = str(
        status.get("operator_view_path") or status.get("effective_network_path") or ""
    ).strip()
    if not effective_network_path:
        return (
            False,
            "Topology runtime did not publish an effective network path for the current source generation.",
            current_generation,
        )
    if not os.path.isfile(effective_network_path):
        return (
            False,
            f"Topology runtime effective network is not available at {effective_network_path}.",
            current_generation,
        )
    if not effective_generation:
        return (
            False,
            "Topology runtime did not publish an effective network generation for the current source generation.",
            current_generation,
        )

    # The operator-view match-assert now lives INSIDE verify as a custody leg
    # (Phase A §1 ruling 2) — the scheduler no longer re-derives the hash.
    if verified is None:
        return (
            False,
            "Topology runtime is still building outputs for the current source generation.",
            current_generation,
        )
    if not verified:
        detail = (
            "Topology runtime shaping inputs do not match the published runtime generation."
        )
        if failed_leg:
            detail += f" Custody leg: {failed_leg}."
        return (False, detail, current_generation)

    return (True, "", current_generation)


def report_topology_runtime_not_ready(
    context: str,
    *,
    phase_label: str,
    step_index: int,
    step_count: int,
):
    ready, detail, generation = topology_runtime_readiness_detail()
    if ready:
        return
    message = f"{context}: {detail}"
    if generation:
        message += f" Generation {generation[:12]}."
    print(message)
    if _topology_runtime_not_ready_is_transient(detail):
        clear_scheduler_error_for_transient_wait()
        scheduler_output(message)
        publish_scheduler_progress(
            False,
            "waiting_for_topology_runtime",
            phase_label,
            step_index,
            step_count,
        )
        return
    scheduler_error(message)
    scheduler_output(message)
    publish_scheduler_progress(
        False,
        "degraded",
        phase_label,
        step_count,
        step_count,
        percent=100,
    )


def _reset_startup_topology_runtime_wait():
    global startup_topology_runtime_pending
    global startup_topology_runtime_generation
    global startup_topology_runtime_started_monotonic
    global startup_topology_runtime_last_report_state

    startup_topology_runtime_pending = False
    startup_topology_runtime_generation = None
    startup_topology_runtime_started_monotonic = None
    startup_topology_runtime_last_report_state = None


def _reset_partial_topology_runtime_wait():
    global partial_topology_runtime_pending
    global partial_topology_runtime_generation
    global partial_topology_runtime_started_monotonic
    global partial_topology_runtime_last_report_state

    partial_topology_runtime_pending = False
    partial_topology_runtime_generation = None
    partial_topology_runtime_started_monotonic = None
    partial_topology_runtime_last_report_state = None


def _topology_runtime_not_ready_is_transient(detail: str) -> bool:
    detail_text = str(detail or "").strip()
    if detail_text.startswith("Topology runtime failed for the current source generation:"):
        return False
    return True


def _is_transient_stale_shaping_inputs_failure(exc: Exception) -> bool:
    if not isinstance(exc, RefreshFailure):
        return False
    detail = str(exc or "").strip()
    return detail.startswith("Missing or stale shaping_inputs.json.")


def _defer_stale_shaping_inputs_failure(exc: Exception, *, startup: bool) -> bool:
    if not _is_transient_stale_shaping_inputs_failure(exc):
        return False

    ready, detail, generation = topology_runtime_readiness_detail()
    if detail and not _topology_runtime_not_ready_is_transient(detail):
        return False

    clear_scheduler_error_for_transient_wait()
    if startup:
        _begin_startup_topology_runtime_wait(generation)
    else:
        _begin_partial_topology_runtime_wait(generation)
    return True


def _publish_startup_topology_runtime_wait(generation: str | None):
    global startup_topology_runtime_last_report_state

    generation_token = str(generation or "").strip() or "unknown"
    state = ("waiting", generation_token)
    if startup_topology_runtime_last_report_state == state:
        return
    startup_topology_runtime_last_report_state = state
    if generation_token != "unknown":
        print(
            "Scheduler startup waiting for topology runtime outputs for "
            f"generation {generation_token[:12]}."
        )
    else:
        print("Scheduler startup waiting for topology runtime outputs.")
    publish_scheduler_progress(
        True,
        "waiting_for_topology_runtime",
        "Waiting for topology runtime",
        4,
        SCHEDULER_STARTUP_STEP_COUNT,
        percent=80,
    )


def _begin_startup_topology_runtime_wait(generation: str | None):
    global startup_topology_runtime_pending
    global startup_topology_runtime_generation
    global startup_topology_runtime_started_monotonic
    global startup_topology_runtime_last_report_state

    generation_token = str(generation or "").strip() or None
    now = time.monotonic()
    if (
        not startup_topology_runtime_pending
        or startup_topology_runtime_generation != generation_token
    ):
        startup_topology_runtime_pending = True
        startup_topology_runtime_generation = generation_token
        startup_topology_runtime_started_monotonic = now
        startup_topology_runtime_last_report_state = None
    _publish_startup_topology_runtime_wait(generation_token)


def _publish_partial_topology_runtime_wait(generation: str | None):
    global partial_topology_runtime_last_report_state

    generation_token = str(generation or "").strip() or "unknown"
    state = ("waiting", generation_token)
    if partial_topology_runtime_last_report_state == state:
        return
    partial_topology_runtime_last_report_state = state
    if generation_token != "unknown":
        print(
            "Scheduled shaping refresh waiting for topology runtime outputs for "
            f"generation {generation_token[:12]}."
        )
    else:
        print("Scheduled shaping refresh waiting for topology runtime outputs.")
    publish_scheduler_progress(
        True,
        "waiting_for_topology_runtime",
        "Waiting for topology runtime",
        3,
        SCHEDULER_REFRESH_STEP_COUNT,
        percent=75,
    )


def _begin_partial_topology_runtime_wait(generation: str | None):
    global partial_topology_runtime_pending
    global partial_topology_runtime_generation
    global partial_topology_runtime_started_monotonic
    global partial_topology_runtime_last_report_state

    generation_token = str(generation or "").strip() or None
    now = time.monotonic()
    if (
        not partial_topology_runtime_pending
        or partial_topology_runtime_generation != generation_token
    ):
        partial_topology_runtime_pending = True
        partial_topology_runtime_generation = generation_token
        partial_topology_runtime_started_monotonic = now
        partial_topology_runtime_last_report_state = None
    _publish_partial_topology_runtime_wait(generation_token)


def _continue_startup_topology_runtime_wait():
    global shaping_runtime_hash
    global startup_topology_runtime_generation
    global startup_topology_runtime_started_monotonic
    global startup_topology_runtime_last_report_state

    ready, detail, generation = topology_runtime_readiness_detail()
    generation_token = str(generation or "").strip() or None
    now = time.monotonic()

    if generation_token != startup_topology_runtime_generation:
        startup_topology_runtime_generation = generation_token
        startup_topology_runtime_started_monotonic = now
        startup_topology_runtime_last_report_state = None

    if ready:
        publish_scheduler_progress(
            True,
            "initial_shaping_reload",
            "Refreshing shaper state",
            5,
            SCHEDULER_STARTUP_STEP_COUNT,
        )
        try:
            refresh_target_hash = calculate_shaping_runtime_hash()
            if not enable_insight_topology():
                refreshShapers()
            mark_shaping_runtime_hash_applied(refresh_target_hash)
        except ValidationFailure as e:
            _reset_startup_topology_runtime_wait()
            report_scheduler_validation_failure(
                "Scheduler startup shaping refresh blocked by validation",
                e,
                startup=True,
                step_count=SCHEDULER_STARTUP_STEP_COUNT,
            )
            shaping_runtime_hash = 0
            return
        except Exception as e:
            if _defer_stale_shaping_inputs_failure(e, startup=True):
                return
            _reset_startup_topology_runtime_wait()
            report_scheduler_runtime_failure(
                "Scheduler startup shaping refresh failed",
                e,
                startup=True,
            )
            shaping_runtime_hash = 0
            return

        _reset_startup_topology_runtime_wait()
        if shaping_runtime_hash != 0:
            clear_scheduler_error_for_transient_wait()
            publish_ready_progress(
                False,
                "ready",
                "Scheduler ready",
                SCHEDULER_STARTUP_STEP_COUNT,
                SCHEDULER_STARTUP_STEP_COUNT,
                percent=100,
            )
            return

        report_scheduler_runtime_failure(
            "Scheduler startup shaping refresh failed",
            RuntimeError("Shaping runtime hash was not published after startup refresh."),
            startup=True,
        )
        return

    started = startup_topology_runtime_started_monotonic
    if started is None:
        startup_topology_runtime_started_monotonic = now
        started = now
    elapsed = max(0.0, now - started)

    if (
        _topology_runtime_not_ready_is_transient(detail)
        and elapsed < STARTUP_TOPOLOGY_RUNTIME_MAX_WAIT_SECONDS
    ):
        _publish_startup_topology_runtime_wait(generation_token)
        return

    state = ("degraded", str(detail or "").strip(), generation_token or "")
    if startup_topology_runtime_last_report_state == state:
        return
    startup_topology_runtime_last_report_state = state
    report_topology_runtime_not_ready(
        "Scheduler startup shaping refresh deferred",
        phase_label="Scheduler waiting for topology runtime",
        step_index=4,
        step_count=SCHEDULER_STARTUP_STEP_COUNT,
    )


def _continue_partial_topology_runtime_wait():
    global partial_topology_runtime_generation
    global partial_topology_runtime_started_monotonic
    global partial_topology_runtime_last_report_state

    ready, detail, generation = topology_runtime_readiness_detail()
    generation_token = str(generation or "").strip() or None
    now = time.monotonic()

    if generation_token != partial_topology_runtime_generation:
        partial_topology_runtime_generation = generation_token
        partial_topology_runtime_started_monotonic = now
        partial_topology_runtime_last_report_state = None

    if ready:
        publish_scheduler_progress(
            True,
            "partial_runtime_hash",
            "Checking shaping inputs",
            4,
            SCHEDULER_REFRESH_STEP_COUNT,
        )
        new_hash = calculate_shaping_runtime_hash()
        if new_hash == 0:
            _reset_partial_topology_runtime_wait()
            report_scheduler_runtime_failure(
                "Scheduled shaping refresh failed",
                RuntimeError(
                    "Shaping runtime hash was not published after topology runtime became ready."
                ),
            )
            return
        if new_hash != shaping_runtime_hash:
            publish_scheduler_progress(
                True,
                "partial_reload",
                "Applying incremental shaper refresh",
                4,
                SCHEDULER_REFRESH_STEP_COUNT,
            )
            try:
                refreshShapers()
            except ValidationFailure as e:
                _reset_partial_topology_runtime_wait()
                report_scheduler_validation_failure(
                    "Scheduled shaping refresh blocked by validation",
                    e,
                )
                return
            except Exception as e:
                if _defer_stale_shaping_inputs_failure(e, startup=False):
                    return
                _reset_partial_topology_runtime_wait()
                report_scheduler_runtime_failure("Scheduled shaping refresh failed", e)
                return
            mark_shaping_runtime_hash_applied(new_hash)
            if shaping_runtime_hash == 0:
                _reset_partial_topology_runtime_wait()
                report_scheduler_runtime_failure(
                    "Scheduled shaping refresh failed",
                    RuntimeError(
                        "Shaping runtime hash was not published after scheduled refresh."
                    ),
                )
                return
        _reset_partial_topology_runtime_wait()
        clear_scheduler_error_for_transient_wait()
        publish_ready_progress(
            False,
            "ready",
            "Scheduler ready",
            SCHEDULER_REFRESH_STEP_COUNT,
            SCHEDULER_REFRESH_STEP_COUNT,
            percent=100,
        )
        return

    started = partial_topology_runtime_started_monotonic
    if started is None:
        partial_topology_runtime_started_monotonic = now
        started = now
    elapsed = max(0.0, now - started)

    if (
        _topology_runtime_not_ready_is_transient(detail)
        and elapsed < PARTIAL_TOPOLOGY_RUNTIME_MAX_WAIT_SECONDS
    ):
        _publish_partial_topology_runtime_wait(generation_token)
        return

    state = ("degraded", str(detail or "").strip(), generation_token or "")
    if partial_topology_runtime_last_report_state == state:
        return
    partial_topology_runtime_last_report_state = state
    report_topology_runtime_not_ready(
        "Scheduled shaping refresh deferred",
        phase_label="Scheduler waiting for topology runtime",
        step_index=3,
        step_count=SCHEDULER_REFRESH_STEP_COUNT,
    )


def wait_for_topology_runtime_ready(timeout_seconds=8.0):
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        ready, _, _ = topology_runtime_readiness_detail()
        if ready:
            return True
        time.sleep(0.1)
    return False


def ensure_topology_runtime_process(wait_for_outputs=False):
    if wait_for_outputs:
        return wait_for_topology_runtime_ready()
    return True


def _record_slow_topology_runtime_refresh_phase(phase: str, started: float, slow_phases: list[str]):
    elapsed = time.monotonic() - started
    if elapsed >= TOPOLOGY_RUNTIME_REFRESH_PHASE_WARN_SECONDS:
        slow_phases.append(f"{phase} {elapsed:.1f}s")
    return elapsed


def topology_runtime_refresh_tick():
    if startup_topology_runtime_pending:
        _continue_startup_topology_runtime_wait()
        return

    if partial_topology_runtime_pending:
        _continue_partial_topology_runtime_wait()
        return

    if shaping_runtime_hash == 0:
        return

    tick_started = time.monotonic()
    slow_phases = []
    phase_started = time.monotonic()
    ensure_topology_runtime_process()
    _record_slow_topology_runtime_refresh_phase("ensure_runtime_process", phase_started, slow_phases)

    phase_started = time.monotonic()
    ready, _, _ = topology_runtime_readiness_detail()
    _record_slow_topology_runtime_refresh_phase("readiness_detail", phase_started, slow_phases)
    if not ready:
        return

    phase_started = time.monotonic()
    new_hash = calculate_shaping_runtime_hash()
    _record_slow_topology_runtime_refresh_phase("calculate_shaping_runtime_hash", phase_started, slow_phases)
    if new_hash == 0 or new_hash == shaping_runtime_hash:
        return

    refresh_started = time.monotonic()
    try:
        refreshShapers()
    except ValidationFailure as e:
        elapsed = time.monotonic() - refresh_started
        phase_summary = f"; slow phases: {', '.join(slow_phases)}" if slow_phases else ""
        print(
            f"Topology runtime refresh for shaping runtime hash {new_hash} "
            f"(previous {shaping_runtime_hash}) blocked by validation after {elapsed:.1f}s{phase_summary}"
        )
        report_scheduler_validation_failure("Topology runtime refresh blocked by validation", e)
        return
    except Exception as e:
        elapsed = time.monotonic() - refresh_started
        phase_summary = f"; slow phases: {', '.join(slow_phases)}" if slow_phases else ""
        print(
            f"Topology runtime refresh for shaping runtime hash {new_hash} "
            f"(previous {shaping_runtime_hash}) failed after {elapsed:.1f}s{phase_summary}"
        )
        if _defer_stale_shaping_inputs_failure(e, startup=False):
            return
        report_scheduler_runtime_failure("Topology runtime refresh failed", e)
        return
    refresh_elapsed = _record_slow_topology_runtime_refresh_phase("refreshShapers", refresh_started, slow_phases)
    mark_shaping_runtime_hash_applied(new_hash)
    if shaping_runtime_hash == 0:
        report_scheduler_runtime_failure(
            "Topology runtime refresh failed",
            RuntimeError("Shaping runtime hash was not published after topology runtime refresh."),
        )
        return
    clear_scheduler_error_for_transient_wait()
    total_elapsed = time.monotonic() - tick_started
    if slow_phases:
        print(
            f"Topology runtime refresh applied shaping runtime hash {new_hash} "
            f"in {total_elapsed:.1f}s (refreshShapers {refresh_elapsed:.1f}s); "
            f"slow phases: {', '.join(slow_phases)}"
        )
    publish_ready_progress(False, "ready", "Scheduler ready", SCHEDULER_REFRESH_STEP_COUNT, SCHEDULER_REFRESH_STEP_COUNT, percent=100)


def not_dead_yet():
    #print(f"Scheduler alive at {datetime.datetime.now()}")
    scheduler_alive()


def ensure_bus_ready():
    """Wait briefly for lqosd to finish binding the local bus socket."""
    wait_for_bus_ready(5000)

def run_scheduler_main():
    global shaping_runtime_hash, scheduler_started_at

    scheduler_started_at = int(time.time())
    set_scheduler_status_bus_enabled(False)
    ensure_bus_ready()
    set_scheduler_status_bus_enabled(True)
    scheduler_alive()
    publish_scheduler_progress(True, "waiting_for_bus", "Waiting for lqosd bus", 1, SCHEDULER_STARTUP_STEP_COUNT)
    try:
        startup_ready = importAndShapeFullReload()
        if startup_ready:
            publish_ready_progress(False, "ready", "Scheduler ready", SCHEDULER_STARTUP_STEP_COUNT, SCHEDULER_STARTUP_STEP_COUNT, percent=100)
    except ValidationFailure as e:
        report_scheduler_validation_failure(
            "Scheduler startup shaping refresh blocked by validation",
            e,
            startup=True,
            step_count=SCHEDULER_STARTUP_STEP_COUNT,
        )
    except Exception as e:
        report_scheduler_runtime_failure("Scheduler startup shaping refresh failed", e, startup=True)
        import traceback
        traceback.print_exc()
        shaping_runtime_hash = 0

    print("Starting scheduler with jobs:")
    print(f"- not_dead_yet every 1 minute")
    refresh_interval = queue_refresh_interval_mins()
    print(f"- topology_runtime_refresh_tick every {TOPOLOGY_RUNTIME_REFRESH_SECONDS} seconds")
    print(f"- importAndShapePartialReload every {refresh_interval} minutes")
    print(f"- reload_request_tick every {RELOAD_REQUEST_TICK_SECONDS} second")

    not_dead_yet()
    ads.add_job(not_dead_yet, 'interval', minutes=1, max_instances=1)
    ads.add_job(topology_runtime_refresh_tick, 'interval', seconds=TOPOLOGY_RUNTIME_REFRESH_SECONDS, max_instances=1)
    ads.add_job(importAndShapePartialReload, 'interval', minutes=refresh_interval, max_instances=1)
    ads.add_job(reload_request_tick, 'interval', seconds=RELOAD_REQUEST_TICK_SECONDS, max_instances=1)

    print("Scheduler starting...")
    ads.start()


if __name__ == '__main__':
    try:
        configure_scheduler_stdio()
        run_scheduler_main()
    except Exception as e:
        print(f"Error starting scheduler: {e}")
        import traceback
        traceback.print_exc()
