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

There are a number of other variables for UISP in `ispConfig.py`. Here's some explanation on some of them.

- `circuitNameUseAcctService` - This variable will create a circuit name in the format of `<customer_name>-<account_number>_<service_id>`. Only enable this if you are using UISP sync. Also set `circuitNameUseAddress` to false for this naming to take effect.
- `suspendedDownload` - This specifies a download limit that will override whatever bandwidth plan the client has assigned to them if the service is not in an "active" state.
- `suspendedUpload` - This specifies a upload limit that will override whatever bandwidth plan the client has assigned to them if the service is not in an "active" state.

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
