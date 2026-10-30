## Configuration reference

Bifrost reads `config.yaml` from the working directory. To use another file,
start Bifrost with `--config <path>`, or set the `BIFROST_CONFIG` environment
variable. Relative `state_file` and `cert_file` paths resolve against the
directory of the configuration file.

Settings Bifrost does not recognize are kept: saving the configuration through
the admin API writes them back unchanged.

Besides `config.yaml`, Bifrost writes these files, by default next to it:

| File | Content |
|---|---|
| `config.draft.yaml` | Changes saved through the admin API but not applied yet. |
| `config.yaml.bak` | The configuration as it was before the last apply. |
| `state.yaml` | Paired Hue apps, resource ids, scenes and room names (`state_file`). |
| `sessions.yaml` | Admin login sessions, beside `state.yaml`, readable by the owner only. |
| `cert.pem` | The HTTPS certificate (`cert_file`). |

Keep all of them together: the Docker Compose setup mounts one `data`
directory for this reason (see [Docker Compose install](docker-compose-install.md)).

```yaml
# Bifrost section [optional!]
#
# Contains bifrost server settings
# [usually omitted, to use defaults]
bifrost:
  # name of yaml file to write state database to
  state_file: "state.yaml"

  # name of x509 certificate for https
  #
  # if this file is missing, bifrost will generate one for you
  #
  # if this file exists, bifrost will check that the mac address
  # matches the specified server mac address
  #
  # to generate a fresh certificate, rename/move this file
  # (this might require pairing the Hue App again)
  cert_file: "cert.pem"

  # Admin login for the Bifrost web interface and the /bifrost API
  # [optional!]
  #
  # This section cannot be changed through the configuration draft. Bifrost
  # writes password_hash itself; see "Admin login" below.
  auth:
    # hash of the admin password, written by Bifrost
    #
    # To reset a lost password: stop Bifrost, delete this line, and start
    # Bifrost again. It prints a new setup code, and every session ends.
    password_hash: "$scrypt$..."

    # a session ends this many hours after login, however active
    session_hours: 12

    # a session ends after this many minutes without a request
    idle_minutes: 60

    # let Home Assistant Ingress requests in without a Bifrost login
    #
    # Home Assistant authenticated the user already. Trust is decided by
    # the address the connection comes from, never by a header.
    trust_ingress: true

    # addresses Ingress requests are trusted from
    #
    # The default is the Home Assistant Supervisor's Ingress proxy,
    # 172.30.32.2, the only address Home Assistant's add-on documentation
    # says Ingress connects from
    # (https://contributors.home-assistant.io/apps/guides/ingress/).
    # Do not widen this to the Supervisor network: other add-ons share it.
    trusted_cidrs: ["172.30.32.2/32"]

    # extra host names the web interface answers to [optional!]
    #
    # Besides the bridge ipaddress and its mDNS name
    # (bifrost-<last 3 MAC bytes>.local), requests naming any other host
    # are refused. This stops DNS rebinding attacks. Add the name here if
    # you reach Bifrost under your own DNS name.
    allowed_hosts: ["bifrost.home.lan"]

  # Entertainment streaming [optional!]
  #
  # Bifrost runs one DTLS session per area. A real Hue bridge advertises one
  # stream at a time; two are what a TV running Ambilight+Hue and a second
  # Sync client need together, which is why that is the default.
  entertainment:
    # how many clients may stream at the same time
    #
    # A further client is refused: it never completes the DTLS handshake,
    # and the streams that are running are not disturbed.
    #
    # The value is also written into the bridge's own `Entertainment`
    # resource in `state.yaml` when Bifrost starts, because clients read it
    # there to decide how many streams they may open. Changing the setting
    # updates both, without a restart.
    max_streams: 2

# Bridge section
#
# Settings for hue bridge emulation
bridge:
  name: Bifrost
  mac: 00:11:22:33:44:55
  ipaddress: 10.0.0.12
  netmask: 255.255.255.0
  gateway: 10.0.0.1
  timezone: Europe/Copenhagen

  # HTTP port for emulated bridge
  #
  # beware: most client programs do NOT support non-standard ports.
  # This is for advanced users (e.g. bifrost behind a reverse proxy)
  http_port: 80

  # HTTPS port for emulated bridge
  #
  # beware: most client programs do NOT support non-standard ports.
  # This is for advanced users (e.g. bifrost behind a reverse proxy)
  https_port: 443

  # DTLS port for emulated bridge (Hue Entertainment streaming)
  #
  # beware: client programs do NOT support non-standard ports.
  # For advanced users (e.g. bifrost behind a port forwarded firewall)
  entm_port: 2100

# Zigbee2mqtt section
#
# Make a sub-section for each zigbee2mqtt server you want to connect
#
# The server names ("some-server", "other-with-tls") are used for logging,
# but have no functional impact.
#
# NOTE: Be sure to use DIFFERENT names for different servers.
# Otherwise the yaml parser will consider it the same server!
z2m:
  some-server:
    # The websocket url for z2m, starting with "ws://".
    #
    # For z2m version 2.x, the url must end in `/api?token=<token>`.
    # For z2m version 1.x, this is optional, but supported.
    #
    # Therefore, Bifrost will adjust the urls if needed.
    # A message will be logged with the rewritten url if this happens.
    #
    # NOTE: The z2m default token is literally the string "your-secret-token",
    # so if unsure, append "/api?token=your-secret-token".
    #
    # Example:
    #
    #   If your z2m frontend is listening on 10.00.0.100:8080, this
    #   is the resuling config:
    #
    url: ws://10.00.0.100:8080/api?token=your-secret-token

    # z2m 2.x auth token [optional!]
    #
    # If set, this token is used instead of any "token=" in the url.
    #
    # A token given in the url keeps working. The first time a configuration
    # is applied through the admin API, Bifrost moves it from the url to
    # this field; the resulting websocket url stays the same.
    #
    # The token is a secret: the admin API never shows it.
    token: your-secret-token

  other-with-tls:
    # This will work, but Bifrost will generate a warning that the url has been
    # adapted to include "/api?token=your-secret-token".
    #
    # NOTE: Using "wss://" instead of "ws://" enables TLS for this connection.
    url: wss://10.10.0.102:8080

    # Disable TLS verify [optional!]
    #
    # If this parameter is included, and has a value of "true", TLS certificate
    # verification will be disabled!
    #
    # NOTE: From a security standpoint, this is almost as bad as disabling
    # encryption entirely. If having a secure connection is important to you,
    # DO NOT enable this option.
    #
    # If you're using self-signed certificates, enabling this option will allow
    # Bifrost to connect to your z2m server.
    disable_tls_verify: false

    # Group prefix [optional!]
    #
    # If you specify this parameter, *only* groups with this prefix
    # will be visible from this z2m server. The prefix will be removed.
    #
    # Example:
    #
    #   With a group_prefix of "bifrost_", the group "bifrost_kitchen"
    #   will be available as "kitchen", but the group "living_room" will
    #   be hidden instead.
    #
    group_prefix: bifrost_

    # Streaming mode ("Entertainment mode" / "Hue Sync") maximum frames per second
    # [optional!]
    #
    # This is the maximum number of light updates attempted per second.
    #
    # The incoming data stream (from a Sync Box, Hue Sync for Windows/Mac,
    # or some other client) determines the maximum possible fps.
    #
    # For example, if Bifrost only receives light updates at 10 fps, setting
    # this limit to 20 will still only cause the lights to update at 10 fps.
    #
    # On the other hand, if the streaming client sends faster than this limit,
    # frames will be dropped to avoid going over it.
    #
    # If not specified, uses a default of 20, which is an attempt to balance
    # responsiveness against load on the Zigbee mesh.
    #
    # Because of the smoothing algorithm Bifrost uses, the results will look
    # *better* if this is not set higher than needed.
    #
    # For example, 30 fps content will look good at 10, 20 or 30 streaming_fps,
    # but worse at streaming_fps: 60, because the frame-to-frame transition
    # time will be wrong for the content.
    #
    # Rules of thumb(s), for best results:
    # - Higher numbers mean greater load on your Zigbee mesh.
    # - If your mesh starts lagging or becoming unresponsive, try a lower number.
    # - Even values as low as 5 fps looks pretty good.
    # - There usually no reason to go above 60.
    # - Have fun experimenting :-)
    streaming_fps: 20
  ...

# Rooms section [optional!]
#
# This section allows you to map zigbee2mqtt "friendly names" to
# a human-readable description you provide.
#
# Each entry under "rooms" must match a zigbee2mqtt "friendly name",
# and can contain the following keys: (both are optional)
#
#   name: The human-readable name presented in the API (for the Hue App, etc)
#
#   icon: The icon to use for this room. Must be selected from the following
#         list of icons supported by the Hue App:
#
#         attic balcony barbecue bathroom bedroom carport closet computer dining
#         downstairs driveway front_door garage garden guest_room gym hallway
#         home kids_bedroom kitchen laundry_room living_room lounge man_cave
#         music nursery office other pool porch reading recreation staircase
#         storage studio terrace toilet top_floor tv upstairs
#
rooms:
  office_group:
    name: Office 1
    icon: office

  carport_group:
    name: Carport Lights
    icon: carport

  ...
```

