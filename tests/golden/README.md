# Golden files for scripts/regress.sh (G5)

Every `*.json` here is what master `12d9e37` answers on the gate fixture:
the mock zigbee2mqtt server (`examples/mock-z2m.rs`) with the two lights and
one room in `tests/fixtures/w02/z2m/bridge.json`, and the configuration that
`write_config` in `scripts/lib.sh` writes. Times and event ids are replaced
by `tests/fixtures/w02/normalize.jq`; a created scene's id reads `<scene>`.

`scripts/regress.sh` compares the working tree against these files.
`scripts/regress.sh --golden` rebuilds master from `git archive` and writes
them again; two runs produce identical files.

Rules:

- Goldens are produced from the pinned master commit only, never from a wave
  binary.
- A golden changes only in the commit that intends the change, and the wave
  report names the file, the field and the reason.
