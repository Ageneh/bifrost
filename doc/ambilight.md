## Ambilight and Hue

Ambilight+Hue makes the lights around a TV follow the picture. The TV sends
the colours over the entertainment API; Bifrost receives them and drives the
lights through zigbee2mqtt. This page is the TV side; the resource model is in
[Entertainment](entertainment.md).

### What the TV needs

- A Philips TV with Ambilight and the Hue Sync support built in (the
  "Ambilight+Hue" or "Hue Sync" option in the TV's settings).
- The TV and the bridge on the same network. The TV discovers bridges over
  mDNS/SSDP, so a bridge it cannot discover is a bridge it cannot stream to.
- The lights behind the TV paired to this bridge, in one zone of type
  `screen`.

A Philips Hue Play HDMI Sync Box is the other way to the same result: the box
takes the HDMI sources and streams their colours. The box is a client like
the TV, and it needs its own zone.

### Pair the TV

1. Press the bridge's link button, or open the pairing window from the
   console's Pairing screen.
2. In the TV's Ambilight settings, choose Hue Sync and let it find the
   bridge.
3. The TV appears in the bridge's pairing list with its device type. It is
   the same shared identity every Hue client uses, so the list shows the type
   and the time, not a per-device key.

The certificate matters here: the TV checks the bridge's certificate against
the bridge id, and a certificate regenerated after pairing has to be trusted
again on the TV.

### Lay out the zone

The zone's positions are what tell the TV which light is left and which is
right. Create the zone from the Entertainment screen, then move each light
where it physically sits:

- The top edge of the screen is `y = 0.8`, just in front of it `z = 0.4`.
- `x = -0.4` is the left edge, `x = 0.4` the right.
- A gradient light chain has one segment per LED group; each gets its own
  position, in the order the chain addresses them.

A zone whose positions do not match the TV still streams, but the colours
appear in the wrong place, which reads as "the TV is wrong" rather than "the
layout is wrong". The segment preview on the zone screen is the quickest way
to see that the left light really is on the left.

### When the picture stutters

The diagnostics panel reports two rates: what the TV sent, and what reached
zigbee2mqtt after the area's throttle. A gap between them is the bridge
dropping frames because the mesh cannot carry them, which is a Zigbee
problem, not a TV problem. A rate that is low on both sides is the TV not
sending, which is usually a network or discovery problem.

### What hardware has proved

Wave 8's hardware acceptance is recorded in
[the hardware matrix](hardware-matrix.md). Until it is signed off, this page
describes what Bifrost does, not what a specific TV model has confirmed.

### See also

- [Entertainment](entertainment.md): zones, channels and stream selection.
- [Home Assistant](home-assistant.md): the bridge inside a Home Assistant
  install.
