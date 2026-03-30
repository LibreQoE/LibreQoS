from pythonCheck import checkPythonVersion
checkPythonVersion()
import requests
import subprocess
from urllib.parse import urlsplit, urlunsplit
from liblqos_python import sonar_api_key, sonar_api_url, snmp_community, sonar_airmax_ap_model_ids, \
  sonar_ltu_ap_model_ids, sonar_active_status_ids, sonar_recurring_service_rates, \
  sonar_recurring_excluded_service_names
all_models = sonar_airmax_ap_model_ids() + sonar_ltu_ap_model_ids()
from integrationCommon import NetworkGraph, NetworkNode, NodeType, apply_client_bandwidth_multiplier
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

_SONAR_SESSION = None


def sonarGraphqlUrl():
  raw_url = sonar_api_url().strip()
  if not raw_url:
    return raw_url

  parsed = urlsplit(raw_url)
  path = parsed.path.rstrip('/')

  if path.endswith('/api/graphql'):
    normalized_path = path
  elif path in ('', '/'):
    normalized_path = '/api/graphql'
  else:
    normalized_path = path + '/api/graphql'

  return urlunsplit((parsed.scheme, parsed.netloc, normalized_path, parsed.query, parsed.fragment))


def sonarAccountNodeId(account_id):
  return f"sonar:account:{account_id}"


def sonarDeviceNodeId(device_id):
  return f"sonar:device:{device_id}"


def sonarRadiusAccountDeviceNodeId(radius_account_id):
  return f"sonar:radius-account:{radius_account_id}"


def sonarSession():
  global _SONAR_SESSION
  if _SONAR_SESSION is None:
    _SONAR_SESSION = requests.Session()
    _SONAR_SESSION.headers.update({
      'Authorization': 'Bearer ' + sonar_api_key()
    })
  return _SONAR_SESSION


def sonarRequest(query, variables=None):
  if variables is None:
    variables = {}

  graphql_url = sonarGraphqlUrl()
  r = sonarSession().post(
    graphql_url,
    json={'query': query, 'variables': variables},
    timeout=10
  )
  try:
    r_json = r.json()
  except requests.exceptions.JSONDecodeError as e:
    body_preview = r.text[:300].strip()
    raise RuntimeError(
      f"Sonar API at {graphql_url} returned non-JSON content "
      f"(status={r.status_code}, content-type={r.headers.get('content-type')}, body-preview={body_preview!r})"
    ) from e

  # Sonar responses look like this: {"data": {"accounts": {"entities": [{"id": '1'},{"id": 2}]}}}
  # I just want to return the list so I need to find what field we're querying. 
  if 'errors' in r_json:
    raise RuntimeError(f"Sonar GraphQL error from {graphql_url}: {r_json['errors']}")
  if 'data' not in r_json:
    raise RuntimeError(f"Sonar API response from {graphql_url} missing 'data': {r_json}")
  field = list(r_json['data'].keys())[0]
  sonar_list = r_json['data'][field]['entities']
  return sonar_list


def sonarPaginatedRequest(query, variables=None, paginator_key='pages', records_per_page=100):
  if variables is None:
    variables = {}

  session = sonarSession()
  graphql_url = sonarGraphqlUrl()
  page = 1
  entities = []

  while True:
    request_variables = dict(variables)
    request_variables[paginator_key] = {
      'records_per_page': records_per_page,
      'page': page
    }
    r = session.post(
      graphql_url,
      json={'query': query, 'variables': request_variables},
      timeout=20
    )
    try:
      r_json = r.json()
    except requests.exceptions.JSONDecodeError as e:
      body_preview = r.text[:300].strip()
      raise RuntimeError(
        f"Sonar API at {graphql_url} returned non-JSON content "
        f"(status={r.status_code}, content-type={r.headers.get('content-type')}, body-preview={body_preview!r})"
      ) from e

    if 'errors' in r_json:
      raise RuntimeError(f"Sonar GraphQL error from {graphql_url}: {r_json['errors']}")
    if 'data' not in r_json:
      raise RuntimeError(f"Sonar API response from {graphql_url} missing 'data': {r_json}")

    field = list(r_json['data'].keys())[0]
    connection = r_json['data'][field]
    entities.extend(connection['entities'])
    page_info = connection.get('page_info') or {}
    total_pages = page_info.get('total_pages', page)
    if page >= total_pages:
      break
    page += 1

  return entities

