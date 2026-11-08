# Remote access

The daemon binds to `127.0.0.1` by default. You should keep it that way and
reach it through one of the two supported setups below. Never expose a
plain-HTTP daemon (or RCON) directly to the internet.

The daemon has exactly one credential: the API token. It is either set
explicitly in `config.toml` (`daemon.token`, keep the file mode `0600`) or
generated on first run into `<data_dir>/token` (mode `0600`, printed once).
The GUI stores it in the OS keyring, or in a `0600` file if no keyring is
available.

## Option 1 — SSH port-forward (recommended)

Machine **A** runs the daemon (loopback only). Machine **B** runs the GUI.

```sh
# On machine B:
ssh -N -L 18080:127.0.0.1:8080 user@machine-a
```

Then in the GUI: **Add remote node**

- Name: `A` (anything)
- URL: `http://127.0.0.1:18080`
- Token: the contents of `machine-a:<data_dir>/token` (or the token you set)

Traffic — including the bearer token and WebSocket console stream — is
encrypted by SSH, and the daemon never listens on a public interface. No
firewall change is needed on A beyond SSH itself.

Keep the tunnel alive with autossh or a systemd **user** service:

```ini
# ~/.config/systemd/user/mcpanel-tunnel.service
[Unit]
Description=mcpanel SSH tunnel to machine A
After=network-online.target

[Service]
ExecStart=/usr/bin/ssh -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=30 -L 18080:127.0.0.1:8080 user@machine-a
Restart=always
RestartSec=5

[Install]
WantedBy=default.target
```

```sh
systemctl --user daemon-reload
systemctl --user enable --now mcpanel-tunnel
```

## Option 2 — reverse proxy with TLS

Use this only if a tunnel is impractical (for example many operators). Put
the proxy on the **same host** as the daemon, keep the daemon on
`127.0.0.1`, and terminate TLS at the proxy.

### Caddy (simplest)

```caddyfile
mcpanel.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

Caddy obtains certificates automatically and proxies WebSockets (the console
and `/events`) without extra configuration. Then add the node in the GUI:

- URL: `https://mcpanel.example.com`
- Token: the daemon token

### nginx

```nginx
server {
    listen 443 ssl;
    http2 on;
    server_name mcpanel.example.com;

    ssl_certificate     /etc/letsencrypt/live/mcpanel.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/mcpanel.example.com/privkey.pem;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_read_timeout 3600s;   # long-lived WebSockets
    }
}
```

Requirements for either proxy:

- TLS is mandatory (the token travels in a header on every request).
- WebSocket upgrade must be forwarded (`/api/v1/instances/<id>/console` and
  `/api/v1/events`).
- Do not cache API responses.
- If you use basic auth or IP allow-lists in front, keep the daemon token too.

## Verifying a remote setup

```sh
curl https://mcpanel.example.com/health
# {"status":"ok","api_version":"0.1", ...}

curl -H "Authorization: Bearer <token>" \
     https://mcpanel.example.com/api/v1/instances
```

In the GUI, the node dot shows:

- green — connected;
- yellow — connecting/reconnecting (exponential backoff, capped at 60 s);
- grey — offline (daemon unreachable);
- red — unauthorized (wrong/missing token; the GUI prompts for it);
- dark red — incompatible daemon API version (upgrade the daemon or GUI).

## RCON

The daemon never exposes RCON. RCON is only used internally (optional TPS
query, behind a feature flag) and its password comes from an environment
variable, never from the API or the GUI.
