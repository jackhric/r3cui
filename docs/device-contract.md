# Device contract — launch env, packaging, install

Source: `unfoldedcircle/remote-ui` (commit 6b3e54a3, 2026-11-18) specs + `core-api` OpenAPI.
Everything here is from public upstream unless marked "not published".

## Rendering / display

- Stock UI renders with **Qt `eglfs` on OpenGL ES 2, KMS/GBM, no window manager**. Firmware Qt is
  built `-no-linuxfb -no-directfb -no-xcb -opengl es2 -qpa eglfs -kms -gbm`. There is no compositor;
  a second client can't open the display concurrently.
- Panel: **480×800 portrait** on UCR3 (UCR2 is 480×854 landscape, rotated −90°; not our target).
  ~50 Hz.
- Firmware sets the eglfs/KMS config, font dir, and input params; exact values **not published**.

## Sandbox permissions (our slot)

- Runs as an unprivileged user, empty read-only root FS.
- Devices: **display, video, media, `hidraw`, audio = read-write; input devices = read-only;**
  everything else closed. (So: direct KMS + evdev-read is exactly what's allowed.)
- Network open (loopback + LAN, system DNS). Binary budget < 100 MB.

## Launch environment (set by firmware for the UI process)

| Variable | Meaning |
|---|---|
| `UC_MODEL` | `UCR3` on device (`DEV` on desktop) |
| `UC_SOCKET_URL` | core WS endpoint; default `ws://127.0.0.1:8080/ws` (device value not published) |
| `UC_TOKEN_PATH` | path to the auth token file — read it, trim whitespace, send in the `auth` message |
| `UC_CONFIG_HOME` | our per-UI config dir (`./config` in the archive) |
| `UC_DATA_HOME` | our per-UI data dir (`./data` in the archive) |
| `UC_STATE_HOME` | writable, shared with factory UI; holds core token + onboarding marker |
| `UC_TOUCHSLIDER_DEV_PATH` | evdev node for the UCR3 capacitive slider (read directly; r3cui finds the slider by its device name instead and doesn't read this) |
| `UC_HAPTIC_DEV_PATH` | sysfs path for the haptic motor (write to pulse) |
| `UC_SHARE_HOME`, `UC_LEGAL_PATH`, `UC_RESOURCE_PATH`, `UC_ONBOARDING_PATH` | misc paths |
| `UC_UI_REQUEST_TIMEOUT` | optional, default 10000 ms |

Not provided to custom builds: `UC_ICON_FONT_PATH`, `UC_SOUND_EFFECTS_PATH` (licensed assets — ship
our own). Only `./bin`, `./config`, `./data` from the archive are accessible at runtime.

## Packaging

Archive: a `.tar.gz`/`.tgz` with, at the root:
- `release.json` (metadata — see `packaging/release.json`),
- `bin/remote-ui` (our binary — fixed path, this is the entrypoint),
- optional `config/`, `data/`.
Symlinks are stripped. `release.json` has **no strictly-required fields** per the schema, but set
`name`, `version`, `release_date`, `requirements.firmware_version` (Cargo-style semver, e.g.
`">=0.13.0, <1.0.0"`).

## Install / manage (Core-API)

```
# install (⚠ writes permanent void_warranty flag)
curl -u 'web-configurator:<PIN>' \
  'http://<IP>/api/system/install/ui?void_warranty=yes' \
  --form 'file=@"out/r3cui.tar.gz"'

GET    /api/system/install          # list installed custom components
GET    /api/system/install/ui       # status of our UI
PUT    /api/system/install/ui?enable=false   # revert to factory UI (keeps ours installed)
DELETE /api/system/install/ui       # remove ours
POST   /api/system  {"cmd":"RESTART_UI"}     # restart UI after install
```
Errors: 400 invalid archive · 403 no admin / missing void_warranty · 409 firmware incompatible ·
507 out of storage. Repeated boot failures → recovery service auto-reverts to factory UI + reboots.
