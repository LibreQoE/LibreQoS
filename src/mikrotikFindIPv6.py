#!/usr/bin/python3
import routeros_api
import csv
import sys

def pullMikrotikIPv6(CsvPath):
	import routeros_api
	import csv
	import json

	ipv4ToIPv6 = {}
	routerList = []
	with open(CsvPath) as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			next(csv_reader)
			for row in csv_reader:
				RouterName, IP, Username, Password, apiPort = row
				routerList.append((RouterName, IP, Username, Password, int(apiPort)))
	for router in routerList:
		RouterName, IP, inputUsername, inputPassword, apiPort = router
		connection = routeros_api.RouterOsApiPool(IP, username=inputUsername, password=inputPassword, port=apiPort, use_ssl=False, ssl_verify=False, ssl_verify_hostname=False, plaintext_login=True)
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
	# If the first argument is a string, that's the path to the csv file
	import sys
	if len(sys.argv) > 1:
		CsvPath = sys.argv[1]
		print(pullMikrotikIPv6(CsvPath))
	else:
		print("Mikrotik IPv6 Finder")

	#print(pullMikrotikIPv6())
