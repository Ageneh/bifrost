## Hardware matrix

What has been run on real hardware, and what each device required. This page
is a record, not a promise: a row is filled in only after someone ran the
steps below on that device and signed it off.

### The matrix

| Device | Model | Firmware | Zone type | Values it required | Result | Date | Signer |
|---|---|---|---|---|---|---|---|
| TV with Ambilight+Hue | _not yet run_ | | `screen` | | | | |
| Hue Play HDMI Sync Box | _not yet run_ | | `screen` | | | | |
| Second client (desktop Sync app) | _not yet run_ | | `screen` | | | | |

"No hardware has been run" is the true state until a row is filled in. A
model number typed from memory is worse than an empty cell.

### What the bench needs

- A TV with Ambilight+Hue, paired to a screen-type zone whose positions match
  the physical lights.
- A Hue Play HDMI Sync Box, paired to this bridge.
- The lights behind the TV paired to zigbee2mqtt and imported by Bifrost.
- A second client for the two-stream step: a desktop running the Hue Sync app,
  or the box plus a TV.

### Running the automated part

`scripts/acceptance.sh` runs every gate in the order a failure is cheapest to
fix and prints one line per gate, then the six hardware steps. `--no-soak`
skips the thirty-minute soak and `--no-docker` the container gate. A gate
whose prerequisite is missing is reported as SKIP with the reason, never as a
pass, and the exit status is 0 only when everything that ran passed.

That run is necessary and not sufficient: it cannot pair a Hue app, start a
stream from a TV, or load the panel inside Home Assistant.

### The steps

Record the device model, its firmware, the date and who signed, in
`evidence/hardware.md`, one section per run.

**1. One stream, ten minutes.** Start a stream from the TV. Watch it for ten
minutes without touching the console. Pass: no visible stutter, and the
diagnostics panel's in and out rates stay close for the whole ten minutes.

**2. Two streams, ten minutes.** Start the TV's stream, then the second
client's on a separate zone. Pass: both rows report at least 20 frames per
second out, for ten minutes, with neither dropping to zero.

**3. One stop, one survivor.** Stop the second client from the console.
Pass: its row disappears, the TV's row keeps rising, and the TV's picture is
unaffected.

**4. Positions match.** Move one light's position in the editor and save.
Pass: the colour that light shows moves with it on the TV, and the zone's
channel table shows the new number.

**5. The values the box checks.** With the matrix row's firmware recorded,
note which of the values in [Entertainment](entertainment.md#values-a-sync-box-checks)
the device actually required: a bridge id it rejected, a certificate it
would not trust, an API version below which it refused to stream. A value
that was not tested is not a value the matrix can claim.

### What to do with a failure

A device that refuses to stream is the useful result, not the problem: it
turns "the box checks something" into "the box checks this". Record the
refusal verbatim (the error the device shows), the value Bifrost sent, and
the value that made it work, then open a ticket rather than editing this
page to say it works.

### See also

- [Entertainment](entertainment.md): the values, and where each comes from.
- [Ambilight and Hue](ambilight.md): the TV setup this follows.
