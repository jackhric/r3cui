# Android TV client (`crates/androidtv`)

r3cui's own client for the **Android TV Remote protocol v2**: the one Google's phone remote app speaks to
Android TV and Google TV boxes. No developer options, no ADB; the box only needs to be on the network and
paired once. The crate has no UI dependency; a driver will wrap it (not written yet, see "Open gaps").

## The protocol

Two TLS services on the box, both carrying **varint-length-prefixed protobuf**:

| Port | Name | Messages | Purpose |
|---|---|---|---|
| 6467 | pairing | `polo.proto` (`OuterMessage`) | introduce our certificate, once |
| 6466 | remote | `remotemessage.proto` (`RemoteMessage`) | keys, state, pings, app launch |

TLS is mutual: the client presents a **self-signed RSA certificate** it generated itself, and the box's
certificate is self-signed too, so it is accepted without verification (`tls.rs`). Pairing is what ties the two
certificates together. The box only accepts a client certificate it has been paired with; otherwise it sends a TLS
alert (we report `Link::NotPaired`). TLS session resumption is switched off on our side so that a box which
forgot us says so instead of resuming past the certificate check.

Field numbers and the flow were verified against androidtvremote2 (`polo.proto`, `remotemessage.proto`,
`pairing.py`, `remote.py`, `certificate_generator.py`, `base.py`); see `docs/research-sources.md`.

### Pairing

1. TLS to :6467. Send `pairing_request { service_name: "atvremote", client_name }`.
2. Box: `pairing_request_ack`. We send `options` (input role, hexadecimal encoding, 6 symbols).
3. Box: `options`. We send `configuration` (same encoding, input role).
4. Box: `configuration_ack`; the **box now shows a 6-symbol hex code**.
5. The secret is `SHA-256(client modulus, client exponent, server modulus, server exponent, code[2..])`, each
   number as minimal big-endian bytes (no leading zeros), the last part the code's last two bytes. The first byte
   of the hash must equal the code's first byte, so a mistyped code is rejected locally (`Error::WrongCode`)
   before anything is sent, and the user can retype it on the same `Pairing`.
6. We send `secret`; the box answers `secret_ack` or status 402 (`WrongCode` as well; after that, pair again).

Every message is `OuterMessage { protocol_version: 2, status: 200, ... }`; any other status is an error.

### Remote channel

1. Box: `configure { code1 = feature bits, device_info }`. We answer `configure` with the features we want and the
   box supports (`ping | key | ime | power | volume | app_link`, IME because it is what makes the box report the
   current app), and our device info.
2. Box: `set_active`; we echo our active features.
3. Box: `start { started }` means the remote is ready; `started` is the box's on/off state (`Link::Up`).
4. Afterwards the box sends `set_volume_level`, `ime_key_inject` (its `app_info.app_package` is the foreground
   app), further `start` messages on power changes, and a `ping_request` about every 5 s. We must answer each with a
   `ping_response` (same `val1`) or the box hangs up. We hang up if nothing is heard for 16 s.
5. We send `key_inject { key_code, direction }` (short press, or start-long then end-long) and
   `app_link_launch_request { app_link }`.

## What the crate offers

```rust
let credentials = Credentials::generate()?;               // RSA 2048, slow: spawn_blocking
let mut pairing = pair("192.168.1.20", &credentials, "r3cui").await?;
pairing.finish("1A2B3C").await?;                          // the code shown on the TV

let tv = connect("192.168.1.20", &credentials)?;          // reconnects with backoff, forever
tv.press(KeyCode::DpadCenter)?;
tv.send_key(KeyCode::DpadCenter, Direction::StartLong)?;
tv.launch("https://www.youtube.com")?;
let state = tv.watch();                                   // watch::Receiver<TvState>
```

- `Credentials` is `{ cert_pem, key_pem }`, serde-serializable, for the driver to keep in `modules.json`. Losing it
  means pairing again.
- `Endpoint` is a host (IP or name) plus the two ports (`REMOTE_PORT` 6466, `PAIRING_PORT` 6467).
- `TvState { link, on, current_app, volume, model }`. `link` is `Connecting`, `Up`, `NotPaired` or `Down`. Everything
  but `link` resets when the connection drops, so a stale "on" never lingers.
- `send_key`/`launch` fail with `Error::NotConnected` until `Link::Up`; a stale key is never replayed after a
  reconnect.
- Reconnecting uses `wsclient::Backoff` (0.5, 1, 2, 4, 8, then 15 s). `wsclient`'s `Session` and `reconnect` are
  WebSocket-shaped (they hand out a `Connection`) so they are not used; only `Backoff` is generic. Depending on
  `wsclient` also pulls in tungstenite; moving `Backoff` to a tiny shared crate would avoid that.
- `protocol::KeyCode` carries Android's `KeyEvent` numbers (d-pad, back, home, menu, media, channel, volume,
  power, digits, colour keys, ...). `KEYCODE_MUTE` (91) is the microphone, so it is `MicMute`; speaker mute is
  `VolumeMute` (164).

Dependencies: `tokio-rustls` + `rustls` with the `ring` provider (no OpenSSL, no aws-lc), `rcgen` (certificate),
`rsa` (RSA key generation: ring cannot generate RSA keys), `x509-parser` (read the RSA numbers out of a
certificate), `sha2`, `prost` (hand-written derive structs, no protoc), `thiserror`, `serde`, `tracing`, `wsclient`.

## Testing

`FakeTv` (feature `testing`, `src/testing.rs`) is a TLS fake box that behaves like the real one on both ports:
pairing shows a code and verifies the secret with the same hash, the remote channel admits only paired
certificates, sends the handshake, initial volume/app/power, and pings, and records the keys and launches it gets.
It can push power, app and volume changes, hang up on every client (`kick`) and forget them (`forget_clients`).

`crates/androidtv/tests/fake.rs` covers: pairing with the right code, a wrong code (nothing is paired, retyping
works), keys and launches arriving with their direction, state arriving and following pushes, pings being
answered, reconnecting after the box hangs up, `NotPaired` after the box forgets us, and an unreachable box.

Examples:

```
cargo run -p androidtv --features testing --example fake-androidtv   # FakeTv on 127.0.0.1:6466/6467, prints codes
cargo run -p androidtv --example atv-pair  -- <host> [credentials.json]        # prompts for the code
cargo run -p androidtv --example atv-watch -- <host> [credentials.json] [key]  # prints state, presses a key
```

`atv-pair` and `atv-watch` work against `fake-androidtv` with host `127.0.0.1`, or against a real box by IP.

## Open gaps

- **Never run against a real box.** Everything is checked against the reference library's source and against
  `FakeTv`, which was written from the same source. The things most likely to differ on hardware: which TLS
  version and signature schemes the box offers, whether a real box accepts our certificate fields (we use
  CN `r3cui`, CA with path length 0 like the reference), how the "not paired" rejection looks (we map a received TLS
  alert to `NotPaired`; a plain reset shows up as `Down`), and whether `ime_key_inject` reports the app on every box
  (the reference notes some boxes do not support IME).
- RSA key generation on the device is untimed; on the Remote 3's Cortex-A53 expect seconds. It can be done once at
  setup, on the desktop, or on first use.
- Not implemented: text entry (IME batch edit), voice, `adjust_volume_level`, preferred audio device. Volume is
  changed with `VolumeUp`/`VolumeDown`/`VolumeMute` keys.
- No mDNS discovery (`_androidtvremote2._tcp`); the user enters an IP.
- The driver (`crates/drivers`) is a follow-up: setup fields (host, pairing code), `Credentials` in `modules.json`,
  an entity mapped from `TvState` and the key commands.
