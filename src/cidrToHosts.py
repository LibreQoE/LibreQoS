import os
import csv
import json
import ipaddress
import shutil

def cidrToHosts():
	# Back up ShapedDevices.csv file
	shutil.copyfile('ShapedDevices.csv', 'ShapedDevices.backup.csv')
	# Process ShapedDevices.csv file
	revised_rows = []
	changed_made = False
	with open('ShapedDevices.csv') as csv_file:
		csv_reader = csv.reader(csv_file, delimiter=',')
		next(csv_reader)
		for row in csv_reader:
			circuitID, circuitName, deviceID, deviceName, ParentNode, mac, ipv4_input, ipv6_input, downloadMin, uploadMin, downloadMax, uploadMax, comment = row
			ipv4_list = ipv4_input.replace(' ','').split(',')
			counter = 10001
			if len(ipv4_list) > 1:
				for ipv4 in ipv4_list:
					if ('/' in ipv4) and not ('/32' in ipv4):
						print("Converting CIDR " + ipv4 + " to a list of hosts.")
						for host in list(ipaddress.ip_network(ipv4).hosts()):
							str_host = str(host)
							extended_circuit_id = circuitID + "_host_" + str(counter)
							extended_device_id = deviceID + "_host_" + str(counter)
							counter += 1
							revised_rows.append((extended_circuit_id, circuitName, extended_device_id, deviceName, ParentNode, mac, str_host, ipv6_input, downloadMin, uploadMin, downloadMax, uploadMax, comment))
					else:
						extended_circuit_id = circuitID + "_host_" + str(counter)
						extended_device_id = deviceID + "_host_" + str(counter)
						counter += 1
						revised_rows.append((extended_circuit_id, circuitName, extended_device_id, deviceName, ParentNode, mac, ipv4, ipv6_input, downloadMin, uploadMin, downloadMax, uploadMax, comment))
					changed_made = True
			else:
				if ('/' in ipv4_input) and not ('/32' in ipv4_input):
					print("Converting CIDR " + ipv4_input + " to a list of hosts.")
					for host in list(ipaddress.ip_network(ipv4_input).hosts()):
						str_host = str(host)
						extended_circuit_id = circuitID + "_host_" + str(counter)
						extended_device_id = deviceID + "_host_" + str(counter)
						counter += 1
						revised_rows.append((extended_circuit_id, circuitName, extended_device_id, deviceName, ParentNode, mac, str_host, ipv6_input, downloadMin, uploadMin, downloadMax, uploadMax, comment))
					changed_made = True
				else:
					revised_rows.append(row)
	# Save new ShapedDevices.csv file, if changes were needed
	if changed_made:
		with open('ShapedDevices.csv','w') as file:
			writer = csv.writer(file)
			writer.writerow(('Circuit ID (Do Not Duplicate)', 'Circuit Name', 'Device ID', 'Device Name', 'Parent Node', 'MAC', 'IPv4', 'IPv6', 'Download Min Mbps', 'Upload Min Mbps', 'Download Max Mbps', 'Upload Max Mbps', 'Comment'))
			for row in revised_rows:
				writer.writerow(row)
	
if __name__ == '__main__':
	cidrToHosts()
