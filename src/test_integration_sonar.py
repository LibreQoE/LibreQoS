import importlib
import sys
import types
import unittest
import requests


def install_sonar_stubs():
    lqlib = types.ModuleType("liblqos_python")
    lqlib.allowed_subnets = lambda: ["0.0.0.0/0"]
    lqlib.ignore_subnets = lambda: []
    lqlib.generated_pn_download_mbps = lambda: 1000
    lqlib.generated_pn_upload_mbps = lambda: 1000
    lqlib.circuit_name_use_address = lambda: False
    lqlib.upstream_bandwidth_capacity_download_mbps = lambda: 1000
    lqlib.upstream_bandwidth_capacity_upload_mbps = lambda: 1000
    lqlib.find_ipv6_using_mikrotik = lambda: False
    lqlib.exclude_sites = lambda: []
    lqlib.bandwidth_overhead_factor = lambda: 1.0
    lqlib.committed_bandwidth_multiplier = lambda: 1.0
    lqlib.exception_cpes = lambda: []
    lqlib.promote_to_root_list = lambda: []
    lqlib.client_bandwidth_multiplier = lambda: 1.0
    lqlib.sonar_api_key = lambda: ""
    lqlib.sonar_api_url = lambda: "http://example.invalid"
    lqlib.snmp_community = lambda: "public"
    lqlib.sonar_airmax_ap_model_ids = lambda: []
    lqlib.sonar_ltu_ap_model_ids = lambda: []
    lqlib.sonar_active_status_ids = lambda: []
    lqlib.sonar_recurring_service_rates = lambda: []
    lqlib.sonar_recurring_excluded_service_names = lambda: []
    sys.modules["liblqos_python"] = lqlib


install_sonar_stubs()
integrationSonar = importlib.import_module("integrationSonar")


def inventory_item(item_id, name, ips=None, mac=""):
    ips = ips or []
    field_entities = []
    if mac:
        field_entities.append({
            "value": mac,
            "ip_assignments": {"entities": []},
            "inventory_model_field": {
                "id": 1,
                "name": "MAC Address",
                "primary": True,
            },
        })
    return {
        "id": item_id,
        "inventory_model": {"name": name},
        "inventory_model_field_data": {"entities": field_entities},
        "ip_assignments": {
            "entities": [{"subnet": subnet} for subnet in ips]
        },
    }


def sonar_account(address_items=None, radius_accounts=None):
    return {
        "addresses": {
            "entities": [
                {
                    "line1": "1 Main",
                    "line2": "",
                    "city": "Town",
                    "subdivision": "TX",
                    "inventory_items": {"entities": address_items or []},
                }
            ]
        },
        "radius_accounts": {"entities": radius_accounts or []},
        "all_account_services": {"entities": []},
    }


def radius_account(radius_id, ips):
    return {
        "id": radius_id,
        "ip_assignments": {
            "entities": [{"subnet": subnet} for subnet in ips]
        },
    }


