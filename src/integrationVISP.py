from pythonCheck import checkPythonVersion
checkPythonVersion()

import json
import os
import time
import hashlib
from typing import Any, Dict, List, Optional

import requests
from collections import Counter

from liblqos_python import (
    get_libreqos_directory,
    overwrite_network_json_always,
    bandwidth_overhead_factor,
    client_bandwidth_multiplier,
    visp_client_id,
    visp_client_secret,
    visp_username,
    visp_password,
    visp_isp_id,
    visp_online_users_domain,
    visp_timeout_secs,
)

from integrationCommon import NetworkGraph, NetworkNode, NodeType, isIpv4Permitted


TOKEN_URL = "https://data.visp.net/token"
GRAPHQL_URL = "https://integrations.visp.net/graphql"


def _apply_rate(plan_rate_mbps: float) -> float:
    plan_rate_mbps = float(plan_rate_mbps or 0.0)
    if plan_rate_mbps <= 0:
        return 0.0
    overhead = bandwidth_overhead_factor()
    minimum = client_bandwidth_multiplier()
    adjusted = plan_rate_mbps * overhead
    floor_value = plan_rate_mbps * minimum
    return max(adjusted, floor_value)


def _unit_to_mbps(value: Any, unit: Any) -> Optional[float]:
    if value is None:
        return None
    try:
        v = float(value)
    except Exception:
        return None
    if v <= 0:
        return None
    u = (str(unit or "").strip().lower())
    if u in ("mbps", "mb", "m"):
        return v
    if u in ("kbps", "kb", "k"):
        return v / 1000.0
    if u in ("gbps", "gb", "g"):
        return v * 1000.0
    # Unknown unit: assume Mbps (VISP mostly uses mbps)
    return v


def _normalize_mac(mac: Any) -> str:
    if mac is None:
        return ""
    s = str(mac).strip()
    if not s:
        return ""
    # VISP sometimes returns MACs without separators. Keep uppercase hex.
    return s.replace(":", "").replace("-", "").upper()


def _safe_isp_id_from_token_payload(payload: Dict[str, Any]) -> Optional[int]:
    isp_ids = payload.get("ispId")
    if isinstance(isp_ids, list) and isp_ids:
        try:
            return int(isp_ids[0])
        except Exception:
            return None
    return None


