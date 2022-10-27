# Provides common functionality shared between
# integrations.

from ispConfig import allowedSubnets, ignoreSubnets
import ipaddress;

def isInAllowedSubnets(inputIP):
    # Check whether an IP address occurs inside the allowedSubnets list
	isAllowed = False
	if '/' in inputIP:
		inputIP = inputIP.split('/')[0]
	for subnet in allowedSubnets:
		if (ipaddress.ip_address(inputIP) in ipaddress.ip_network(subnet)):
			isAllowed = True
	return isAllowed

def isInIgnoredSubnets(inputIP):
    # Check whether an IP address occurs within the ignoreSubnets list
	isIgnored = False
	if '/' in inputIP:
		inputIP = inputIP.split('/')[0]
	for subnet in ignoreSubnets:
		if (ipaddress.ip_address(inputIP) in ipaddress.ip_network(subnet)):
			isIgnored = True
	return isIgnored

def isIpv4Permitted(inputIP):
    # Checks whether an IP address is in Allowed Subnets.
    # If it is, check that it isn't in Ignored Subnets.
    # If it is allowed and not ignored, returns true.
    # Otherwise, returns false.
    return isInIgnoredSubnets(inputIP)==False and isInAllowedSubnets(inputIP)
