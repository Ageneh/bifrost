# Wave 10 fixture — the small network the harness tests use

`bridge.json` is a complete message array in the shape
`tests/support/mock_z2m.rs` replays on connect: `bridge/state`, `bridge/info`,
`bridge/devices`, `bridge/groups`, two device states with their availability,
and one motion sensor with its availability and reading.

The network is deliberately small: two lights in one group, one sensor. It
exercises the paths the suite leans on — a per-light `/set`, a group-level
scene command, a bridge request with its answer, a device that reports on its
own topic, and a server that hangs up — without any of them hiding behind a
third device.

## Where the payloads come from

The device records are the wave-4 captures and the reading is the wave-9
motion capture, both **reconstructed** from zigbee2mqtt's documented expose
shapes (see `tests/fixtures/w04/README.md` and `tests/fixtures/w09/README.md`).
They are not recorded from a live server.
