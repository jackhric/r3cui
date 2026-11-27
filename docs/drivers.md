# Drivers: how r3cui connects to other software

Status: implemented (2026-11-25). Settings → Integrations edits everything below; so does r3cui's web configurator from a browser (`docs/web-configurator.md`), where a token is pasted.

r3cui talks to other software through **drivers**. A driver is one connection to one program
(Home Assistant, Kodi, ...). The frontend never names a program: it talks to the *ports* a driver
offers, and the user decides which driver fills which job in Settings → Integrations.

## The model

```
Global control                        what the whole remote drives
├─ Volume  → driver + target          (Home Assistant: a media_player entity)
├─ Inputs  → driver + target          (may be a different entity from Volume; `global.source` in modules.json)
├─ Power   → driver + target          (own entity, or follows Inputs when unset)
├─ Voice   → driver + target          (where the voice key's speech goes: a Home Assistant Assist pipeline)
└─ Input list                         the Inputs target's input list, in the user's order
   ├─ KODI    remote slot: Kodi    UI slot: Kodi
   ├─ TV      remote slot: —       UI slot: —        → a blank screen
   └─ ...
```

- **Global control** gets volume, mute, power, Home (a tap goes home, a hold opens the input list), whatever input is on. Settings →
  Integrations → Global control has one row each for Volume, Inputs, Power and Voice. Voice is the `VoicePort`
  (`Driver::voice`, `Provides.voice`): start, audio, end or cancel a session; the view says what was heard and the
  answer. See docs/voice.md.
- **Remote slot**: where every other button goes while that input is on (d-pad, OK, back, menu,
  channel, transport). Empty: those buttons do nothing.
- **UI slot**: what the stage shows while that input is on: Now Playing, a library wall, or
  nothing. Empty: a blank stage.
- **No Inputs target**: there is one implicit input, *Default*, with its own two slots, so a
  Kodi-only setup without Home Assistant still works.

Drivers are set up once under Settings → Integrations (address, port, token, ...), then picked per
input like a slot. One instance per driver type for now; the config keys instances by an id, so
several later is a config change, not a redesign.

Today's drivers:

