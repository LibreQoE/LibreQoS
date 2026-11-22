# Optional HTTPS with Caddy

**Version: LibreQoS v3.**

LibreQoS can put the WebUI and the API behind Caddy so operators reach it over
HTTPS. This is optional; LibreQoS works without it.

## Turn it on

Open **Configuration → SSL Setup** in the WebUI and choose **Setup SSL**. The
first-run setup offers the same option. LibreQoS installs Caddy and writes its
configuration for you.

## Two certificate modes

- **Enter an external hostname** such as `libreqos.example.com`. Caddy requests
  a public certificate from Let's Encrypt. Browsers trust it automatically once
  DNS and inbound access to the host are correct.
- **Leave the hostname blank.** Caddy secures LibreQoS by its management IP
  address, using Caddy's local certificate authority. Traffic is still
  encrypted, but each operator computer must trust Caddy's local root
  certificate before browser warnings stop.

## What changes

- Operators use `https://your-hostname/` or `https://your-management-ip/`
  instead of `http://your_shaper_ip:9123`.
- The WebUI listener moves to `127.0.0.1:9123`, and Caddy proxies to it.
- The API at `/api/v3`, with its documentation at `/api/v3/docs`, is part of
  `lqosd` and is served on the same HTTPS address. There is no second upstream
  to configure. Every API request still needs a local API key in the
  `x-bearer` header; create keys under
  **Configuration → License & Services → Local API Keys**.

HTTPS covers only the management interface. The DPDK dataplane ports are not
affected.

## Local certificate mode

With the hostname left blank, Caddy's local root certificate is at:

```text
/var/lib/caddy/.local/share/caddy/pki/authorities/local/root.crt
```

Trust that certificate on each operator workstation that opens the WebUI.

## Turn it off

Open **Configuration → SSL Setup** and choose **Disable SSL**. LibreQoS then:

- removes the Caddy configuration it manages;
- restores the WebUI listener that was set before HTTPS was turned on, or, if
  none was set, the default (port `9123` on all IPv4 addresses);
- returns operators to plain HTTP on the management IP and port `9123`.

On hosts that turned HTTPS on with a beta before beta20.23, turning it off can leave
the WebUI on `127.0.0.1:9123`, which only the host itself can reach. To fix
that, remove the `webserver_listen` line from `/etc/libreqos/lqos.conf` (or set
it to `"0.0.0.0:9123"`) and run `sudo systemctl restart libreqos-lqosd`.
