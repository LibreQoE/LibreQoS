# Copyright (C) 2020  Robert Chacón
# This file is part of LibreQoS.
#
# LibreQoS is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 2 of the License, or
# (at your option) any later version.
#
# LibreQoS is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with LibreQoS.  If not, see <http://www.gnu.org/licenses/>.
#
#            _     _ _               ___       ____  
#           | |   (_) |__  _ __ ___ / _ \  ___/ ___| 
#           | |   | | '_ \| '__/ _ \ | | |/ _ \___ \ 
#           | |___| | |_) | | |  __/ |_| | (_) |__) |
#           |_____|_|_.__/|_|  \___|\__\_\\___/____/
#                           v.0.4-alpha
#
import random
import os
import subprocess
import time
from datetime import date
from UNMS_Integration import pullUNMSCustomers
from ispConfig import fqOrCAKE, pipeBandwidthCapacityMbps, interfaceA, interfaceB, enableActualShellCommands, runShellCommandsAsSudo, importFromUNMS

def shell(inputCommand):
	if enableActualShellCommands:
		if runShellCommandsAsSudo:
			inputCommand = 'sudo ' + inputCommand
		inputCommandSplit = inputCommand.split(' ')
		print(inputCommand)
		result = subprocess.run(inputCommandSplit, stdout=subprocess.PIPE)
		print(result.stdout)
	else:
		print(inputCommand)
	
def clearPriorSettings(interfaceA, interfaceB):
	shell('tc filter delete dev ' + interfaceA)
	shell('tc filter delete dev ' + interfaceA + ' root')
	shell('tc qdisc delete dev ' + interfaceA)
	shell('tc qdisc delete dev ' + interfaceA + ' root')
	shell('tc filter delete dev ' + interfaceB)
	shell('tc filter delete dev ' + interfaceB + ' root')
	shell('tc qdisc delete dev ' + interfaceB)
	shell('tc qdisc delete dev ' + interfaceB + ' root') 

def getHashList():
	twoDigitHash = []
	letters = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't', 'u', 'v', 'w', 'x', 'y', 'z']
	for i in range(10):
		for x in range(26):
			twoDigitHash.append(str(i) + letters[x])
	return twoDigitHash

def createTestClientsPool(slash16, quantity):
	if quantity<65534:
		tempList = []
		counterC = 0
		counterD = 1
		mainCounter = 0
		while mainCounter < quantity:
			if counterD <= 255:
				ipAddr = slash16.replace('X.X', '') + str(counterC) + '.' + str(counterD)
				tempList.append((ipAddr, 100, 15))
				counterD += 1
			else:
				counterC += 1
				counterD = 1
			mainCounter += 1
		return tempList
	else:
		raise Exception

