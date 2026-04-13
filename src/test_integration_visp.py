import unittest
from unittest.mock import patch

from integrationVISP import (
    _bulk_ipv4_candidates,
    _select_attachment_equipment,
    _service_is_shapable,
    _split_ipv4_candidates,
)


class IntegrationVispTests(unittest.TestCase):
    def test_split_ipv4_candidates_filters_and_deduplicates(self):
        with patch(
            "integrationVISP.isIntegrationOutputIpAllowed",
            side_effect=lambda ip: ip in {"192.0.2.10", "198.51.100.20"},
        ):
            result = _split_ipv4_candidates(
                "192.0.2.10, 192.0.2.10, not-an-ip",
                ["198.51.100.20", "", None],
            )
        self.assertEqual(result, ["192.0.2.10", "198.51.100.20"])

    def test_bulk_ipv4_candidates_uses_subscriber_and_equipment_fallbacks(self):
        with patch(
            "integrationVISP.isIntegrationOutputIpAllowed",
            side_effect=lambda ip: ip in {"198.51.100.20", "203.0.113.7"},
        ):
            result = _bulk_ipv4_candidates(
                subscriber_row={
                    "package_ip": "198.51.100.20",
                    "equipment_ip": "",
                },
                service_row={
                    "ip_address": "",
                },
                equipment_rows=[
                    {"ip_address": "203.0.113.7"},
                    {"ip_address": ""},
                ],
            )
        self.assertEqual(result, ["198.51.100.20", "203.0.113.7"])

    def test_split_ipv4_candidates_keeps_public_ip_when_not_ignored(self):
        with patch("integrationVISP.isIntegrationOutputIpAllowed", return_value=True):
            result = _split_ipv4_candidates("203.0.113.191")
        self.assertEqual(result, ["203.0.113.191"])

    def test_split_ipv4_candidates_rejects_unspecified_placeholder_ips(self):
        with patch("integrationVISP.isIntegrationOutputIpAllowed", return_value=True):
            result = _split_ipv4_candidates("0.0.0.0, 255.255.255.255, 203.0.113.191")
        self.assertEqual(result, ["203.0.113.191"])

    def test_service_is_shapable_accepts_wifi_typed_service(self):
        self.assertTrue(
            _service_is_shapable(
                "wifi",
                {"__typename": "ServiceTypeWifi"},
            )
        )

    def test_service_is_shapable_rejects_non_internet_service(self):
        self.assertFalse(
            _service_is_shapable(
                "voip",
                {"__typename": "ServiceTypeVoip"},
            )
        )

    def test_select_attachment_prefers_parented_fiber_cpe(self):
        rows = [
            {
                "id": 1450,
                "parent_id": None,
                "location_name": "Example Subscriber",
                "description": "Router-side device",
                "equipment_data": {
                    "mac_address": "02:00:00:00:00:10",
                    "Router Mode": "false",
                },
            },
            {
                "id": 1296,
                "parent_id": 1409,
                "location_name": "Example Subscriber",
                "description": "Fiber ONU",
                "equipment_data": {
                    "Fiber MAC": "02:00:00:00:00:20",
                    "VLAN-1": "Example-VLAN",
                },
            },
        ]
        selected = _select_attachment_equipment(
            candidate_rows=rows,
            customer_names={"Example Subscriber"},
            preferred_ids={1450, 1296},
            preferred_macs={"020000000020"},
        )
        self.assertIsNotNone(selected)
        self.assertEqual(selected["id"], 1296)


if __name__ == "__main__":
    unittest.main()
