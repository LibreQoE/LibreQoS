from pythonCheck import checkPythonVersion
checkPythonVersion()
import requests
import subprocess
from ispConfig import sonar_api_url,sonar_api_key,sonar_airmax_ap_model_ids,sonar_active_status_ids,sonar_ltu_ap_model_ids,snmp_community
all_models = sonar_airmax_ap_model_ids + sonar_ltu_ap_model_ids
from integrationCommon import NetworkGraph, NetworkNode, NodeType
from multiprocessing.pool import ThreadPool

### Requirements
# snmpwalk needs to be installed. "sudo apt-get install snmp"

### Assumptions
# Network sites are in Sonar and the APs are assigned to those sites.
# The MAC address is a required and primary field for all of the radios/routers.
# There is 1 IP assigned to each AP.
# For customers, IPs are assigned to equipment and not directly to the customer. I can add that later if people actually do that.
# Service plans can be expressed as an integer in Mbps. So 20Mbps and not 20.3Mbps or something like that.
# Every customer should have a data service. I'm currently not adding them if they don't.

###Notes
# I need to deal with other types of APs in the future I'm currently testing with Prism AC Gen2s, Rocket M5s, and LTU Rockets.
# There should probably be a log file somewhere and output status to the Web UI.
# If snmp fails to get the name of the AP then it's just called "Not found via snmp" We won't see that happen unless it is able to get the connected cpe and not the name.


def sonarRequest(query,variables={}):

  r = requests.post(sonar_api_url, json={'query': query, 'variables': variables}, headers={'Authorization': 'Bearer ' + sonar_api_key}, timeout=10)
  r_json = r.json()

  # Sonar responses look like this: {"data": {"accounts": {"entities": [{"id": '1'},{"id": 2}]}}}
  # I just want to return the list so I need to find what field we're querying. 
  field = list(r_json['data'].keys())[0]
  sonar_list = r_json['data'][field]['entities']
  return sonar_list

def getActiveStatuses():
  if not sonar_active_status_ids:
    query = """query getActiveStatuses {
                account_statuses (activates_account: true) {
                  entities {
                    id
                    activates_account
                  }
                }
              }"""
    
    statuses_from_sonar = sonarRequest(query)
    status_ids = []
    for status in statuses_from_sonar:
      status_ids.append(status['id'])
    return status_ids
  else:
     return sonar_active_status_ids

# Sometimes the IP will be under the field data for an item and sometimes it will be assigned to the inventory item itself.
def findIPs(inventory_item):
  ips = []
  for ip in inventory_item['inventory_model_field_data']['entities'][0]['ip_assignments']['entities']:
    ips.append(ip['subnet'])
  for ip in inventory_item['ip_assignments']['entities']:
    ips.append(ip['subnet'])
  return ips

def getSitesAndAps():
  query = """query getSitesAndAps($pages: Paginator, $rr_ap_models: ReverseRelationFilter, $ap_models: Search){
                network_sites (paginator: $pages,reverse_relation_filters: [$rr_ap_models]) {
                  entities {
                    name
                    id
                    inventory_items (search: [$ap_models]) {
                      entities {
                        id
                        inventory_model_id
                        inventory_model_field_data {
                          entities {
                            ip_assignments {
                              entities {
                                subnet
                              }
                            }
                          }
                        }
                        ip_assignments {
                          entities {
                            subnet
                          }
                        }
                      }
                    }
                  }
                }
              }"""
  
  search_aps = []
  for ap_id in all_models:
     search_aps.append({
                      "attribute": "inventory_model_id",
                      "operator": "EQ",
                      "search_value": ap_id
                    })

  variables = {"pages": 
                {
                  "records_per_page": 5,
                  "page": 1
                },
                "rr_ap_models": {
                  "relation": "inventory_items",
                  "search": [{
                    "integer_fields": search_aps
                  }]
                },
                "ap_models": {
                  "integer_fields": search_aps
                }
              }

  sites_and_aps = sonarRequest(query,variables)
  # This should only return sites that have equipment on them that is in the list sonar_ubiquiti_ap_model_ids in ispConfig.py
  sites = []
  aps = []
  for site in sites_and_aps:
    for item in site['inventory_items']['entities']:
      ips = findIPs(item)
      if ips:
        aps.append({'parent': f"site_{site['id']}",'id': f"ap_{item['id']}", 'model': item['inventory_model_id'], 'ip': ips[0]}) # Using the first IP in the list here because each IP should only have 1 IP assigned.

    if aps: #We don't care about sites that have equipment but no IP addresses.
      sites.append({'id': f"site_{site['id']}", 'name': site['name']})
  return sites, aps

