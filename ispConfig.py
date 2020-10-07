#'fq_codel' or 'cake'
# Cake requires many specific packages and kernel changes:
# 	https://www.bufferbloat.net/projects/codel/wiki/Cake/
# 	https://github.com/dtaht/tc-adv
fqOrCAKE = 'fq_codel'

# How many symmetrical Mbps are available to the edge of this network
pipeBandwidthCapacityMbps = 500

# Interface connected to edge
interfaceA = 'eth4'

# Interface connected to core
interfaceB = 'eth5'

# Allow shell commands. Default is False where commands print to console. MUST BE ENABLED FOR PROGRAM TO FUNCTION
enableActualShellCommands = False

# Add 'sudo' before execution of any shell commands. May be required depending on distribution and environment.
runShellCommandsAsSudo = False

# Import customer QoS rules from UNMS
importFromUNMS = False

# Import customer QoS rules from LibreNMS
importFromLibreNMS = False

# So that new clients are client shaped with something by default, and don't get their traffic de-prioritized,
# you can add specific subnets of hosts to be set to specific speeds.
# These will not override any imports from actual customer data via UNMS or LibreNMS

addTheseSubnets = [
	('100.64.0.0/22', 115, 20),
	('100.72.4.0/22', 115, 20)
]

# Available on LibreNMS site as https://exampleLibreNMSsite.net/api-access
orgLibreNMSxAuthToken = ''

# Do not include trailing forward slash. For example https://exampleLibreNMSsite.net
libreNMSBaseURL = ''

# Which LibreNMS groups to import. Please create groups in LibreNMS to match these group names such as Plan A
libreNMSDeviceGroups = {
	'Plan A':	{
				'downloadMbps': 25,
				'uploadMbps': 3
				},
	'Plan B':	{
				'downloadMbps': 50,
				'uploadMbps': 5
				}
}

# Available under UNMS > Settings > Users
orgUNMSxAuthToken = ''

# Everything before /nms/. Use https:// For example: https://unms.exampleISP.com (no slash after)
unmsBaseURL = ''

# For bridged CPE radios on UNMS, you can exclude matching radio models from rate limiting
deviceModelBlacklistEnabled = False
