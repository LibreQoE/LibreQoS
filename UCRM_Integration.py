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
#                           v.0.3-alpha
#
import requests

#####################################################################
orgUCRMxAuthToken = ''
#####################################################################

def pullUCRMCustomers():
	url = 'https://unms.exampleISP.com/crm/api/v1.0/clients?organizationId=1'
	headers = {'accept':'application/json', 'x-auth-token': orgUCRMxAuthToken}
	r = requests.get(url, headers=headers)
	jsonData = r.json()
	customerList = []
	for customer in jsonData:
		try:
			if customer['isActive'] == True:
				try:
					ipAddr = customer['attributes'][0]['value']
					idNum = customer['id']
					download, upload = getCaps(idNum)
					customerList.append((idNum, ipAddr, download, upload))
				except:
					print("Customer ID ", idNum, " did not have IP address listed on UCRM")
		except:
			print("Failed to load customer #", customer['id'])
	return customerList
	
def getUCRMCaps(idNum):
	url = 'https://unms.exampleISP.com/crm/api/v1.0/clients/services?clientId=' + str(idNum)
	headers = {'accept':'application/json', 'x-auth-token': orgUCRMxAuthToken}
	r = requests.get(url, headers=headers)
	jsonData = r.json()
	for customer in jsonData:
		download = customer['downloadSpeed']
		upload = customer['uploadSpeed']
		downUp = (download, upload)
	return downUp