def getAccounts(sonar_active_status_ids):
  query = """query getAccounts ($pages: Paginator, $account_search: Search, $data: ReverseRelationFilter,$primary: ReverseRelationFilter) {
                accounts (paginator: $pages,search: [$account_search]) {
                  entities {
                    account_status_id
                    id
                    name
                    account_services (reverse_relation_filters: [$data]) {
                      entities {
                        service {
                          data_service_detail {
                            download_speed_kilobits_per_second
                            upload_speed_kilobits_per_second
                          }
                        }
                      }
                    }
                    addresses {
                      entities {
                        line1
                        line2
                        city
                        subdivision
                        inventory_items {
                          entities {
                            id
                            inventory_model {
                            name
                            }
                            inventory_model_field_data (reverse_relation_filters: [$primary]) {
                              entities {
                                value
                                ip_assignments {
                                  entities {
                                    subnet
                                  }
                                }
                              }
                            }
                            ip_assignments {
                              entities {
                                subnet
                              }
                            }
                          }
                        }
                      }
                    }
                  }
                }
              }"""
  
  active_status_ids = []
  for status_id in sonar_active_status_ids:
     active_status_ids.append({
                      "attribute": "account_status_id",
                      "operator": "EQ",
                      "search_value": status_id
                    })
  
  variables = {"pages": 
                {
                  "records_per_page": 5,
                  "page": 1
                },
                "account_search": {
                  "integer_fields": active_status_ids
                },
                "data": {
                  "relation": "service",
                  "search": [{
                    "string_fields": [{
                      "attribute": "type",
                      "match": True,
                      "search_value": "DATA"
                    }]
                  }]
                },
                "primary": {
                  "relation": "inventory_model_field",
                  "search": [{
                    "boolean_fields": [{
                      "attribute": "primary",
                      "search_value": True
                      }]
                    }]
                  }
              }
  
  accounts_from_sonar = sonarRequest(query,variables)
  accounts = []
  for account in accounts_from_sonar:
    # We need to make sure the account has an address because Sonar assignments go account -> address (only 1 per account) -> equipment -> ip assignments unless the IP is assigned to the account directly.
    if account['addresses']['entities']:
      line1 = account['addresses']['entities'][0]['line1']
      line2 = account['addresses']['entities'][0]['line2']
      city = account['addresses']['entities'][0]['city']
      state = account['addresses']['entities'][0]['subdivision'][-2:]
      address = f"{line1},{f' {line2},' if line2 else ''} {city}, {state}"
      devices = []
      for item in account['addresses']['entities'][0]['inventory_items']['entities']:
        devices.append({'id': item['id'], 'name': item['inventory_model']['name'], 'ips': findIPs(item), 'mac': item['inventory_model_field_data']['entities'][0]['value']})
      if account['account_services']['entities'] and devices: # Make sure there is a data plan and devices on the account.
        download = int(account['account_services']['entities'][0]['service']['data_service_detail']['download_speed_kilobits_per_second']/1000)
        upload = int(account['account_services']['entities'][0]['service']['data_service_detail']['upload_speed_kilobits_per_second']/1000)
        if download < 2:
           download = 2
        if upload < 2:
           upload = 2
        accounts.append({'id': account['id'],'name': account['name'], 'address': address, 'download': download, 'upload': upload ,'devices': devices})
  return accounts