## Configuration admin API

Bifrost keeps the active configuration and an optional draft. A draft is
saved to `config.draft.yaml` next to `config.yaml`, survives a restart, and
takes effect only when you apply it.

Every endpoint below requires the admin login; see "Admin login". `/bifrost`
sends no CORS headers: only the Hue routes (`/api`, `/clip`, `/eventstream`,
`/auth`, `/updater`, `/licenses`, `/description.xml`) stay open to every
origin.

| Endpoint | Purpose |
|---|---|
| `GET /bifrost/config` | Active configuration, with secrets redacted. |
| `GET /bifrost/config/draft` | Current revision and the draft, if any. |
| `PUT /bifrost/config/draft` | Replace the draft: `{"revision": n, "config": {...}}`. |
| `PATCH /bifrost/config/draft` | Change the draft with a JSON merge patch: `{"revision": n, "patch": {...}}`. |
| `DELETE /bifrost/config/draft` | Discard the draft: `{"revision": n}`. |
| `GET /bifrost/config/diff` | Settings that differ between the active configuration and the draft. |
| `GET /bifrost/config/validate` | Findings for the draft, or for the active configuration without one. |
| `POST /bifrost/config/validate` | Findings for the configuration in the request body. |
| `POST /bifrost/config/apply` | Apply the draft: `{"revision": n}`. Returns the apply plan. |
| `POST /bifrost/config/restart` | Apply the draft (if any, naming its revision), then exit so the supervisor restarts Bifrost. |
| `GET /bifrost/config/export` | Active configuration as YAML, with secrets redacted. |
| `POST /bifrost/config/import` | Replace the draft from YAML: `{"revision": n, "yaml": "..."}`. |