| Driver | Global (volume, inputs, power) | Remote slot | UI slot |
|---|---|---|---|
| Home Assistant (`crates/homeassistant`, r3cui's own client) | yes | later | — |
| Kodi (`crates/kodi`) | — | yes | yes (Now Playing, library wall) |
| Android TV (`crates/androidtv`) | power only | yes | yes (the app in front) |
| Demo (desktop builds, `demo` feature) | yes | yes | yes |

The core's own integrations are not a path any more: the receiver, the key-backlight helpers and
the volume-limit sensor all come through r3cui's Home Assistant client. The core still owns the
remote itself (power, battery, Wi-Fi, display, the key backlight hardware).

## Crates

```
crates/wsclient        Shared WebSocket plumbing: connection with keep-alive, backoff, pending calls, a reconnecting
                       `Session`. `testing` feature: the FakeServer scaffold the fakes are built on.
crates/sdk             r3cui-sdk: what an integration is written against. No Slint, no client.
  api.rs                 the contract: ports, views, intents, the setup schema (Field), Status, Spec
  entry.rs               DriverEntry: one driver's setup as modules.json holds it
  run.rs                 helpers most drivers' tasks share (follow a watch, commands in order)
  testing.rs             `testing` feature: `until`, `entry`, for an integration's own tests
  lib.rs                 Integration: a spec and how to start one, what each integration crate exports
crates/homeassistant   The Home Assistant integration. The client: auth, states, entity subscriptions, service
                       calls, Assist; `protocol` holds its wire constants; an engine with no IO, a service that
                       reconnects, a handle. `driver` (default feature): global control, voice, the key backlight.
                       `testing` feature: FakeHa.
crates/kodi            The Kodi integration. The JSON-RPC client; `driver/`: the remote and UI slots, the library
                       wall. `testing` feature: FakeKodi.
crates/androidtv       The Android TV integration. The Remote v2 client (pairing, keys, state); `driver`: the remote,
                       the app in front, power, pairing by code. `testing` feature: FakeTv.
crates/demo            The demo integration: a made-up receiver, film and shelf; data.rs holds every made-up
                       value, and `modules()` the setup that binds it. Carried only by the app's `demo` feature.
crates/host            r3cui's side. Names no integration.
  registry.rs            the driver types a build can run, from the app's list
  config.rs              modules.json: instances, global bindings, inputs
  inputs.rs              the input list: the source's list with the user's order, names, hiding
  known.rs               targets.json: what each driver last offered, for the pickers offline
  host.rs                runs the configured drivers and routes keys and views
  testing.rs             FakeDriver: records intents, publishes views on demand
crates/r3cui           The app (binary remote-ui): renders the host's view, sends it intents, and draws
                       Settings → Integrations from the driver specs. `src/app/registry.rs` lists the
                       integrations it carries.
```

## Writing an integration

An integration is one crate, depending on `r3cui-sdk` and nothing else of r3cui's:

1. Its own client for the device, built on `wsclient` if it speaks WebSocket, with its wire names in a `protocol`
   module and a `testing` feature with a fake server.
2. A `driver` module, behind a default `driver` feature so the client stays usable alone: a `Spec`, a `field`
   module for its setup keys, a type implementing `api::Driver`, and
   `pub const INTEGRATION: Integration = Integration { spec: &SPEC, start: ... }`.
3. Its tests, against its own fake, with `r3cui_sdk::testing`.

The app carries it with a dependency in `crates/r3cui/Cargo.toml` and one line in `src/app/registry.rs`. Nothing in
`host` or `r3cui-sdk` changes, and the Integrations pages (and the web configurator's forms) are drawn from the `Spec`, so no Slint or HTML either.

## The contract (`r3cui_sdk::api`)

Every port is the same shape: state out on a `watch`, intents in on an unbounded channel.

```rust
pub struct Port<View, Intent> { pub view: watch::Receiver<View>, pub tx: mpsc::UnboundedSender<Intent> }

pub trait Driver: Send + Sync {
    fn status(&self) -> watch::Receiver<Status>;           // one row on the status page
    fn volume(&self, target: &str) -> Option<Port<Option<Volume>, VolumeIntent>> { None }
    fn source(&self, target: &str) -> Option<Port<Option<Sources>, SourceIntent>> { None }
    fn power(&self, target: &str) -> Option<Port<Option<Power>, PowerIntent>> { None }
    fn remote(&self) -> Option<mpsc::UnboundedSender<RemoteKey>> { None }
    fn ui(&self) -> Option<Port<Stage, UiIntent>> { None }
    fn pair(&self) -> Option<PairStart> { None }                     // only for a spec with `pairing`
    fn key_light(&self) -> Option<watch::Receiver<KeyLight>> { None }
    fn targets(&self, kind: TargetKind) -> BoxFuture<Vec<Target>>;   // for the entity pickers
}
```

- `Volume { level, max, muted }`, on a 0–100 scale (Home Assistant's `volume_level` × 100).
- `Sources { current, list }`; `SourceIntent::Select`.
- `Power { on }`; `PowerIntent::Set(bool)`. A Home Assistant media_player is on unless its state is off, standby,
  unavailable or unknown. With no power binding the UI shows the remote as on and holding Power does nothing.
- `Stage { media: Option<Media>, library: Option<Library>, app: Option<App> }`. All `None` is a blank stage. `App`
  is the program in front on a box that runs apps; the mark stage names it under the input's mark.
- `UiIntent`: `Shown(bool)` (the host says whether the slot is on screen and the remote awake, so
  a driver only polls when it matters), `PlayPause`, `Seek(secs)`, `Audio(i)`, `Subtitle(Option<i>)`.
- Images are URLs. The frontend fetches, scales and caches them, so no driver depends on image
  code.

"Shown until confirmed" (`Optimistic`) and the slider's pacing stay in the frontend: they are the
same for every driver.

## Setup schema

Each driver type has a static `Spec { id, name, icon, fields, address, provides, pairing }`. A `Field` is
`{ key, label, kind: Text | Url | Port | Secret | Credential | Target(kind), required, default }`. The
Integrations pages are drawn from it, so a new driver needs no new Slint. `address` names the field that says where
the device is: when the source has listed no inputs, the empty Inputs page shows the driver's mark and status and
asks whether that address is online.

`Target(kind)` fields and global control are picked from the driver's `targets(kind)`. The last list each driver gave
is kept in `targets.json` beside `modules.json` (`host::KnownTargets`): a picker opens on it at once, keeps it (its
title marked "· offline") while the driver is down, and names what is bound as the driver last named it. Whatever is
bound shows even with nothing to list.

`Secret` fields (the HA token) are never shown back; the row reads "Set" or "Not set". A token is
pasted into `modules.json` for now; pairing through an HA integration (HACS) comes later.

`Credential` fields are written by pairing and nothing else: they get no row and are never typed, shown or edited.
`Provides.power` marks a driver that can be bound as global Power without being global control (its Power picker row
is the driver itself; the target is its id).

### Pairing by code

A spec with `pairing: Some(CodePairing { code_len })` is paired from Settings → Integrations: its page has a **Pair**
row, and setting the last required field of a driver with nothing paired goes straight on to pairing. The flow:

1. `Driver::pair()` returns a future that contacts the device, which then shows its code, and gives back a
   `Pairer` (the `Pairing` page reads "Contacting …" meanwhile).
2. The keyboard opens (`KbPurpose::Code`, `code_len` characters, action "Pair").
3. `Pairer::finish(code)` answers `Ok(Paired)`: the field values to store (the credentials), which are saved and the
   driver restarted; `Err(WrongCode)`: back to the keyboard with "WRONG CODE, TRY AGAIN" and the same `Pairer`; or
   `Err(Failed(why))`: the page shows why, with Try again.

`app/integrations.rs` holds this as a pure state machine (`Effect::Pair`, `Effect::Code`, `Effect::Finish`; `pair_started`
and `pair_checked` take the results); the session runs the futures and brings the results back as `Later::PairStarted`
and `Later::PairChecked`.

### Android TV

Setup is the address alone; pairing stores `cert_pem` and `key_pem` (a self-signed client certificate generated at the
first pairing). Remote: every `RemoteKey` is an Android key code (`key_code` in `crates/androidtv/src/driver.rs`). UI: the
app in front while the box is on, named from a small table of package names, otherwise by the last part of the
package. Power: `Power { on }` from the box's state; `Set(x)` presses the power key only when the state differs.
Status: Connected, Connecting, Unreachable, or "Needs pairing" (`Health::NotPaired`) when the box turns the
certificate away. No volume: that stays on the receiver.

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
    "source": { "driver": "homeassistant", "target": "media_player.theater_receiver" },
    "power": { "driver": "homeassistant", "target": "media_player.theater_tv" }
  },
  "inputs": [
    { "id": "KODI", "remote": "kodi", "ui": "kodi" },
    { "id": "NET", "hidden": true },
    { "id": "PLAYSTATION 5", "name": "PS5" }
  ]
}
```

- **Power.** `global.power` is optional. A `modules.json` without it keeps working: power follows `global.source`
  (same driver and target), resolved in one place, `Modules::power_binding`, which the host binds. Picking None for
  Power therefore returns it to following Inputs; a setup with no Inputs binding and no Power has no power control.
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
| Power (held) | Global power: on/off |
| Input (voice key), Home | r3cui itself (the input list, the home page) |
| Everything else | The current input's remote slot |

No per-input remapping.

## Status page

One row per configured driver (Connected, Connecting, Unreachable, Not set up), plus Wi-Fi.
A driver's status may name what is wrong ("Token rejected", "media_player.x not found").

## Testing

- Every engine (HA, Kodi, the input list, the host's routing) is plain logic with unit tests.
- `FakeHa` and `FakeKodi` are real WebSocket servers in-process, built on `wsclient::testing`: the clients are
  tested end to end, including reconnects.
- `FakeDriver` lets host tests check routing without any network. `FAKE_PAIRABLE` is its spec with a code pairing.
- `FakeTv` (crates/androidtv) is a TLS Android TV with both ports; the Android TV driver's tests pair with it and
  check keys, power, the app in front and `NotPaired`.
- `scripts/test.sh` runs what CI runs: the whole workspace's tests, then clippy with warnings as errors.
- On the desktop, `--steps <script> --out <dir>` drives the live window key by key and takes pictures, against the
  core simulator and the fakes run as servers (`cargo run -p kodi --features testing --example fake-kodi`, and `fake-ha` in
  `homeassistant`). See `scripts/dev.md`.

## Later

- An HA integration (HACS) for pairing by code instead of a pasted token, and to show the remote
  in HA.
- Home Assistant in an input's remote slot (`remote.send_command` / media_player keys).
- Input logos per input; several instances of one driver type.
- Out-of-process drivers speaking the same port messages.
