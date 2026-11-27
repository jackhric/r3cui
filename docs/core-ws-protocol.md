# Core-API WebSocket protocol

This is reference for the Core-API, not a description of what r3cui sends. The UI uses the core only for
the remote's own state (`crates/r3cui/src/coreapi`). It no longer reads entities, entity commands or activities from
the core: the receiver, Kodi and the hard keys' targets are drivers (`docs/drivers.md`), and the entity,
activity and button-mapping rows below are kept as spec notes only.

Requests the UI sends today (`crates/r3cui/src/remote/`, `hardware/backlight.rs`): `get_power_mode`,
`get_battery_charger`, `get_display_cfg` / `set_display_cfg`, `get_haptic_cfg`, `get_sound_cfg`,
`get_power_saving_cfg`, `get_network_cfg` / `set_network_cfg`, `get_wifi_status`, `get_wifi_scan_status`,
`wifi_scan_start`, `add_wifi_network`, `set_button_cfg`, and for the System page `version`, `set_ui_info` (after every sign-in, as
the stock UI does), `check_system_update`, `update_system`, `get_api_access`, `set_api_access`, `system_cmd` for the menu's restart and shut down, and for voice the requests under Voice below. Events it acts on
(`coreapi::events`): `power_mode_change`, `battery_status`, `wifi_change`, `software_update`, `assistant_event`. Message names are the
constants in `crates/r3cui/src/coreapi/src/messages.rs`; the System page's requests are typed in `crates/r3cui/src/coreapi/src/system.rs`. The connection runs on `r3cui_sdk::ws`.

Authoritative spec: `unfoldedcircle/core-api`, `core-api/websocket/UCR-core-asyncapi.yaml`
(AsyncAPI 2.6, v0.50.5). Reference implementation: `unfoldedcircle/remote-ui`, `src/core/core.cpp`.
`coreapi` is coded against these; the `msg_data` shapes below are summarized, not exhaustive.

## Transport

- One channel: `/ws` on the same port as REST. Device URL via `UC_SOCKET_URL`
  (default `ws://127.0.0.1:8080/ws`). Simulator: `ws://127.0.0.1:8090/ws` (`scripts/sim.sh env`; upstream listens on 8080).
- The `/intg`, `/profiles`, `/events` "channels" in the spec are doc groupings — all messages
  share `/ws`.

## Envelope

```jsonc
// request  (client → server); id increases per request
{"kind":"req","id":42,"msg":"get_entities","msg_data":{ ... }}
// response (server → client)
{"kind":"resp","req_id":42,"msg":"entities","code":200,"msg_data":{ ... }}
// event    (server → client, unsolicited)
{"kind":"event","msg":"entity_change","cat":"ENTITY","ts":"...","msg_data":{ ... }}
```
Error `msg_data`: `{"code":..,"message":".."}`.

## Auth handshake (message-based — what the stock UI does)

1. On connect, server sends event `auth_required`.
2. Client reads the whole `UC_TOKEN_PATH` file, trims whitespace, and sends:
   `{"kind":"req","id":N,"msg":"auth","msg_data":{"token":"<file contents>"}}`
3. Server replies `authentication` with `code:200` (ok) or `401` (closes the connection).
4. Until authenticated all other messages are ignored; unauthenticated connections close after 15 s.

(Alternatively the upgrade request can carry Basic auth, a session cookie, or an `API-KEY` header —
handy for the simulator. The stock on-device path is the token-file `auth` message.)

## Keep-alive

Server pings every 10 s, closes after ~30 s silence. There's also an app-level `ping`/`pong`.
`tokio-tungstenite` answers WS pings automatically. The UI retries every 2 s after a drop (`RECONNECT_DELAY` in `crates/r3cui/src/app/session.rs`).

## Messages a UI could use

| Area | Messages |
|---|---|
| Events | `subscribe_events` (new session is auto-subscribed to `all`), `unsubscribe_events`, `get_event_channels`. Channels: `entities`, `entity_<type>`, `activity_groups`, `battery_status`, `wifi`, `network`, `integrations`, `software_updates`, `docks`, `configuration`, `profiles`, `bt`, `assistant` |
| Profiles/pages | `get_active_profile`, `get_profiles`, `switch_profile`, `get_pages`, `get_groups` |
| Entities | `get_entities` (filterable; paginated via `msg_data.paging: {page, limit}`, default limit **10**, max 100; response carries `paging: {count, limit, page}`), `get_entity`, `get_entity_types`, `get_entity_commands`, `execute_entity_command` (`{entity_id, cmd_id, params}`), `browse_media`, `search_media`; live updates via `entity_change` event |
| Activities | Activities are entities (`entity_activity`); start/stop via `execute_entity_command`; progress via `entity_change`. Button mappings via `get_activity_button_mappings`-style messages (`button_mapping` data) |
| Battery | `get_battery_charger`; event `battery_status` (`cat:REMOTE`) |
| WiFi/net | `get_network_state`; events `wifi_change`, `network_change` |
| Power | `get_power_mode`, `set_power_mode`; event `power_mode_change` (incl. `SUSPEND`). `system_cmd` `{command: STANDBY \| REBOOT \| POWER_OFF \| RESTART \| RESTART_UI \| RESTART_CORE}` (🧪): on 200 the core closes the connection and does it. The spec's schema calls it `service_cmd`; the stock UI sends `system_cmd` |
| System | `version`, `system`, `get_configuration`, `get_*_cfg` (display/haptic/button/device/sound/localization); event `warning` |
| Voice | `get_voice_control_cfg`, `set_voice_control_cfg`, `get_voice_assistants`; event `assistant_event` (see Voice below) |

