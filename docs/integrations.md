# Integrations & the entity model (what the UI renders)

The UI never talks to integrations directly. **Integrations are backend drivers**; the core
aggregates their **entities** and the UI reads those over the Core-API. This doc covers what the UI
needs to know about that pool. Driver-side protocol (for writing an integration) is summarized at
the end.

## How entities reach the UI

1. A driver (Kodi, Home Assistant, Denon, a custom one) reports its **available entities** to the core.
2. Those entities must be **configured** (added) to appear to the UI. Already-set-up integrations
   have their active entities configured already.
3. The UI reads **configured** entities via the Core-API `get_entities`, subscribes, renders
   controls, sends `execute_entity_command`, and receives live `entity_change` events.

> Correction worth remembering: entities are **not** auto-visible just because an integration is
> installed. Only *configured* entities are returned by `get_entities` and accepted by
> `subscribe_events`. To add one the user hasn't: Core-API `configure_entities_from_integration`
> (empty `entity_ids` = configure all of that driver's entities), or add it in web-configurator.

## Entity fields the UI cares about (Core-API `Entity`)

`entity_id` (unique across all integrations), `entity_type`, `integration_id`, `name`,
`features[]`, `device_class?`, `icon?` (falls back to type default), `options{}`
(incl. `options.user` overrides), `area?`, `attributes{}`.

`entity_type` + `features` + `device_class` decide which controls to show. Types:
`button`, `switch`, `light`, `cover`, `media_player`, `climate`, `sensor`, `remote`, `ir_emitter`,
`select`, `number`, `voice_assistant`. Commands go out as
`execute_entity_command{entity_id, cmd_id, params}`; state comes back as `entity_change`.

## First-app target: Kodi remote + HA volume keys

Both integrations are already connected on the unit, so this needs **no integration code** — just UI:

- **Screen = Kodi.** The Kodi integration exposes a `media_player` (likely also a `remote`) entity.
  Render transport/nav; each control → `execute_entity_command(<kodi media_player>, cmd_id, params)`
  (`play_pause`, `next`, `previous`, `stop`, cursor/`home`/`back` on the `remote`). Now-playing and
  play state arrive via `entity_change`.
- **Hard volume keys = Home Assistant.** Physical buttons are read via **evdev in our process**
  (not over the WS), so we map them ourselves to a *different* entity than the screen shows:
  - `VOLUME_UP`   → `execute_entity_command(<HA media_player>, "volume_up", {})`
  - `VOLUME_DOWN` → `execute_entity_command(<HA media_player>, "volume_down", {})`
  - (or `volume_set` with a `volume` param for absolute control)
- **On-device check:** confirm the HA target is a `media_player` exposing `volume_up`/`volume_down`
  — pull its `entity_id` + command ids from `get_entities` / `get_entity_commands`. (If "volume" is
  really the AVR, the **Denon** integration is already LOCAL on the unit and exposes volume directly
  — an alternative target that skips HA.)

This mixing of sources per-control is the whole point of owning the UI: the screen and the hard keys
can target whatever entities we choose, independent of the core's activity/button-mapping model
(which we can also use if we prefer core-managed mappings).

## Writing an integration (only if we need new device support)

Not required for the Kodi/HA app. Reference for later — a custom integration is a `.tar.gz`
(`driver.json` at root + `bin/driver` static aarch64 **or** `bin/driver.js` Node; ≤100 MB; max 10;
`POST /intg/install`; **does not void warranty**, unlike the custom-UI install). It runs in a tight
sandbox (DynamicUser, caps dropped, RO root, private namespaces; see `../ucr3-cfw` survey). Protocol:

- **Roles reversed vs the UI:** the **driver is a WebSocket server**, the **core connects in** as
  client (binds `UC_INTEGRATION_INTERFACE` / `UC_INTEGRATION_HTTP_PORT`). On-device custom drivers
  use **no auth** (the driver still sends `authentication` code 200, `req_id 0` first).
- **Same envelope** as the Core-API (`kind/id/msg/msg_data`; `resp` has `req_id`+`code`; `event` has
  `cat`+`ts`).
- **Must-answer requests:** `get_driver_version`, `get_device_state`, `get_available_entities`,
  `subscribe_events`, `get_entity_states`, `entity_command`. **Must-send events:** `entity_change`,
  `device_state`.
- **Setup flow** from `driver.json` `setup_data_schema`: `setup_driver` → `driver_setup_change`
  events (`WAIT_USER_ACTION` with an input page or confirmation) → `set_driver_user_data` →
  `STOP/OK`.
- **Spec:** `core-api` repo, `integration-api/UCR-integration-asyncapi.yaml`. Libs: Python `ucapi`,
  Node `@unfoldedcircle/integration-api`. Rust: `uc_api` (`unfoldedcircle/api-model-rs`) provides
  the message models (no transport).

## Note for `coreapi`

`uc_api` (`unfoldedcircle/api-model-rs`, Apache-2.0, git dep — not on crates.io) has serde types for
Core-API/Integration-API/REST messages. Worth adopting in `crates/coreapi` to replace the loose
`serde_json::Value` handling with typed messages.
