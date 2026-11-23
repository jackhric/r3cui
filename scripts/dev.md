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
the keyboard stands in (`crates/ui/src/input.rs`); the footer line shows what each press resolved to.

| Key | Button | Key | Button |
|---|---|---|---|
| arrows / Enter | d-pad / OK | `+` `-` `0` | volume up / down / mute |
| Backspace, Esc | BACK | Space `,` `.` | play / prev / next |
| `H` `M` | HOME, MENU | `S` `R` | stop, record |
| PgUp / PgDn | channel up / down | | |

Volume keys go to `UC_VOLUME_ENTITY`, everything else to `UC_SCREEN_ENTITY` (entity ids). Unset, the
screen picks a Kodi `media_player` if one exists, else the first `media_player`.

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