def mapApCpeMacs(ap):
    macs = []
    macs_output = None
    if ap['model'] in sonar_airmax_ap_model_ids: #Tested with Prism Gen2AC and Rocket M5.
      macs_output = subprocess.run(['snmpwalk', '-Os', '-v', '1', '-c', snmp_community, ap['ip'], '.1.3.6.1.4.1.41112.1.4.7.1.1.1'], capture_output=True).stdout.decode('utf8')
    if ap['model'] in sonar_ltu_ap_model_ids: #Tested with LTU Rocket
      macs_output = subprocess.run(['snmpwalk', '-Os', '-v', '1', '-c', snmp_community, ap['ip'], '.1.3.6.1.4.1.41112.1.10.1.4.1.11'], capture_output=True).stdout.decode('utf8')
    if macs_output:
      name_output  = subprocess.run(['snmpwalk', '-Os', '-v', '1', '-c', snmp_community, ap['ip'], '.1.3.6.1.2.1.1.5.0'], capture_output=True).stdout.decode('utf8')
      ap['name'] = name_output[name_output.find('"')+1:name_output.rfind('"')]
      for mac_line in macs_output.splitlines():
         mac = mac_line[mac_line.find(':')+1:]
         mac = mac.strip().replace(' ',':')
         macs.append(mac)
    else:
      ap['name'] = 'Not found via snmp'
    ap['cpe_macs'] = macs
    
    return ap

def mapMacAP(mac,aps):
  for ap in aps:
    for cpe_mac in ap['cpe_macs']:
        if cpe_mac == mac:
          return ap['id']
  return None

def createShaper():
  net = NetworkGraph()

  #Get active statuses from Sonar if necessary.
  sonar_active_status_ids = getActiveStatuses()

  # Get Network Sites and Access Points from Sonar.
  sites, aps = getSitesAndAps()

  # Get Customer equipment and IPs.
  accounts = getAccounts(sonar_active_status_ids)

  # Get CPE macs on each AP.
  pool = ThreadPool(30) #30 is arbitrary at the moment.
  for i, ap in enumerate(pool.map(mapApCpeMacs, aps)):
      ap = aps[i]
  pool.close()
  pool.join()

  # Update customers with the AP to which they are connected.
  for account in accounts:
    for device in account['devices']:
        account['parent'] = mapMacAP(device['mac'],aps)
        if account['parent']:
          break

  for site in sites:
      net.addRawNode(NetworkNode(id=site['id'], displayName=site['name'], parentId="", type=NodeType.site))

  for ap in aps:
    if ap['cpe_macs']: # I don't think we care about Aps with no customers.
      net.addRawNode(NetworkNode(id=ap['id'], displayName=ap['name'], parentId=ap['parent'], type=NodeType.ap))

  for account in accounts:
    if account['parent']:
      customer = NetworkNode(id=account['id'],displayName=account['name'],parentId=account['parent'],type=NodeType.client,download=account['download'], upload=account['upload'], address=account['address'])
    else:
      customer = NetworkNode(id=account['id'],displayName=account['name'],type=NodeType.client,download=account['download'], upload=account['upload'], address=account['address'])
    net.addRawNode(customer)

    for device in account['devices']:
      libre_device = NetworkNode(id=device['id'], displayName=device['name'],parentId=account['id'],type=NodeType.device, ipv4=device['ips'],ipv6=[],mac=device['mac'])
      net.addRawNode(libre_device)

  net.prepareTree()
  net.plotNetworkGraph(False)
  net.createNetworkJson()
  net.createShapedDevices()

def importFromSonar():
	createShaper()

if __name__ == '__main__':
	importFromSonar()
