#!/usr/bin/python3
import csv
import ipaddress
import json
import sys
from enum import IntEnum
from typing import NamedTuple

import routeros_api

from integrationUtils import normalize_mac
from liblqos_python import load_mikrotik_ipv6_routers_json, mikrotik_ipv6_config_path

class SourcePriority(IntEnum):
	ARP = 0
	NEIGHBOR = 1
	DHCP = 2

class Ipv6Rank(IntEnum):
	INVALID = -1
	LINK_LOCAL = 0
	HOST = 1
	PREFIX = 2

class IpRecord(NamedTuple):
	source_priority: SourcePriority
	address_rank: int
	canonical_address: str
	address: str
	ambiguous: bool = False

class ClientAddressRecord(NamedTuple):
	canonical_address: str
	address: str
	ambiguous: bool = False

def _load_legacy_router_csv(csv_path):
	router_list = []
	with open(csv_path) as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			next(csv_reader)
			for row in csv_reader:
				RouterName, IP, Username, Password, apiPort = row
				router_list.append({
					"name": RouterName,
					"host": IP,
					"username": Username,
					"password": Password,
					"port": int(apiPort),
					"use_ssl": False,
					"plaintext_login": True,
				})
	return router_list

def _load_router_list(configPath=None):
	if configPath is None:
		return json.loads(load_mikrotik_ipv6_routers_json())
	if configPath.endswith('.csv'):
		return _load_legacy_router_csv(configPath)
	raise ValueError("Explicit Mikrotik IPv6 config overrides must currently point to a legacy .csv file")

def _parse_ip_address(address):
	try:
		return ipaddress.ip_address(address.split('/')[0])
	except (AttributeError, ValueError):
		return None

def _canonical_ip_text(address):
	parsed_address = _parse_ip_address(address)
	if parsed_address is None:
		return None
	return str(parsed_address)

def _canonical_ip_network_text(address):
	try:
		network = ipaddress.ip_network(address, strict=False)
	except (AttributeError, ValueError):
		return None
	return network.with_prefixlen

def _entry_value(entry, key):
	if not isinstance(entry, dict):
		return None
	return entry.get(key)

def _record_address_by_mac(entry, mac_to_ipv4, mac_to_ipv6):
	mac = _entry_value(entry, 'mac-address')
	address = _entry_value(entry, 'address')
	parsed_address = _parse_ip_address(address)
	if mac is None or parsed_address is None:
		return
	normalized_mac = normalize_mac(mac)
	if parsed_address.version == 4:
		_record_ipv4_for_mac(mac_to_ipv4, normalized_mac, address, source_priority=SourcePriority.ARP)
	else:
		_record_ipv6_for_mac(mac_to_ipv6, normalized_mac, address, source_priority=SourcePriority.ARP)

def _record_dhcp4_address_by_mac(entry, mac_to_ipv4):
	mac = _entry_value(entry, 'mac-address')
	address = _entry_value(entry, 'address')
	parsed_address = _parse_ip_address(address)
	if mac is None or parsed_address is None or parsed_address.version != 4:
		return
	_record_ipv4_for_mac(mac_to_ipv4, normalize_mac(mac), address, source_priority=SourcePriority.DHCP)

def _mac_from_dhcpv6_duid(duid):
	duid_hex = str(duid).strip().replace(":", "").replace("-", "").lower()
	if duid_hex.startswith("0x"):
		duid_hex = duid_hex[2:]
	try:
		duid_bytes = bytes.fromhex(duid_hex)
	except ValueError:
		return None
	if len(duid_bytes) == 14 and duid_bytes[0:2] == b'\x00\x01':
		return normalize_mac(duid_bytes[8:14].hex())
	if len(duid_bytes) == 10 and duid_bytes[0:2] == b'\x00\x03':
		return normalize_mac(duid_bytes[4:10].hex())
	return None

def _record_ipv4_for_mac(mac_to_ipv4, mac, address, source_priority):
	if not mac or not address:
		return
	parsed_address = _parse_ip_address(address)
	if parsed_address is None:
		return
	current = mac_to_ipv4.get(mac)
	candidate = IpRecord(source_priority, int(parsed_address), str(parsed_address), address)
	if current is None or source_priority > current.source_priority:
		mac_to_ipv4[mac] = candidate
	elif source_priority == current.source_priority and candidate.canonical_address != current.canonical_address:
		mac_to_ipv4[mac] = current._replace(ambiguous=True)

def _ipv6_rank(address):
	try:
		network = ipaddress.ip_network(address, strict=False)
	except ValueError:
		return Ipv6Rank.INVALID
	if network.version != 6:
		return Ipv6Rank.INVALID
	ip = network.network_address
	if ip.is_unspecified or ip.is_loopback or ip.is_multicast:
		return Ipv6Rank.INVALID
	if ip.is_link_local:
		address_priority = Ipv6Rank.LINK_LOCAL
	elif network.prefixlen < 128:
		address_priority = Ipv6Rank.PREFIX
	else:
		address_priority = Ipv6Rank.HOST
	return address_priority

def _record_ipv6_for_mac(mac_to_ipv6, mac, address, source_priority):
	if not mac or not address:
		return
	address_rank = _ipv6_rank(address)
	if address_rank in (Ipv6Rank.INVALID, Ipv6Rank.LINK_LOCAL):
		return
	canonical_address = _canonical_ip_network_text(address)
	if canonical_address is None:
		return
	current = mac_to_ipv6.get(mac)
	candidate = IpRecord(source_priority, address_rank, canonical_address, address)
	if current is None or source_priority > current.source_priority:
		mac_to_ipv6[mac] = candidate
	elif source_priority == current.source_priority:
		if address_rank > current.address_rank:
			mac_to_ipv6[mac] = candidate
		elif address_rank == current.address_rank and canonical_address != current.canonical_address:
			mac_to_ipv6[mac] = current._replace(ambiguous=True)