class VispClient:
    def __init__(self) -> None:
        # Allow env overrides for quick testing without editing system config.
        self.client_id = (os.environ.get("VISP_CLIENT_ID", "") or visp_client_id()).strip()
        self.client_secret = (os.environ.get("VISP_CLIENT_SECRET", "") or visp_client_secret()).strip()
        self.username = (os.environ.get("VISP_USERNAME", "") or visp_username()).strip()
        self.password = (os.environ.get("VISP_PASSWORD", "") or visp_password()).strip()

        missing: List[str] = []
        if not self.client_id:
            missing.append("visp_client_id / VISP_CLIENT_ID")
        if not self.client_secret:
            missing.append("visp_client_secret / VISP_CLIENT_SECRET")
        if not self.username:
            missing.append("visp_username / VISP_USERNAME")
        if not self.password:
            missing.append("visp_password / VISP_PASSWORD")
        if missing:
            raise RuntimeError(
                "VISP integration is missing required credentials: "
                + ", ".join(missing)
                + ". Configure these in `lqos.conf` (or set `LQOS_CONFIG=/path/to/lqos.conf`) "
                + "or provide them via env vars for a one-off test."
            )
        self.config_isp_id = int(visp_isp_id() or 0)
        self.session = requests.Session()
        self.session.headers.update(
            {
                "x-visp-client-id": self.client_id,
                "x-visp-client-secret": self.client_secret,
            }
        )
        self._token: Optional[str] = None
        self._token_exp: int = 0
        self._isp_id: Optional[int] = None
        # Default to a fast-fail timeout. Operators can raise this in config if needed.
        self.timeout_secs: int = max(10, int(visp_timeout_secs() or 20))

    def _token_cache_path(self) -> str:
        base = get_libreqos_directory()
        h = hashlib.sha256((self.client_id + "|" + self.username).encode("utf-8")).hexdigest()[:16]
        return os.path.join(base, f".visp_token_cache_{h}.json")

    def _load_cached_token(self) -> None:
        path = self._token_cache_path()
        if not os.path.isfile(path):
            return
        try:
            with open(path, "r") as f:
                data = json.load(f)
            self._token = data.get("token") or None
            self._token_exp = int(data.get("exp") or 0)
            self._isp_id = int(data.get("isp_id") or 0) or None
        except Exception:
            # Cache is best-effort.
            self._token = None
            self._token_exp = 0
            self._isp_id = None

    def _store_cached_token(self) -> None:
        path = self._token_cache_path()
        try:
            with open(path, "w") as f:
                json.dump(
                    {"token": self._token, "exp": self._token_exp, "isp_id": self._isp_id or 0},
                    f,
                )
        except Exception:
            pass

    def _token_valid(self) -> bool:
        if not self._token or not self._token_exp:
            return False
        # Refresh token if less than 24h remaining.
        return int(time.time()) < int(self._token_exp) - 24 * 3600

    def ensure_token(self) -> None:
        if self._token_valid() and self._isp_id:
            return
        if self._token is None or self._token_exp == 0:
            self._load_cached_token()
            if self._token_valid() and self._isp_id:
                return

        headers = {
            "x-visp-client-id": self.client_id,
            "x-visp-client-secret": self.client_secret,
            "x-visp-username": self.username,
            "x-visp-password": self.password,
        }
        last_err = None
        for attempt in range(1, 4):
            try:
                r = self.session.get(TOKEN_URL, headers=headers, timeout=self.timeout_secs)
                if r.status_code in (502, 503, 504):
                    time.sleep(min(10, 1.5 ** attempt))
                    continue
                if r.status_code != 200:
                    # Include a small prefix of the body for debugging (should not contain secrets).
                    body_prefix = (r.text or "").replace("\n", "\\n")[:300]
                    raise RuntimeError(f"VISP token request failed: HTTP {r.status_code}: {body_prefix}")
                r.raise_for_status()
                data = r.json()
                token = data.get("token")
                payload = data.get("payload") or {}
                exp = int((payload.get("exp") or 0))
                isp_id = self.config_isp_id if self.config_isp_id > 0 else _safe_isp_id_from_token_payload(payload)
                if not token or not exp or not isp_id:
                    raise RuntimeError("VISP token response missing required fields")
                self._token = token
                self._token_exp = exp
                self._isp_id = int(isp_id)
                self._store_cached_token()
                return
            except Exception as e:
                last_err = e
                time.sleep(min(5, 1.5 ** attempt))
        raise RuntimeError(f"Unable to obtain VISP token: {last_err}")

    @property
    def isp_id(self) -> int:
        self.ensure_token()
        return int(self._isp_id or 0)

    def gql(self, query: str, variables: Dict[str, Any], op_name: str) -> Dict[str, Any]:
        self.ensure_token()
        headers = {
            "authorization": self._token or "",
        }
        body = {"operationName": op_name, "query": query, "variables": variables}
        last_err = None
        for attempt in range(1, 4):
            try:
                r = self.session.post(GRAPHQL_URL, headers=headers, json=body, timeout=self.timeout_secs)
                if r.status_code in (502, 503, 504):
                    time.sleep(min(10, 1.5 ** attempt))
                    continue
                # Unauthorized: refresh token and retry once.
                if r.status_code in (401, 403):
                    self._token = None
                    self._token_exp = 0
                    self.ensure_token()
                    headers["authorization"] = self._token or ""
                    time.sleep(1)
                    continue
                if r.status_code != 200:
                    body_prefix = (r.text or "").replace("\n", "\\n")[:300]
                    raise RuntimeError(f"VISP GraphQL HTTP {r.status_code}: {body_prefix}")
                r.raise_for_status()
                data = r.json()
                if isinstance(data, dict) and data.get("errors"):
                    raise RuntimeError(str(data.get("errors")[:1]))
                return data.get("data") or {}
            except Exception as e:
                last_err = e
                time.sleep(min(5, 1.5 ** attempt))
        raise RuntimeError(f"VISP GraphQL request failed: {last_err}")


