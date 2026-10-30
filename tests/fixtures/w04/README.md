# Wave 4 fixtures — one z2m capture per device class

Each file is a complete message array, the same shape `tests/support/mock_z2m.rs`
replays on connect: `bridge/state`, `bridge/info`, `bridge/devices`,
`bridge/groups`, then one availability report per device.

`tests/inventory.rs` starts a mock on each of them and asserts what the
inventory makes of the device.

## Where the payloads come from

- `devices-light.json` — **recorded**. The two Hue lamps and their state
  messages are copied verbatim out of the wave-3 capture,
  `tests/fixtures/w03/z2m/bridge.json`. `bridge/info`, `bridge/groups` and
  `bridge/state` are that capture's too.
- `devices-plug.json`, `devices-motion.json`, `devices-temperature.json`,
  `devices-illuminance.json`, `devices-contact.json`, `devices-remote.json` —
  **reconstructed**, not recorded. The repository holds no capture of a plug,
  a sensor or a remote, and inventing one and calling it recorded would be
  worse than saying so. Each is built from zigbee2mqtt's documented expose
  shapes for a real model (Innr SP 120, Aqara RTCGQ11LM, WSDCGQ11LM,
  GZCGQ01LM, MCCGQ11LM, WXKG11LM), with the same field set the recorded
  devices carry, so a parser that reads the recorded light also reads these.

When a real z2m is available, record a replacement with `examples/z2mdump.rs`
and swap the file; the test only cares about the classes, not the models.
