#!/usr/bin/python3
import routeros_api
import csv

def pullMikrotikIPv6():
	ipv4ToIPv6 = {}
	routerList = []
	with open('mikrotikDHCPRouterList.csv') as csv_file:
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
		list_dhcp = api.get_resource('/ip/dhcp-server/lease')
		entries = list_dhcp.get()
		for entry in entries:
			try:
				macToIPv4[entry['mac-address']] = entry['address']
			except:
				pass
		list_dhcp = api.get_resource('/ipv6/dhcp-server/binding')
		entries = list_dhcp.get()
		for entry in entries:
			try:
				clientAddressToIPv6[entry['client-address']] = entry['address']
			except:
				pass
		list_dhcp = api.get_resource('/ipv6/neighbor')
		entries = list_dhcp.get()
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
				print('Failed to find associated IPv4 for ' + ipv6)
	return ipv4ToIPv6

if __name__ == '__main__':
	print(pullMikrotikIPv6())
