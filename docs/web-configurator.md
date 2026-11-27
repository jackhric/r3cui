# r3cui web configurator

r3cui's own settings page in a browser, separate from Unfolded Circle's. It sets up the same things as Settings →
Integrations on the remote (integrations, global control, inputs) and writes the same `modules.json`. Nothing in it is
written per integration: every form is laid out from the integration's `Spec` (`crates/sdk/src/api.rs`), so an
integration added to the app shows up here as it is.

## Switching it on

Settings → Integrations → **Set up from a computer** opens a page on the remote that switches the configurator on and
shows its address, a QR code and a four-digit PIN. It listens on port **10781** (`webconfig::PORT`), beside remote
control's 10780. It switches off with Turn off, when that page's New PIN replaces the PIN, after 30 minutes without a
request, or when r3cui restarts. Off, nothing listens.

## The page

`GET /tune` is a tuning page for the voice warm-up: its curve and stage points (`view::voice::WarmTuning`), read
with `GET /api/tune` and set with `PUT /api/tune` (`{"curve": [x1, y1, x2, y2], "spread_until", "climb_from"}`),
applied at once and kept until r3cui restarts. It shows the lines to paste into `view/voice.rs` to keep a curve.

`GET /` is the page (`crates/r3cui/src/webconfig/page.html`, one file: HTML, CSS and script). `GET /fonts/<name>.ttf`
serves the app's own Inter and JetBrains Mono, so the page needs nothing from the internet.

## The API

JSON over HTTP on the same port. Every call but `POST /api/login` needs `Authorization: Bearer <token>`; without a
valid one the answer is `401`. Errors answer `4xx` with `{"error": "<what went wrong, for a person>"}`.

| Call | Body | Answer |
|---|---|---|
| `POST /api/login` | `{"pin": "4817"}` | `{"token": "<hex>"}`, or `401` |
| `GET /api/config` | | `Config` |
| `GET /api/targets?driver=<id>&kind=<target kind>` | | `[Target]`, asked of the integration afresh (may take a few seconds) |
| `POST /api/integrations` | `{"kind": "homeassistant"}` | `Config` (the new integration is last in `integrations`) |
| `PUT /api/integrations/<id>` | `{"fields": {"<key>": <value>}}` | `Config` |
| `DELETE /api/integrations/<id>` | | `Config` |
| `POST /api/integrations/<id>/pair` | | `{}` once the device shows its code |
| `POST /api/integrations/<id>/pair/code` | `{"code": "A1B2C3"}` | `Config`, or `400` with why |
| `PUT /api/global/<role>` | `{"driver": "<id>" or null, "target": "<id>"}` | `Config` |
| `PUT /api/inputs/<id>` | any of `{"name", "hidden", "remote", "ui"}` | `Config` |

In `PUT /api/integrations/<id>`, `fields` holds only what changed. A text, URL or port field takes a string (empty
clears it); a toggle takes `true` or `false`; a target takes the target's id or `null`; a secret takes a string, and is
left alone when left out (a secret is never sent back).

### Config

```json
{
  "remote": { "version": "0.1.0+g926bd44" },
  "integrations": [
    {
      "id": "homeassistant",
      "kind": "homeassistant",
      "name": "Home Assistant",
      "icon": "<SVG path data on a 24×24 box>",
      "status": { "health": "not_set_up", "text": "Not set up" },
      "fields": [
        { "key": "url", "label": "Address", "kind": "url", "required": true, "value": "https://homeassistant.internal:8123" },
        { "key": "token", "label": "Access token", "kind": "secret", "required": true, "set": false },
        { "key": "self_signed", "label": "Allow self-signed certificate", "kind": "toggle", "required": false, "value": true },
        { "key": "volume_limit", "label": "Volume limit", "kind": "target", "target_kind": "sensor", "required": false,
          "value": null, "value_name": null }
      ],
      "pairing": null
    }
  ],
  "addable": [ { "kind": "kodi", "name": "Kodi", "icon": "..." } ],
  "global": {
    "driver": "homeassistant",
    "roles": [
      { "role": "volume", "label": "Volume", "target_kind": "media_player",
        "bound": { "driver": "homeassistant", "target": "media_player.receiver", "name": "Receiver" },
        "providers": [] },
      { "role": "power", "label": "Power", "target_kind": "media_player", "bound": null,
        "providers": [ { "driver": "androidtv", "name": "Android TV" } ] }
    ]
  },
  "inputs": [
    { "id": "KODI", "name": "Kodi", "hidden": false, "remote": "kodi", "ui": "kodi",
      "remote_choices": [ { "driver": "kodi", "name": "Kodi" } ], "ui_choices": [ { "driver": "kodi", "name": "Kodi" } ] }
  ]
}
```

- `health` is one of `up`, `connecting`, `down`, `not_set_up`, `not_paired`; `text` is what to show.
- A field's `kind` is `text`, `url`, `port`, `secret`, `toggle` or `target`. Credentials (written by pairing) are left
  out. `value` is a string, a bool for a toggle, a target id (with its `value_name` when known) for a target, or
  `null` when unset; a secret has `set` instead of `value`.
- `pairing` is `{"code_len": 6}` for an integration paired by a code the device shows, else `null`.
- `global.driver` is the integration that can be global control (`null` when none is set up). A role is bound to one
  of its targets of `target_kind`, or, for power, to one of the `providers`, or to nothing. Roles: `volume`, `source`
  (shown as Inputs), `power`, `voice`; voice's `target_kind` is `pipeline`, whose empty id is the preferred pipeline.
- `inputs` are in the order the remote lists them, hidden ones included; `remote` and `ui` are driver ids or `null`.
