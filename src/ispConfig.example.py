# 'fq_codel' or 'cake diffserv4'
# 'cake diffserv4' is recommended
# sqm = 'fq_codel'
sqm = 'cake diffserv4'

# Used to passively monitor the network for before / after comparisons. Leave as False to
# ensure actual shaping. After changing this value, run "sudo systemctl restart LibreQoS.service"
monitorOnlyMode = False

# How many Mbps are available to the edge of this network.
# Any circuits, generated nodes, or network.json nodes, will all be capped at no more than this amount.
upstreamBandwidthCapacityDownloadMbps = 1000
upstreamBandwidthCapacityUploadMbps = 1000

# Consider these values your bandwidth bottleneck per-CPU-core.
# This will depend on your CPU's single-thread passmark score.
# Devices in ShapedDevices.csv without a defined ParentNode (such as if you have a flat {} network)
# will be placed under one of these generated parent node, evenly spread out across CPU cores.
# This defines the bandwidth limit for each of those generated parent nodes.
generatedPNDownloadMbps = 1000
generatedPNUploadMbps = 1000

# Interface connected to core router
interfaceA = 'eth1'

# Interface connected to edge router
interfaceB = 'eth2'

# Queue refresh scheduler (lqos_scheduler). Minutes between reloads.
queueRefreshIntervalMins = 30

# WORK IN PROGRESS. Note that interfaceA determines the "stick" interface
# I could only get scanning to work if I issued ethtool -K enp1s0f1 rxvlan off
OnAStick = False
# VLAN facing the core router
StickVlanA = 0
# VLAN facing the edge router
StickVlanB = 0

# Allow shell commands. False causes commands print to console only without being executed.
# MUST BE ENABLED FOR PROGRAM TO FUNCTION
enableActualShellCommands = True

# Add 'sudo' before execution of any shell commands. May be required depending on distribution and environment.
runShellCommandsAsSudo = False

# Allows overriding queues / CPU cores used. When set to 0, the max possible queues / CPU cores are utilized. Please
# leave as 0.
queuesAvailableOverride = 0

# Devices in in ShapedDevices.csv without defined Parent Nodes are placed in generated parent nodes.
# When set True, this option balances the subscribers across generatedPNs / CPU cores based on the subscriber's bandwidth plan.
# When set False, devices are placed in generatedPNs sequentially with a near equal number of subscribers per core.
# Whether this impacts balance across CPUs will depend on your subscribers' usage patterns, but if you are observing
# unequal CPU load, and have most subscribers without a defined Parent Node, it is recommended to try this option.
# Most subscribers average about the same bandwidth load regardless of speed plan (typically 5Mbps or so).
# Past 25,000 subscribers this option becomes inefficient and is not advised.
useBinPackingToBalanceCPU = False

# Bandwidth & Latency Graphing
influxDBEnabled = True
influxDBurl = "http://localhost:8086"
influxDBBucket = "libreqos"
influxDBOrg = "Your ISP Name Here"
influxDBtoken = ""

# NMS/CRM Integration

# Use Customer Name or Address as Circuit Name
circuitNameUseAddress = True

# Should integrationUISP overwrite network.json on each run?
overwriteNetworkJSONalways = False

# If a device shows a WAN IP within these subnets, assume they are behind NAT / un-shapable, and ignore them
ignoreSubnets = ['192.168.0.0/16']
allowedSubnets = ['100.64.0.0/10']

# Splynx Integration
automaticImportSplynx = False
splynx_api_key = ''
splynx_api_secret = ''
# Everything before /api/2.0/ on your Splynx instance
splynx_api_url = 'https://YOUR_URL.splynx.app'

# UISP integration
automaticImportUISP = False
uispAuthToken = ''
# Everything before /nms/ on your UISP instance
UISPbaseURL = 'https://examplesite.com'
# UISP Site - enter the name of the root site in your network tree
# to act as the starting point for the tree mapping
uispSite = ''
# Strategy:
# * "flat" - create all client sites directly off the top of the tree,
#   provides maximum performance - at the expense of not offering AP,
#   or site options.
# * "full" - build a complete network map
uispStrategy = "full"
# List any sites that should not be included, with each site name surrounded by '' and separated by commas
excludeSites = []
# If you use IPv6, this can be used to find associated IPv6 prefixes for your clients' IPv4 addresses, and match them
# to those devices
findIPv6usingMikrotik = False
# If you want to provide a safe cushion for speed test results to prevent customer complains, you can set this to
# 1.15 (15% above plan rate). If not, you can leave as 1.0
bandwidthOverheadFactor = 1.0
# For edge cases, set the respective ParentNode for these CPEs
exceptionCPEs = {}
# exceptionCPEs = {
#  'CPE-SomeLocation1': 'AP-SomeLocation1',
#  'CPE-SomeLocation2': 'AP-SomeLocation2',
# }

# API Auth
apiUsername = "testUser"
apiPassword = "changeme8343486806"
apiHostIP = "127.0.0.1"
apiHostPost = 5000


httpRestIntegrationConfig = {
    'enabled': False,
    'baseURL': 'https://domain',
    'networkURI': '/some/path',
    'shaperURI': '/some/path/etc',
    'requestsConfig': {
        'verify': True,  # Good for Dev if your dev env doesnt have cert
         'params': {  # params for query string ie uri?some-arg=some-value
           'search': 'hold-my-beer'
         },
        #'headers': {
           # 'Origin': 'SomeHeaderValue',
        #},
    },
    # If you want to store a timestamped copy/backup of both network.json and Shaper.csv each time they are updated,
    # provide a path
    # 'logChanges': '/var/log/libreqos'
}
