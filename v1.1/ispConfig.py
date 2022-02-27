#'fq_codel' or 'cake diffserv4'
#'cake diffserv4' is recommended

#fqOrCAKE = 'fq_codel'
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
shapeBySite = True

# Allow shell commands. False causes commands print to console only without being executed. MUST BE ENABLED FOR PROGRAM TO FUNCTION
enableActualShellCommands = True

# Add 'sudo' before execution of any shell commands. May be required depending on distribution and environment.
runShellCommandsAsSudo = False

#Graphing
graphingEnabled = True
ppingLocation = "pping"
influxDBurl = "http://localhost:8086"
influxDBBucket = "libreqos"
influxDBOrg = "Your ISP Name Here"
influxDBtoken = ""

# NMS/CRM Integration
# If a device shows a WAN IP wihtin these subnets, assume they are behind NAT / un-shapable, and ignore them
ignoreSubnets = ['192.168.0.0/16']

# Optional UISP integration
automaticImportUISP = False
# Everything before /nms/ on your UISP instance
UISPbaseURL = 'https://examplesite.com'
# UISP Auth Token
uispAuthToken = ''
# UISP | Whether to shape router at customer premises, or instead shape the station radio. When station radio is in router mode, use 'station'. Otherwise, use 'router'.
shapeRouterOrStation = 'router'