def _wifi_bulk(visp: VispClient) -> List[Dict[str, Any]]:
    q = "query Bulk($isp_id:Int){ customerCPEwithWirelessServiceSpeeds(isp_id:$isp_id) }"
    data = visp.gql(q, {"isp_id": visp.isp_id}, "Bulk")
    res = data.get("customerCPEwithWirelessServiceSpeeds")
    return res if isinstance(res, list) else []

def _extract_ipv4_list(ip_val: Any) -> List[str]:
    out: List[str] = []
    if not ip_val:
        return out
    ip = str(ip_val).strip()
    if not ip:
        return out
    if isIpv4Permitted(ip):
        out.append(ip)
    return out


def _service_is_active(details: Dict[str, Any]) -> bool:
    # VISP can surface multiple status-like fields. Be conservative about skipping:
    # only skip if it is clearly terminated/cancelled/deleted. Many VISP tenants appear
    # to report "INACTIVE" for services that still have IP+speed and should be shaped.
    def norm(val: Any) -> str:
        return str(val or "").strip().upper()

    def status_bool(val: Any) -> Optional[bool]:
        s = norm(val)
        if not s:
            return None
        if s in ("ACTIVE", "CURRENT", "ENABLED", "ONLINE"):
            return True
        # Treat only hard-stop states as inactive. Everything else is "unknown" and
        # should not prevent shaping if IP+speed are present.
        if s in ("CANCELLED", "CANCELED", "TERMINATED", "DELETED"):
            return False
        return None

    pkg = status_bool(details.get("package_status"))
    if pkg is False:
        return False

    svc = status_bool(details.get("status"))
    if svc is not None:
        return svc

    if pkg is True:
        return True
    return True


def _add_circuit_and_device(
    net: NetworkGraph,
    circuit_id: str,
    customer_name: str,
    down_mbps: float,
    up_mbps: float,
    ipv4s: List[str],
    mac: str,
    device_id_suffix: str = "dev",
) -> bool:
    if not ipv4s:
        return False
    if down_mbps <= 0 or up_mbps <= 0:
        return False
    # Ensure circuit IDs are unique; adding duplicates breaks the graph indexes.
    if net.findNodeIndexById(circuit_id) != -1:
        return False
    net.addRawNode(
        NetworkNode(
            id=circuit_id,
            displayName=customer_name,
            type=NodeType.client,
            parentId="",
            address=customer_name,
            customerName=customer_name,
            download=_apply_rate(down_mbps),
            upload=_apply_rate(up_mbps),
        )
    )
    net.addRawNode(
        NetworkNode(
            id=f"{circuit_id}_{device_id_suffix}",
            displayName=f"{customer_name}_device",
            type=NodeType.device,
            parentId=circuit_id,
            mac=mac,
            ipv4=ipv4s,
            ipv6=[],
        )
    )
    return True


