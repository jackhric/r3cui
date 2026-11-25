# Drivers: how r3cui connects to other software

Status: implemented (2026-11-25). Settings → Integrations edits everything below except tokens, which are pasted into `modules.json` for now.

r3cui talks to other software through **drivers**. A driver is one connection to one program
(Home Assistant, Kodi, ...). The frontend never names a program: it talks to the *ports* a driver
offers, and the user decides which driver fills which job in Settings → Integrations.

## The model

```
Global control                        what the whole remote drives
├─ Volume  → driver + target          (Home Assistant: a media_player entity)
├─ Source  → driver + target          (may be a different entity from Volume)
└─ Inputs                             the Source target's input list, in the user's order
   ├─ KODI    remote slot: Kodi    UI slot: Kodi
   ├─ TV      remote slot: —       UI slot: —        → a blank screen
   └─ ...
```

- **Global control** gets volume, mute, power, the input switcher and Home, whatever input is on.
- **Remote slot**: where every other button goes while that input is on (d-pad, OK, back, menu,
  channel, transport). Empty: those buttons do nothing.
- **UI slot**: what the stage shows while that input is on: Now Playing, a library wall, or
  nothing. Empty: a blank stage.
- **No Source target**: there is one implicit input, *Default*, with its own two slots, so a
  Kodi-only setup without Home Assistant still works.

Drivers are set up once under Settings → Integrations (address, port, token, ...), then picked per
input like a slot. One instance per driver type for now; the config keys instances by an id, so
several later is a config change, not a redesign.

Today's drivers:

