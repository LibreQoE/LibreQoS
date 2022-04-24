from flask import Flask
from flask_restful import Resource, Api, reqparse
from flask_httpauth import HTTPBasicAuth
import ast
import csv
from werkzeug.security import generate_password_hash, check_password_hash
from ispConfig import apiUsername, apiPassword, apiHostIP, apiHostPost
from LibreQoS import refreshShapers

app = Flask(__name__)
api = Api(app)
auth = HTTPBasicAuth()

users = {
	apiUsername: generate_password_hash(apiPassword)
}

@auth.verify_password
def verify_password(username, password):
	if username in users and check_password_hash(users.get(username), password):
		return username

class Devices(Resource):
	# Get
	@auth.login_required
	def get(self):
		devices = []
		with open('Shaper.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			header_store = next(csv_reader)
			for row in csv_reader:
				deviceID, parentNode, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = row
				ipv4 = ipv4.strip()
				ipv6 = ipv6.strip()
				if parentNode == "":
					parentNode = "none"
				parentNode = parentNode.strip()
				thisDevice = {
				  "id": deviceID,
				  "mac": mac,
				  "parentNode": parentNode,
				  "hostname": hostname,
				  "ipv4": ipv4,
				  "ipv6": ipv6,
				  "downloadMin": int(downloadMin),
				  "uploadMin": int(uploadMin),
				  "downloadMax": int(downloadMax),
				  "uploadMax": int(uploadMax),
				  "qdisc": '',
				}
				devices.append(thisDevice)
		return {'data': devices}, 200  # return data and 200 OK code
	
	# Post
	@auth.login_required
	def post(self):
		devices = []
		idOnlyList = []
		ipv4onlyList = []
		ipv6onlyList = []
		hostnameOnlyList = []
		with open('Shaper.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			header_store = next(csv_reader)
			for row in csv_reader:
				deviceID, parentNode, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = row
				ipv4 = ipv4.strip()
				ipv6 = ipv6.strip()
				if parentNode == "":
					parentNode = "none"
				parentNode = parentNode.strip()
				thisDevice = {
				  "id": deviceID,
				  "mac": mac,
				  "parentNode": parentNode,
				  "hostname": hostname,
				  "ipv4": ipv4,
				  "ipv6": ipv6,
				  "downloadMin": int(downloadMin),
				  "uploadMin": int(uploadMin),
				  "downloadMax": int(downloadMax),
				  "uploadMax": int(uploadMax),
				  "qdisc": '',
				}
				devices.append(thisDevice)
				ipv4onlyList.append(ipv4)
				ipv6onlyList.append(ipv6)
				idOnlyList.append(deviceID)
				hostnameOnlyList.append(hostname)
		
		parser = reqparse.RequestParser()  # initialize
		
		parser.add_argument('id', required=False)
		parser.add_argument('mac', required=False)
		parser.add_argument('parentNode', required=False)
		parser.add_argument('hostname', required=False)
		parser.add_argument('ipv4', required=False)
		parser.add_argument('ipv6', required=False)
		parser.add_argument('downloadMin', required=True)
		parser.add_argument('uploadMin', required=True)
		parser.add_argument('downloadMax', required=True)
		parser.add_argument('uploadMax', required=True)
		parser.add_argument('qdisc', required=False)
		
		args = parser.parse_args()  # parse arguments to dictionary
		
		args['downloadMin'] = int(args['downloadMin'])
		args['uploadMin'] = int(args['uploadMin'])
		args['downloadMax'] = int(args['downloadMax'])
		args['uploadMax'] = int(args['uploadMax'])
		
		if (args['id'] in idOnlyList):
			return {
				'message': f"'{args['id']}' already exists."
			}, 401
		elif (args['ipv4'] in ipv4onlyList):
			return {
				'message': f"'{args['ipv4']}' already exists."
			}, 401
		elif (args['ipv6'] in ipv6onlyList):
			return {
				'message': f"'{args['ipv6']}' already exists."
			}, 401
		elif (args['hostname'] in hostnameOnlyList):
			return {
				'message': f"'{args['hostname']}' already exists."
			}, 401
		else:
			if args['parentNode'] == None:
				args['parentNode'] = "none"
			
			newDevice = {
						  "id": args['id'],
						  "mac": args['mac'],
						  "parentNode": args['parentNode'],
						  "hostname": args['hostname'],
						  "ipv4": args['ipv4'],
						  "ipv6": args['ipv6'],
						  "downloadMin": int(args['downloadMin']),
						  "uploadMin": int(args['uploadMin']),
						  "downloadMax": int(args['downloadMax']),
						  "uploadMax": int(args['uploadMax']),
						  "qdisc": '',
						}
			
			entryExistsAlready = False
			revisedDevices = []
			revisedDevices.append(newDevice)
			
			# create new Shaper.csv containing new values
			with open('Shaper.csv', 'w') as csvfile:
				wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
				wr.writerow(header_store)
				for device in revisedDevices:
					wr.writerow((device['id'], device['parentNode'], device['mac'], device['hostname'] , device['ipv4'], device['ipv6'], device['downloadMin'], device['uploadMin'], device['downloadMax'], device['uploadMax']))
		   
			return {'data': newDevice}, 200  # return data with 200 OK
	
	# Put
	@auth.login_required
	def put(self):
		devices = []
		idOnlyList = []
		ipv4onlyList = []
		ipv6onlyList = []
		hostnameOnlyList = []
		with open('Shaper.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			header_store = next(csv_reader)
			for row in csv_reader:
				deviceID, parentNode, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = row
				ipv4 = ipv4.strip()
				ipv6 = ipv6.strip()
				if parentNode == "":
					parentNode = "none"
				parentNode = parentNode.strip()
				thisDevice = {
				  "id": deviceID,
				  "mac": mac,
				  "parentNode": parentNode,
				  "hostname": hostname,
				  "ipv4": ipv4,
				  "ipv6": ipv6,
				  "downloadMin": int(downloadMin),
				  "uploadMin": int(uploadMin),
				  "downloadMax": int(downloadMax),
				  "uploadMax": int(uploadMax),
				  "qdisc": '',
				}
				devices.append(thisDevice)
				ipv4onlyList.append(ipv4)
				ipv6onlyList.append(ipv6)
				idOnlyList.append(deviceID)
				hostnameOnlyList.append(hostname)
		
		parser = reqparse.RequestParser()  # initialize
		
		parser.add_argument('id', required=False)
		parser.add_argument('mac', required=False)
		parser.add_argument('parentNode', required=False)
		parser.add_argument('hostname', required=False)
		parser.add_argument('ipv4', required=False)
		parser.add_argument('ipv6', required=False)
		parser.add_argument('downloadMin', required=True)
		parser.add_argument('uploadMin', required=True)
		parser.add_argument('downloadMax', required=True)
		parser.add_argument('uploadMax', required=True)
		parser.add_argument('qdisc', required=False)
		
		args = parser.parse_args()  # parse arguments to dictionary
		
		args['downloadMin'] = int(args['downloadMin'])
		args['uploadMin'] = int(args['uploadMin'])
		args['downloadMax'] = int(args['downloadMax'])
		args['uploadMax'] = int(args['uploadMax'])
		
		if (args['id'] in idOnlyList) or (args['ipv4'] in ipv4onlyList) or (args['ipv6'] in ipv6onlyList) or (args['hostname'] in hostnameOnlyList):
			
			if args['parentNode'] == None:
				args['parentNode'] = "none"
			
			newDevice = {
						  "id": args['id'],
						  "mac": args['mac'],
						  "parentNode": args['parentNode'],
						  "hostname": args['hostname'],
						  "ipv4": args['ipv4'],
						  "ipv6": args['ipv6'],
						  "downloadMin": int(args['downloadMin']),
						  "uploadMin": int(args['uploadMin']),
						  "downloadMax": int(args['downloadMax']),
						  "uploadMax": int(args['uploadMax']),
						  "qdisc": '',
						}
			
			successfullyFoundMatch = False
			revisedDevices = []
			for device in devices:
				if (device['id'] == args['id']) or (device['mac'] == args['mac']) or (device['hostname'] == args['hostname']) or (device['ipv4'] == args['ipv4']) or (device['ipv6'] == args['ipv6']):
					revisedDevices.append(newDevice)
					successfullyFoundMatch = True
				else:
					revisedDevices.append(device)
			
			# create new Shaper.csv containing new values
			with open('Shaper.csv', 'w') as csvfile:
				wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
				wr.writerow(header_store)
				for device in revisedDevices:
					wr.writerow((device['id'], device['parentNode'], device['mac'], device['hostname'] , device['ipv4'], device['ipv6'], device['downloadMin'], device['uploadMin'], device['downloadMax'], device['uploadMax']))
		   
			return {'data': newDevice}, 200  # return data with 200 OK
		else:
			return {
                'message': f" Matching device entry not found."
            }, 404
	
	# Delete
	@auth.login_required
	def delete(self):
		devices = []
		idOnlyList = []
		ipv4onlyList = []
		ipv6onlyList = []
		hostnameOnlyList = []
		with open('Shaper.csv') as csv_file:
			csv_reader = csv.reader(csv_file, delimiter=',')
			header_store = next(csv_reader)
			for row in csv_reader:
				deviceID, parentNode, mac, hostname,ipv4, ipv6, downloadMin, uploadMin, downloadMax, uploadMax = row
				ipv4 = ipv4.strip()
				ipv6 = ipv6.strip()
				if parentNode == "":
					parentNode = "none"
				parentNode = parentNode.strip()
				thisDevice = {
				  "id": deviceID,
				  "mac": mac,
				  "parentNode": parentNode,
				  "hostname": hostname,
				  "ipv4": ipv4,
				  "ipv6": ipv6,
				  "downloadMin": int(downloadMin),
				  "uploadMin": int(uploadMin),
				  "downloadMax": int(downloadMax),
				  "uploadMax": int(uploadMax),
				  "qdisc": '',
				}
				devices.append(thisDevice)
				ipv4onlyList.append(ipv4)
				ipv6onlyList.append(ipv6)
				idOnlyList.append(deviceID)
				hostnameOnlyList.append(hostname)
		
		parser = reqparse.RequestParser()  # initialize
		
		parser.add_argument('id', required=False)
		parser.add_argument('mac', required=False)
		parser.add_argument('parentNode', required=False)
		parser.add_argument('hostname', required=False)
		parser.add_argument('ipv4', required=False)
		parser.add_argument('ipv6', required=False)
		parser.add_argument('downloadMin', required=False)
		parser.add_argument('uploadMin', required=False)
		parser.add_argument('downloadMax', required=False)
		parser.add_argument('uploadMax', required=False)
		parser.add_argument('qdisc', required=False)
		
		args = parser.parse_args()  # parse arguments to dictionary
		
		if (args['id'] in idOnlyList) or (args['ipv4'] in ipv4onlyList) or (args['ipv6'] in ipv6onlyList) or (args['hostname'] in hostnameOnlyList):
			
			successfullyFoundMatch = False
			revisedDevices = []
			for device in devices:
				if (device['id'] == args['id']) or (device['mac'] == args['mac']) or (device['hostname'] == args['hostname']) or (device['ipv4'] == args['ipv4']) or (device['ipv6'] == args['ipv6']):
					# Simply do not add device to revisedDevices
					successfullyFoundMatch = True
				else:
					revisedDevices.append(device)
			
			# create new Shaper.csv containing new values
			with open('Shaper.csv', 'w') as csvfile:
				wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
				wr.writerow(header_store)
				for device in revisedDevices:
					wr.writerow((device['id'], device['parentNode'], device['mac'], device['hostname'] , device['ipv4'], device['ipv6'], device['downloadMin'], device['uploadMin'], device['downloadMax'], device['uploadMax']))
		   
			return {
                'message': "Matching device entry successfully deleted."
            },  200  # return data with 200 OK
		else:
			return {
                'message': f" Matching device entry not found."
            }, 404

class Shaper(Resource):
	# Post
	@auth.login_required
	def post(self):
		parser = reqparse.RequestParser()  # initialize
		parser.add_argument('refresh', required=True)
		args = parser.parse_args()  # parse arguments to dictionary
		if (args['refresh'] == True):
			refreshShapers()
		return {
                'message': "Successfully refreshed LibreQoS device shaping."
            }, 200  # return data and 200 OK code

api.add_resource(Devices, '/devices')  # '/devices' is our 1st entry point
api.add_resource(Shaper, '/shaper')  # '/shaper' is our 2nd entry point

if __name__ == '__main__':
    from waitress import serve
    #app.run(debug=True) # debug mode
    serve(app, host=apiHostIP, port=apiHostPost)
