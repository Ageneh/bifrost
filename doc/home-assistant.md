## Home Assistant

Bifrost runs beside Home Assistant as an add-on, or as its own container on
the same network. Either way it answers the Hue API, so Home Assistant's
existing Hue integration is the client.

### As an add-on

The add-on is the upstream [bifrost-hassio](https://github.com/chrivers/bifrost-hassio)
project. It is not part of this repository, and the parts Home Assistant
requires of an add-on live there: the Ingress route, the watchdog, and the
`/data` directory the configuration and state are written to.

**Ownership.** This repository does not own the add-on. The Ingress route,
the Supervisor watchdog and the `/data` layout are upstream, in
`bifrost-hassio`, which no row of this programme's plan owns. This
programme does not fork it. The dependency is pinned at upstream commit
`af62c32ae90e585d3e72d12156f7c85f64c33ea6` (2025-05-20); a fork is only
considered by reading that tree first, as part of a later ticket. What this
repository provides is the image the add-on runs and the readiness for it:
prefix-safe assets and the `<base>` injection below are readiness, not
ownership.

What this repository ships for that install is the container image and the
compose file. It ships no add-on file, no `config.yaml` for the Supervisor
and no `run.sh`: those live upstream, and a copy here would drift from the
repository that owns them.

Bifrost itself only has to serve its UI behind a path prefix and trust the
Supervisor's Ingress proxy, which it does:

- `bifrost.auth.trust_ingress` makes a request carrying `X-Ingress-Path`
  skip the Bifrost login, because Home Assistant has already authenticated
  the user. Trust is decided by the connecting address, and only the
  addresses in `bifrost.auth.trusted_cidrs` count (the Supervisor's Ingress
  proxy, `172.30.32.2`, is the documented one).
- The UI is built with relative asset paths and reads the prefix the server
  injects, so it works at `/` and under `/api/hassio_ingress/<token>/`
  without a rebuild.

Ports the bridge listens on, and the defaults:

| Port | Setting | Purpose |
|---|---|---|
| 80 | `bridge.http_port` | Hue API v1/v2 and the admin console |
| 443 | `bridge.https_port` | the same over TLS, which the Sync box requires |
| 2100/udp | `bridge.entm_port` | the entertainment stream (DTLS) |

### As the Hue integration

Point Home Assistant's Hue integration at the bridge's address and press the
link button (or open the pairing window from the console) when it asks. The
integration sees a Hue bridge: lights, rooms, scenes and grouped lights, with
the names zigbee2mqtt already has.

An entertainment zone is a Hue resource, not a Home Assistant entity. Home
Assistant does not start or stop a stream; the TV or the Sync app does. What
Home Assistant can do is everything else about the lights in the zone.

### What Ingress does not protect

Ingress puts a login in front of the console for the people Home Assistant
already authenticated. It changes nothing about the rest of the bridge, and
two facts follow from that:

- **The Hue API stays open.** `/api`, `/clip` and `/eventstream` need no
  password, because Hue apps on the network need no password. Anyone who can
  reach the bridge's address and port can control the lights and add Zigbee
  devices, whether or not they have a Home Assistant account.
- **Trusting Ingress trusts an address.** `trust_ingress` skips the Bifrost
  login for a request carrying `X-Ingress-Path` from one of
  `trusted_cidrs`. The default is the Supervisor's Ingress proxy alone. Any
  address added to that list can reach the console without a Bifrost login,
  so widen it only to a proxy you run.

Keep the bridge on a network you trust. If it must sit on an untrusted one,
put it behind a firewall rule that admits only the Home Assistant host and
the Hue clients, rather than relying on the console's login.

### The container's data directory

One mounted directory holds everything the bridge reads and writes:
`config.yaml`, `config.draft.yaml` (an unapplied draft), `config.yaml.bak`
(the previous configuration), `state.yaml`, `sessions.yaml` and the
certificate. `docker-compose.yaml` mounts `./data` there and points
`BIFROST_CONFIG` into it, so recreating the container from the compose file
keeps all of them. The container gate (`scripts/docker-boot.sh`) recreates a
container and compares those three files, because a fixture-based gate runs
on a directory and cannot see a container losing them.

### Behind the prefix

The console's own client calls are relative to the document, and its socket
URL is derived from the same base, so an add-on install needs no extra
configuration. If Home Assistant is served over HTTPS, the socket upgrades to
`wss` with it.

### See also

- [Entertainment](entertainment.md): zones and streaming.
- [Ambilight and Hue](ambilight.md): the TV side.
- [Configuration reference](config-reference.md): `trust_ingress`,
  `trusted_cidrs` and the ports.
