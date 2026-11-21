# Build plan

## Goal

A from-scratch Rust `remote-ui` for the UCR3, installed via the Core-API custom-UI slot,
rendering to the device's GLES2/KMS display and driving the closed `remote-core` over its
WebSocket.

## Why Slint (library decision)

Surveyed Rust GUI toolkits for a no-compositor, direct-to-KMS, aarch64 touchscreen target:

| Library | Direct KMS backend? | Input built in | Verdict |
|---|---|---|---|
| **Slint** | **Yes — `linuxkms`** (GLES2 via FemtoVG, or software on DRM dumb buffers) | **Yes (libinput)** | **Chosen** |
| egui/eframe | No (winit); custom integration possible | No | Fallback — hand-roll EGL/GBM + evdev |
| iced | No | No | Needs a custom shell — out |
| embedded-graphics | No (drawing lib only) | No | You build the whole toolkit — out |
| tiny-skia / raqote DIY | No | No | Only if we want full manual control |
| Makepad / Freya / Dioxus-Blitz | No confirmed KMS path | winit | Out |
| FerroUI | Yes (fb + DRM/EGL) | libinput/evdev | Too early-stage to bet on |

Slint wins because it ships the one maintained direct-to-KMS backend **and** input, matches the
device's GLES2/KMS/GBM stack, has a software-renderer fallback (same code), is GPL-3.0, and plays
nicely with tokio. Caveat: the LinuxKMS backend is labelled "experimental" and we haven't yet
confirmed the remote's GPU driver (likely Panfrost/Mali) — verify on device.

## Architecture

| Layer | Choice |
|---|---|
| UI / render | `slint`, `backend-linuxkms` + FemtoVG (GLES2) on device; software renderer compiled in as fallback; `backend-winit` for the desktop spike |
| Display | GBM/DRM-KMS, 480×800 portrait; `SLINT_KMS_ROTATION`, `SLINT_DRM_OUTPUT`/`SLINT_DRM_MODE` as needed |
| Input | Slint libinput for touch/keys **+** a dedicated `evdev` reader thread for the UCR3 capacitive slider and for resolving physical button → `execute_entity_command` |
| Async / WS | `tokio` + `tokio-tungstenite` → `UC_SOCKET_URL`; auth by reading `UC_TOKEN_PATH`; `serde` for the `kind/id/msg/msg_data` envelope; bridge to Slint via `invoke_from_event_loop` |
| Protocol | `crates/coreapi`: typed Core-API WS client, coded against the `core-api` AsyncAPI spec and diffed against stock `src/core/core.cpp` |
| Packaging | cross-compile aarch64 → `.tar.gz` (`release.json` + `bin/remote-ui`); install via `POST /system/install/ui?void_warranty=yes` |

## Milestones

1. **Desktop spike vs `core-simulator` (no device).**
   `docker compose up` the sim (see `scripts/`), read its token, authenticate over WS, dump
   `get_entities`, show them in a minimal Slint window (`backend-winit`). Proves the full protocol
   path with zero device risk. ← the scaffold in this repo is aimed here.
2. **First real app: Kodi remote + HA volume keys** (still desktop, vs simulator). Flesh out the
   protocol module (typed messages; consider the `uc_api` crate), then build the target app:
   render the Kodi `media_player`/`remote` entity as the on-screen remote, and map the physical
   `VOLUME_UP`/`VOLUME_DOWN` evdev keys to a Home Assistant `media_player`'s `volume_up`/`volume_down`
   commands. See `docs/integrations.md` for the entity/command specifics and the one-time
   "is the entity configured?" check. This exercises get_entities, subscribe/`entity_change`,
   `execute_entity_command`, and the evdev→command wiring — the core of the whole UI.
3. **Cross-compile to aarch64** with `backend-linuxkms` + `dlopen`, using the `ucr2-toolchain`
   Buildroot sysroot (has fontconfig/freetype/openssl — Slint needs fontconfig). Produce the
   `.tar.gz` via `packaging/pack.sh`.
4. **Device bring-up (irreversible: writes `void_warranty`).** Minimal first goal: a Slint screen
   on the real panel + a confirmed WS connect with the real token — validates KMS/EGL/input on the
   actual GPU. Then iterate the UI. Revert with `?enable=false` / `DELETE` if needed.

   Pre-step: probe the device GPU (`/dev/dri`, `eglinfo`, driver name) to confirm the GLES2 path.

## Open questions to resolve on-device

- GPU + driver (Panfrost? Mali model?) → GLES2 FemtoVG vs software renderer.
- Real `/dev/input/event*` mapping (touch, keys, slider) and `UC_TOUCHSLIDER_DEV_PATH` value.
- Actual `UC_SOCKET_URL` and token filename on the device.
- Whether the core accepts a partial/empty `release.json`.
