#!/usr/bin/python3
import csv
import json
import sys

import routeros_api

from liblqos_python import load_mikrotik_ipv6_routers_json, mikrotik_ipv6_config_path

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

def pullMikrotikIPv6(configPath=None):
	import routeros_api

	ipv4ToIPv6 = {}
	routerList = _load_router_list(configPath)
	for router in routerList:
		RouterName = router['name']
		IP = router['host']
		inputUsername = router['username']
		inputPassword = router['password']
		apiPort = int(router.get('port', 8728))
		use_ssl = bool(router.get('use_ssl', False))
		plaintext_login = bool(router.get('plaintext_login', True))
		connection = routeros_api.RouterOsApiPool(IP, username=inputUsername, password=inputPassword, port=apiPort, use_ssl=use_ssl, ssl_verify=False, ssl_verify_hostname=False, plaintext_login=plaintext_login)
		api = connection.get_api()
		macToIPv4 = {}
		macToIPv6 = {}
		clientAddressToIPv6 = {}
		# list_dhcp4 = api.get_resource('/ip/dhcp-server/lease')
		# entries = list_dhcp4.get()
		# for entry in entries:
			# try:
				# macToIPv4[entry['mac-address']] = entry['address']
			# except:
				# pass
		list_arp4 = api.get_resource('/ip/arp')
		entries = list_arp4.get()
		for entry in entries:
			try:
				macToIPv4[entry['mac-address']] = entry['address']
			except:
				pass
		list_dhcp4 = api.get_resource('/ip/dhcp-server/lease')
		entries = list_dhcp4.get()
		for entry in entries:
			try:
				macToIPv4[entry['mac-address']] = entry['address']
			except:
				pass
		list_binding6 = api.get_resource('/ipv6/dhcp-server/binding')
		entries = list_binding6.get()
		for entry in entries:
			if len(entry['duid']) ==  14:
				mac = entry['duid'][2:14].upper()
				macNew = mac[0:2] + ':' + mac[2:4] + ':' + mac[4:6] + ':' + mac[6:8] + ':' + mac[8:10] + ':' + mac[10:12]
				macToIPv6[macNew] = entry['address']
			else:
				try:
					clientAddressToIPv6[entry['client-address']] = entry['address']
				except:
					pass
		list_neighbor6 = api.get_resource('/ipv6/neighbor')
		entries = list_neighbor6.get()
		for entry in entries:
			try:
				realIPv6 = clientAddressToIPv6[entry['address']]
				macToIPv6[entry['mac-address']] = realIPv6
			except:
				pass
		for mac, ipv6 in macToIPv6.items():
			try:
				ipv4 = macToIPv4[mac]
				ipv4ToIPv6[ipv4] = ipv6
			except:
				print('Failed to find associated IPv4 for ' + ipv6, file=sys.stderr)
	
	return json.dumps(ipv4ToIPv6)

def pullMikrotikIPv6_Mock(CsvPath):
	return "{\n\"172.29.200.2\": \"2602:fdca:800:1500::/56\"\n}"

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
