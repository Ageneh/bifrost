# Wave 2 gate fixtures

- `ui/`: a stand-in for a built web UI (`frontend/.output/public`). Its
  `index.html` references its assets relative to the document, the way the
  real build must. Used by the `routes::ui` tests and `scripts/ui-prefix.sh`.
- `nginx-prefix.conf`: a path-prefix proxy in the shape of Home Assistant
  ingress, for `scripts/ui-prefix.sh`.
- `z2m/bridge.json`: what the mock zigbee2mqtt server sends on connect: a
  coordinator, a color light and a white-ambiance light, one group holding
  both, and one state report per light. Used by `scripts/regress.sh`.
- `normalize.jq`: removes wall-clock times from captured answers.
- `legacy/`: an install as master `12d9e37` leaves it, for
  `scripts/upgrade.sh`. `config.yaml` has only settings master knows and the
  z2m token in the URL. `state.yaml` was written by master after pairing a
  Hue user and storing the scene "Evening"; `z2m.json` is `z2m/bridge.json`
  with that scene stored in the group, as zigbee2mqtt would report it.

  `cert.pem` is deliberately not stored here: it holds a private key, and a
  copy in git would be a key nobody can rotate. `scripts/upgrade.sh` lets
  master generate it on the first boot, exactly as the released binary does
  on a real install, and then requires the working tree to keep it.
