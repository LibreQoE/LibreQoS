#!/usr/bin/env python3
"""
Standalone Netzur → LibreQoS integration
Supports zones and customers as separate arrays from API.
"""
from pythonCheck import checkPythonVersion
checkPythonVersion()

from liblqos_python import (
    bandwidth_overhead_factor,
    client_bandwidth_multiplier,
    exclude_sites,
    netzur_api_key,
    netzur_api_url,
    netzur_api_timeout,
    overwrite_network_json_always,
)

import logging
from typing import Dict, List, Tuple

import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry

from integrationCommon import NetworkGraph, NetworkNode, NodeType

logging.basicConfig(level=logging.INFO)
LOG = logging.getLogger("netzur_integration")


def _build_session() -> requests.Session:
    retry = Retry(
        total=3,
        backoff_factor=1,
        status_forcelist=[429, 500, 502, 503, 504],
        allowed_methods=["GET"],
    )
    adapter = HTTPAdapter(max_retries=retry)

    session = requests.Session()
    session.headers.update({"Authorization": f"Bearer {netzur_api_key()}"})
    session.mount("http://", adapter)
    session.mount("https://", adapter)
    return session


def fetch_netzur_data() -> Tuple[List[dict], List[dict]]:
    api_url = netzur_api_url()
    timeout = max(5, int(netzur_api_timeout()))
    session = _build_session()

    response = session.get(api_url, timeout=timeout)
    response.raise_for_status()
    payload = response.json()
    zones = payload.get("zones", [])
    customers = payload.get("customers", [])
    LOG.info("[Netzur] Fetched %d zones and %d customers", len(zones), len(customers))
    return zones, customers


def _build_exclusion_set() -> set:
    return {entry.lower().strip() for entry in exclude_sites() if entry}


def _apply_rate(plan_rate: float) -> float:
    plan_rate = float(plan_rate or 0.0)
    if plan_rate <= 0:
        return 0.0
    overhead = bandwidth_overhead_factor()
    minimum = client_bandwidth_multiplier()
    adjusted = plan_rate * overhead
    floor_value = plan_rate * minimum
    return max(adjusted, floor_value)


def createShaper() -> NetworkGraph:
    LOG.info("[Netzur] Starting sync")
    zones, subscribers = fetch_netzur_data()
    exclusion = _build_exclusion_set()
    net = NetworkGraph()
    parents: Dict[str, int] = {}
    parent_counter = 30000

    for zone in zones:
        name = str(zone.get("name", "")).strip()
        if not name or name.lower() in exclusion:
            continue
        if name not in parents:
            parents[name] = parent_counter
            net.addRawNode(
                NetworkNode(
                    id=parent_counter,
                    displayName=name,
                    type=NodeType.site,
                    download=_apply_rate(zone.get("capacity_download_mbps")),
                    upload=_apply_rate(zone.get("capacity_upload_mbps")),
                )
            )
            parent_counter += 1

    for subscriber in subscribers:
        subscriber_id = str(subscriber.get("subscriber_id") or "").strip()
        if not subscriber_id:
            LOG.warning("[Netzur] Skipping customer without subscriber_id: %s", subscriber)
            continue

        zone_name = str(subscriber.get("zone") or "").strip()
        if zone_name and zone_name.lower() in exclusion:
            continue

        parent_id = parents.get(zone_name)
        customer_name = str(subscriber.get("customerName") or subscriber_id)
        address = str(subscriber.get("address") or "")
        circuit_id = f"netzur_{subscriber_id}"

        net.addRawNode(
            NetworkNode(
                id=circuit_id,
                displayName=customer_name,
                type=NodeType.client,
                parentId=parent_id,
                address=address,
                customerName=customer_name,
                download=_apply_rate(subscriber.get("download")),
                upload=_apply_rate(subscriber.get("upload")),
            )
        )

        ipv4 = [subscriber["ip"]] if subscriber.get("ip") else []
        ipv6 = [subscriber["ipv6"]] if subscriber.get("ipv6") else []
        mac = str(subscriber.get("mac") or "").strip()
        if ipv4 or ipv6 or mac:
            net.addRawNode(
                NetworkNode(
                    id=f"{circuit_id}_device",
                    displayName=f"{customer_name}_device",
                    type=NodeType.device,
                    parentId=circuit_id,
                    mac=mac,
                    ipv4=ipv4,
                    ipv6=ipv6,
                )
            )

    LOG.info("[Netzur] Built graph with %d nodes", len(net.nodes))
    net.prepareTree()

    if net.doesNetworkJsonExist() and not overwrite_network_json_always():
        LOG.info("[Netzur] network.json exists and overwrite disabled; preserving current file")
    else:
        net.createNetworkJson()
    net.createShapedDevices()
    LOG.info("[Netzur] ShapedDevices.csv updated")
    return net


def importFromNetzur() -> None:
    try:
        createShaper()
    except Exception:
        LOG.exception("[Netzur] Import failed")
        raise


if __name__ == '__main__':
    importFromNetzur()
