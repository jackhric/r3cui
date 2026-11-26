# Remote control

Watch and drive r3cui on the remote (or the desktop demo) from a computer on the same network, for live testing
without the simulator. Off by default.

## Using it

1. On the remote: Settings → System → Remote control → Turn on. The page shows the access key, a QR code and the
   address (`ip:10780`). A dot by the battery shows while a client is connected.
2. On the computer: `cargo build -p r3ctl`, then

   ```sh
   export R3CTL_HOST=<remote ip> R3CTL_KEY=<key>
   r3ctl state                      # page, focus, input, voice stage, battery
   r3ctl tap ok | press voice | release voice | hold home 1200
   r3ctl touch 435 32               # the ☰ icon
   r3ctl swipe 240 600 240 200 300  # a finger, 300 ms
   r3ctl slide 3                    # the touch slider, three steps up
   r3ctl page system                # straight to a page, by its --page name
   r3ctl screenshot -o shot.png
   ```

   Against the desktop demo: run it with `R3CUI_REMOTE_CONTROL_KEY=<any key>` and leave `R3CTL_HOST` unset.

## How it works

- `crates/remotewire`: the protocol, named once for both ends: hard keys (`HardKey`; no power key: switching the
  remote off stays physical), requests, replies, and the key proof (HMAC-SHA256 of a fresh nonce; the key never
  crosses the network).
- `crates/ui/src/remote_control/`: the server. It listens on port 10780 only while switched on, answers only
  local-network addresses, and locks for a minute after five wrong keys. Keys and touches go in through
  `input/inject.rs`, the same door as the remote's own keys and touchscreen, so they behave exactly alike.
  Screenshots are taken only when asked (34-61 ms on the UI thread on the remote, encoded off it).
- The switch and key live in `remote-control.json` in r3cui's data folder. `R3CUI_REMOTE_CONTROL_KEY` (in
  `settings.json`'s `env`) switches it on with a given key for one test install, without saving it.

## Battery

Nothing runs while it is off. While on, it waits on its socket: no polling, no timers, no capture unless a client asks.

## Next

A live stream (reading back frames the screen has just drawn rather than re-rendering), and r3cui's own web page:
the live screen, click to touch, the hard keys, and a state inspector.
