#'fq_codel' or 'cake'
# Cake requires many specific packages and kernel changes:
# 	https://www.bufferbloat.net/projects/codel/wiki/Cake/
# 	https://github.com/dtaht/tc-adv
fqOrCAKE = 'fq_codel'

# How many symmetrical Mbps are available to the edge of this test network
pipeBandwidthCapacityMbps = 500

# Interface connected to edge
interfaceA = 'eth4'

# Interface connected to core
interfaceB = 'eth5'

# Allow shell commands. Default is False where commands print to console. Must be enabled to function
enableActualShellCommands = False

# Add 'sudo' before execution of any shell commands. Default is False.
runShellCommandsAsSudo = False

# Import customer QoS rules from UNMS
importFromUNMS = False

# Available under UNMS > Settings > Users
orgUNMSxAuthToken = ''

# Everything before /nms/. Use https:// For example: https://unms.exampleISP.com (no slash after)
unmsBaseURL = ''

# For bridged CPE radios, you can exclude matching radio models from rate limiting
deviceModelBlacklistEnabled = False
