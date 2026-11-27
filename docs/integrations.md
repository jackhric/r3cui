# The core's integrations (reference only)

r3cui doesn't use the core's integrations or its entity model. Kodi, Home Assistant and the rest
are **drivers** that r3cui runs itself, and the UI talks to them directly. For how that works, how
a driver is set up and how the hard keys are routed, see `docs/drivers.md`. For the Core-API
messages the UI does use (the remote's own state), see `docs/core-ws-protocol.md`.

The first plan was to read the core's entities (`get_entities`, `entity_change`,
`execute_entity_command`) and map the volume keys to a Home Assistant `media_player` through the
core. That path is gone: `crates/r3cui/src/coreapi` has no entity calls, and the core's receiver path and Kodi
fallback were removed when the drivers landed. The core-side entity notes that used to be here are in
the git history.

What is left is a summary of the Integration-API, in case a driver ever has to run as a core
integration instead (for example to show the remote in another system).

## Writing a core integration

Not used by r3cui. A custom integration is a `.tar.gz`
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
