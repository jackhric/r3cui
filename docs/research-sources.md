# Research sources & provenance

The facts in these docs come from reading Unfolded Circle's public repos (not from memory).
Recon on the specific physical unit lives in the sibling repo `../ucr3-cfw`.

## Upstream repos

| Repo | What we used it for |
|---|---|
| [`unfoldedcircle/remote-ui`](https://github.com/unfoldedcircle/remote-ui) | Stock GPLv3 Qt/QML UI. ADRs (rendering, input, hardware minimalism), `src/core/core.cpp` (WS client reference), `docs/`, `openspec/` specs, launch env, packaging. Commit 6b3e54a3. |
| [`unfoldedcircle/core-api`](https://github.com/unfoldedcircle/core-api) | **Authoritative API specs.** `websocket/UCR-core-asyncapi.yaml` (v0.50.5) and `rest/UCR-core-openapi.yaml` (install endpoints, `release.json` schema). |
| [`unfoldedcircle/ucr2-toolchain`](https://github.com/unfoldedcircle/ucr2-toolchain) | aarch64 Buildroot SDK + sysroot (glibc, fontconfig, freetype, openssl, glib). Reused as the cross toolchain for our Rust build (we don't need its Qt). |
| [`unfoldedcircle/core-simulator`](https://github.com/unfoldedcircle/core-simulator) | Dockerized real core for desktop dev. Milestone-1 target. |

## Device facts (UCR3 under test — from ../ucr3-cfw)

- Model UCR3, OS 2.8.4, core 0.69.4-bt, UI 0.70.1, Core-API 0.16.0. Admin API key available.
- Custom-UI slot is empty (clean). A custom integration (Kodi) already runs → on-device custom
  code path is proven live on this unit. `remote-ui` scope exists for the UI WS role.

## Rust GUI survey (library decision — see plan.md)

Slint chosen. Key source: [Slint LinuxKMS backend docs](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_linuxkms/).
egui fallback reference for a hand-rolled DRM/GBM/EGL path: the `wizdraw` crate's `drm_kms.rs`.

## Not published upstream (discover on device)

- Exact `/dev/input/event*` paths, `UC_TOUCHSLIDER_DEV_PATH`, `QT_QPA_*`/eglfs KMS config values.
- Real `UC_SOCKET_URL` and token filename on the device.
- GPU + driver (likely Panfrost/Mali) — probe `/dev/dri` + `eglinfo`.
- Whether the core accepts a partial/empty `release.json`, and per-message `msg_data` details for
  every message (enumerate from the AsyncAPI YAML as needed).
