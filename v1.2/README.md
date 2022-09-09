# v1.2 (IPv4) (Alpha)

## Features

- Support for multiple devices per subscriber circuit. This allows for multiple IPv4s to be filtered into the same queue, without necessarily being in the same subnet.

- Support for multiple IPv4s or IPv6s per device

- Reduced reload time by 80%

## ShapedDevices.csv
Shaper.csv is now ShapedDevices.csv

New minimums apply to upload and download parameters:

* Download minimum must be 1Mbps or more
* Upload minimum must be 1Mbps or more
* Download maximum must be 3Mbps or more
* Upload maximum must be 3Mbps or more
    
ShapedDevices.csv now has a field for Circuit ID. If the listed Circuit ID is the same between two or more devices, those devices will all be placed into the same queue. If a Circuit ID is not provided for a device, it gets its own circuit. Circuit Name is optional, but recommended. The client's service loction address might be good to use as the Circuit Name.

## UISP Integration
This integration fully maps out your entire UISP network.

To use:
1. Delete network.json and, if you have it, integrationUISPbandwidths.csv
2. run ```python3 integrationUISP.py```

It will create a network.json with approximated bandwidths for APs based on UISP's reported capacities, and fixed bandwidth of 1000/1000 for sites.
You can modify integrationUISPbandwidths.csv to correct bandwidth rates. It will load integrationUISPbandwidths.csv on each run and use those listed bandwidths to create network.json. It will always overwrite ShapedDevices.csv on each run by pulling devices from UISP.
