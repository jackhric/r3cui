# Dev loop (desktop, no device)

`scripts/test.sh` is what CI runs. To see the app with nothing set up, `cargo run -p remote-ui -- --demo` (see
"The demo" below). The rest of this page is for running against the simulator and the fakes.

## 1. Run the core simulator (the remote's own state)

Clone Unfolded Circle's dockerized core next to this repo (or anywhere, and set `SIM_DIR`):

```sh
git clone https://github.com/unfoldedcircle/core-simulator ../core-simulator
scripts/sim.sh up        # also: seed | down | logs | env
```

`sim.sh up` starts the core plus its Home Assistant demo integration (`integration-hass` + a demo
HA), then configures all of that integration's entities in the core. r3cui doesn't read those entities
(its drivers talk to Home Assistant and Kodi directly), so they only matter if you open the simulator's
own configurator. What the UI takes from the simulator is the remote's state: power mode, battery,
Wi-Fi, display and sound settings.

The core listens on **`127.0.0.1:8090`** (override with `SIM_PORT`), not upstream's 8080, which is
commonly taken on a workstation (Steam's CEF debug port sits there). `scripts/sim.override.yml`
does the remap, binds everything to localhost and drops the demo HA's `privileged` flag.

The UI token is written to `core-simulator/docker/ui-env/ws-token`; `UC_MODEL` defaults to `UCR3`.
A WS test console is at `http://localhost:8090/ws.html`, the configurator at `/configurator`.
REST demo accounts: `admin`/`remote2`, `web-configurator`/`1234`.

## 2. Run our UI against it

```sh
eval "$(scripts/sim.sh env)"   # UC_SOCKET_URL + UC_TOKEN_PATH
export RUST_LOG=info           # remote_ui=debug dumps every entity and event
cargo run -p remote-ui
```

A 480×800 window opens and authenticates with the core. The rest of the screen is filled by the drivers set up in
`config/modules.json` (Settings → Integrations; see below). It reconnects to the core every 2 s if the core goes away.
(`UC_TOKEN=<raw token>` also works instead of `UC_TOKEN_PATH`.)

## Notes / limits

- The simulator can't install custom UIs and has no suspend/BT/mic; it's for the WS + UI loop only.
- Device build: `scripts/build-device.sh` cross-compiles the `device` feature for aarch64 in a container and runs
  `packaging/pack.sh`; `scripts/test.sh` also clippy-checks that build. See docs/plan.md and docs/device-contract.md.

## Simulating the hard keys

Physical buttons never come over the WebSocket, so the simulator can't inject them. On the desktop
the keyboard stands in (`crates/ui/src/input/mod.rs`); the footer line shows what each press resolved to.

| Key | Button | Key | Button |
|---|---|---|---|
| arrows / Enter | d-pad / OK | `+` `-` `0` | volume up / down / mute |
| Backspace, Esc | BACK | Space `,` `.` | play / prev / next |
| `H` `M` | HOME, MENU | `S` `R` | stop, record |
| PgUp / PgDn | channel up / down | | |

Volume and mute go to global control; Home and the input key (`V`) are the remote's own; everything else goes to
the current input's remote slot. What fills those is `config/modules.json` (see docs/drivers.md); on the first run
it is filled from the old `kodi` and `hidden_sources` keys of `config/settings.json`, if any.

## Stand-ins for Kodi and Home Assistant

The drivers can run against in-process fakes, so the whole UI works with no Kodi or Home Assistant at hand:

```sh
cargo run -p kodi --features testing --example fake-kodi -- 127.0.0.1:9095          # six films, d-pad walks them
cargo run -p homeassistant --features testing --example fake-ha -- 127.0.0.1:8124 t # a receiver with inputs, token t
```

Then add each in Settings → Integrations (Kodi: host `127.0.0.1` and the port as its WebSocket port; Home Assistant:
`http://127.0.0.1:8124` and the token).

The simulator's own demo Home Assistant (`127.0.0.1:8123`, token `HASS_TOKEN` in the simulator's `docker/.env`) has
a `media_player.receiver` that works as global control too.

## The demo

`cargo run -p remote-ui -- --demo` (or `UC_DEMO=1`, or `--record` without `--live`) runs the real session with nothing
to connect to: an in-memory setup (`drivers::demo::modules()`) puts the Demo driver behind global volume and source
and every input's remote, and behind Kodi's screen, and a made-up remote stands in for the core (battery, Wi-Fi to
join; keys `c` charger, `z` asleep, `x` every driver down). Nothing is written to `config/`. All the made-up values
are in `crates/drivers/src/demo/data.rs`; the `--shots` stills read the same ones.

## Driving the live app

`--steps <file> --out <dir>` presses keys through the window and takes pictures along the way
(`crates/ui/src/preview/steps.rs` lists the steps; `page <name>` opens a page by name, the same names `--page <name>`
takes). `awake` puts away the charger card the simulator always shows.
`scripts/tour.steps` visits every page and overlay (36 pictures): run it before and after a change to the Slint files
and compare the pictures to see that nothing moved:

```sh
cargo run -p remote-ui -- --demo --steps scripts/tour.steps --out <dir>
```

Other preview flags (the default desktop build has the `preview` feature): `--shots <dir>` saves a still per screen
from the demo data, `--snap <png>` one picture of the running window, `--press <key>` a key before `--snap`,
`--record <file.mp4> --frame <remote photo>` films a session (the demo, or the real one with `--live`;
`--script steps:<file>` films a steps file).

## Hot reload

`scripts/dev.sh` runs the UI against the simulator with Slint live preview
(`SLINT_LIVE_PREVIEW=1` + `--features slint/live-preview`): saving any `.slint` file under `crates/ui/ui/` reloads
the running window. Rust edits need a restart; with `watchexec` installed the script does that too.

## A real Kodi

Kodi is a driver: add it in Settings → Integrations with its host and ports. Kodi needs "Allow remote control via HTTP"
and remote control from other systems enabled (HTTP 8080, WebSocket 9090 by default).

`scripts/sim.sh kodi` starts the community Kodi *core integration* (`albator78/kodi-integration`) in the simulator and
registers it with the core. r3cui doesn't use it; it is only there to try the simulator's own configurator.
