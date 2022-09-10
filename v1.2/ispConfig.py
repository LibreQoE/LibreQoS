# 'fq_codel' or 'cake diffserv4'
# 'cake diffserv4' is recommended

# fqOrCAKE = 'fq_codel'
fqOrCAKE = 'cake diffserv4'

# How many Mbps are available to the edge of this network
upstreamBandwidthCapacityDownloadMbps = 1000
upstreamBandwidthCapacityUploadMbps = 1000

# Devices in ShapedDevices.csv without a defined ParentNode will be placed under a generated
# parent node, evenly spread out across CPU cores. Here, define the bandwidth limit for each
# of those generated parent nodes.
generatedPNDownloadMbps = 1000
generatedPNUploadMbps = 1000

# Traffic from devices not specified in Shaper.csv will be rate limited by an HTB of this many Mbps
defaultClassCapacityDownloadMbps = 500
defaultClassCapacityUploadMbps = 500

# Interface connected to core router
interfaceA = 'eth1'

# Interface connected to edge router
interfaceB = 'eth2'

# Use XDP? If yes, multiple CPU cores can be used. Limits to IPv4 only. Throughput of 11 Gbps+
# If using IPv6, choose False. False will limit throughput to 3-6 Gbps
usingXDP = True

# Allow shell commands. False causes commands print to console only without being executed.
# MUST BE ENABLED FOR PROGRAM TO FUNCTION
enableActualShellCommands = True

# Add 'sudo' before execution of any shell commands. May be required depending on distribution and environment.
runShellCommandsAsSudo = False

# Bandwidth Graphing
bandwidthGraphingEnabled = True
influxDBurl = "http://localhost:8086"
influxDBBucket = "libreqos"
influxDBOrg = "Your ISP Name Here"
influxDBtoken = ""

# Latency Graphing
latencyGraphingEnabled = False
ppingLocation = "pping"

# NMS/CRM Integration
# If a device shows a WAN IP within these subnets, assume they are behind NAT / un-shapable, and ignore them
ignoreSubnets = ['192.168.0.0/16']
allowedSubnets = ['100.64.0.0/10']
# Optional UISP integration
automaticImportUISP = False
# Everything before /nms/ on your UISP instance
uispBaseURL = 'https://examplesite.com'
# UISP Auth Token
uispAuthToken = ''
# UISP | Whether to shape router at customer premises, or instead shape the station radio. When station radio is in
# router mode, use 'station'. Otherwise, use 'router'.
shapeRouterOrStation = 'router'
# List any sites that should not be included, with each site name surrounded by '' and seperated by commas
excludeSites = []
# If you use IPv6, this can be used to find associated IPv6 prefixes for your clients' IPv4 addresses, and match them to those devices
findIPv6usingMikrotik = False
# If you want to provide a safe cushion for speed test results to prevent customer complains, you can set this to 1.15 (15% above plan rate).
# If not, you can leave as 1.0
bandwidthOverheadFactor = 1.0

# API Auth
apiUsername = "testUser"
apiPassword = "changeme8343486806"
apiHostIP = "127.0.0.1"
apiHostPost = 5000
