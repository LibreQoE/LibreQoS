#!/usr/bin/python3
import ispConfig
import json

# Function that tries to retrieve a value from ispConfig and returns it if it exists, otherwise returns a default value
def getIspConfigValue(value, defaultValue):
    try:
        return getattr(ispConfig, value)
    except AttributeError:
        return defaultValue

oldConfig = {
    'sqm': getIspConfigValue('sqm', 'cake diffserv4'),
    'monitorOnlyMode': getIspConfigValue('monitorOnlyMode', False),
    'upstreamBandwidthCapacityDownloadMbps': getIspConfigValue('upstreamBandwidthCapacityDownloadMbps', 1000),
    'upstreamBandwidthCapacityUploadMbps': getIspConfigValue('upstreamBandwidthCapacityUploadMbps', 1000),
    'generatedPNDownloadMbps': getIspConfigValue('generatedPNDownloadMbps', 1000),
    'generatedPNUploadMbps': getIspConfigValue('generatedPNUploadMbps', 1000),
    'interfaceA': getIspConfigValue('interfaceA', 'veth_tointernal'),
    'interfaceB': getIspConfigValue('interfaceB', 'veth_toexternal'),
    'queueRefreshIntervalMins': getIspConfigValue('queueRefreshIntervalMins', 30),
    'OnAStick': getIspConfigValue('OnAStick', False),
    'StickVlanA': getIspConfigValue('StickVlanA', 0),
    'StickVlanB': getIspConfigValue('StickVlanB', 0),
    'enableActualShellCommands': getIspConfigValue('enableActualShellCommands', True),
    'runShellCommandsAsSudo': getIspConfigValue('runShellCommandsAsSudo', False),
    'queuesAvailableOverride': getIspConfigValue('queuesAvailableOverride', 0),
    'useBinPackingToBalanceCPU': getIspConfigValue('useBinPackingToBalanceCPU', False),

    # Influx
    'influxEnabled': getIspConfigValue('influxDBEnabled', False),
    'influxDBurl': getIspConfigValue('influxDBurl', 'http://localhost:8086'),
    'influxDBBucket': getIspConfigValue('influxDBBucket', 'libreqos'),
    'influxDBOrg': getIspConfigValue('influxDBOrg', 'libreqos'),
    'influxDBtoken': getIspConfigValue('influxDBtoken', ''),

    # Common
    'circuitNameUseAddress': getIspConfigValue('circuitNameUseAddress', True),
    'overwriteNetworkJSONalways': getIspConfigValue('overwriteNetworkJSONalways', False),
    'ignoreSubnets': getIspConfigValue('ignoreSubnets', ["192.168.0.0/16"]),
    'allowedSubnets': getIspConfigValue('allowedSubnets', ["100.64.0.0/10"]),
    'excludeSites': getIspConfigValue('excludeSites', []),
    'findIPv6usingMikrotikAPI': getIspConfigValue('findIPv6usingMikrotikAPI', False),

    # Splynx
    'automaticImportSplynx': getIspConfigValue('automaticImportSplynx', False),
    'splynx_api_key': getIspConfigValue('splynx_api_key', ''),
    'splynx_api_secret': getIspConfigValue('splynx_api_secret', ''),
    'splynx_api_url': getIspConfigValue('splynx_api_url', 'https://splynx.example.com/api/v1/'),

    # UISP
    'automaticImportUISP': getIspConfigValue('automaticImportUISP', False),
    'uispAuthToken': getIspConfigValue('uispAuthToken', ''),
    'UISPbaseURL': getIspConfigValue('UISPbaseURL', 'https://unms.example.com'),
    'uispSite': getIspConfigValue('uispSite', 'Main Site'),
    'uispStrategy': getIspConfigValue('uispStrategy', 'full'),
    'uispSuspendedStrategy': getIspConfigValue('uispSuspendedStrategy', 'none'),
    'airMax_capacity': getIspConfigValue('airMax_capacity', 0.65),
    'ltu_capacity': getIspConfigValue('ltu_capacity', 0.90),
    'bandwidthOverheadFactor': getIspConfigValue('bandwidthOverheadFactor', 1.0),
    'committedBandwidthMultiplier': getIspConfigValue('committedBandwidthMultiplier', 0.98),
    'exceptionCPEs': getIspConfigValue('exceptionCPEs', {}),

    # API
    'apiUsername': getIspConfigValue('apiUsername', 'testUser'),
    'apiPassword': getIspConfigValue('apiPassword', 'testPassword'),
    'apiHostIP': getIspConfigValue('apiHostIP', '127.0.0.1'),
    'apiHostPost': getIspConfigValue('apiHostPost', 5000),

    # Powercode
    'automaticImportPowercode': getIspConfigValue('automaticImportPowercode', False),
    'powercode_api_key': getIspConfigValue('powercode_api_key', ''),
    'powercode_api_url': getIspConfigValue('powercode_api_url', 'https://powercode.example.com/api/v1/'),

    # Sonar
    'automaticImportSonar': getIspConfigValue('automaticImportSonar', False),
    'sonar_api_key': getIspConfigValue('sonar_api_key', ''),
    'sonar_api_url': getIspConfigValue('sonar_api_url', 'https://sonar.example.com/api/v1/'),
    'snmp_community': getIspConfigValue('snmp_community', 'public'),
    'sonar_active_status_ids': getIspConfigValue('sonar_active_status_ids', []),
    'sonar_airmax_ap_model_ids': getIspConfigValue('sonar_airmax_ap_model_ids', []),
    'sonar_ltu_ap_model_ids': getIspConfigValue('sonar_ltu_ap_model_ids', []),
}

print(json.dumps(oldConfig))