Every write names the revision it was based on. A stale revision fails with
`409 Conflict`; read the draft again and retry.

Applying writes `config.yaml.bak` first, then `config.yaml`. The new file is
generated, so comments from the old one are not carried over; they remain in
`config.yaml.bak`. The response
lists which changes took effect immediately (`hot`) and which need a restart
(`restart_required`). Port and address changes restart the affected
listeners, and z2m server changes start, stop or re-create that backend. The
bridge MAC address and the `state_file` and `cert_file` paths take effect
only after a restart.

Secrets (z2m tokens, password hashes) are write-only. Reads show
`<<REDACTED>>` for a secret that is set. On write, an absent, `null` or
`<<REDACTED>>` secret keeps the existing value, and `""` clears it.

### Web UI

Bifrost serves its web UI from `/` on the HTTP and HTTPS ports. Paths that
belong to the Hue API and the admin API (`/api`, `/auth`, `/bifrost`,
`/clip`, `/description.xml`, `/eventstream`, `/licenses`, `/updater`) never
return the UI: a path no route matches under one of them is an empty
`404 Not Found`.

The UI is built into the binary by the default `ui` Cargo feature, from
`frontend/.output/public`. To serve a build from disk instead, start Bifrost
with `--ui-dir <path>`. A binary built without a UI answers `/` with a
plain-text `404`.

Behind a reverse proxy that serves Bifrost under a path prefix, such as Home
Assistant ingress, the proxy sends the prefix in the `X-Ingress-Path` header.
Bifrost writes it into the page as `<base href>` and as
`window.__BIFROST_BASE__`, both ending in `/`, so asset and API URLs resolve
under the prefix. A header value that is not a plain absolute path is
ignored, and the page uses `/`.

## Admin login

The Bifrost web interface and every endpoint under `/bifrost` require the
admin password. The Hue API does not: Hue apps on your network need no
password. Anyone on this network can control your lights and add Zigbee
devices, so keep Bifrost on a trusted network.

