# 'fq_codel' or 'cake diffserv4'
# 'cake diffserv4' is recommended

# fqOrCAKE = 'fq_codel'
fqOrCAKE = 'cake diffserv4'

# How many Mbps are available to the edge of this network
upstreamBandwidthCapacityDownloadMbps = 1000
upstreamBandwidthCapacityUploadMbps = 1000

# Traffic from devices not specified in Shaper.csv will be rate limited by an HTB of this many Mbps
defaultClassCapacityDownloadMbps = 500
defaultClassCapacityUploadMbps = 500

# Interface connected to core router
interfaceA = 'eth1'

# Interface connected to edge router
interfaceB = 'eth2'

# Shape by Site in addition to by AP and Client
# Now deprecated, was only used prior to v1.1
# shapeBySite = True

# Allow shell commands. False causes commands print to console only without being executed. MUST BE ENABLED FOR
# PROGRAM TO FUNCTION
enableActualShellCommands = True

# Add 'sudo' before execution of any shell commands. May be required depending on distribution and environment.
runShellCommandsAsSudo = False

# Graphing
graphingEnabled = True
ppingLocation = "pping"
influxDBurl = "http://localhost:8086"
influxDBBucket = "libreqos"
influxDBOrg = "Your ISP Name Here"
influxDBtoken = ""

# NMS/CRM Integration
# If a device shows a WAN IP within these subnets, assume they are behind NAT / un-shapable, and ignore them
ignoreSubnets = ['192.168.0.0/16']

# Optional UISP integration
automaticImportUISP = False
# Everything before /nms/ on your UISP instance
uispBaseURL = 'https://examplesite.com'
# UISP Auth Token
uispAuthToken = ''
# UISP | Whether to shape router at customer premises, or instead shape the station radio. When station radio is in
# router mode, use 'station'. Otherwise, use 'router'.
shapeRouterOrStation = 'router'

# API Auth
apiUsername = "testUser"
apiPassword = "changeme8343486806"
apiHostIP = "127.0.0.1"
apiHostPost = 5000

httpAPIConfig = {
    'enabled': False,
    'baseURL': 'https://my.api.domain.tld',
    'networkURI': '/api/path/network-json-data',
    'devicesURI': '/api/path/devices-json-path',
    'devicesRemap': [],  # if your devices json data aint perfect, you can remap keys for csv cols
    'requestsConfig': {
        # 'verify': False,  # Good for Dev if your dev env doesnt have cert
        # 'params': {  # params for query string ie uri?some-arg=some-value
        #   'some-arg': 'some-value'
        # },
        'headers': {
            'some-header': 'some-value',  # ie simple api keys etc
        },
    },
    # If you want to store a timestamped copy/backup of both network.json and Shaper.csv each time they are updated,
    # provide a path
    # TODO Figure out how to expire old backups as not to exhaust disk space
    'logChanges': False  # or '/var/log/libreqos' etc
}
