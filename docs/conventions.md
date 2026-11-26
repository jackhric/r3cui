# Conventions

## No magic strings

A string or number that means something is named once, and everything else uses the name.

| What | Where |
|---|---|
| Home Assistant's wire vocabulary: message types, frame fields, domains, services, states, attributes | `crates/homeassistant/src/protocol.rs` (`MessageType`, the `field` module, ...) |
| Kodi's methods, parameters and shared value types | `crates/kodi/src/methods/` (one struct per method, checked against the vendored schema by `tests/schema.rs`) and `crates/kodi/src/wire.rs` |
| Core-API message names and the events the UI acts on | `crates/coreapi/src/messages.rs`, `crates/coreapi/src/events.rs`; the System page's requests are typed in `system.rs` |
| A driver's setup field keys | the driver's own `field` module: `drivers/src/homeassistant.rs`, `drivers/src/kodi/mod.rs` |
| The characters hard keys arrive as | `keys` in `crates/ui/src/input/mod.rs`; the window gets the same values through `KeyChars` in `crates/ui/ui/keys.slint` |
| Anything Slint and Rust both know about (pages, haptic events, library modes, ...) | a Slint `enum` in `crates/ui/ui/types.slint`, matched exhaustively in Rust |
| Timings | a named `const` at the top of the module that uses it, with a `Duration` type |

A new protocol message, field or domain goes in the client crate's constants first. A driver or
the UI never spells a wire string itself.

Slint and Rust agree through enums, not integers or strings. `PageId`, for one, is spelled for
`--page` and the `page` step in `view/screen.rs` by an exhaustive `match`, so a new page cannot be
left out. Slint contracts are checked when the crate compiles.

## Comments

Only where the code can't say it: a constraint, a gotcha, the reason for an unusual choice. Not
what the next line does. Match the density of the file you are in.

## Commits

Conventional commits: `type(scope): subject`, for example `refactor(ui): ...` or
`feat(drivers): ...`. CI checks pull request titles (`.github/workflows/pr-title.yml`), and
release-please builds the releases from them.

## Adding a driver

The recipe is in `docs/drivers.md` ("The contract", "Setup schema", and the note under "Crates").
In short:

1. A client crate if the program needs one, built on `wsclient` if it speaks WebSocket, with its
   wire names in a `protocol` module and a `testing` feature with a fake server.
2. A module under `crates/drivers/src/` with a `Spec`, a `field` module for its setup keys, and a
   type implementing `api::Driver`. It uses only `crate::api` and its own client crate.
3. One line in `BUILTIN` in `crates/drivers/src/registry.rs`.

The Integrations pages are drawn from the `Spec`, so no Slint changes.

## Tests

`scripts/test.sh` runs what CI runs: every crate's tests, then clippy with warnings as errors for
the desktop and the device build.

| Fake | Where | Used by |
|---|---|---|
| `FakeServer` | `crates/wsclient/src/testing.rs` (feature `testing`) | the scaffold under every fake below, and `coreapi`'s tests |
| `FakeCore` | `crates/coreapi/src/testing.rs` (feature `testing`) | `coreapi`'s tests: sign-in, `version`, firmware updates, API access |
| `FakeKodi` | `crates/kodi/src/testing.rs` | `crates/kodi/tests/fake.rs`, the Kodi driver |
| `FakeHa` | `crates/homeassistant/src/testing.rs` | `crates/homeassistant/tests/fake.rs`, the HA driver |
| `FakeTv` | `crates/androidtv/src/testing.rs` | `crates/androidtv/tests/fake.rs`, the Android TV driver (also `examples/fake-androidtv.rs`) |
| `FakeDriver` and its specs | `crates/drivers/src/testing.rs` (feature `testing`) | host routing and the Integrations screens, with no network |

`FakeKodi` and `FakeHa` also run as stand-alone servers for the desktop app (`examples/fake-kodi.rs`,
`examples/fake-ha.rs`). See `scripts/dev.md`.

Pure logic takes the time as an argument so tests can drive it (`Player` in
`crates/ui/src/hardware/feedback/player.rs`).
