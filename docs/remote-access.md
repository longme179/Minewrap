# Remote access to minewrapd (v2)

`minewrapd` is the headless Axum daemon (REST + WebSocket) that runs
`minewrap-core`. It binds **`127.0.0.1` by default** — it is **not**
meant to be reachable directly from the internet.

> **v1.10.0 (P0 daemon-first):** the desktop GUI is now *also* a
> client of this daemon — the local node is simply `minewrapd` on
> `127.0.0.1:{port}` (auto-spawned by the GUI). Local and remote
> nodes share one architecture: REST + `/events` WS + bearer token.
> The GUI keeps its remote-node registry in
> `~/.config/minewrap/nodes.json` (0600).

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

## Headless always-on daemon

Run `minewrapd` under the systemd user unit
(`packaging/minewrapd.service`) so servers, SLP status polling and
the periodic Discord status posts keep running even when the GUI is
closed (the unit also pins `TOKIO_WORKER_THREADS=2` and
`MALLOC_ARENA_MAX=2` for the smallest steady-state footprint).
`systemctl --user stop minewrapd` sends SIGTERM, which stops every
instance gracefully (stdin `stop` → SIGTERM → SIGKILL) and exits 0 —
that is the supported "stop everything" path.

## Tests

```bash
cargo test -p minewrapd           # state keeper, poller, discord service, REST/WS integration
cargo test -p minewrap-remote
```