def getActiveStatuses():
  if not sonar_active_status_ids():
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
     return sonar_active_status_ids()

# Sometimes the IP will be under the field data for an item and sometimes it will be assigned to the inventory item itself.
def findIPs(inventory_item):
  ips = []
  for field_data in inventory_item.get('inventory_model_field_data', {}).get('entities', []):
    for ip in field_data.get('ip_assignments', {}).get('entities', []):
      ips.append(ip['subnet'])
  for ip in inventory_item.get('ip_assignments', {}).get('entities', []):
    ips.append(ip['subnet'])
  return ips


def dedupeSubnets(subnets):
  deduped = []
  seen = set()
  for subnet in subnets:
    if not subnet or subnet in seen:
      continue
    seen.add(subnet)
    deduped.append(subnet)
  return deduped


def normalizeServiceName(name):
  return (name or "").strip().casefold()


def findRadiusAccountIPs(radius_account):
  ips = []
  for ip in radius_account.get('ip_assignments', {}).get('entities', []):
    subnet = ip.get('subnet')
    if subnet:
      ips.append(subnet)
  return dedupeSubnets(ips)


def recurringServiceRateRules():
  rules = {}
  for enabled, service_name, download_mbps, upload_mbps in sonar_recurring_service_rates():
    normalized_name = normalizeServiceName(service_name)
    if not enabled or not normalized_name:
      continue
    rules[normalized_name] = (
      float(download_mbps),
      float(upload_mbps),
    )
  return rules


def recurringServiceExclusions():
  return {
    normalizeServiceName(name)
    for name in sonar_recurring_excluded_service_names()
    if normalizeServiceName(name)
  }


def resolveAccountBandwidth(account, recurring_rules=None, excluded_names=None):
  account_services = account.get('account_services', {}).get('entities', [])
  if account_services:
    service = account_services[0].get('service') or {}
    service_detail = service.get('data_service_detail')
    if service_detail:
      return (
        float(service_detail['download_speed_kilobits_per_second']) / 1000,
        float(service_detail['upload_speed_kilobits_per_second']) / 1000,
      )

  if recurring_rules is None:
    recurring_rules = recurringServiceRateRules()
  if not recurring_rules:
    return None
  if excluded_names is None:
    excluded_names = recurringServiceExclusions()
  all_services = account.get('all_account_services', {}).get('entities', [])
  for row in all_services:
    service = row.get('service') or {}
    if service.get('type') != 'RECURRING':
      continue
    normalized_name = normalizeServiceName(service.get('name'))
    if not normalized_name or normalized_name in excluded_names:
      continue
    if normalized_name in recurring_rules:
      return recurring_rules[normalized_name]
  return None


def findPrimaryMac(inventory_item):
  field_data_entities = inventory_item.get('inventory_model_field_data', {}).get('entities', [])

  def mac_candidates(primary_only):
    for field_data in field_data_entities:
      field = field_data.get('inventory_model_field') or {}
      value = (field_data.get('value') or '').strip()
      field_name = (field.get('name') or '').strip().lower()
      if not value:
        continue
      if 'mac' not in field_name:
        continue
      if primary_only and not field.get('primary', False):
        continue
      yield value

  for value in mac_candidates(primary_only=True):
    return value
  for value in mac_candidates(primary_only=False):
    return value
  for field_data in field_data_entities:
    value = (field_data.get('value') or '').strip()
    if value:
      return value
  return ""


def formatAccountAddress(account):
  addresses = account.get('addresses', {}).get('entities', [])
  if not addresses:
    return ""
  primary_address = addresses[0]
  line1 = primary_address.get('line1') or ""
  line2 = primary_address.get('line2') or ""
  city = primary_address.get('city') or ""
  subdivision = primary_address.get('subdivision') or ""
  state = subdivision[-2:] if subdivision else ""

  address = f"{line1},{f' {line2},' if line2 else ''} {city}, {state}".strip()
  return address.strip(', ')

