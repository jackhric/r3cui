# Core-API WebSocket protocol

This is reference for the Core-API, not a description of what r3cui sends. The UI uses the core only for
the remote's own state (`crates/coreapi`). It no longer reads entities, entity commands or activities from
the core: the receiver, Kodi and the hard keys' targets are drivers (`docs/drivers.md`), and the entity,
activity and button-mapping rows below are kept as spec notes only.

Requests the UI sends today (`crates/ui/src/remote/`, `hardware/backlight.rs`): `get_power_mode`,
`get_battery_charger`, `get_display_cfg` / `set_display_cfg`, `get_haptic_cfg`, `get_sound_cfg`,
`get_power_saving_cfg`, `get_network_cfg` / `set_network_cfg`, `get_wifi_status`, `get_wifi_scan_status`,
`wifi_scan_start`, `add_wifi_network`, `set_button_cfg`. Events it acts on (`coreapi::events`):
`power_mode_change`, `battery_status`, `wifi_change`. The connection runs on `crates/wsclient`.

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
`tokio-tungstenite` answers WS pings automatically. The UI retries every 2 s after a drop (`RECONNECT_DELAY` in `crates/ui/src/app/session.rs`).

## Messages a UI could use

| Area | Messages |
|---|---|
| Events | `subscribe_events` (new session is auto-subscribed to `all`), `unsubscribe_events`, `get_event_channels`. Channels: `entities`, `entity_<type>`, `activity_groups`, `battery_status`, `wifi`, `network`, `integrations`, `software_updates`, `docks`, `configuration`, `profiles`, `bt`, `assistant` |
| Profiles/pages | `get_active_profile`, `get_profiles`, `switch_profile`, `get_pages`, `get_groups` |
| Entities | `get_entities` (filterable; paginated via `msg_data.paging: {page, limit}`, default limit **10**, max 100; response carries `paging: {count, limit, page}`), `get_entity`, `get_entity_types`, `get_entity_commands`, `execute_entity_command` (`{entity_id, cmd_id, params}`), `browse_media`, `search_media`; live updates via `entity_change` event |
| Activities | Activities are entities (`entity_activity`); start/stop via `execute_entity_command`; progress via `entity_change`. Button mappings via `get_activity_button_mappings`-style messages (`button_mapping` data) |
| Battery | `get_battery_charger`; event `battery_status` (`cat:REMOTE`) |
| WiFi/net | `get_network_state`; events `wifi_change`, `network_change` |
| Power | `get_power_mode`, `set_power_mode`; event `power_mode_change` (incl. `SUSPEND`) |
| System | `version`, `system`, `get_configuration`, `get_*_cfg` (display/haptic/button/device/sound/localization); event `warning` |
| Voice | `get_voice_assistants`; event `assistant_event` |

## Gotcha: physical buttons are not on the WS

Button presses and the capacitive slider arrive via **evdev in our process**, not as WS events.
The stock UI fetches the activity's button mappings and sends the mapped `execute_entity_command`
(short vs long press distinguished ~800 ms). r3cui doesn't: the drivers host routes the keys
(`docs/drivers.md`, "Buttons").