def createShaper() -> None:
    t0 = time.time()
    visp = VispClient()
    net = NetworkGraph()

    counters = {
        "wifi_bulk_services": 0,
        "shaped": 0,
        "skipped_inactive": 0,
        "skipped_no_ip": 0,
        "skipped_no_speed": 0,
    }

    disable_wifi_bulk = os.environ.get("VISP_DISABLE_WIFI_BULK", "").strip() in ("1", "true", "yes", "on")
    # IMPORTANT: VISP imports must be fast. By default, we only use the known bulk resolver.
    # Set VISP_ENABLE_CUSTOMER_PAGING=1 to opt into the slow, full-tenant scan mode.
    enable_customer_paging = os.environ.get("VISP_ENABLE_CUSTOMER_PAGING", "").strip() in ("1", "true", "yes", "on")

    # 1) WiFi bulk shaping (fast path with IP + speed + MAC)
    debug_status = os.environ.get("VISP_DEBUG_STATUS", "").strip() in ("1", "true", "yes", "on")
    pkg_status_counts: Counter = Counter()
    status_counts: Counter = Counter()

    if disable_wifi_bulk:
        print("[VISP] VISP_DISABLE_WIFI_BULK enabled; skipping wireless bulk import")
    else:
        bulk = _wifi_bulk(visp)
        for rec in bulk:
            if not isinstance(rec, dict):
                continue
            customer_name = str(rec.get("username") or "").strip()
            ws_list = rec.get("wireless_services") or []
            if not customer_name or not isinstance(ws_list, list):
                continue
            for ws in ws_list:
                if not isinstance(ws, dict):
                    continue
                counters["wifi_bulk_services"] += 1
                if debug_status:
                    pkg_status_counts[str(ws.get("package_status") or "").strip().upper()] += 1
                    status_counts[str(ws.get("status") or "").strip().upper()] += 1
                if not _service_is_active(ws):
                    counters["skipped_inactive"] += 1
                    continue
                service_number = ws.get("service_number")
                if not service_number:
                    continue
                circuit_id = str(service_number)
                down = _unit_to_mbps(ws.get("down_speed"), ws.get("down_speed_unit"))
                up = _unit_to_mbps(ws.get("up_speed"), ws.get("up_speed_unit"))
                if not down or not up:
                    counters["skipped_no_speed"] += 1
                    continue
                ipv4s = _extract_ipv4_list(ws.get("ip_address"))
                if not ipv4s:
                    counters["skipped_no_ip"] += 1
                    continue
                mac = _normalize_mac(ws.get("mac_address"))
                if _add_circuit_and_device(net, circuit_id, customer_name, down, up, ipv4s, mac, "wifi"):
                    counters["shaped"] += 1

    # 2) Optional slow mode: full tenant scan for additional service types
    # We intentionally keep this off by default to avoid long waits during scheduled imports.
    if enable_customer_paging:
        print("[VISP] VISP_ENABLE_CUSTOMER_PAGING enabled, but full-tenant scan is currently disabled in this build.")
        print("[VISP] If you need non-wireless bulk support, we should first confirm VISP provides bulk resolvers for them.")

    # 3) Optional onlineUsers enrichment (only supplements; doesn't create new circuits)
    domain = str(visp_online_users_domain() or "").strip()
    if domain:
        try:
            q = "query Online($domain:String!){ onlineUsers(domain:$domain){ username ip } }"
            data = visp.gql(q, {"domain": domain}, "Online")
            rows = data.get("onlineUsers") or []
            if isinstance(rows, list):
                ip_by_user = {}
                for r in rows:
                    if isinstance(r, dict):
                        u = str(r.get("username") or "").strip()
                        ip = str(r.get("ip") or "").strip()
                        if u and ip and isIpv4Permitted(ip):
                            ip_by_user[u] = ip
                # Best-effort: if device has no IPs, populate from username match
                for node in net.nodes:
                    if node.type == NodeType.device and (not node.ipv4 or len(node.ipv4) == 0):
                        ip = ip_by_user.get(node.displayName.replace("_device", ""))
                        if ip:
                            node.ipv4 = [ip]
        except Exception:
            pass

    net.prepareTree()

    if net.doesNetworkJsonExist() and not overwrite_network_json_always():
        print("[VISP] network.json exists and overwrite disabled; preserving current file")
    else:
        net.createNetworkJson()
    net.createShapedDevices()

    print(
        "[VISP] shaped={shaped} wifi_bulk_services={wifi_bulk_services} "
        "skipped_inactive={skipped_inactive} "
        "skipped_no_ip={skipped_no_ip} skipped_no_speed={skipped_no_speed} elapsed_s={elapsed_s}".format(
            **counters,
            elapsed_s=int(time.time() - t0),
        )
    )
    if debug_status:
        print("[VISP] Debug: top package_status values:")
        for k, v in pkg_status_counts.most_common(12):
            print(f"[VISP]   {k or '<empty>'}: {v}")
        print("[VISP] Debug: top status values:")
        for k, v in status_counts.most_common(12):
            print(f"[VISP]   {k or '<empty>'}: {v}")


def importFromVISP() -> None:
    createShaper()


if __name__ == "__main__":
    importFromVISP()
