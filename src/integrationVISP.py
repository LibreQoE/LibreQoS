from pythonCheck import checkPythonVersion
checkPythonVersion()

import json
import os
import time
import hashlib
import ipaddress
from typing import Any, Dict, List, Optional, Set, Tuple

import requests
from collections import Counter

from liblqos_python import (
    get_libreqos_directory,
    visp_client_id,
    visp_client_secret,
    visp_username,
    visp_password,
    visp_isp_id,
    visp_online_users_domain,
    visp_timeout_secs,
)

try:
    from liblqos_python import get_libreqos_state_directory as _get_state_dir_native
except Exception:
    _get_state_dir_native = None

from integrationCommon import (
    NetworkGraph,
    NetworkNode,
    NodeType,
    apply_client_bandwidth_multiplier,
    isIntegrationOutputIpAllowed,
)


TOKEN_URL = "https://data.visp.net/token"
GRAPHQL_URL = "https://integrations.visp.net/graphql"


def _state_directory() -> str:
    if _get_state_dir_native is not None:
        return _get_state_dir_native()
    base_dir = get_libreqos_directory()
    if os.path.basename(base_dir.rstrip("/")) == "src":
        parent = os.path.dirname(base_dir.rstrip("/"))
        if parent:
            return os.path.join(parent, "state")
    return os.path.join(base_dir, "state")


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
        base = os.path.join(_state_directory(), "cache")
        os.makedirs(base, exist_ok=True)
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


def _subscribers(visp: VispClient) -> List[Dict[str, Any]]:
    q = """
    query Subs($isp_id:Int!, $limit:Int!, $offset:Int!){
      rows: subscribers(isp_id:$isp_id, limit:$limit, offset:$offset){
        customer_id
        username
        first_name
        last_name
        status
        package
        package_ip
        equipment_ip
        equipment_status
        tower
        isp_site
        access_point
        subscriber_type
      }
      count: subscribersCount(isp_id:$isp_id)
    }
    """
    rows: List[Dict[str, Any]] = []
    limit = 500
    offset = 0
    total = None
    while True:
        data = visp.gql(q, {"isp_id": visp.isp_id, "limit": limit, "offset": offset}, "Subs")
        batch = data.get("rows") or []
        if not isinstance(batch, list) or not batch:
            break
        rows.extend([row for row in batch if isinstance(row, dict)])
        if total is None:
            try:
                total = int(data.get("count") or 0)
            except Exception:
                total = 0
        offset += len(batch)
        if len(batch) < limit:
            break
        if total and offset >= total:
            break
    return rows


def _customer_services(visp: VispClient, customer_id: int) -> List[Dict[str, Any]]:
    q = """
    query CustomerServices($customer_id:Int!){
      rows: customerServices(customer_id:$customer_id, active:true){
        id
        service_id
        service_name
        service_type
        service_label
        status
        package_number
        bill_separately
        service_details {
          __typename
          ... on ServiceTypeWifi {
            service_number
            service_name
            network_id
            network_name
            down_speed
            down_speed_unit
            up_speed
            up_speed_unit
            ip_address
            mac_address
            username
          }
          ... on ServiceTypeVoip {
            service_number
          }
        }
      }
    }
    """
    data = visp.gql(q, {"customer_id": int(customer_id)}, "CustomerServices")
    rows = data.get("rows") or []
    return rows if isinstance(rows, list) else []


def _customer_ip_list(visp: VispClient, customer_id: int) -> Dict[str, Any]:
    q = """
    query CustomerIpList($customer_id:Int!){
      row: customerIPList(customer_id:$customer_id){
        package_ip
        equipment_ip
      }
    }
    """
    data = visp.gql(q, {"customer_id": int(customer_id)}, "CustomerIpList")
    row = data.get("row") or {}
    return row if isinstance(row, dict) else {}


