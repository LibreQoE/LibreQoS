# Integrations

## UISP Integration

First, set the relevant parameters for UISP (uispAuthToken, UISPbaseURL, etc.) in `/etc/lqos.conf`.

To test the UISP Integration, use

```shell
python3 integrationUISP.py
```

On the first successful run, it will create a network.json and ShapedDevices.csv file.
If a network.json file exists, it will not be overwritten.
You can modify the network.json file to more accurately reflect bandwidth limits.
ShapedDevices.csv will be overwritten every time the UISP integration is run.
You have the option to run integrationUISP.py automatically on boot and every 10 minutes, which is recommended. This can be enabled by setting ```enable_uisp = true``` in `/etc/lqos.conf`

## Powercode Integration

First, set the relevant parameters for Powercode (powercode_api_key, powercode_api_url, etc.) in `/etc/lqos.conf`.

To test the Powercode Integration, use

```shell
python3 integrationPowercode.py
```

On the first successful run, it will create a ShapedDevices.csv file.
You can modify the network.json file manually to reflect Site/AP bandwidth limits.
ShapedDevices.csv will be overwritten every time the Powercode integration is run.
You have the option to run integrationPowercode.py automatically on boot and every 10 minutes, which is recommended. This can be enabled by setting ```enable_powercode = true``` in `/etc/lqos.conf`

## Sonar Integration

First, set the relevant parameters for Sonar (sonar_api_key, sonar_api_url, etc.) in `/etc/lqos.conf`.

To test the Sonar Integration, use

```shell
python3 integrationSonar.py
```

On the first successful run, it will create a ShapedDevices.csv file.
If a network.json file exists, it will not be overwritten.
You can modify the network.json file to more accurately reflect bandwidth limits.
ShapedDevices.csv will be overwritten every time the Sonar integration is run.
You have the option to run integrationSonar.py automatically on boot and every 10 minutes, which is recommended. This can be enabled by setting ```enable_sonar = true``` in `/etc/lqos.conf`

## Splynx Integration

First, set the relevant parameters for Splynx (splynx_api_key, splynx_api_secret, etc.) in `/etc/lqos.conf`.

The Splynx Integration uses Basic authentication. For using this type of authentication, please make sure you enable [Unsecure access](https://splynx.docs.apiary.io/#introduction/authentication) in your Splynx API key settings. Also the Splynx API key should be granted access to the necessary permissions.

To test the Splynx Integration, use

```shell
python3 integrationSplynx.py
```

On the first successful run, it will create a ShapedDevices.csv file.
You can manually create your network.json file to more accurately reflect bandwidth limits.
ShapedDevices.csv will be overwritten every time the Splynx integration is run.
You have the option to run integrationSplynx.py automatically on boot and every 10 minutes, which is recommended. This can be enabled by setting ```enable_spylnx = true``` in `/etc/lqos.conf`.
