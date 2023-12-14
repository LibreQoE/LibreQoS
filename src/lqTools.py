#!/usr/bin/python3

import csv
import io
import ipaddress
import json
import os
import os.path
import subprocess
import warnings
import argparse
import logging
from liblqos_python import interface_a, interface_b, enable_actual_shell_commands, upstream_bandwidth_capacity_download_mbps, \
	upstream_bandwidth_capacity_upload_mbps, generated_pn_download_mbps, generated_pn_upload_mbps

def shell(command):
	if enable_actual_shell_commands():
		logging.info(command)
		commands = command.split(' ')
		proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
		for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
			print(line)
	else:
		print(command)

def safeShell(command):
	safelyRan = True
	if enable_actual_shell_commands():
		commands = command.split(' ')
		proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
		for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
			#logging.info(line)
			print(line)
			if ("RTNETLINK answers" in line) or ("We have an error talking to the kernel" in line):
				safelyRan = False
	else:
		print(command)
		safelyRan = True
	return safelyRan

def getQdiscForIPaddress(ipAddress):
	qDiscID = ''
	foundQdisc = False
	with open('statsByCircuit.json', 'r') as j:
		subscriberCircuits = json.loads(j.read())
	for circuit in subscriberCircuits:
		for device in circuit['devices']:
			for ipv4 in device['ipv4s']:
				strippedIPv4 = ipv4.replace('/32','')
				if strippedIPv4 == ipAddress:
					qDiscID = circuit['classid']
					foundQdisc = True
			for ipv6 in device['ipv6s']:
				if ipv6 == ipAddress:
					qDiscID = circuit['qdisc']
					foundQdisc = True
	if foundQdisc:
		return qDiscID
	else:
		return None

def printStatsFromIP(ipAddress):
	qDiscID = getQdiscForIPaddress(ipAddress)
	if qDiscID != None:
		interfaces = [interface_a(), interface_b()]
		for interface in interfaces:		
			command = 'tc -s qdisc show dev ' + interface + ' parent ' + qDiscID
			commands = command.split(' ')
			proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
			for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
				print(line.replace('\n',''))
	else:
		print("Invalid IP address provided")

def printCircuitClassInfo(ipAddress):
	qDiscID = getQdiscForIPaddress(ipAddress)
	if qDiscID != None:
		print("IP: " + ipAddress + " | Class ID: " + qDiscID)
		print()
		theClassID = ''
		interfaces = [interface_a(), interface_b()]
		downloadMin = ''
		downloadMax = ''
		uploadMin = ''
		uploadMax = ''
		cburst = ''
		burst = ''
		for interface in interfaces:		
			command = 'tc class show dev ' + interface + ' classid ' + qDiscID
			commands = command.split(' ')
			proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
			for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
				if "htb" in line:
					listOfThings = line.split(" ")
					if interface == interface_a():
						downloadMin = line.split(' rate ')[1].split(' ')[0]
						downloadMax = line.split(' ceil ')[1].split(' ')[0]
						burst = line.split(' burst ')[1].split(' ')[0]
						cburst = line.split(' cburst ')[1].replace('\n','')
					else:
						uploadMin = line.split(' rate ')[1].split(' ')[0]
						uploadMax = line.split(' ceil ')[1].split(' ')[0]
		print("Download rate/ceil: " + downloadMin + "/" + downloadMax)
		print("Upload rate/ceil: " + uploadMin + "/" + uploadMax)
		print("burst/cburst: " + burst + "/" + cburst)
	else:
		download = min(upstream_bandwidth_capacity_download_mbps(), generated_pn_download_mbps())
		upload = min(upstream_bandwidth_capacity_upload_mbps(), generated_pn_upload_mbps())
		bwString = str(download) + '/' + str(upload)
		print("Invalid IP address provided (default queue limit is " + bwString + " Mbps)")

def findClassIDForCircuitByIP(data, inputIP, classID):
	for node in data:
		if 'circuits' in data[node]:
			for circuit in data[node]['circuits']:
				for device in circuit['devices']:
					if device['ipv4s']:
						for ipv4 in device['ipv4s']:
							if ipv4 == inputIP:
								classID = circuit['qdisc']
					if device['ipv6s']:
						for ipv6 in device['ipv6s']:
							if inputIP == ipv6:
								classID = circuit['qdisc']
		# Recursive call this function for children nodes attached to this node
		if 'children' in data[node]:
			classID = findClassIDForCircuitByIP(data[node]['children'], inputIP, classID)
	return classID

def findClassIDForCircuitByID(data, inputID, classID):
	for node in data:
		if 'circuits' in data[node]:
			for circuit in data[node]['circuits']:
				if circuit['circuitID'] == inputID:
					classID = circuit['qdisc']
		# Recursive call this function for children nodes attached to this node
		if 'children' in data[node]:
			classID = findClassIDForCircuitByID(data[node]['children'], inputID, classID)
	return classID
	
if __name__ == '__main__':
	parser = argparse.ArgumentParser()
	subparsers = parser.add_subparsers(dest='command')
	
	planFromIP = subparsers.add_parser('show-active-plan-from-ip', help="Provide tc class info by IP",)
	planFromIP.add_argument('ip', type=str,)
	statsFromIP = subparsers.add_parser('tc-statistics-from-ip', help="Provide tc qdisc stats by IP",)
	statsFromIP.add_argument('ip', type=str,)
	
	args = parser.parse_args()

	if (args.command == 'tc-statistics-from-ip'):
		printStatsFromIP(args.ip)
	elif (args.command == 'show-active-plan-from-ip'):
		printCircuitClassInfo(args.ip)
	else:
		print("Invalid parameters. Use --help to learn more.")
