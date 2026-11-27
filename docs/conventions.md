# Conventions

## No magic strings

A string or number that means something is named once, and everything else uses the name.

| What | Where |
|---|---|
| Home Assistant's wire vocabulary: message types, frame fields, domains, services, states, attributes | `crates/homeassistant/src/protocol.rs` (`MessageType`, the `field` module, ...) |
| Kodi's methods, parameters and shared value types | `crates/kodi/src/methods/` (one struct per method, checked against the vendored schema by `tests/schema.rs`) and `crates/kodi/src/wire.rs` |
| Core-API message names and the events the UI acts on | `crates/coreapi/src/messages.rs`, `crates/coreapi/src/events.rs`; the System page's requests are typed in `system.rs` |
| A driver's setup field keys | the integration's own `driver::field` module: `crates/homeassistant/src/driver.rs`, `crates/kodi/src/driver/mod.rs` |
| The characters hard keys arrive as | `keys` in `crates/ui/src/input/mod.rs`; the window gets the same values through `KeyChars` in `crates/ui/ui/globals/keys.slint` |
| Anything Slint and Rust both know about (pages, haptic events, library modes, ...) | a Slint `enum` in `crates/ui/ui/types.slint`, matched exhaustively in Rust |
| Timings | a named `const` at the top of the module that uses it, with a `Duration` type |
| The UI's colours, lengths, text sizes and animation timings | `Theme`, `Layout`, `Type` and `Motion` in `crates/ui/ui/theme.slint`, named by role (`Layout.margin`, `Type.label`), never a literal that repeats one |

A new protocol message, field or domain goes in the client crate's constants first. A driver or
the UI never spells a wire string itself.

Slint and Rust agree through enums, not integers or strings. `PageId`, for one, is spelled for
`--page` and the `page` step in `view/screen.rs` by an exhaustive `match`, so a new page cannot be
left out. Slint contracts are checked when the crate compiles.

## One primitive per look

Every page inherits `Layer` (`widgets/layer.slint`), which slides it in and out and slides its own nested screens
(`Nav.inner`). A page is either a list or unique:

- A **list** hands its rows to `ChoiceList`.
- A **unique** page (System, an update, the web configurator) puts its parts in a `PageBody`
  (`widgets/page-body.slint`): they stack down from under the header in the page's order, with `Gap`s from the
  `Layout.gap-*` scale between them, `Inset` keeps a part inside the side margins, and parts after a `Spring` sit at
  the bottom. A unique page sets no `y` of its own.

Every row on either is `Row` (`widgets/row.slint`), with `size`, a leading glyph or mark, `danger` and a slot at the
right edge for what a row needs beyond that; a section label over rows is `Caption`. A new look goes into the
primitive, not into a page.

## Comments

Only where the code can't say it: a constraint, a gotcha, the reason for an unusual choice. Not
what the next line does. Match the density of the file you are in.

## Commits

Conventional commits: `type(scope): subject`, for example `refactor(ui): ...` or
`feat(drivers): ...`. CI checks pull request titles (`.github/workflows/pr-title.yml`), and
release-please builds the releases from them.

## Adding an integration

An integration is one crate on `r3cui-sdk`: its client, its `Spec` and `field` keys, its `Driver`, its fake, its
tests, exported as `driver::INTEGRATION`. The app carries it with a Cargo dependency and one line in
`crates/ui/src/app/registry.rs`. The recipe is in `docs/drivers.md`, "Writing an integration". The Integrations
pages are drawn from the `Spec`, so no Slint changes.

## Tests

`scripts/test.sh` runs what CI runs: every crate's tests, then clippy with warnings as errors for
the desktop and the device build.

| Fake | Where | Used by |
|---|---|---|
| `FakeServer` | `crates/wsclient/src/testing.rs` (feature `testing`) | the scaffold under every fake below, and `coreapi`'s tests |
| `FakeCore` | `crates/coreapi/src/testing.rs` (feature `testing`) | `coreapi`'s tests: sign-in, `version`, firmware updates, API access |
| `FakeKodi` | `crates/kodi/src/testing.rs` | `crates/kodi/tests/fake.rs`, its driver |
| `FakeHa` | `crates/homeassistant/src/testing.rs` | `crates/homeassistant/tests/fake.rs`, its driver |
| `FakeTv` | `crates/androidtv/src/testing.rs` | `crates/androidtv/tests/fake.rs`, its driver (also `examples/fake-androidtv.rs`) |
| `FakeDriver` and its specs | `crates/host/src/testing.rs` (feature `testing`) | host routing and the Integrations screens, with no network |

`FakeKodi` and `FakeHa` also run as stand-alone servers for the desktop app (`examples/fake-kodi.rs`,
`examples/fake-ha.rs`). See `scripts/dev.md`.

Pure logic takes the time as an argument so tests can drive it (`Player` in
`crates/ui/src/hardware/feedback/player.rs`).
