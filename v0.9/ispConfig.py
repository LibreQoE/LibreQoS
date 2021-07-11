#'fq_codel' or 'cake diffserv4'
fqOrCAKE = 'fq_codel'
#fqOrCAKE = 'cake diffserv4'

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

# Allow shell commands. False causes commands print to console only without being executed. MUST BE ENABLED FOR PROGRAM TO FUNCTION
enableActualShellCommands = True

# Add 'sudo' before execution of any shell commands. May be required depending on distribution and environment.
runShellCommandsAsSudo = False