| Driver | Global (volume, source) | Remote slot | UI slot |
|---|---|---|---|
| Home Assistant (`crates/homeassistant`, r3cui's own client) | yes | later | — |
| Kodi (`crates/kodi`) | — | yes | yes (Now Playing, library wall) |
| Demo | yes | yes | yes |

The core's own integrations are not a path any more: the receiver, the key-backlight helpers and
the volume-limit sensor all come through r3cui's Home Assistant client. The core still owns the
remote itself (power, battery, Wi-Fi, display, the key backlight hardware).

## Crates

```
crates/homeassistant   HA WebSocket client: auth, states, entity subscriptions, service calls.
                       Like crates/kodi: an engine with no IO, a service that reconnects, a handle.
                       `testing` feature: FakeHa, an in-process HA server for tests.
crates/kodi            (exists) `testing` feature: FakeKodi.
crates/drivers         The contract and the drivers. No Slint.
  api/                   ports, views, intents, the setup schema (Field), Status
  homeassistant/         the HA driver (global control, key backlight)
  kodi/                  the Kodi driver (remote + UI slots, library wall)
  demo/                  canned data for previews, recordings and the desktop
  registry.rs            every driver type, by id
  config.rs              modules.json: instances, global bindings, inputs
  inputs.rs              the input list: the source's list with the user's order, names, hiding
  host.rs                runs the configured drivers and routes keys and views
  testing.rs             FakeDriver: records intents, publishes views on demand
crates/ui              The frontend: renders the host's view, sends it intents, and draws
                       Settings → Integrations from the driver specs.
```

A driver module only uses `crate::api` and its own client crate. Nothing outside `registry.rs`
names a driver type. Adding a driver: a client crate (if it needs one), a module under
`crates/drivers/src/`, one line in the registry.

## The contract (`drivers::api`)

Every port is the same shape: state out on a `watch`, intents in on an unbounded channel.

```rust
pub struct Port<View, Intent> { pub view: watch::Receiver<View>, pub tx: mpsc::UnboundedSender<Intent> }

pub trait Driver: Send + Sync {
    fn status(&self) -> watch::Receiver<Status>;           // one row on the status page
    fn volume(&self, target: &str) -> Option<Port<Option<Volume>, VolumeIntent>> { None }
    fn source(&self, target: &str) -> Option<Port<Option<Sources>, SourceIntent>> { None }
    fn remote(&self) -> Option<mpsc::UnboundedSender<RemoteKey>> { None }
    fn ui(&self) -> Option<Port<Stage, UiIntent>> { None }
    fn key_light(&self) -> Option<watch::Receiver<KeyLight>> { None }
    fn targets(&self, kind: TargetKind) -> BoxFuture<Vec<Target>>;   // for the entity pickers
}
```

- `Volume { level, max, muted }`, on a 0–100 scale (Home Assistant's `volume_level` × 100).
- `Sources { on, current, list }`.
- `Stage { media: Option<Media>, library: Option<Library> }`. Both `None` is a blank stage.
- `UiIntent`: `Shown(bool)` (the host says whether the slot is on screen and the remote awake, so
  a driver only polls when it matters), `PlayPause`, `Seek(secs)`, `Audio(i)`, `Subtitle(Option<i>)`.
- Images are URLs. The frontend fetches, scales and caches them, so no driver depends on image
  code.

"Shown until confirmed" (`Optimistic`) and the slider's pacing stay in the frontend: they are the
same for every driver.

## Setup schema

Each driver type has a static `Spec { id, name, fields, provides }`. A `Field` is
`{ key, label, kind: Text | Url | Port | Secret | Entity(domain), required, default }`. The
Integrations pages are drawn from it, so a new driver needs no new Slint.

`Secret` fields (the HA token) are never shown back; the row reads "Set" or "Not set". A token is
pasted into `modules.json` for now; pairing through an HA integration (HACS) comes later.

## Configuration: `modules.json`

Read from `UC_DATA_HOME/modules.json` (the remote's writable per-UI directory; `./config` on the
desktop), seeded from the archive's `config/modules.json` on first run. Written whenever the
Integrations pages change something.

```json
{
  "drivers": {
    "homeassistant": { "type": "homeassistant", "url": "http://homeassistant.local:8123", "token": "…",
                       "keys_light": "light.theater_remote_keys", "keys_mode": "input_select.theater_remote_keys_mode",
                       "volume_limit": "sensor.theater_receiver_volume_limit" },
    "kodi": { "type": "kodi", "host": "kodi.local", "http_port": 8080, "ws_port": 9090 }
  },
  "global": {
    "volume": { "driver": "homeassistant", "target": "media_player.theater_receiver" },
    "source": { "driver": "homeassistant", "target": "media_player.theater_receiver" }
  },
  "inputs": [
    { "id": "KODI", "remote": "kodi", "ui": "kodi" },
    { "id": "NET", "hidden": true },
    { "id": "PLAYSTATION 5", "name": "PS5" }
  ]
}
```

- **Input ids.** Home Assistant's `source_list` is a list of names with no stable id, so the
  source's name is the key. If the receiver renames an input, its settings no longer apply and it
  shows as new.
- **Order.** `inputs` is the user's order. Sources not listed follow in the receiver's order.
  Listed ids the receiver no longer reports are kept, but not shown.
- **Overrides.** A rename, hiding or a moved position shows in Settings with an accent mark on the
  input's row, so it's clear it differs from what the receiver reports.
- Removing a driver also clears the slots and global bindings that pointed at it.
- The old `settings.json` keys (`kodi.*`, `hidden_sources`, `keys_light`, `keys_mode`,
  `volume_limit_sensor`) are read once to fill a first `modules.json`, then ignored.

## Buttons

| Button | Goes to |
|---|---|
| Volume up/down, mute, the slider | Global volume |
| Power (held) | Global source: on/off |
| Input (voice key), Home | r3cui itself (the input list, the home page) |
| Everything else | The current input's remote slot |

No per-input remapping.

## Status page

One row per configured driver (Connected, Connecting, Unreachable, Not set up), plus Wi-Fi.
A driver's status may name what is wrong ("Token rejected", "media_player.x not found").

## Testing

- Every engine (HA, Kodi, the input list, the host's routing) is plain logic with unit tests.
- `FakeHa` and `FakeKodi` are real WebSocket servers in-process: the clients are tested end to
  end, including reconnects.
- `FakeDriver` lets host tests check routing without any network.
- `scripts/test.sh` runs what CI runs: format check, clippy, the whole workspace's tests.

## Later

- An HA integration (HACS) for pairing by code instead of a pasted token, and to show the remote
  in HA.
- Home Assistant in an input's remote slot (`remote.send_command` / media_player keys).
- Input logos per input; several instances of one driver type.
- Out-of-process drivers speaking the same port messages.
