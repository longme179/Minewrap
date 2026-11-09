# Remote access to minewrapd (v2)

`minewrapd` is the headless Axum daemon (REST + WebSocket) that runs
the same `minewrap-core` as the Tauri app. It binds **`127.0.0.1` by
default** — it is **not** meant to be reachable directly from the
internet.

## Recommended: SSH port-forward

The daemon only needs to be reachable from your own machine. Use an
SSH tunnel from the GUI workstation to the remote host:

```bash
# On your workstation (forwards local 8080 to the remote 8080):
ssh -L 8080:127.0.0.1:8080 user@remote-host
```

Then add the node in Minewrap's **Nodes** page as:

- URL: `http://127.0.0.1:8080`
- Token: the value printed **once** at `minewrapd` startup (or in
  `data/token`, mode `0600` on the remote)

The tunnel encrypts the token + API traffic with SSH; no extra TLS
is needed, and the daemon never binds a public interface.

## Alternative (not recommended)

Binding `daemon.host = "0.0.0.0"` exposes the REST/WS API to the
network. The token is the only auth. If you do this, put the daemon
behind a TLS-terminating reverse proxy and keep the token secret.

## Headless always-on Discord

Run `minewrapd` under the systemd user unit
(`packaging/minewrapd.service`) so the 10-minute Discord status
message keeps posting even when the GUI is closed. The GUI's embedded
core is only for the local node; `minewrapd` is the always-on path
for remotes (see `ARCHITECTURE.md`).

## Tests

```bash
cargo test -p minewrapd   # auth + WS handshake
cargo test -p minewrap-remote
```