### First start

Without a password, Bifrost prints a one-time setup code to standard output
(not to the log) when it starts:

```
No admin password is set. Open Bifrost in a browser and enter this setup code to choose one: ABCD-EFGH-JKLM
```

With Docker, `docker logs` shows it. Enter the code and a password of at
least eight characters in the web interface. Each start prints a new code.

### Setup mode

When the configuration file is missing or cannot be read, Bifrost does not
exit. It starts in setup mode: the Hue bridge stays off, and only the admin
login and a configuration draft are served, on port 80 of the address of the
default network route. Choose another address with `--setup-listen
<address:port>` or the `BIFROST_SETUP_LISTEN` environment variable. Bifrost
prints a setup code to standard output, as on a first start:

```
Bifrost is in setup mode. Open http://10.0.0.12:80/ in a browser and enter this setup code: ABCD-EFGH-JKLM
```

Setup mode answers to its IP address only, not to a host name. Every
endpoint except the login needs a session, and the first session needs the
setup code and sets the admin password. Then:

| Endpoint | Purpose |
|---|---|
| `GET /bifrost/setup/status` | The configuration file, why it could not be used, and whether `config.yaml.bak` exists. |
| `POST /bifrost/setup/restore` | Load `config.yaml.bak` into the draft: `{"revision": n}`. |
| `GET`/`PUT`/`PATCH`/`DELETE /bifrost/config/draft` | The draft, as on a running bridge; it may be incomplete. |
| `GET`/`POST /bifrost/config/validate` | Findings for the draft, or for the posted document. |
| `POST /bifrost/config/import` | Replace the draft from YAML. |
| `POST /bifrost/config/apply` | Write the draft, with the new password, as the configuration file and start the bridge. |

Applying keeps an unreadable file as `config.yaml.broken` and leaves
`config.yaml.bak` alone. Bifrost then starts the bridge in the same process,
and the session from setup mode stays logged in.

### Lost password

There is no reset by email or command-line flag. To reset the password:

1. Stop Bifrost.
2. Delete the `password_hash` line under `bifrost:` → `auth:` in
   `config.yaml`.
3. Start Bifrost again. It prints a new setup code, and every session ends.

### Sessions

A login starts a session held in the `bifrost_session` cookie (`HttpOnly`,
`SameSite=Strict`, `Secure` over HTTPS). Sessions are kept in
`sessions.yaml` beside `state.yaml`, readable by the owner only, so a
restart does not log you out; the file holds a hash of each session token,
never the token. A session works only from the address that logged in, and
ends after `idle_minutes` without a request or `session_hours` after login.
Logging in or changing the password starts a new session; changing the
password ends every other one.

Failed logins and setup codes from one address are slowed down: after three
failures, each further one doubles the wait (up to 15 minutes) before that
address may try again. Other addresses are never affected.

### Request rules

Every request under `/bifrost` passes these checks, in order:

1. A request carrying `X-Ingress-Path` is refused with `403` unless it comes
   from a `trusted_cidrs` address and `trust_ingress` is on. From a trusted
   address it needs no login.
2. The `Host` must be the bridge `ipaddress`, its mDNS name, or one of
   `allowed_hosts`; otherwise `403`.
3. Without a live session: `401`. Only `GET /bifrost/auth/session`,
   `POST /bifrost/auth/login` and `POST /bifrost/auth/setup` answer without
   one.
4. Every request that changes something, and the `/bifrost/ws` websocket
   upgrade, needs an `Origin` (or `Referer`) of this bridge: same host,
   same port. Otherwise `403`.
5. A changing request with a session needs the session's CSRF token, from
   `GET /bifrost/auth/session`, in the `X-Bifrost-Csrf` header; otherwise
   `403`.

| Endpoint | Purpose |
|---|---|
| `GET /bifrost/auth/session` | `{authenticated, setup_required, ingress, csrf}` for the current request. |
| `POST /bifrost/auth/setup` | Set the first password: `{"setup_code": "...", "password": "..."}`. |
| `POST /bifrost/auth/login` | `{"password": "..."}`; sets the session cookie. |
| `POST /bifrost/auth/logout` | End this session, or every session with `{"all": true}`. |
| `POST /bifrost/auth/password` | `{"current_password": "...", "new_password": "..."}`. |
