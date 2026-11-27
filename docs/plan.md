# Build plan

## Goal

A from-scratch Rust `remote-ui` for the UCR3, installed via the Core-API custom-UI slot,
rendering to the device's GLES2/KMS display. The closed `remote-core` is used for the remote's own
state (power, battery, Wi-Fi, display); the receiver, Kodi and the hard keys' targets are r3cui's own
drivers (`docs/drivers.md`).

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
nicely with tokio. Caveat: the LinuxKMS backend is labelled "experimental". The device build tries GLES2 first and falls back to
the software renderer if the GPU path won't start (`select_device_renderer` in `crates/ui/src/main.rs`).

## Architecture

| Layer | Choice |
|---|---|
| UI / render | `slint`; `backend-winit` + FemtoVG on the desktop (the default `desktop` feature); `backend-linuxkms` + FemtoVG (GLES2) on the device (the `device` feature), with the software renderer compiled in as fallback |
| Slint source | `crates/ui/ui/`: `app.slint` (window, key router, layer order), `theme.slint` (colours, sizes, timing), `types.slint` (enums and structs shared with Rust), `globals/` (state per area, with `Nav` for pages), `marks/` (icons, logos, codec and brand marks as path data, and what draws them), `widgets/` (shared pieces: `Layer`, `PageHeader`, `ChoiceList`, `Row`, `Bar`), `pages/` (one per page; `system/` and `wifi/` group their flows), `stage/` (the home screen and the library wall), `overlays/`, `assets/` (fonts, icons, rating badges) |
| Display | GBM/DRM-KMS, 480×800 portrait; `SLINT_KMS_ROTATION`, `SLINT_DRM_OUTPUT`/`SLINT_DRM_MODE` as needed |
| Input | On the device, `crates/ui/src/input/evdev.rs` reads the touchscreen, the hard keys and the capacitive slider itself and hands them to the window (libinput is left out of the device build). On the desktop the keyboard stands in |
| WebSocket | `tokio` + `tokio-tungstenite`. `crates/wsclient` is the shared connection, backoff, pending-call table and reconnecting `Session`; `coreapi`, `kodi` and `homeassistant` each run on it and keep their own protocol. Bridged to Slint via `invoke_from_event_loop` |
| Remote's own state | `crates/coreapi`: auth by reading `UC_TOKEN_PATH`, the `kind/id/msg/msg_data` envelope, typed power, battery and Wi-Fi events. No entities: see `docs/core-ws-protocol.md` |
| Other programs | Integrations, each one crate with its client and driver on `crates/sdk` (`r3cui-sdk`): `homeassistant`, `kodi`, `androidtv`, `demo`. `crates/host`: `modules.json`, the input list, the host that routes keys and views. The app lists what it carries in `crates/ui/src/app/registry.rs` |
| Packaging | `scripts/build-device.sh` cross-compiles aarch64 in a container against the manufacturer's SDK, `packaging/pack.sh` makes the `.tar.gz` (`release.json`, `bin/remote-ui`, `config/`); CI builds it and release-please publishes it. Install via `POST /system/install/ui?void_warranty=yes`, or `install.sh` / `install.bat` |

The `crates/ui` binary is the only crate that knows Slint. `coreapi`, `wsclient`, `kodi`,
`homeassistant`, `androidtv`, `r3cui-sdk` and `host` have no UI dependency and are tested on their own, against the fakes
described in `docs/conventions.md`.

## Milestones

The plan as first written, with where each stands.

1. **Desktop spike vs `core-simulator` (no device).** Done. Authenticate over WS, show the result in
   a Slint window (`backend-winit`). The spike listed the core's entities; that path was later
   removed (see 2).
2. **First real app: Kodi remote + HA volume keys.** Done, but not as first planned. The plan was to
   read the Kodi and HA entities through the core. Instead r3cui got its own Kodi and Home Assistant
   clients, then the driver layer on top (global volume and source, a remote slot and a UI slot per
   input, `modules.json`, Settings → Integrations), and the core's entity path was deleted.
   The app grew a Now Playing stage, a poster wall, track pickers, key backlighting and haptics.
3. **Cross-compile to aarch64** with `backend-linuxkms`, using the `ucr2-toolchain`
   Buildroot sysroot. Done: `scripts/build-device.sh`, `packaging/pack.sh`, and a CI job that builds
   the package.
4. **Device bring-up (irreversible: writes `void_warranty`).** Done on the owned unit; the repo has
   install and uninstall scripts and a release workflow. Revert with `?enable=false` / `DELETE`.
   Later device work (haptics, key backlight, 60 fps animation) is in the git history.

Since then: the Slint side was split into pages, stages, overlays, widgets and globals with typed
enums; the demo became the real session over the Demo driver; `scripts/test.sh` is what CI runs.

## Open questions

- The real GPU and driver, and whether the GLES2 path always starts or falls back to software. The
  device build logs which renderer it picked.
- Whether the core accepts a partial/empty `release.json` (we ship a complete one).
- Whether the sandbox lets the UI open the sound card (`docs/haptics.md`, "Audio").
- Pairing drivers by code instead of a pasted token (`docs/drivers.md`, "Later").