## Gotcha: physical buttons are not on the WS

Button presses and the capacitive slider arrive via **evdev in our process**, not as WS events.
The stock UI fetches the activity's button mappings and sends the mapped `execute_entity_command`
(short vs long press distinguished ~800 ms). r3cui doesn't: the drivers host routes the keys
(`docs/drivers.md`, "Buttons").

## Software: firmware updates, the web configurator, r3cui's own updates

`crates/r3cui/src/coreapi/src/system.rs` types these, `crates/r3cui/src/app/software.rs` holds the state, `crates/r3cui/src/remote/software.rs`
does the work. `coreapi::testing::FakeCore` (in the app) answers all of them.

| Message | Notes |
|---|---|
| `version` | `version_info`: model, device name, hostname, `address` (the remote's own, for the configurator link), api, core, ui, os |
| `check_system_update {force_update}` | `system_update_info`: `installed_version`, `update_in_progress`, `available[]` with `download` PENDING / DOWNLOADING / DOWNLOADED / ERROR |
| `update_system {update_id}` | `DOWNLOAD` when it starts fetching, `START` when it starts installing what was fetched. 503 battery under 50 %, 409 already running, 422 already downloaded |
| event `software_update` | `START` / `PROGRESS` / `STOP` with `progress.state` and `download_percent`, `current_step` of `total_steps`, `current_percent`. The UI blocks all input while an update runs, as the stock UI does |
| `get_api_access` / `set_api_access` | `web_configurator {enabled, pin}`. The PIN is never returned, so the UI makes a four-digit one itself and keeps it in memory to show |

The UI does the firmware chain itself: `update_system` (fetch), wait for the download to finish (a `software_update` STOP or
DONE, then `check_system_update` says DOWNLOADED), `update_system` again (install). Against a real core that sequence is
read from the spec and the stock UI and has not been watched on a remote.

r3cui's own updates are not the core's: `GET https://api.github.com/repos/jackhric/r3cui/releases/latest` (override with
`UC_RELEASES_URL`), the `r3cui-<version>.tar.gz` asset checked against its `digest`, then REST `POST /api/system/install/ui`
(multipart `file`, `?void_warranty=yes`) with basic auth `web-configurator:<PIN>`, on `UC_SOCKET_URL`'s host and port, or
port 80 if that does not answer. Uninstalling empties `UC_CONFIG_HOME` and `UC_DATA_HOME` (never `UC_STATE_HOME`), then
`DELETE /api/system/install/ui`, or `PUT ...?enable=false` if that is refused. Both turn the web configurator on with a PIN
of r3cui's own first and off again after if it was off (`UC_DATA_HOME/web-configurator-was-off` tells a restarted UI to
finish that). The REST calls are tested against a local fake server (`crates/r3cui/src/remote/selfupdate/`), not a remote.

## Voice: control settings, sessions, assistant events

`crates/r3cui/src/coreapi/src/voice.rs` types the requests, `events.rs` the event. `FakeCore` answers all of them and records
the entity commands. The session design is in `docs/voice.md`.

| Message | Notes |
|---|---|
| `get_voice_control_cfg` | answered with `voice_control_cfg`: `microphone`, `voice_assistant {active, profile_id, speech_response}`. No `active` means no assistant is set |
| `set_voice_control_cfg` | `microphone`, `voice_assistant {entity_id, profile_id, speech_response}`; a field left out keeps its value, an empty `entity_id` removes the assistant. r3cui sends it only when the user turns voice on in Settings |
| `get_voice_assistants` | `voice_assistants`: `entities[]` with `entity_id`, `name` (by language), `state`, `features` (`transcription`, `response_text`, `response_speech`), `profiles[]`, `preferred_profile` |
| `execute_entity_command` `voice_start` | `entity_id`, `cmd_id: "voice_start"`, `params {session_id, timeout, speech_response, profile_id?, audio_cfg?}`. The UI picks `session_id`. The stock UI sends `timeout` 15 and `speech_response` always, `audio_cfg` only from the entity's options. The reply comes once the integration accepted; `ready` follows |
| `execute_entity_command` `voice_end` | `entity_id`, `cmd_id: "voice_end"`, no `params`, as the stock UI sends it. The core ends the session it has running for that entity |
| event `assistant_event` (`cat: ENTITY`) | `msg_data {type, entity_id, session_id, data?}`. `type`: `ready`, `stt_response` (`data.text`), `text_response` (`data.success`, `data.text`), `speech_response` (`data.url`, `data.mime_type`), `finished`, `error` (`data.code`, `data.message`). Codes: `SERVICE_UNAVAILABLE`, `INVALID_AUDIO`, `NO_TEXT_RECOGNIZED`, `INTENT_FAILED`, `TTS_FAILED`, `TIMEOUT`, `UNEXPECTED_ERROR` |

`entity_id` is the core's id over all integrations (`<integration_id>.<entity_id>`), as `get_voice_assistants`
returns it. A new session is subscribed to every channel, so `assistant_event` arrives without a `subscribe_events`
naming the `assistant` channel; the client's `subscribe_events` sends no `channels` (the spec marks it required).