class TestIntegrationSonarDevices(unittest.TestCase):
    def setUp(self):
        integrationSonar.sonar_recurring_service_rates = lambda: []
        integrationSonar.sonar_recurring_excluded_service_names = lambda: []

    def test_inventory_only_devices_keep_existing_shape(self):
        devices = integrationSonar.buildAccountDevices(
            sonar_account(
                address_items=[
                    inventory_item(
                        42,
                        "ONU",
                        ips=["100.64.1.10/32"],
                        mac="AA:BB:CC:DD:EE:FF",
                    )
                ]
            )
        )

        self.assertEqual(len(devices), 1)
        self.assertEqual(devices[0]["id"], "sonar:device:42")
        self.assertEqual(devices[0]["ips"], ["100.64.1.10/32"])
        self.assertEqual(devices[0]["mac"], "AA:BB:CC:DD:EE:FF")

    def test_radius_only_accounts_emit_stable_radius_devices(self):
        devices = integrationSonar.buildAccountDevices(
            sonar_account(
                radius_accounts=[
                    radius_account(77, ["100.64.2.20/32"])
                ]
            )
        )

        self.assertEqual(len(devices), 1)
        self.assertEqual(devices[0]["id"], "sonar:radius-account:77")
        self.assertEqual(devices[0]["name"], "Radius Account 77")
        self.assertEqual(devices[0]["ips"], ["100.64.2.20/32"])
        self.assertEqual(devices[0]["mac"], "")

    def test_radius_ips_do_not_duplicate_inventory_ips(self):
        devices = integrationSonar.buildAccountDevices(
            sonar_account(
                address_items=[
                    inventory_item(
                        42,
                        "ONU",
                        ips=["100.64.3.30/32"],
                        mac="AA:BB:CC:DD:EE:01",
                    )
                ],
                radius_accounts=[
                    radius_account(88, ["100.64.3.30/32", "100.64.3.31/32"])
                ],
            )
        )

        self.assertEqual(len(devices), 2)
        self.assertEqual(devices[0]["id"], "sonar:device:42")
        self.assertEqual(devices[1]["id"], "sonar:radius-account:88")
        self.assertEqual(devices[1]["ips"], ["100.64.3.31/32"])

    def test_inventory_mac_devices_are_preserved_when_radius_supplies_ips(self):
        devices = integrationSonar.buildAccountDevices(
            sonar_account(
                address_items=[
                    inventory_item(
                        42,
                        "ONU",
                        ips=[],
                        mac="AA:BB:CC:DD:EE:02",
                    )
                ],
                radius_accounts=[
                    radius_account(99, ["100.64.4.40/32"])
                ],
            )
        )

        self.assertEqual(len(devices), 2)
        self.assertEqual(devices[0]["id"], "sonar:device:42")
        self.assertEqual(devices[0]["ips"], [])
        self.assertEqual(devices[0]["mac"], "AA:BB:CC:DD:EE:02")
        self.assertEqual(devices[1]["id"], "sonar:radius-account:99")
        self.assertEqual(devices[1]["ips"], ["100.64.4.40/32"])

    def test_child_account_inherits_parent_address_when_missing_own_address(self):
        child = sonar_account(
            address_items=[],
            radius_accounts=[radius_account(100, ["100.64.5.50/32"])],
        )
        child["addresses"] = {"entities": []}
        child["id"] = 500
        child["name"] = "Suite 500"
        child["account_services"] = {
            "entities": [{
                "service": {
                    "data_service_detail": {
                        "download_speed_kilobits_per_second": 100000,
                        "upload_speed_kilobits_per_second": 20000,
                    }
                }
            }]
        }
        record = integrationSonar.buildAccountRecord(child, fallback_address="Parent Address")

        self.assertIsNotNone(record)
        self.assertEqual(record["address"], "Parent Address")
        self.assertEqual(record["id"], "sonar:account:500")

    def test_child_accounts_are_added_without_duplicate_top_level_records(self):
        top_level = sonar_account(
            address_items=[
                inventory_item(
                    42,
                    "ONU",
                    ips=["100.64.6.60/32"],
                    mac="AA:BB:CC:DD:EE:06",
                )
            ]
        )
        top_level["id"] = 600
        top_level["name"] = "Parent"
        top_level["account_services"] = {
            "entities": [{
                "service": {
                    "data_service_detail": {
                        "download_speed_kilobits_per_second": 100000,
                        "upload_speed_kilobits_per_second": 20000,
                    }
                }
            }]
        }

        child_only = sonar_account(
            address_items=[],
            radius_accounts=[radius_account(6010, ["100.64.6.61/32"])],
        )
        child_only["addresses"] = {"entities": []}
        child_only["id"] = 601
        child_only["name"] = "Child Only"
        child_only["account_services"] = {
            "entities": [{
                "service": {
                    "data_service_detail": {
                        "download_speed_kilobits_per_second": 50000,
                        "upload_speed_kilobits_per_second": 10000,
                    }
                }
            }]
        }

        child_duplicate = sonar_account(
            address_items=[],
            radius_accounts=[radius_account(6020, ["100.64.6.62/32"])],
        )
        child_duplicate["addresses"] = {"entities": []}
        child_duplicate["id"] = 602
        child_duplicate["name"] = "Child Duplicate"
        child_duplicate["account_services"] = {
            "entities": [{
                "service": {
                    "data_service_detail": {
                        "download_speed_kilobits_per_second": 50000,
                        "upload_speed_kilobits_per_second": 10000,
                    }
                }
            }]
        }

        top_level["child_accounts"] = {"entities": [child_only, child_duplicate]}

        duplicate_top_level = sonar_account(
            address_items=[],
            radius_accounts=[radius_account(6021, ["100.64.6.63/32"])],
        )
        duplicate_top_level["id"] = 602
        duplicate_top_level["name"] = "Child Duplicate"
        duplicate_top_level["account_services"] = {
            "entities": [{
                "service": {
                    "data_service_detail": {
                        "download_speed_kilobits_per_second": 75000,
                        "upload_speed_kilobits_per_second": 15000,
                    }
                }
            }]
        }
        duplicate_top_level["child_accounts"] = {"entities": []}

        accounts = integrationSonar.buildAccountsFromSonarEntities([top_level, duplicate_top_level])

        self.assertEqual(len(accounts), 3)
        self.assertEqual([account["raw_id"] for account in accounts], [600, 602, 601])
        duplicate = next(account for account in accounts if account["raw_id"] == 602)
        self.assertEqual(duplicate["download"], 75.0)
        self.assertEqual(duplicate["address"], "1 Main, Town, TX")

    def test_recurring_rate_rule_is_used_when_data_service_is_missing(self):
        integrationSonar.sonar_recurring_service_rates = lambda: [
            (True, "1 Gig Bulk Tenant Service", 1000.0, 1000.0)
        ]
        account = sonar_account(
            radius_accounts=[radius_account(7000, ["100.64.7.70/32"])],
        )
        account["id"] = 700
        account["name"] = "Bulk Tenant"
        account["account_services"] = {"entities": []}
        account["all_account_services"] = {
            "entities": [{
                "service": {
                    "id": 1,
                    "name": "1 Gig Bulk Tenant Service",
                    "type": "RECURRING",
                    "enabled": True,
                    "data_service_detail": None,
                }
            }]
        }

        record = integrationSonar.buildAccountRecord(account)

        self.assertIsNotNone(record)
        self.assertEqual(record["download"], 1000.0)
        self.assertEqual(record["upload"], 1000.0)

    def test_recurring_exclusion_blocks_recurring_fallback(self):
        integrationSonar.sonar_recurring_service_rates = lambda: [
            (True, "Equipment Rental", 1000.0, 1000.0)
        ]
        integrationSonar.sonar_recurring_excluded_service_names = lambda: ["Equipment Rental"]
        account = sonar_account(
            radius_accounts=[radius_account(7100, ["100.64.7.71/32"])],
        )
        account["id"] = 701
        account["name"] = "Rental"
        account["account_services"] = {"entities": []}
        account["all_account_services"] = {
            "entities": [{
                "service": {
                    "id": 1,
                    "name": "Equipment Rental",
                    "type": "RECURRING",
                    "enabled": True,
                    "data_service_detail": None,
                }
            }]
        }

        record = integrationSonar.buildAccountRecord(account)

        self.assertIsNone(record)