def _record_client_address_binding(client_address_to_ipv6, client_address, address):
	if not client_address or not address or _ipv6_rank(address) in (Ipv6Rank.INVALID, Ipv6Rank.LINK_LOCAL):
		return
	canonical_address = _canonical_ip_network_text(address)
	if canonical_address is None:
		return
	current = client_address_to_ipv6.get(client_address)
	if current is None:
		client_address_to_ipv6[client_address] = ClientAddressRecord(canonical_address, address)
	elif canonical_address != current.canonical_address:
		client_address_to_ipv6[client_address] = current._replace(ambiguous=True)

def _build_ipv4_to_ipv6_map(arp_entries, dhcp4_entries, dhcp6_bindings, neighbor6_entries):
	ipv4_to_ipv6 = {}
	mac_to_ipv4 = {}
	mac_to_ipv6 = {}
	client_address_to_ipv6 = {}

	for entry in arp_entries:
		_record_address_by_mac(entry, mac_to_ipv4, mac_to_ipv6)
	for entry in dhcp4_entries:
		_record_dhcp4_address_by_mac(entry, mac_to_ipv4)
	for entry in dhcp6_bindings:
		mac = _mac_from_dhcpv6_duid(_entry_value(entry, 'duid') or '')
		address = _entry_value(entry, 'address')
		if mac is not None and address:
			_record_ipv6_for_mac(mac_to_ipv6, mac, address, source_priority=SourcePriority.DHCP)
		elif _entry_value(entry, 'client-address') and address:
			client_address = _canonical_ip_text(_entry_value(entry, 'client-address'))
			if client_address is not None:
				_record_client_address_binding(client_address_to_ipv6, client_address, address)
	for entry in neighbor6_entries:
		mac = _entry_value(entry, 'mac-address')
		address = _entry_value(entry, 'address')
		if mac is None or _parse_ip_address(address) is None:
			continue
		normalized_mac = normalize_mac(mac)
		neighbor_address = _canonical_ip_text(address)
		mapped_ipv6_record = client_address_to_ipv6.get(neighbor_address)
		if mapped_ipv6_record is not None and mapped_ipv6_record.ambiguous:
			print('Skipped ambiguous DHCPv6 client-address entries for ' + neighbor_address, file=sys.stderr)
			_record_ipv6_for_mac(mac_to_ipv6, normalized_mac, address, source_priority=SourcePriority.NEIGHBOR)
		elif mapped_ipv6_record is not None:
			_record_ipv6_for_mac(mac_to_ipv6, normalized_mac, mapped_ipv6_record.address, source_priority=SourcePriority.DHCP)
		else:
			_record_ipv6_for_mac(mac_to_ipv6, normalized_mac, address, source_priority=SourcePriority.NEIGHBOR)

	for mac, ipv6_record in mac_to_ipv6.items():
		if ipv6_record.ambiguous:
			print('Skipped ambiguous IPv6 entries for MAC ' + mac, file=sys.stderr)
			continue
		ipv6 = ipv6_record.address
		try:
			ipv4_record = mac_to_ipv4[mac]
			if ipv4_record.ambiguous:
				print('Skipped ambiguous IPv4 entries for MAC ' + mac, file=sys.stderr)
				continue
			ipv4 = ipv4_record.address
			ipv4_to_ipv6[ipv4] = ipv6
		except KeyError:
			print('Failed to find associated IPv4 for ' + ipv6, file=sys.stderr)

	return ipv4_to_ipv6

def pullMikrotikIPv6(configPath=None):
	ipv4ToIPv6 = {}
	routerList = _load_router_list(configPath)
	for router in routerList:
		IP = router['host']
		inputUsername = router['username']
		inputPassword = router['password']
		apiPort = int(router.get('port', 8728))
		use_ssl = bool(router.get('use_ssl', False))
		plaintext_login = bool(router.get('plaintext_login', True))
		connection = routeros_api.RouterOsApiPool(IP, username=inputUsername, password=inputPassword, port=apiPort, use_ssl=use_ssl, ssl_verify=False, ssl_verify_hostname=False, plaintext_login=plaintext_login)
		api = connection.get_api()
		list_arp4 = api.get_resource('/ip/arp')
		arp_entries = list_arp4.get()
		list_dhcp4 = api.get_resource('/ip/dhcp-server/lease')
		dhcp4_entries = list_dhcp4.get()
		list_binding6 = api.get_resource('/ipv6/dhcp-server/binding')
		dhcp6_bindings = list_binding6.get()
		list_neighbor6 = api.get_resource('/ipv6/neighbor')
		neighbor6_entries = list_neighbor6.get()
		ipv4ToIPv6.update(_build_ipv4_to_ipv6_map(arp_entries, dhcp4_entries, dhcp6_bindings, neighbor6_entries))
	
	return json.dumps(ipv4ToIPv6)

if __name__ == '__main__':
	# If the first argument is a string, it's treated as a legacy CSV path override.
	if len(sys.argv) > 1 and sys.argv[1] == '--show-config':
		print("Configured secrets file: " + mikrotik_ipv6_config_path())
	elif len(sys.argv) > 1:
		configPath = sys.argv[1]
		print(pullMikrotikIPv6(configPath))
	else:
		print(pullMikrotikIPv6())

	#print(pullMikrotikIPv6())
