# Integrations

## UISP Integration

First, set the relevant parameters for UISP (uispAuthToken, UISPbaseURL, etc.) in ispConfig.py.

To test the UISP Integration, use

```shell
python3 integrationUISP.py
```

On the first successful run, it will create a network.json and ShapedDevices.csv file.
If a network.json file exists, it will not be overwritten.
You can modify the network.json file to more accurately reflect bandwidth limits.
ShapedDevices.csv will be overwritten every time the UISP integration is run.
You have the option to run integrationUISP.py automatically on boot and every 30 minutes, which is recommended. This can be enabled by setting ```automaticImportUISP = True``` in ispConfig.py

## Splynx Integration

First, set the relevant parameters for Splynx (splynx_api_key, splynx_api_secret, etc.) in ispConfig.py.

To test the Splynx Integration, use

```shell
python3 integrationSplynx.py
```

On the first successful run, it will create a ShapedDevices.csv file.
You can manually create your network.json file to more accurately reflect bandwidth limits.
ShapedDevices.csv will be overwritten every time the Splynx integration is run.
You have the option to run integrationSplynx.py automatically on boot and every 30 minutes, which is recommended. This can be enabled by setting ```automaticImportSplynx = True``` in ispConfig.py
