## Entertainment

An entertainment configuration (a zone) tells Bifrost which lights change
colour with a stream, and where each of them sits. A TV running
Ambilight+Hue or the Hue Sync app creates one through the Hue app; Bifrost's
own editor creates and edits the same resource.

Zones are Hue resources. They live in `state.yaml` beside the paired apps and
scenes, never in `config.yaml`, and a zigbee2mqtt reconnect re-imports devices
without touching them.

### Create a zone

`POST /clip/v2/resource/entertainment_configuration` with the body the Hue app
sends:

```json
{
  "configuration_type": "screen",
  "metadata": {"name": "TV"},
  "stream_proxy": null,
  "locations": {
    "service_locations": [
      {"positions": [{"x": -0.4, "y": 0.8, "z": 0.4}],
       "service": {"rid": "<entertainment service id>", "rtype": "entertainment"}}
    ]
  }
}
```

Use `screen` for a TV: the Hue app and the Sync box treat a `screen` zone as a
rectangle in front of the TV, and that is the type their layout tools expect.
`monitor` and `music` are accepted and behave the same way in Bifrost.

The service in each `service_locations` entry is the **entertainment**
service of the device, not its light. `GET /bifrost/entertainment/capabilities`
lists both, with the light each entertainment service renders.

### Positions

A position is a point in the Hue coordinate system: `x` runs left to right
from -1 to 1, `y` bottom to top, `z` front to back, with 0,0,0 at the centre
of the screen and 0.8,0.4 at the top edge just in front of the surface.

The editor supplies one position per segment, in the order the device's
segments are addressed. Bifrost stores them unchanged: a `PUT` writes exactly
the points it is given, and the next `GET` returns the same numbers.

A location that supplies no positions, or fewer than its device has segments,
is laid out with the default: the segments spread evenly along the top edge,
left to right, at `y = 0.8`, `z = 0.4`, from `x = -0.4` to `x = 0.4`. The
default depends on the segment count alone, so the editor can show the same
layout before anything is saved.

### Channels

The channel list is derived, not stored separately: one channel per segment,
in segment order, with `channel_id` counting from 0 across every location in
the zone. A channel's position is the position of its segment. A stream frame
carries one colour per channel, so the number of channels is the number of
colours a frame must carry.

Editing the members or the positions rebuilds the channels. Nothing writes
channels directly.

### Which zone a stream belongs to

A v2 stream names its zone in the packet header. A v1 stream (the Hue app's
older entertainment API) names lights instead, so Bifrost has to infer the
zone. The rule is deterministic:

1. A zone that is already streaming wins, when exactly one such zone claims
   any of the frame's lights.
2. Otherwise the zone claiming strictly more of the frame's lights wins.
3. Otherwise the stream is refused. The refusal names the zones that tie, so
   the answer is "move a light" rather than "try again".

Two zones may share lights; the rule above is what keeps the second TV from
being served colours meant for the first. A zone that claims none of the
frame's lights is never selected.

### Multiple streams

`bifrost.entertainment.max_streams` decides how many sessions may run at once.
The default is 2: one for the TV and one for a second Sync client, which is the
case Bifrost exists to serve. A session beyond the limit is refused before the
DTLS handshake completes, so the client sees a bridge that did not answer
rather than a stream that half works.

The value is written into the bridge's own `Entertainment` resource on load.
That resource is persisted with the state, so an install created before the
setting existed would otherwise keep advertising one stream. After a change,
the configuration is the source of truth and the resource follows on the next
start; an admin screen shows the effective value and names the setting.

### Identity and certificate

The bridge id and the HTTPS certificate both derive from the MAC address:

- The bridge id is the MAC with `FFFE` inserted in the middle, uppercase and
  without separators. `00:11:22:33:44:55` becomes `001122FFFE334455`.
- The certificate's common name is that bridge id, and Bifrost refuses to use
  a certificate whose common name does not match the configured MAC.

Changing the MAC changes both, so every paired client has to pair again. Do
not change it to fix a pairing problem.

### Values a Sync box checks

The Hue Sync box checks the bridge before it will stream. These are the values
Bifrost sends, and where they come from:

| Check | Value | Source |
|---|---|---|
| Bridge id | MAC with `FFFE` inserted, uppercase | `hue::bridge_id` |
| Model | `BSB002` | `src/resource.rs`, v1 `config` |
| API version | `1.70.0` | `HUE_BRIDGE_V2_DEFAULT_APIVERSION` |
| Software version | `1970084010` | `HUE_BRIDGE_V2_DEFAULT_SWVERSION` |
| Zigbee channel | `25` | `src/resource.rs`, v1 `config` |
| Certificate common name | the bridge id | `src/server/certificate.rs` |
| Entertainment stream limit | `bifrost.entertainment.max_streams` | bridge `Entertainment` resource |

The Sync box reaches these over HTTPS on the bridge's HTTPS port. It also
checks the certificate, so a certificate regenerated after pairing has to be
trusted again on the box, or the box will refuse to start a stream.

Wave 8's hardware acceptance is what confirms which of these the box actually
insists on. The matrix and the exact steps are in
[the hardware matrix](hardware-matrix.md); until that run is signed off, the
table above records what Bifrost sends, not what hardware has proved.

### See also

- [Ambilight and Hue](ambilight.md): the TV side of the setup.
- [Home Assistant](home-assistant.md): how Bifrost fits a Home Assistant
  install.
- [Configuration reference](config-reference.md): `max_streams` and the rest
  of the `bifrost` section.
