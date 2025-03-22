# LibreQoS Long-Term-Stats (LTS)
## Signup
Find your LibreQoS Node ID by logging into your LibreQoS Shaper Box by SSH, and running:
```
sed -n 's/node_id = //p' /etc/lqos.conf | sed -e 's/^"//' -e 's/"$//'`
```
The output will be a long number - the unique identifier of your Shaper Box.

Now please visit:
```
https://stats.libreqos.io/trial1/YOUR_NODE_ID
```
Where YOUR_NODE_ID is the Node ID you found in the prior step.

If this is the first time you're using LTS - choose "Sign Up - Regular Long-Term Stats".

Complete the enrollment for the 30 day free trial by entering your payment information.

The signup process will provide you with an LTS License Key.

Head back to your Shaper Box, and edit `/etc/lqos.conf` to modify the [long_term_stats] section as follows:
```
[long_term_stats]
gather_stats = true
collation_period_seconds = 60
license_key = "YOUR_LICENSE_KEY"
uisp_reporting_interval_seconds = 300
```
Where YOUR_LICENSE_KEY is your unique LTS License Key provided in the prior step. Be sure to include the surrounding quotes.

Now, save the file, and run `sudo systemctl restart lqosd lqos_scheduler`. This will reload the lqosd process, allowing it to start submitting data to LTS.

## Accessing LTS
To access LTS, please visit [https://stats.libreqos.io/](https://stats.libreqos.io/) - then enter your LTS key, username (email), and password.