def refreshShapers():
	#Clients
	clientsList = []
	#Add arbitrary number of test clients in /16 subnet
	#clientsList = createTestClientsPool('100.64.X.X', 5)
	#Add specific test clients
	#clientsList.append((100, '100.65.1.1'))

	#Bring in clients from UCRM if enabled
	if importFromUNMS:
		tempList = pullUNMSCustomers()
		for cust in tempList:
			downloadSpeed = cust['downloadSpeed']
			uploadSpeed = cust['uploadSpeed']
			for ipAddr in cust['deviceIPs']:
				if '/' in ipAddr:
					ipAddr = ipAddr.split('/')[0]
				#Use UCRM plan speed values to place into corresponding plan defined here in LibreQoS
				clientsList.append((ipAddr, downloadSpeed, uploadSpeed))

	#Categorize Clients By IPv4 /16
	listOfSlash16SubnetsInvolved = []
	clientsListWithSubnet = []
	for customer in clientsList:
		ipAddr, downloadSpeed, uploadSpeed = customer
		dec1, dec2, dec3, dec4 = ipAddr.split('.')
		slash16 = dec1 + '.' + dec2 + '.0.0'
		if slash16 not in listOfSlash16SubnetsInvolved:
			listOfSlash16SubnetsInvolved.append(slash16)
		clientsListWithSubnet.append((ipAddr, downloadSpeed, uploadSpeed, slash16, dec1, dec2, dec3, dec4))
	#Clear Prior Configs
	clearPriorSettings(interfaceA, interfaceB)
	#InterfaceA
	parentIDFirstPart = 1
	srcOrDst = 'dst'
	classIDCounter = 101
	hashIDCounter = parentIDFirstPart + 1
	shell('tc qdisc replace dev ' + interfaceA + ' root handle ' + str(parentIDFirstPart) + ': htb default 1') 
	shell('tc class add dev ' + interfaceA + ' parent ' + str(parentIDFirstPart) + ': classid ' + str(parentIDFirstPart) + ':1 htb rate '+ str(pipeBandwidthCapacityMbps) + 'mbit')
	for slash16 in listOfSlash16SubnetsInvolved:
		#X.X.0.0
		thisSlash16Dec1 = slash16.split('.')[0]
		thisSlash16Dec2 = slash16.split('.')[1]
		groupedCustomers = []	
		for i in range(255):
			tempList = []
			for customer in clientsListWithSubnet:
				ipAddr, downloadSpeed, uploadSpeed, slash16, dec1, dec2, dec3, dec4 = customer
				if (dec1 == thisSlash16Dec1) and (dec2 == thisSlash16Dec2) and (dec4 == str(i)):
					tempList.append(customer)
			if len(tempList) > 0:
				groupedCustomers.append(tempList)
		shell('tc filter add dev ' + interfaceA + ' parent ' + str(parentIDFirstPart) + ': prio 5 u32')
		shell('tc filter add dev ' + interfaceA + ' parent ' + str(parentIDFirstPart) + ': prio 5 handle ' + str(hashIDCounter) + ': u32 divisor 256')
		thirdDigitCounter = 0
		handleIDSecond = 1
		while thirdDigitCounter <= 255:	
			if len(groupedCustomers) > 0:
				currentCustomerList = groupedCustomers.pop()
				tempHashList = getHashList()
				for cust in currentCustomerList:
					ipAddr, downloadSpeed, uploadSpeed, slash16, dec1, dec2, dec3, dec4 = cust
					twoDigitHashString = hex(int(dec4)).replace('0x','')
					shell('tc class add dev ' + interfaceA + ' parent ' + str(parentIDFirstPart) + ':1 classid ' + str(parentIDFirstPart) + ':' + str(classIDCounter) + ' htb rate '+ str(downloadSpeed) + 'mbit ceil '+ str(downloadSpeed) + 'mbit prio 3') 
					shell('tc qdisc add dev ' + interfaceA + ' parent ' + str(parentIDFirstPart) + ':' + str(classIDCounter) + ' ' + fqOrCAKE)
					shell('tc filter add dev ' + interfaceA + ' parent ' + str(parentIDFirstPart) + ': prio 5 u32 ht ' + str(hashIDCounter) + ':' + twoDigitHashString + ' match ip ' + srcOrDst + ' ' + ipAddr + ' flowid ' + str(parentIDFirstPart) + ':' + str(classIDCounter))
					classIDCounter += 1
			thirdDigitCounter += 1
		if (srcOrDst == 'dst'):
			startPointForHash = '16' #Position of dst-address in IP header
		elif  (srcOrDst == 'src'):
			startPointForHash = '12' #Position of src-address in IP header
		shell('tc filter add dev ' + interfaceA + ' parent ' + str(parentIDFirstPart) + ': prio 5 u32 ht 800:: match ip ' + srcOrDst + ' '+ thisSlash16Dec1 + '.' + thisSlash16Dec2 + '.0.0/16 hashkey mask 0x000000ff at ' + startPointForHash + ' link ' + str(hashIDCounter) + ':')
		hashIDCounter += 1
	#InterfaceB
	parentIDFirstPart = hashIDCounter + 1
	hashIDCounter = parentIDFirstPart + 1
	srcOrDst = 'src'
	shell('tc qdisc replace dev ' + interfaceB + ' root handle ' + str(parentIDFirstPart) + ': htb default 1') 
	shell('tc class add dev ' + interfaceB + ' parent ' + str(parentIDFirstPart) + ': classid ' + str(parentIDFirstPart) + ':1 htb rate '+ str(pipeBandwidthCapacityMbps) + 'mbit')
	for slash16 in listOfSlash16SubnetsInvolved:
		#X.X.0.0
		thisSlash16Dec1 = slash16.split('.')[0]
		thisSlash16Dec2 = slash16.split('.')[1]
		groupedCustomers = []	
		for i in range(255):
			tempList = []
			for customer in clientsListWithSubnet:
				ipAddr, downloadSpeed, uploadSpeed, slash16, dec1, dec2, dec3, dec4 = customer
				if (dec1 == thisSlash16Dec1) and (dec2 == thisSlash16Dec2) and (dec4 == str(i)):
					tempList.append(customer)
			if len(tempList) > 0:
				groupedCustomers.append(tempList)
		shell('tc filter add dev ' + interfaceB + ' parent ' + str(parentIDFirstPart) + ': prio 5 u32')
		shell('tc filter add dev ' + interfaceB + ' parent ' + str(parentIDFirstPart) + ': prio 5 handle ' + str(hashIDCounter) + ': u32 divisor 256')
		thirdDigitCounter = 0
		handleIDSecond = 1
		while thirdDigitCounter <= 255:	
			if len(groupedCustomers) > 0:
				currentCustomerList = groupedCustomers.pop()
				tempHashList = getHashList()
				for cust in currentCustomerList:
					ipAddr, downloadSpeed, uploadSpeed, slash16, dec1, dec2, dec3, dec4 = cust
					twoDigitHashString = hex(int(dec4)).replace('0x','')
					shell('tc class add dev ' + interfaceB + ' parent ' + str(parentIDFirstPart) + ':1 classid ' + str(parentIDFirstPart) + ':' + str(classIDCounter) + ' htb rate '+ str(uploadSpeed) + 'mbit ceil '+ str(uploadSpeed) + 'mbit prio 3') 
					shell('tc qdisc add dev ' + interfaceB + ' parent ' + str(parentIDFirstPart) + ':' + str(classIDCounter) + ' ' + fqOrCAKE)
					shell('tc filter add dev ' + interfaceB + ' parent ' + str(parentIDFirstPart) + ': prio 5 u32 ht ' + str(hashIDCounter) + ':' + twoDigitHashString + ' match ip ' + srcOrDst + ' ' + ipAddr + ' flowid ' + str(parentIDFirstPart) + ':' + str(classIDCounter))
					classIDCounter += 1
			thirdDigitCounter += 1
		if (srcOrDst == 'dst'):
			startPointForHash = '16' #Position of dst-address in IP header
		elif  (srcOrDst == 'src'):
			startPointForHash = '12' #Position of src-address in IP header
		shell('tc filter add dev ' + interfaceB + ' parent ' + str(parentIDFirstPart) + ': prio 5 u32 ht 800:: match ip ' + srcOrDst + ' '+ thisSlash16Dec1 + '.' + thisSlash16Dec2 + '.0.0/16 hashkey mask 0x000000ff at ' + startPointForHash + ' link ' + str(hashIDCounter) + ':')
		hashIDCounter += 1
	#Recap
	for customer in clientsList:
		ipAddr, downloadSpeed, uploadSpeed = customer
		downloadSpeed = str(downloadSpeed)
		uploadSpeed = str(uploadSpeed)
		print("Applied rate limiting |\t" + ipAddr + "\t| " + downloadSpeed + " down " + uploadSpeed + " up")
	#Done
	today = date.today()
	d1 = today.strftime("%d/%m/%Y")
	print("Successful run completed at ", d1)

if __name__ == '__main__':
	refreshShapers()
	print("Program complete")
