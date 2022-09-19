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
from ispConfig import interfaceA, interfaceB, enableActualShellCommands

def shell(command):
	if enableActualShellCommands:
		logging.info(command)
		commands = command.split(' ')
		proc = subprocess.Popen(commands, stdout=subprocess.PIPE)
		for line in io.TextIOWrapper(proc.stdout, encoding="utf-8"):  # or another encoding
			print(line)
	else:
		print(command)

def safeShell(command):
	safelyRan = True
	if enableActualShellCommands:
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
				if ipv4 == ipAddress:
					qDiscID = circuit['qdisc']
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
		interfaces = [interfaceA, interfaceB]
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
		interfaces = [interfaceA, interfaceB]
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
					if interface == interfaceA:
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
		print("Invalid IP address provided")

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

def changeQueuingStructureCircuitBandwidth(data, classid, minDownload, minUpload, maxDownload, maxUpload):
	for node in data:
		if 'circuits' in data[node]:
			for circuit in data[node]['circuits']:
				if circuit['qdisc'] == classid:
					circuit['minDownload'] = minDownload
					circuit['minUpload'] = minUpload
					circuit['maxDownload'] = maxDownload
					circuit['maxUpload'] = maxUpload
		# Recursive call this function for children nodes attached to this node
		if 'children' in data[node]:
			data[node]['children'] = changeQueuingStructureCircuitBandwidth(data[node]['children'], classid, minDownload, minUpload, maxDownload, maxUpload)
	return data

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

def changeCircuitBandwidthGivenID(circuitID, minDownload, minUpload, maxDownload, maxUpload):
	with open('queuingStructure.json') as file:
		queuingStructure = json.load(file)
	classID = findClassIDForCircuitByID(queuingStructure, circuitID, None)
	if classID:
		didThisCommandRunSafely_1 = safeShell("tc class change dev " + interfaceA + " classid " + classID + " htb rate " + str(minDownload) + "Mbit ceil " + str(maxDownload) + "Mbit")
		didThisCommandRunSafely_2 = safeShell("tc class change dev " + interfaceB + " classid " + classID + " htb rate " + str(minUpload) + "Mbit ceil " + str(maxUpload) + "Mbit")
		if (didThisCommandRunSafely_1 == False) or (didThisCommandRunSafely_2 == False):
			raise ValueError('Execution had errors. Halting now.')
		queuingStructure = changeQueuingStructureCircuitBandwidth(queuingStructure, classID, minDownload, minUpload, maxDownload, maxUpload)
		with open('queuingStructure.json', 'w') as infile:
			json.dump(queuingStructure, infile, indent=4)
	else:
		print("Unable to find associated Class ID")
	
def changeCircuitBandwidthGivenIP(ipAddress, minDownload, minUpload, maxDownload, maxUpload):
	with open('queuingStructure.json') as file:
		queuingStructure = json.load(file)
	classID = findClassIDForCircuitByIP(queuingStructure, ipAddress, None)
	if classID:
		didThisCommandRunSafely_1 = safeShell("tc class change dev " + interfaceA + " classid " + classID + " htb rate " + str(minDownload) + "Mbit ceil " + str(maxDownload) + "Mbit")
		didThisCommandRunSafely_2 = safeShell("tc class change dev " + interfaceB + " classid " + classID + " htb rate " + str(minUpload) + "Mbit ceil " + str(maxUpload) + "Mbit")
		if (didThisCommandRunSafely_1 == False) or (didThisCommandRunSafely_2 == False):
			raise ValueError('Execution had errors. Halting now.')
		queuingStructure = changeQueuingStructureCircuitBandwidth(queuingStructure, classID, minDownload, minUpload, maxDownload, maxUpload)
		with open('queuingStructure.json', 'w') as infile:
			json.dump(queuingStructure, infile, indent=4)
	else:
		print("Unable to find associated Class ID")
	
if __name__ == '__main__':
	parser = argparse.ArgumentParser()
	subparsers = parser.add_subparsers(dest='command')
	
	changeBW = subparsers.add_parser('change-circuit-bandwidth', help='Change bandwidth rates of a given circuit using circuit ID')
	changeBW.add_argument('min-download', type=int, )
	changeBW.add_argument('min-upload', type=int,)
	changeBW.add_argument('max-download', type=int,)
	changeBW.add_argument('max-upload', type=int,)
	changeBW.add_argument('circuit-id', type=str,)
	
	changeBWip = subparsers.add_parser('change-circuit-bandwidth-using-ip', help='Change bandwidth rates of a given circuit using IP')
	changeBWip.add_argument('min-download', type=int,)
	changeBWip.add_argument('min-upload', type=int,)
	changeBWip.add_argument('max-download', type=int,)
	changeBWip.add_argument('max-upload', type=int,)
	changeBWip.add_argument('ip-address', type=str,)
	
	planFromIP = subparsers.add_parser('show-active-plan-from-ip', help="Provide tc class info by IP",)
	planFromIP.add_argument('ip', type=str,)
	statsFromIP = subparsers.add_parser('tc-statistics-from-ip', help="Provide tc qdisc stats by IP",)
	statsFromIP.add_argument('ip', type=str,)
	
	args = parser.parse_args()

	if (args.command == 'change-circuit-bandwidth'):
		changeCircuitBandwidthGivenID(getattr(args, 'circuit-id'), getattr(args, 'min-download'), getattr(args, 'min-upload'), getattr(args, 'max-download'), getattr(args, 'max-upload'))
	elif(args.command == 'change-circuit-bandwidth-using-ip'):
		changeCircuitBandwidthGivenIP(getattr(args, 'ip'), getattr(args, 'min-download'), getattr(args, 'min-upload'), getattr(args, 'max-download'), getattr(args, 'max-upload'))
	elif (args.command == 'tc-statistics-from-ip'):
		printStatsFromIP(args.ip)
	elif (args.command == 'show-active-plan-from-ip'):
		printCircuitClassInfo(args.ip)
	else:
		print("Invalid parameters. Use --help to learn more.")
