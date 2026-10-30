# Wave 9 fixtures — one z2m capture per sensor class, with a reading

Each file is a complete message array, the same shape
`tests/support/mock_z2m.rs` replays on connect: `bridge/state`, `bridge/info`,
`bridge/devices`, `bridge/groups`, one availability report, and then the
reading the device sends when something happens.

`tests/sensors.rs` starts a mock on each of them and asserts the mapping table
of its class: the reading reaches the inventory, the Hue resource its class
owns, the event stream within a second, and the v1 row.

## Where the payloads come from

The device records and the room/state messages are the wave-4 captures
(`tests/fixtures/w04/`), which are **reconstructed** from zigbee2mqtt's
documented expose shapes for real models — Aqara RTCGQ11LM (motion),
WSDCGQ11LM (temperature), GZCGQ01LM (light level), MCCGQ11LM (contact) and
WXKG11LM (remote). They are not recorded from a live server, and calling them
recorded would be worse than saying so.

The reading messages are **reconstructed** the same way: they carry the field
names and units zigbee2mqtt publishes for those models, which is what the
import and the v1 serialization read. Nothing here depends on a model number,
only on the classes.

When a real z2m is available, record a replacement with `examples/z2mdump.rs`
and swap the file; the tests only care about the classes.
