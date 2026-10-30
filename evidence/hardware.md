# Hardware acceptance evidence

One section per run. A section is evidence only when every field is filled in
and someone other than the implementer signed it. The steps are in
[the hardware matrix](../doc/hardware-matrix.md); the values under test are in
[Entertainment](../doc/entertainment.md#values-a-sync-box-checks).

## Run template

Copy this block for each run and delete what does not apply.

```
### <device> — <date>

- Device: <model as printed on the device>
- Firmware: <version as the device reports it>
- Bridge: Bifrost <version>, certificate common name <bridge id>
- Zone: <name>, type <configuration type>, <n> channels, <m> lights
- Steps run: 1, 2, 3, 4, 5 (name any step skipped and why)
- Step 1 (one stream, 10 min): pass/fail — <what was seen>
- Step 2 (two streams, 10 min): pass/fail — <fps in/out for both rows>
- Step 3 (stop one, the other survives): pass/fail
- Step 4 (positions match the TV): pass/fail
- Step 5 (values the device required): <value> — required / not required / not tested
- Signer: <name, not the implementer>
```

## Runs

_No run recorded yet. Wave 8 built and tested the editor and the streaming
path; the bench acceptance is the step this file exists for._
