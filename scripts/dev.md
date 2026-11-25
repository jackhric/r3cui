# Dev loop (milestone 1 — desktop spike, no device)

## 1. Run the core simulator

Clone Unfolded Circle's dockerized core next to this repo (or anywhere, and set `SIM_DIR`):

```sh
git clone https://github.com/unfoldedcircle/core-simulator ../core-simulator
scripts/sim.sh up        # also: seed | down | logs | env
```

`sim.sh up` starts the core plus its Home Assistant demo integration (`integration-hass` + a demo
HA), then configures all of that integration's entities in the core. A fresh simulator has **no
configured entities** — they exist only as "available" on the integration until added — so without
the seed step `get_entities` returns an empty list.

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

A 480×800 window opens, authenticates, and lists the simulator's 41 demo entities. It reconnects
every 2 s if the core goes away. (`UC_TOKEN=<raw token>` also works instead of `UC_TOKEN_PATH`.)

## Notes / limits

- The simulator can't install custom UIs and has no suspend/BT/mic; it's for the WS + UI loop only.
- Device build (later): switch `crates/ui` slint features to `backend-linuxkms`, cross-compile for
  aarch64, then `packaging/pack.sh`. See docs/plan.md and docs/device-contract.md.

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
cargo run -p kodi --features testing --example fake -- 127.0.0.1:9095            # six films, d-pad walks them
cargo run -p homeassistant --features testing --example fake -- 127.0.0.1:8124 t # a receiver with inputs
```

The simulator's own demo Home Assistant (`127.0.0.1:8123`, token `HASS_TOKEN` in the simulator's `docker/.env`) has
a `media_player.receiver` that works as global control too.

## Driving the live app

`--steps <file> --out <dir>` presses keys through the window and takes pictures along the way
(`crates/ui/src/preview/steps.rs` lists the steps). `awake` puts away the charger card the simulator always shows.

## Hot reload

`scripts/dev.sh` runs the UI against the simulator with Slint live preview
(`SLINT_LIVE_PREVIEW=1` + `--features slint/live-preview`): saving `crates/ui/ui/*.slint` reloads
the running window. Rust edits need a restart; with `watchexec` installed the script does that too.

## Kodi integration (optional)

`scripts/sim.sh kodi` starts the community Kodi driver (`albator78/kodi-integration`) on the
simulator's network and registers it with the core. Finish setup in the configurator
(`http://127.0.0.1:8090/configurator`, `web-configurator`/`1234`): enter the Kodi host and add its
entities. Kodi needs "Allow remote control via HTTP" and remote control from other systems enabled
(HTTP 8080, WebSocket 9090 by default). For a Kodi on this machine use the host `host.docker.internal`.
