import contextlib
import io
import json
import sys
import types
import unittest


sys.modules.setdefault("routeros_api", types.SimpleNamespace())
lqlib = sys.modules.setdefault("liblqos_python", types.SimpleNamespace())
lqlib.load_mikrotik_ipv6_routers_json = lambda: "[]"
lqlib.mikrotik_ipv6_config_path = lambda: "/etc/libreqos/mikrotik_ipv6.toml"

import mikrotikFindIPv6
from mikrotikFindIPv6 import _build_ipv4_to_ipv6_map, _mac_from_dhcpv6_duid


class MikrotikFindIPv6Tests(unittest.TestCase):
	def test_pull_mikrotik_ipv6_fetches_resources_and_returns_json(self):
		resources = {
			"/ip/arp": [
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.30"},
			],
			"/ip/dhcp-server/lease": [],
			"/ipv6/dhcp-server/binding": [
				{"duid": "00030001AABBCCDDEEFF", "address": "2001:db8:30::/56"},
			],
			"/ipv6/neighbor": [
				{"mac-address": "00:11:22:33:44:55", "address": "2001:db8::99"},
			],
		}

		class Resource:
			def __init__(self, entries):
				self.entries = entries

			def get(self):
				return self.entries

		class Api:
			def get_resource(self, name):
				return Resource(resources[name])

		class Pool:
			def __init__(self, *args, **kwargs):
				pass

			def get_api(self):
				return Api()

		original_pool = getattr(mikrotikFindIPv6.routeros_api, "RouterOsApiPool", None)
		original_load_router_list = mikrotikFindIPv6._load_router_list
		mikrotikFindIPv6.routeros_api.RouterOsApiPool = Pool
		mikrotikFindIPv6._load_router_list = lambda configPath=None: [
			{
				"name": "edge",
				"host": "198.51.100.1",
				"username": "admin",
				"password": "secret",
			}
		]
		stderr = io.StringIO()
		try:
			with contextlib.redirect_stderr(stderr):
				result = json.loads(mikrotikFindIPv6.pullMikrotikIPv6())
		finally:
			if original_pool is None:
				delattr(mikrotikFindIPv6.routeros_api, "RouterOsApiPool")
			else:
				mikrotikFindIPv6.routeros_api.RouterOsApiPool = original_pool
			mikrotikFindIPv6._load_router_list = original_load_router_list

		self.assertEqual(result, {"192.0.2.30": "2001:db8:30::/56"})
		self.assertIn("Failed to find associated IPv4 for 2001:db8::99", stderr.getvalue())

	def test_dhcpv6_duid_ll_extracts_mac(self):
		self.assertEqual(_mac_from_dhcpv6_duid("00030001AABBCCDDEEFF"), "AABBCCDDEEFF")
		self.assertEqual(_mac_from_dhcpv6_duid("0x00030001aabbccddeeff"), "AABBCCDDEEFF")
		self.assertEqual(_mac_from_dhcpv6_duid("00:03:00:01:aa:bb:cc:dd:ee:ff"), "AABBCCDDEEFF")

	def test_dhcpv6_duid_llt_extracts_mac(self):
		self.assertEqual(_mac_from_dhcpv6_duid("0001000100000000AABBCCDDEEFF"), "AABBCCDDEEFF")

	def test_neighbor_ipv6_matches_ipv4_by_mac(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "aa:bb:cc:dd:ee:ff", "address": "192.0.2.10"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::10"},
			],
		)

		self.assertEqual(result, {"192.0.2.10": "2001:db8::10"})

	def test_link_local_neighbor_ipv6_is_not_emitted(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "aa:bb:cc:dd:ee:ff", "address": "192.0.2.11"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "fe80::11"},
			],
		)

		self.assertEqual(result, {})

	def test_unspecified_ipv6_prefix_is_not_emitted(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "aa:bb:cc:dd:ee:ff", "address": "192.0.2.12"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[
				{"duid": "00030001AABBCCDDEEFF", "address": "::/64"},
			],
			neighbor6_entries=[],
		)

		self.assertEqual(result, {})

	def test_neighbor_client_address_preserves_dhcpv6_prefix(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.10"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[
				{"client-address": "fe80:0:0:0:0:0:0:1", "address": "2001:db8:100::/56"},
			],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "fe80::1"},
			],
		)

		self.assertEqual(result, {"192.0.2.10": "2001:db8:100::/56"})

	def test_arp_ipv6_matches_dhcp4_by_mac(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::20"},
			],
			dhcp4_entries=[
				{"mac-address": "aa:bb:cc:dd:ee:ff", "address": "192.0.2.20"},
			],
			dhcp6_bindings=[],
			neighbor6_entries=[],
		)

		self.assertEqual(result, {"192.0.2.20": "2001:db8::20"})

	def test_duplicate_ipv6_entries_prefer_non_link_local_prefix(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.40"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "fe80::1"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8:40::/56"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::40"},
			],
		)

		self.assertEqual(result, {"192.0.2.40": "2001:db8:40::/56"})

	def test_duplicate_same_priority_ipv6_entries_are_ambiguous(self):
		neighbors = [
			{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::50"},
			{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::9"},
		]
		for neighbor6_entries in (neighbors, list(reversed(neighbors))):
			stderr = io.StringIO()
			with contextlib.redirect_stderr(stderr):
				result = _build_ipv4_to_ipv6_map(
					arp_entries=[
						{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.50"},
					],
					dhcp4_entries=[],
					dhcp6_bindings=[],
					neighbor6_entries=neighbor6_entries,
				)

			self.assertEqual(result, {})
			self.assertIn("Skipped ambiguous IPv6 entries for MAC AABBCCDDEEFF", stderr.getvalue())

	def test_neighbor_ipv6_recovers_from_ambiguous_arp_ipv6(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.51"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::51"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::52"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::53"},
			],
		)

		self.assertEqual(result, {"192.0.2.51": "2001:db8::53"})

	def test_neighbor_ipv6_preferred_over_arp_ipv6_for_same_mac(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.52"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::99"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::52"},
			],
		)

		self.assertEqual(result, {"192.0.2.52": "2001:db8::52"})

	def test_neighbor_client_address_matches_cidr_text(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.53"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[
				{"client-address": "fe80::53/128", "address": "2001:db8:53::/56"},
			],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "fe80:0:0:0:0:0:0:53"},
			],
		)

		self.assertEqual(result, {"192.0.2.53": "2001:db8:53::/56"})

	def test_bad_duid_falls_back_to_client_address(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.54"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[
				{"duid": "not-hex", "client-address": "fe80::54", "address": "2001:db8:54::/56"},
			],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "fe80::54"},
			],
		)

		self.assertEqual(result, {"192.0.2.54": "2001:db8:54::/56"})

	def test_duplicate_client_address_bindings_are_ambiguous(self):
		stderr = io.StringIO()
		with contextlib.redirect_stderr(stderr):
			result = _build_ipv4_to_ipv6_map(
				arp_entries=[
					{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.57"},
				],
				dhcp4_entries=[],
				dhcp6_bindings=[
					{"client-address": "fe80::57", "address": "2001:db8:57::/56"},
					{"client-address": "fe80::57", "address": "2001:db8:58::/56"},
				],
				neighbor6_entries=[
					{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "fe80::57/128"},
				],
			)

		self.assertEqual(result, {})
		self.assertIn("Skipped ambiguous DHCPv6 client-address entries for fe80::57", stderr.getvalue())

	def test_equivalent_client_address_bindings_are_not_ambiguous(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.58"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[
				{"client-address": "fe80::58", "address": "2001:db8:58::/56"},
				{"client-address": "fe80:0:0:0:0:0:0:58", "address": "2001:db8:58:0::/56"},
			],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "fe80::58"},
			],
		)

		self.assertEqual(result, {"192.0.2.58": "2001:db8:58::/56"})

	def test_dhcpv6_binding_preferred_over_arp_and_neighbor_ipv6(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.55"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::55"},
			],
			dhcp4_entries=[],
			dhcp6_bindings=[
				{"duid": "00030001AABBCCDDEEFF", "address": "2001:db8:55::/56"},
			],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::56"},
			],
		)

		self.assertEqual(result, {"192.0.2.55": "2001:db8:55::/56"})

	def test_dhcp4_ipv4_preferred_over_arp_for_same_mac(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.60"},
			],
			dhcp4_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.61"},
			],
			dhcp6_bindings=[],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::60"},
			],
		)

		self.assertEqual(result, {"192.0.2.61": "2001:db8::60"})

	def test_dhcp4_ipv4_recovers_from_ambiguous_arp_ipv4(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.62"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.63"},
			],
			dhcp4_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.64"},
			],
			dhcp6_bindings=[],
			neighbor6_entries=[
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::64"},
			],
		)

		self.assertEqual(result, {"192.0.2.64": "2001:db8::64"})

	def test_duplicate_same_priority_ipv4_entries_are_ambiguous(self):
		arp_entries = [
			{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.9"},
			{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.10"},
		]
		for entries in (arp_entries, list(reversed(arp_entries))):
			stderr = io.StringIO()
			with contextlib.redirect_stderr(stderr):
				result = _build_ipv4_to_ipv6_map(
					arp_entries=entries,
					dhcp4_entries=[],
					dhcp6_bindings=[],
					neighbor6_entries=[
						{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::60"},
					],
				)

			self.assertEqual(result, {})
			self.assertIn("Skipped ambiguous IPv4 entries for MAC AABBCCDDEEFF", stderr.getvalue())

	def test_malformed_routeros_rows_are_ignored_while_valid_rows_match(self):
		result = _build_ipv4_to_ipv6_map(
			arp_entries=[
				None,
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "not an address"},
				{"address": "192.0.2.70"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "192.0.2.70"},
			],
			dhcp4_entries=[
				["not", "a", "dict"],
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "still not an address"},
			],
			dhcp6_bindings=[
				{"duid": "not-hex", "address": "2001:db8:70::/56"},
				{"duid": "00030001AABBCCDDEEFF", "address": "not an address"},
				{"client-address": "not an address", "address": "2001:db8:71::/56"},
			],
			neighbor6_entries=[
				None,
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "also not an address"},
				{"address": "2001:db8::70"},
				{"mac-address": "AA:BB:CC:DD:EE:FF", "address": "2001:db8::70"},
			],
		)

		self.assertEqual(result, {"192.0.2.70": "2001:db8::70"})


if __name__ == "__main__":
	unittest.main()