def getSitesAndAps():
  query = """query getSitesAndAps($pages: Paginator, $rr_ap_models: ReverseRelationFilter, $ap_models: Search){
                network_sites (paginator: $pages,reverse_relation_filters: [$rr_ap_models]) {
                  page_info {
                    page
                    records_per_page
                    total_pages
                    total_count
                  }
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

  variables = {"rr_ap_models": {
                  "relation": "inventory_items",
                  "search": [{
                    "integer_fields": search_aps
                  }]
                },
                "ap_models": {
                  "integer_fields": search_aps
                }
              }

  sites_and_aps = sonarPaginatedRequest(query,variables)
  # This should only return sites that have equipment on them that is in the list sonar_ubiquiti_ap_model_ids in lqos.conf
  sites = []
  aps = []
  for site in sites_and_aps:
    site_has_ap = False
    for item in site['inventory_items']['entities']:
      ips = findIPs(item)
      if ips:
        aps.append({'parent': f"site_{site['id']}",'id': f"ap_{item['id']}", 'raw_id': item['id'], 'model': item['inventory_model_id'], 'ip': ips[0]}) # Using the first IP in the list here because each IP should only have 1 IP assigned.
        site_has_ap = True

    if site_has_ap: #We don't care about sites that have equipment but no IP addresses.
      sites.append({'id': f"site_{site['id']}", 'raw_id': site['id'], 'name': site['name']})
  return sites, aps


def buildAccountDevices(account):
  devices = []
  known_ip_subnets = set()

  for address in account.get('addresses', {}).get('entities', []):
    for item in address.get('inventory_items', {}).get('entities', []):
      ips = dedupeSubnets(findIPs(item))
      known_ip_subnets.update(ips)
      devices.append({
        'id': sonarDeviceNodeId(item['id']),
        'raw_id': item['id'],
        'name': item['inventory_model']['name'],
        'ips': ips,
        'mac': findPrimaryMac(item)
      })

  for radius_account in account.get('radius_accounts', {}).get('entities', []):
    radius_ips = [
      subnet for subnet in findRadiusAccountIPs(radius_account)
      if subnet not in known_ip_subnets
    ]
    if not radius_ips:
      continue
    known_ip_subnets.update(radius_ips)
    devices.append({
      'id': sonarRadiusAccountDeviceNodeId(radius_account['id']),
      'raw_id': radius_account['id'],
      'name': f"Radius Account {radius_account['id']}",
      'ips': radius_ips,
      'mac': ''
    })

  return devices


def buildAccountRecord(account, fallback_address="", recurring_rules=None, excluded_names=None):
  address = formatAccountAddress(account) or fallback_address
  if not address:
    return None

  devices = buildAccountDevices(account)
  if not devices:
    return None

  bandwidth = resolveAccountBandwidth(
    account,
    recurring_rules=recurring_rules,
    excluded_names=excluded_names,
  )
  if bandwidth is None:
    return None

  download_raw, upload_raw = bandwidth
  download = apply_client_bandwidth_multiplier(download_raw)
  upload = apply_client_bandwidth_multiplier(upload_raw)
  if download < 2:
     download = 2
  if upload < 2:
     upload = 2

  return {
    'id': sonarAccountNodeId(account['id']),
    'raw_id': account['id'],
    'name': account['name'],
    'address': address,
    'download': download,
    'upload': upload,
    'devices': devices
  }


def buildAccountsFromSonarEntities(accounts_from_sonar, recurring_rules=None, excluded_names=None):
  accounts = []
  seen_account_ids = set()
  child_candidates = []

  for account in accounts_from_sonar:
    account_record = buildAccountRecord(
      account,
      recurring_rules=recurring_rules,
      excluded_names=excluded_names,
    )
    if account_record:
      accounts.append(account_record)
      seen_account_ids.add(account_record['raw_id'])

    parent_address = formatAccountAddress(account)
    for child_account in account.get('child_accounts', {}).get('entities', []):
      child_candidates.append((child_account, parent_address))

  for child_account, parent_address in child_candidates:
    child_id = child_account['id']
    if child_id in seen_account_ids:
      continue
    child_record = buildAccountRecord(
      child_account,
      fallback_address=parent_address,
      recurring_rules=recurring_rules,
      excluded_names=excluded_names,
    )
    if child_record:
      accounts.append(child_record)
      seen_account_ids.add(child_record['raw_id'])

  return accounts


def getAccounts(sonar_active_status_ids):
  query = """query getAccounts ($pages: Paginator, $account_search: Search, $data: ReverseRelationFilter) {
                accounts (paginator: $pages,search: [$account_search]) {
                  page_info {
                    page
                    records_per_page
                    total_pages
                    total_count
                  }
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
                            inventory_model_field_data {
                              entities {
                                value
                                ip_assignments {
                                  entities {
                                    subnet
                                  }
                                }
                                inventory_model_field {
                                  id
                                  name
                                  primary
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
                    radius_accounts {
                      entities {
                        id
                        ip_assignments {
                          entities {
                            subnet
                          }
                        }
                      }
                    }
                    child_accounts {
                      entities {
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
                                inventory_model_field_data {
                                  entities {
                                    value
                                    ip_assignments {
                                      entities {
                                        subnet
                                      }
                                    }
                                    inventory_model_field {
                                      id
                                      name
                                      primary
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
                        radius_accounts {
                          entities {
                            id
                            ip_assignments {
                              entities {
                                subnet
                              }
                            }
                          }
                        }
                        all_account_services: account_services {
                          entities {
                            service(enabled: true) {
                              id
                              name
                              type
                              enabled
                              data_service_detail {
                                download_speed_kilobits_per_second
                                upload_speed_kilobits_per_second
                              }
                            }
                          }
                        }
                      }
                    }
                    all_account_services: account_services {
                      entities {
                        service(enabled: true) {
                          id
                          name
                          type
                          enabled
                          data_service_detail {
                            download_speed_kilobits_per_second
                            upload_speed_kilobits_per_second
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
  
  variables = {"account_search": {
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
                }
              }
  
  accounts_from_sonar = sonarPaginatedRequest(query,variables)
  recurring_rules = recurringServiceRateRules()
  excluded_names = recurringServiceExclusions() if recurring_rules else set()
  return buildAccountsFromSonarEntities(
    accounts_from_sonar,
    recurring_rules=recurring_rules,
    excluded_names=excluded_names,
  )

def mapApCpeMacs(ap):
    macs = []
    macs_output = None
    if ap['model'] in sonar_airmax_ap_model_ids(): #Tested with Prism Gen2AC and Rocket M5.
      macs_output = subprocess.run(['snmpwalk', '-Os', '-v', '1', '-c', snmp_community(), ap['ip'], '.1.3.6.1.4.1.41112.1.4.7.1.1.1'], capture_output=True).stdout.decode('utf8')
    if ap['model'] in sonar_ltu_ap_model_ids(): #Tested with LTU Rocket
      macs_output = subprocess.run(['snmpwalk', '-Os', '-v', '1', '-c', snmp_community(), ap['ip'], '.1.3.6.1.4.1.41112.1.10.1.4.1.11'], capture_output=True).stdout.decode('utf8')
    if macs_output:
      name_output  = subprocess.run(['snmpwalk', '-Os', '-v', '1', '-c', snmp_community(), ap['ip'], '.1.3.6.1.2.1.1.5.0'], capture_output=True).stdout.decode('utf8')
      ap['name'] = name_output[name_output.find('"')+1:name_output.rfind('"')]
      for mac_line in macs_output.splitlines():
         mac = mac_line[mac_line.find(':')+1:]
         mac = mac.strip().replace(' ',':')
         macs.append(mac)
    else:
      ap['name'] = 'Not found via snmp'
    ap['cpe_macs'] = macs
    
    return ap

def buildMacToApMap(aps):
  mac_to_ap = {}
  for ap in aps:
    for cpe_mac in ap['cpe_macs']:
      mac_to_ap[cpe_mac] = ap['id']
  return mac_to_ap

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

  mac_to_ap = buildMacToApMap(aps)

  # Update customers with the AP to which they are connected.
  for account in accounts:
    for device in account['devices']:
        account['parent'] = mac_to_ap.get(device['mac'])
        if account['parent']:
          break

  for site in sites:
      net.addRawNode(NetworkNode(id=site['id'], displayName=site['name'], parentId="", type=NodeType.site, networkJsonId=f"sonar:site:{site['raw_id']}"))

  for ap in aps:
    if ap['cpe_macs']: # I don't think we care about Aps with no customers.
      net.addRawNode(NetworkNode(id=ap['id'], displayName=ap['name'], parentId=ap['parent'], type=NodeType.ap, networkJsonId=f"sonar:ap:{ap['raw_id']}"))

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