def _equipment_assemblies_cpe(visp: VispClient) -> List[Dict[str, Any]]:
    q = """
    query EquipmentAssemblies($isp_id:Int!){
      rows: equipmentAssemblies(isp_id:$isp_id, ap_upstream:true, type:"CPE"){
        id
        parent_id
        description
        location_name
        item_id
        parent_site_id
        equipment_data
        parent_data
      }
    }
    """
    data = visp.gql(q, {"isp_id": visp.isp_id}, "EquipmentAssemblies")
    rows = data.get("rows") or []
    return rows if isinstance(rows, list) else []


def _equipment_pg_bulk(visp: VispClient) -> List[Dict[str, Any]]:
    q = """
    query EquipmentPg($isp_id:Int!, $limit:Int!, $offset:Int!){
      rows: equipmentPg(isp_id:$isp_id, limit:$limit, offset:$offset){
        id
        parent_id
        name
        description
        type
        default_type
        location_id
        location_name
        parent_site_id
        item_id
        child_count
        equipment_data
        parent_data
      }
    }
    """
    rows: List[Dict[str, Any]] = []
    limit = 500
    offset = 0
    while True:
        data = visp.gql(q, {"isp_id": visp.isp_id, "limit": limit, "offset": offset}, "EquipmentPg")
        batch = data.get("rows") or []
        if not isinstance(batch, list) or not batch:
            break
        rows.extend([row for row in batch if isinstance(row, dict)])
        offset += len(batch)
        if len(batch) < limit:
            break
    return rows

def _extract_ipv4_list(ip_val: Any) -> List[str]:
    out: List[str] = []
    if not ip_val:
        return out
    ip = str(ip_val).strip()
    if not ip:
        return out
    base_ip = ip.split("/", 1)[0].strip()
    try:
        parsed = ipaddress.ip_address(base_ip)
    except ValueError:
        return out
    if parsed.version != 4:
        return out
    # VISP sometimes surfaces placeholder addresses such as 0.0.0.0 for
    # unassigned subscriber/service IPs. These should never become shapeable
    # device addresses.
    if parsed.is_unspecified or base_ip == "255.255.255.255":
        return out
    try:
        if isIntegrationOutputIpAllowed(base_ip):
            out.append(base_ip)
    except Exception:
        return out
    return out


def _split_ipv4_candidates(*values: Any) -> List[str]:
    seen: Set[str] = set()
    out: List[str] = []
    for value in values:
        if value is None:
            continue
        if isinstance(value, list):
            items = value
        else:
            items = str(value).split(",")
        for item in items:
            ip = str(item or "").strip()
            if not ip:
                continue
            for parsed in _extract_ipv4_list(ip):
                if parsed not in seen:
                    seen.add(parsed)
                    out.append(parsed)
    return out


def _bulk_ipv4_candidates(
    subscriber_row: Optional[Dict[str, Any]],
    service_row: Dict[str, Any],
    equipment_rows: List[Dict[str, Any]],
) -> List[str]:
    candidate_values: List[Any] = [service_row.get("ip_address")]
    if subscriber_row:
        candidate_values.extend(
            [
                subscriber_row.get("package_ip"),
                subscriber_row.get("equipment_ip"),
            ]
        )
    for equipment_row in equipment_rows:
        if not isinstance(equipment_row, dict):
            continue
        candidate_values.append(equipment_row.get("ip_address"))
    return _split_ipv4_candidates(*candidate_values)


def _full_name(rec: Dict[str, Any]) -> str:
    first = str(rec.get("first_name") or "").strip()
    last = str(rec.get("last_name") or "").strip()
    full = f"{first} {last}".strip()
    return full


def _service_is_shapable(service_type: Any, service_details: Any) -> bool:
    type_name = str(service_type or "").strip().lower()
    details = service_details if isinstance(service_details, dict) else {}
    typename = str(details.get("__typename") or "").strip()
    return type_name == "wifi" or typename == "ServiceTypeWifi"


