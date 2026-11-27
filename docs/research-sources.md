# Research sources & provenance

The facts in these docs come from reading Unfolded Circle's public repos (not from memory).
Device-specific research on this owned unit lives in the sibling repo `../ucr3-cfw`.

## Why the custom-UI route

This is the manufacturer's **supported** way to replace the UI on your own remote: the stock
`remote-ui` is GPLv3 and explicitly meant to be swapped via `POST /system/install/ui`. We take
this route because the device offers no shell — SSH is Dropbear, public-key only, and we hold no
key, so there is no filesystem or OS-level access to work with. The custom-UI slot is the intended,
documented interface for exactly this, and it needs no shell.

## Upstream repos

| Repo | What we used it for |
|---|---|
| [`unfoldedcircle/remote-ui`](https://github.com/unfoldedcircle/remote-ui) | Stock GPLv3 Qt/QML UI. ADRs (rendering, input, hardware minimalism), `src/core/core.cpp` (WS client reference), `docs/`, `openspec/` specs, launch env, packaging. Commit 6b3e54a3. |
| [`unfoldedcircle/core-api`](https://github.com/unfoldedcircle/core-api) | **Authoritative API specs.** `websocket/UCR-core-asyncapi.yaml` (v0.50.5) and `rest/UCR-core-openapi.yaml` (install endpoints, `release.json` schema). |
| [`unfoldedcircle/ucr2-toolchain`](https://github.com/unfoldedcircle/ucr2-toolchain) | aarch64 Buildroot SDK + sysroot (glibc, fontconfig, freetype, openssl, glib). Reused as the cross toolchain for our Rust build (we don't need its Qt). |
| [`unfoldedcircle/core-simulator`](https://github.com/unfoldedcircle/core-simulator) | Dockerized real core for desktop dev (`scripts/sim.sh`). |

## Device facts (UCR3 under test — from ../ucr3-cfw)

- Model UCR3, OS 2.8.4, core 0.69.4-bt, UI 0.70.1, Core-API 0.16.0. Admin API key available (made
  through the normal web-configurator PIN flow).
- Custom-UI slot is empty (clean). The device already runs a user-installed custom integration
  (Kodi), confirming the supported custom-code install paths work on this unit. The `remote-ui`
  scope exists for the UI's WebSocket role.

## Android TV Remote protocol (crates/androidtv)

- [`tronikos/androidtvremote2`](https://github.com/tronikos/androidtvremote2) (Apache-2.0): `polo.proto` and
  `remotemessage.proto` are the source of the protobuf field numbers and enum values in
  `crates/androidtv/src/protocol/`; `pairing.py`, `remote.py`, `base.py` and `certificate_generator.py` are the source
  of the pairing hash, the handshake order, the varint framing, the ping and idle behaviour, and the ports. Those
  files in turn credit Google's polo pairing protocol and `louis49/androidtv-remote`. Verified by reading the
  source; not yet by talking to a real box.

## Rust GUI survey (library decision — see plan.md)

Slint chosen. Key source: [Slint LinuxKMS backend docs](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_linuxkms/).
egui fallback reference for a hand-rolled DRM/GBM/EGL path: the `wizdraw` crate's `drm_kms.rs`.

## Not published upstream (read from our own device at runtime)

- Exact `/dev/input/event*` paths, `UC_TOUCHSLIDER_DEV_PATH`, `QT_QPA_*`/eglfs KMS config values.
- Real `UC_SOCKET_URL` and token filename on the device.
- GPU + driver (likely Panfrost/Mali) — probe `/dev/dri` + `eglinfo`.
- Whether the core accepts a partial/empty `release.json`, and per-message `msg_data` details for
  every message (enumerate from the AsyncAPI YAML as needed).