class FakeResponse:
    def __init__(self, payload):
        self._payload = payload
        self.status_code = 200
        self.headers = {}
        self.text = ""

    def json(self):
        return self._payload


class FakeSession:
    def __init__(self, outcomes):
        self.outcomes = list(outcomes)
        self.calls = []

    def post(self, url, json, timeout):
        self.calls.append({
            "url": url,
            "json": json,
            "timeout": timeout,
        })
        outcome = self.outcomes.pop(0)
        if isinstance(outcome, Exception):
            raise outcome
        return outcome


class TestIntegrationSonarTimeoutHandling(unittest.TestCase):
    def setUp(self):
        self.original_session = integrationSonar._SONAR_SESSION
        self.original_sleep = integrationSonar.time.sleep
        self.sleep_calls = []
        integrationSonar.time.sleep = lambda seconds: self.sleep_calls.append(seconds)

    def tearDown(self):
        integrationSonar._SONAR_SESSION = self.original_session
        integrationSonar.time.sleep = self.original_sleep

    def test_paginated_request_retries_read_timeout(self):
        fake_session = FakeSession([
            requests.exceptions.ReadTimeout("timed out"),
            FakeResponse({
                "data": {
                    "accounts": {
                        "entities": [{"id": "1"}],
                        "page_info": {"total_pages": 1},
                    }
                }
            }),
        ])
        integrationSonar._SONAR_SESSION = fake_session

        entities = integrationSonar.sonarPaginatedRequest("query test {}", {})

        self.assertEqual(entities, [{"id": "1"}])
        self.assertEqual(len(fake_session.calls), 2)
        self.assertEqual(fake_session.calls[0]["timeout"], (10, 60))
        self.assertEqual(self.sleep_calls, [2])

    def test_paginated_request_raises_after_retry_budget_exhausted(self):
        fake_session = FakeSession([
            requests.exceptions.ReadTimeout("timed out 1"),
            requests.exceptions.ReadTimeout("timed out 2"),
            requests.exceptions.ReadTimeout("timed out 3"),
        ])
        integrationSonar._SONAR_SESSION = fake_session

        with self.assertRaises(RuntimeError) as ctx:
            integrationSonar.sonarPaginatedRequest("query test {}", {})

        self.assertIn("after 3 attempts", str(ctx.exception))
        self.assertEqual(self.sleep_calls, [2, 5])


if __name__ == "__main__":
    unittest.main()
