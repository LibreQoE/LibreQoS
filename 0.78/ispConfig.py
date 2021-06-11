#'fq_codel' or 'cake'
# Cake requires many specific packages and kernel changes:
# 	https://www.bufferbloat.net/projects/codel/wiki/Cake/
# 	https://github.com/dtaht/tc-adv
fqOrCAKE = 'fq_codel'

# How many symmetrical Mbps are available to the edge of this network
pipeBandwidthCapacityMbps = 1000

defaultClassCapacityMbps = 750

# Interface connected to core router
interfaceA = 'eth1'

# Interface connected to edge router
interfaceB = 'eth2'

# Allow shell commands. False causes commands print to console only without being executed. MUST BE ENABLED FOR PROGRAM TO FUNCTION
enableActualShellCommands = True

# Add 'sudo' before execution of any shell commands. May be required depending on distribution and environment.
runShellCommandsAsSudo = False
