print("Deprecated for now.")

# import csv
# import os
# import shutil
# from datetime import datetime

# from requests import get

# from ispConfig import automaticImportRestHttp as restconf
# from pydash import objects

# requestsBaseConfig = {
#     'verify': True,
#     'headers': {
#         'accept': 'application/json'
#     }
# }


# def createShaper():

#     # shutil.copy('Shaper.csv', 'Shaper.csv.bak')
#     ts = datetime.now().strftime('%Y-%m-%d.%H-%M-%S')

#     devicesURL = restconf.get('baseURL') + '/' + restconf.get('devicesURI').strip('/')

#     requestConfig = objects.defaults_deep({'params': {}}, restconf.get('requestsConfig'), requestsBaseConfig)

#     raw = get(devicesURL, **requestConfig, timeout=10)

#     if raw.status_code != 200:
#         print('Failed to request ' + devicesURL + ', got ' + str(raw.status_code))
#         return False

#     devicesCsvFP = os.path.dirname(os.path.realpath(__file__)) + '/ShapedDevices.csv'

#     with open(devicesCsvFP, 'w') as csvfile:
#         wr = csv.writer(csvfile, quoting=csv.QUOTE_ALL)
#         wr.writerow(
#             ['Circuit ID', 'Circuit Name', 'Device ID', 'Device Name', 'Parent Node', 'MAC', 'IPv4', 'IPv6',
#                 'Download Min Mbps', 'Upload Min Mbps', 'Download Max Mbps', 'Upload Max Mbps', 'Comment'])
#         for row in raw.json():
#             wr.writerow(row.values())

#     if restconf['logChanges']:
#         devicesBakFilePath = restconf['logChanges'].rstrip('/') + '/ShapedDevices.' + ts + '.csv'
#         try:
#             shutil.copy(devicesCsvFP, devicesBakFilePath)
#         except:
#             os.makedirs(restconf['logChanges'], exist_ok=True)
#             shutil.copy(devicesCsvFP, devicesBakFilePath)

#     networkURL = restconf['baseURL'] + '/' + restconf['networkURI'].strip('/')

#     raw = get(networkURL, **requestConfig, timeout=10)

#     if raw.status_code != 200:
#         print('Failed to request ' + networkURL + ', got ' + str(raw.status_code))
#         return False

#     networkJsonFP = os.path.dirname(os.path.realpath(__file__)) + '/network.json'

#     with open(networkJsonFP, 'w') as handler:
#         handler.write(raw.text)

#     if restconf['logChanges']:
#         networkBakFilePath = restconf['logChanges'].rstrip('/') + '/network.' + ts + '.json'
#         try:
#             shutil.copy(networkJsonFP, networkBakFilePath)
#         except:
#             os.makedirs(restconf['logChanges'], exist_ok=True)
#             shutil.copy(networkJsonFP, networkBakFilePath)


# def importFromRestHttp():
#     createShaper()


# if __name__ == '__main__':
#     importFromRestHttp()