def _extract_mac_candidates(row: Dict[str, Any]) -> List[str]:
    candidates: List[str] = []
    seen: Set[str] = set()
    equipment_data = row.get("equipment_data") or {}
    if not isinstance(equipment_data, dict):
        equipment_data = {}
    for key in ("Fiber MAC", "mac_address", "Ethernet MAC", "WAN MAC", "MAC Addr", "radio_mac"):
        mac = _normalize_mac(equipment_data.get(key))
        if mac and mac not in seen:
            seen.add(mac)
            candidates.append(mac)
    return candidates


def _score_attachment_row(
    row: Dict[str, Any],
    customer_names: Set[str],
    preferred_ids: Set[int],
    preferred_macs: Set[str],
) -> int:
    score = 0
    parent_id = row.get("parent_id")
    if parent_id:
        score += 100
    location_name = str(row.get("location_name") or "").strip()
    if location_name and location_name in customer_names:
        score += 40
    try:
        if int(row.get("id") or 0) in preferred_ids:
            score += 20
    except Exception:
        pass
    macs = set(_extract_mac_candidates(row))
    if preferred_macs and macs.intersection(preferred_macs):
        score += 15
    equipment_data = row.get("equipment_data") or {}
    if isinstance(equipment_data, dict):
        if equipment_data.get("Fiber MAC"):
            score += 10
        if equipment_data.get("VLAN") or equipment_data.get("VLAN-1"):
            score += 5
        if str(equipment_data.get("Router Mode") or "").strip().lower() == "true":
            score -= 20
    description = str(row.get("description") or "").strip().lower()
    if "router" in description:
        score -= 10
    return score


def _select_attachment_equipment(
    candidate_rows: List[Dict[str, Any]],
    customer_names: Set[str],
    preferred_ids: Set[int],
    preferred_macs: Set[str],
) -> Optional[Dict[str, Any]]:
    if not candidate_rows:
        return None
    ranked = sorted(
        candidate_rows,
        key=lambda row: (
            _score_attachment_row(row, customer_names, preferred_ids, preferred_macs),
            int(row.get("id") or 0),
        ),
        reverse=True,
    )
    return ranked[0]


def _visp_site_node_id(location_id: Any, location_name: Any) -> str:
    if location_id:
        return f"visp_site_{location_id}"
    name = str(location_name or "unknown").strip().lower().replace(" ", "_")
    return f"visp_site_name_{name}"


def _visp_ap_node_id(equipment_id: Any) -> str:
    return f"visp_eq_{equipment_id}"


def _ensure_site_node(
    net: NetworkGraph,
    site_cache: Dict[str, str],
    location_id: Any,
    location_name: Any,
) -> str:
    site_id = _visp_site_node_id(location_id, location_name)
    existing = site_cache.get(site_id)
    if existing:
        return existing
    if net.findNodeIndexById(site_id) == -1:
        display_name = str(location_name or "VISP Site").strip() or "VISP Site"
        net.addRawNode(
            NetworkNode(
                id=site_id,
                displayName=display_name,
                parentId="",
                type=NodeType.site,
                networkJsonId=f"visp:site:{location_id or display_name}",
            )
        )
    site_cache[site_id] = site_id
    return site_id


