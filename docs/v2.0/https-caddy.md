# Optional HTTPS With Caddy

LibreQoS can run the WebUI and API docs behind Caddy so operators can use HTTPS. This is optional. LibreQoS still works without it.

## When To Use It

Use `Configuration -> SSL Setup` when you want operators to open LibreQoS with HTTPS instead of plain HTTP on port `9123`.

You can also enable the same option during the first-run setup flow.

## Two Certificate Modes

LibreQoS supports two simple choices:

- Enter an external hostname such as `libreqos.example.com`: Caddy requests a public certificate from Let's Encrypt. Browsers should trust it automatically once DNS and inbound access are correct.
- Leave the hostname blank: Caddy secures LibreQoS by management IP address with Caddy's local certificate authority. Traffic is still encrypted, but operator computers must trust Caddy's local root certificate before browser warnings stop.

## What Changes After You Enable It

- Operators stop using `http://your_shaper_ip:9123` and instead use `https://your-hostname/` or `https://your-management-ip/`.
- LibreQoS moves the WebUI listener to `127.0.0.1:9123`.
- Caddy proxies the WebUI and API docs over HTTPS.
- Swagger moves to `/api/v1/api-docs` on the same HTTPS origin as the WebUI.

## If You Use The Local Certificate Mode

When you leave the hostname blank, the Caddy local root certificate is stored on the LibreQoS host at:

```text
/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt
```

Trust that certificate on each operator workstation that will open the HTTPS WebUI.

## Disable HTTPS

If you want to back out, open `Configuration -> SSL Setup` and choose `Disable SSL`.

LibreQoS then:

- removes the managed Caddy configuration
- restores the previous direct WebUI listener, or the normal default `:::9123` if no custom listener was set first
- returns operators to direct HTTP access on the management IP and port `9123`

## Related Pages

- [Quickstart](quickstart.md)
- [Configure LibreQoS](configuration.md)
- [LibreQoS WebUI (Node Manager)](node-manager-ui.md)
- [LibreQoS Node API](api.md)
- [Troubleshooting](troubleshooting.md)
