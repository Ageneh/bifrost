# Wave 3 fixtures

Everything a wave 3 gate or integration test reads lives here, under version
control, so it runs in CI. `.scratch/` is untracked and must not be read by
anything that runs there.

- `z2m/bridge.json`: what the mock zigbee2mqtt server replays on connect, as
  an array of `{"topic", "payload"}` messages in file order: `bridge/state`,
  `bridge/info`, `bridge/devices` (a coordinator, a color light and a
  white-ambiance light), `bridge/groups` (one group holding both lights), one
  state report per light and one availability report per light. Read by
  `tests/support/mock_z2m.rs`.

The device, group and state payloads are the ones in
`tests/fixtures/w02/z2m/bridge.json`, which the wave 2 gate already drives;
`bridge/state`, `bridge/info` and the two availability reports are added here
in the shape zigbee2mqtt 2.x sends them. New payloads are recorded from a real
bridge with `cargo run --example z2mdump -- ws://<bridge>:8080/api?token=...`,
which prints every `bridge/*` message until the first device report, then
exits. Add a message by appending it to the array; `tests/mock_z2m.rs` asserts
every `bridge/*` entry still parses as a z2m message, so a fixture cannot
drift away from the protocol the backend speaks.