def _ensure_infrastructure_node(
    net: NetworkGraph,
    pg_by_id: Dict[int, Dict[str, Any]],
    site_cache: Dict[str, str],
    ap_cache: Dict[int, str],
    equipment_id: Any,
) -> str:
    try:
        eq_id = int(equipment_id or 0)
    except Exception:
        return ""
    if eq_id <= 0:
        return ""
    existing = ap_cache.get(eq_id)
    if existing:
        return existing
    row = pg_by_id.get(eq_id)
    if not row:
        return ""

    parent_node_id = ""
    parent_id = row.get("parent_id")
    try:
        parent_eq_id = int(parent_id or 0)
    except Exception:
        parent_eq_id = 0

    if parent_eq_id > 0 and parent_eq_id != eq_id and parent_eq_id in pg_by_id:
        parent_node_id = _ensure_infrastructure_node(net, pg_by_id, site_cache, ap_cache, parent_eq_id)
    else:
        parent_node_id = _ensure_site_node(
            net,
            site_cache,
            row.get("location_id"),
            row.get("location_name"),
        )

    node_id = _visp_ap_node_id(eq_id)
    if net.findNodeIndexById(node_id) == -1:
        display_name = str(row.get("description") or row.get("name") or f"VISP {eq_id}").strip() or f"VISP {eq_id}"
        net.addRawNode(
            NetworkNode(
                id=node_id,
                displayName=display_name,
                parentId=parent_node_id,
                type=NodeType.ap,
                networkJsonId=f"visp:eq:{eq_id}",
            )
        )
    ap_cache[eq_id] = node_id
    return node_id


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
    parent_id: str = "",
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
            parentId=parent_id,
            address=customer_name,
            customerName=customer_name,
            download=apply_client_bandwidth_multiplier(down_mbps),
            upload=apply_client_bandwidth_multiplier(up_mbps),
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
    # Some deployed configs surface exception_cpes() as a list instead of a map.
    # Normalize locally so the VISP importer doesn't rely on integrationCommon
    # accepting either shape.
    if not isinstance(net.exceptionCPEs, dict):
        net.exceptionCPEs = {}

    counters = {
        "subscribers_rows": 0,
        "wifi_bulk_services": 0,
        "bulk_customers": 0,
        "backfill_customers": 0,
        "backfill_service_candidates": 0,
        "backfill_shaped": 0,
        "topology_sites": 0,
        "topology_equipment": 0,
        "topology_attached": 0,
        "shaped": 0,
        "skipped_inactive": 0,
        "skipped_no_ip": 0,
        "skipped_no_speed": 0,
    }

    disable_wifi_bulk = os.environ.get("VISP_DISABLE_WIFI_BULK", "").strip() in ("1", "true", "yes", "on")
    debug_topology = os.environ.get("VISP_DEBUG_TOPOLOGY", "").strip() in ("1", "true", "yes", "on")

    # 1) WiFi bulk shaping (fast path with IP + speed + MAC)
    debug_status = os.environ.get("VISP_DEBUG_STATUS", "").strip() in ("1", "true", "yes", "on")
    pkg_status_counts: Counter = Counter()
    status_counts: Counter = Counter()
    bulk_customers: Set[int] = set()
    subscriber_by_customer_id: Dict[int, Dict[str, Any]] = {}
    subscribers = _subscribers(visp)
    counters["subscribers_rows"] = len(subscribers)
    for sub in subscribers:
        try:
            customer_id = int(sub.get("customer_id") or 0)
        except Exception:
            continue
        if customer_id > 0:
            subscriber_by_customer_id[customer_id] = sub

    assembly_rows = _equipment_assemblies_cpe(visp)
    assembly_by_id: Dict[int, Dict[str, Any]] = {}
    assemblies_by_location_name: Dict[str, List[Dict[str, Any]]] = {}
    for row in assembly_rows:
        try:
            row_id = int(row.get("id") or 0)
        except Exception:
            row_id = 0
        if row_id > 0:
            assembly_by_id[row_id] = row
        location_name = str(row.get("location_name") or "").strip()
        if location_name:
            assemblies_by_location_name.setdefault(location_name, []).append(row)

    pg_rows = _equipment_pg_bulk(visp)
    pg_by_id: Dict[int, Dict[str, Any]] = {}
    for row in pg_rows:
        try:
            row_id = int(row.get("id") or 0)
        except Exception:
            row_id = 0
        if row_id > 0:
            pg_by_id[row_id] = row
    site_cache: Dict[str, str] = {}
    ap_cache: Dict[int, str] = {}

    def topology_parent_for_customer(
        customer_id: Optional[int],
        bulk_customer_name: str,
        equipment_ids: Set[int],
        preferred_macs: Set[str],
    ) -> str:
        customer_names: Set[str] = set()
        if bulk_customer_name:
            customer_names.add(bulk_customer_name)
        sub = subscriber_by_customer_id.get(int(customer_id or 0))
        if sub:
            full = _full_name(sub)
            if full:
                customer_names.add(full)
            username = str(sub.get("username") or "").strip()
            if username:
                customer_names.add(username)

        candidates: List[Dict[str, Any]] = []
        for equipment_id in equipment_ids:
            row = assembly_by_id.get(equipment_id)
            if row:
                candidates.append(row)
        if not candidates:
            for customer_name in customer_names:
                candidates.extend(assemblies_by_location_name.get(customer_name, []))

        # Deduplicate candidate rows while preserving first occurrence.
        deduped: List[Dict[str, Any]] = []
        seen_candidate_ids: Set[int] = set()
        for row in candidates:
            try:
                candidate_id = int(row.get("id") or 0)
            except Exception:
                candidate_id = 0
            if candidate_id in seen_candidate_ids:
                continue
            seen_candidate_ids.add(candidate_id)
            deduped.append(row)

        selected = _select_attachment_equipment(deduped, customer_names, equipment_ids, preferred_macs)
        if debug_topology:
            selected_id = selected.get("id") if selected else None
            selected_parent = selected.get("parent_id") if selected else None
            print(
                f"[VISP] Topology select customer_id={customer_id} names={sorted(customer_names)} "
                f"equipment_ids={sorted(equipment_ids)} selected_id={selected_id} selected_parent={selected_parent}"
            )
        if not selected:
            return ""
        parent_id = selected.get("parent_id")
        node_id = _ensure_infrastructure_node(net, pg_by_id, site_cache, ap_cache, parent_id)
        if node_id:
            counters["topology_attached"] += 1
        return node_id

    def add_service_record(
        circuit_id: str,
        customer_id: Optional[int],
        customer_name: str,
        down: Optional[float],
        up: Optional[float],
        ipv4s: List[str],
        mac: str,
        device_suffix: str,
        equipment_ids: Set[int],
        preferred_macs: Set[str],
    ) -> bool:
        if not down or not up:
            counters["skipped_no_speed"] += 1
            return False
        if not ipv4s:
            counters["skipped_no_ip"] += 1
            return False
        parent_id = topology_parent_for_customer(customer_id, customer_name, equipment_ids, preferred_macs)
        if _add_circuit_and_device(
            net,
            circuit_id,
            customer_name,
            down,
            up,
            ipv4s,
            mac,
            device_suffix,
            parent_id=parent_id,
        ):
            counters["shaped"] += 1
            return True
        return False

    if disable_wifi_bulk:
        print("[VISP] VISP_DISABLE_WIFI_BULK enabled; skipping wireless bulk import")
    else:
        bulk = _wifi_bulk(visp)
        for rec in bulk:
            if not isinstance(rec, dict):
                continue
            try:
                customer_id = int(rec.get("customer_id") or 0)
            except Exception:
                customer_id = 0
            if customer_id > 0:
                bulk_customers.add(customer_id)
            subscriber_row = subscriber_by_customer_id.get(customer_id)
            customer_name = str(rec.get("username") or "").strip()
            ws_list = rec.get("wireless_services") or []
            if not customer_name or not isinstance(ws_list, list):
                continue
            preferred_macs: Set[str] = set()
            equipment_ids: Set[int] = set()
            for eq in rec.get("equipment") or []:
                if not isinstance(eq, dict):
                    continue
                try:
                    eq_id = int(eq.get("equipment_id") or 0)
                except Exception:
                    eq_id = 0
                if eq_id > 0:
                    equipment_ids.add(eq_id)
                for key in ("mac_address", "radio_mac"):
                    mac = _normalize_mac(eq.get(key))
                    if mac:
                        preferred_macs.add(mac)
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
                ipv4s = _bulk_ipv4_candidates(subscriber_row, ws, rec.get("equipment") or [])
                if not ipv4s:
                    counters["skipped_no_ip"] += 1
                    continue
                mac = _normalize_mac(ws.get("mac_address"))
                if mac:
                    preferred_macs.add(mac)
                add_service_record(
                    circuit_id=circuit_id,
                    customer_id=customer_id,
                    customer_name=customer_name,
                    down=down,
                    up=up,
                    ipv4s=ipv4s,
                    mac=mac,
                    device_suffix="wifi",
                    equipment_ids=equipment_ids,
                    preferred_macs=preferred_macs,
                )

    counters["bulk_customers"] = len(bulk_customers)

    # 2) Coverage-first backfill: discover active customers not present in the fast bulk
    # path, then selectively hydrate only the missing internet services.
    for sub in subscribers:
        try:
            customer_id = int(sub.get("customer_id") or 0)
        except Exception:
            continue
        if customer_id <= 0 or customer_id in bulk_customers:
            continue
        counters["backfill_customers"] += 1
        try:
            customer_services = _customer_services(visp, customer_id)
        except Exception:
            continue
        if not customer_services:
            continue

        package_ips = _split_ipv4_candidates(sub.get("package_ip"), sub.get("equipment_ip"))
        ip_cache = package_ips
        customer_name = str(sub.get("username") or "").strip() or _full_name(sub) or str(customer_id)
        customer_full_name = _full_name(sub)
        equipment_ids: Set[int] = set()
        preferred_macs: Set[str] = set()
        if customer_full_name:
            for row in assemblies_by_location_name.get(customer_full_name, []):
                try:
                    equipment_ids.add(int(row.get("id") or 0))
                except Exception:
                    pass
                preferred_macs.update(_extract_mac_candidates(row))

        for svc in customer_services:
            if not isinstance(svc, dict):
                continue
            service_details = svc.get("service_details") or {}
            counters["backfill_service_candidates"] += 1
            if not _service_is_shapable(svc.get("service_type"), service_details):
                continue
            if not _service_is_active(
                {
                    "status": svc.get("status"),
                }
            ):
                counters["skipped_inactive"] += 1
                continue

            service_number = service_details.get("service_number") or svc.get("id")
            if not service_number:
                continue
            circuit_id = str(service_number)
            if net.findNodeIndexById(circuit_id) != -1:
                continue

            down = _unit_to_mbps(service_details.get("down_speed"), service_details.get("down_speed_unit"))
            up = _unit_to_mbps(service_details.get("up_speed"), service_details.get("up_speed_unit"))
            ipv4s = _split_ipv4_candidates(service_details.get("ip_address"), ip_cache)
            if not ipv4s:
                try:
                    ip_row = _customer_ip_list(visp, customer_id)
                    ip_cache = _split_ipv4_candidates(
                        ip_cache,
                        ip_row.get("package_ip"),
                        ip_row.get("equipment_ip"),
                    )
                    ipv4s = list(ip_cache)
                except Exception:
                    ipv4s = list(ip_cache)
            mac = _normalize_mac(service_details.get("mac_address"))
            if not mac and preferred_macs:
                mac = sorted(preferred_macs)[0]

            if add_service_record(
                circuit_id=circuit_id,
                customer_id=customer_id,
                customer_name=str(service_details.get("username") or customer_name).strip() or customer_name,
                down=down,
                up=up,
                ipv4s=ipv4s,
                mac=mac,
                device_suffix="backfill",
                equipment_ids={eid for eid in equipment_ids if eid > 0},
                preferred_macs=preferred_macs,
            ):
                counters["backfill_shaped"] += 1

    counters["topology_sites"] = len(site_cache)
    counters["topology_equipment"] = len(ap_cache)

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
                        if u and ip and isIntegrationOutputIpAllowed(ip):
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
    net.materializeCompiledTopology("python/visp", "full")

    print(
        "[VISP] shaped={shaped} wifi_bulk_services={wifi_bulk_services} "
        "subscribers_rows={subscribers_rows} bulk_customers={bulk_customers} "
        "backfill_customers={backfill_customers} backfill_service_candidates={backfill_service_candidates} "
        "backfill_shaped={backfill_shaped} topology_sites={topology_sites} "
        "topology_equipment={topology_equipment} topology_attached={topology_attached} "
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
