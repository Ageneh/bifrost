# Hardware acceptance evidence

**Nothing on this page has been run.** Every step below needs a real device
that no gate in this repository can stand in for. The programme's automated
gates exercise the same code paths against the mock zigbee2mqtt and a
simulated stream; that is not a hardware sign-off, and no line here may say
otherwise until a person filled in the fields and someone else signed it.

The steps are the six of the testing memo, in the order the bench does them.
The device matrix is in [the hardware matrix](../doc/hardware-matrix.md); the
values a Sync box checks are in
[Entertainment](../doc/entertainment.md#values-a-sync-box-checks).

## The six steps

| # | Step | Needs | Success |
|---|---|---|---|
| 1 | Hue app pairing | a phone with the Hue app | pairs, lists the lights, the window closes |
| 2 | Desktop Hue Sync | a desktop with the Hue Sync app | one stream, no visible stutter |
| 3 | Hue Sync box | a Sync box on the LAN | the box streams to a screen zone |
| 4 | TV Ambilight + Hue | a TV with Ambilight and Hue Sync | the TV streams, colours land where the layout says |
| 5 | Multi-stream | any two of the above | both run at once, one stop leaves the other |
| 6 | Home Assistant Ingress | a Home Assistant instance with the add-on | panel, apply, restart, direct LAN login |

Each step's device, actions and pass condition are in
`data/bifrost-hardware-bench.md`, which is the bench list the captain works
through in one sitting.

## Run template

Copy this block once per step and fill in every field. A block with an empty
field is not evidence.

```
### Step <n> — <device> — <date>

- Device: <model as printed on the device>
- Firmware: <version as the device reports it>
- Bridge: Bifrost <version>, certificate common name <bridge id>
- Steps run: <the step numbers actually run, in order>
- Result: pass / fail
- What was seen: <one or two sentences; on a failure, the device's own error>
- Values the device required: <value> — required / not required / not tested
- Signer: <name, not the implementer>
```

For step 5, add the two stream rows exactly as the diagnostics panel showed
them, with the in and out rates at the start and after ten minutes. For step
6, add the Ingress path the panel loaded under, and whether the direct LAN
login still worked.

## Runs

_No run recorded. Waves 6 to 10 shipped without hardware; the bench pass is
the step this file exists for, and it has not happened._
