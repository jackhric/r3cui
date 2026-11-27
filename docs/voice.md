# Voice: hold to talk, sent where r3cui says

Status (2026-11-26): built end to end and tested against fakes; the demo answers made-up speech
(`scripts/voice.steps`). Nothing has run on the remote yet: the relay is not installed, the microphone is off and
no voice assistant is set (see "Setting it up on the remote").

Hold the voice key, talk, let go. r3cui shows that it is listening the moment the key goes down, sends the audio
to wherever Global control → Voice points (Home Assistant Assist first), and shows what was heard and the answer.

## Why an integration

`remote-core` owns the microphone. Nothing else on the remote can record, and the core sends the audio to one
place only: the integration whose `voice_assistant` entity is the active voice assistant, over the
Integration-API, while a session runs. r3cui is the UI, not an integration, so it cannot receive it directly.

So r3cui ships its own small custom integration, **r3cui-voice** (`crates/voice-relay`), installed on the remote
the supported way (`POST /api/intg/install`; custom integrations do not set the warranty flag). It does one
thing: it receives the core's audio and hands it straight to r3cui. Where the audio goes, and everything after,
is decided in r3cui, which already holds the Home Assistant connection. The relay has no settings of its own.

## The path

```
voice key down (evdev)
  r3cui: listening screen at once
  r3cui → HA: assist_pipeline/run (stt → intent), so HA is ready before the first chunk
  r3cui → core: execute_entity_command voice_start {session_id, timeout}   (Core-API)
      core → relay: entity_command voice_start      relay: OK at once, then assistant_event ready
      core → relay: binary frames voice_begin, voice_data..., (Integration-API, protobuf)
      relay → r3cui: begin, PCM chunks                (voice link, localhost)
  r3cui → HA: [stt_binary_handler_id] + PCM per chunk
voice key up
  r3cui → core: voice_end;  core → relay: voice_end;  relay → r3cui: end
  r3cui → HA: [stt_binary_handler_id] alone (end of audio)
  HA → r3cui: stt-end (what was heard), intent-end (the answer), run-end
  r3cui → relay: finished (or error);  relay → core: assistant_event finished (or error)
```

Audio is what the core sends by default: PCM, 16 kHz, mono, signed 16-bit, in chunks every 100-200 ms.

## The pieces

| Piece | Where | What |
|---|---|---|
| Voice link | `crates/voicelink` | The localhost protocol between the relay and r3cui, defined once and used by both (see below). |
| Relay | `crates/voice-relay` | The integration: answers the Integration-API (driver version, device state, the one `voice_assistant` entity, entity states, subscribe, `entity_command`), decodes the core's protobuf audio frames, serves the voice link to r3cui, reports `ready`/`finished`/`error` back to the core. A static aarch64 binary plus `driver.json`, packed as a `.tar.gz`. |
| Assist | `crates/homeassistant` | `assist_pipeline/pipeline/list` and `assist_pipeline/run`, the binary audio frames, typed pipeline events. Needs binary frames in `crates/wsclient`. |
| Voice port | `crates/drivers` | A driver can take voice (`Provides.voice`, `Driver::voice()`): start a session, push chunks, end, and report what was heard and the answer. Global control gains **Voice**, bound like Volume and Inputs. Home Assistant provides it, one target per Assist pipeline. |
| Core | `crates/coreapi` | `execute_entity_command` (`voice_start`, `voice_end`) and the `assistant_event` stream. |
| UI | `crates/ui` | The voice key held starts a session, released ends it (a tap sends nothing; the inputs are Home's, held); the voice overlay: listening, what was heard, the answer or an error. |

## The voice link

A WebSocket on `127.0.0.1:10790` (`voicelink::PORT`; custom integrations may not use ports below 1024 or 8000-9200).
The relay listens, r3cui connects and reconnects; a new connection replaces the old one.

| Direction | Frame | Meaning |
|---|---|---|
| relay → r3cui | text `{"type":"begin","session_id":7,"sample_rate":16000,"channels":1,"format":"i16"}` | audio for session 7 starts |
| relay → r3cui | binary | PCM samples of the session that began |
| relay → r3cui | text `{"type":"end","session_id":7}` | the user let go: no more audio |
| r3cui → relay | text `{"type":"finished","session_id":7}` | done; the relay tells the core |
| r3cui → relay | text `{"type":"error","session_id":7,"code":"NO_TEXT_RECOGNIZED","message":"..."}` | failed; `code` is one of the core's: `TIMEOUT`, `NO_TEXT_RECOGNIZED`, `INVALID_AUDIO`, `SERVICE_UNAVAILABLE`, `INTENT_FAILED`, `TTS_FAILED`, `UNEXPECTED_ERROR` |

`session_id` is the one r3cui chose for `voice_start`; the relay passes it through.

## The relay

- The Integration-API server listens on `$UC_INTEGRATION_INTERFACE:$UC_INTEGRATION_HTTP_PORT` (127.0.0.1:10791 when
  unset, for desktop runs) and the voice link on 127.0.0.1:10790. Logs go to stdout.
- One entity: id `voice`, type `voice_assistant`, name "r3cui", features `transcription` and `response_text`,
  default `audio_cfg` (16 kHz, 1 channel, `I16`) and one profile. The core may show it prefixed with the driver id
  (`r3cui_voice`); that is unverified.
- `voice_start` is answered `200` at once, then `assistant_event` `ready`. With no r3cui on the link it is still
  answered `200`, followed by `assistant_event` `error` `SERVICE_UNAVAILABLE` ("r3cui is not listening"). If the
  link drops mid-session the core gets the same error ("r3cui disconnected").
- `setup_driver` finishes at once (`driver_setup_change` `STOP`/`OK`): there is nothing to set up.
- `driver.json` is `crates/voice-relay/driver.json`; a test keeps its version equal to the crate's. `min_core_api`
  `0.20.0` is copied from the manufacturer's example and is not enforced by the core yet.
- The core's frames are decoded with prost structs written out in `crates/voice-relay/src/proto.rs` (no protoc in the
  build), checked against bytes encoded by hand from the vendored `proto/ucr_integration_voice.proto` (CC BY-SA 4.0).

### Building it

The device build container (`scripts/build-device.sh`) builds `-p remote-ui` for `aarch64-unknown-linux-gnu`
against the SDK's glibc 2.33, dynamically linked. A custom integration must be a **statically linked** aarch64
binary, so the relay cannot simply be built the same way. Its dependencies are pure Rust (no C, no TLS), so the
proposal is `aarch64-unknown-linux-musl`:
`cargo build --release -p voice-relay --bin driver --target aarch64-unknown-linux-musl`, linked with the SDK's
`aarch64-none-linux-gnu-gcc` (or `rust-lld`), in the same container after `rustup target add
aarch64-unknown-linux-musl`. Not implemented or tried yet. Then `packaging/voice-relay.sh <driver binary>` writes
`packaging/out/r3cui-voice.tar.gz` (`driver.json` at the root, `bin/driver`).

### Assist API (`crates/homeassistant`)

- `HaHandle::pipelines() -> Result<Pipelines>`: `list` of `Pipeline { id, name, language }` and the `preferred` id.
- `HaHandle::run_voice(pipeline: Option<&str>, sample_rate, EndStage::{Intent, Tts}, timeout) -> Result<VoiceRun>`:
  sends `assist_pipeline/run` from `stt`, returns once HA's `run-start` (with the `stt_binary_handler_id`) is seen,
  whether it came before or after the reply. A run started while another is going cancels the other.
- `VoiceRun::send_audio(&[u8])` sends `[handler_id] + PCM` as a binary frame (empty audio is skipped: a lone id ends the
  stream), `end_audio()` sends the lone id, `next().await` yields `AssistEvent`s: `Listening`, `VadStart`, `VadEnd`,
  `Heard { text }`, `Answer { text, success }`, `Speech { url, mime_type }`, `Finished`, `Failed { code, message }`,
  then `None` (also `None` when the connection is lost first). Dropping a run that has not finished cancels it in HA.
- `VoiceError` (`Timeout`, `NoTextRecognized`, `InvalidAudio`, `ServiceUnavailable`, `Unexpected`) mirrors the core's
  codes; the HA strings are mapped in one place (`assist.rs`), named in `protocol::assist::error_code`.
- `crates/wsclient` sends binary frames through `Connection::out` (`Outbox::send_binary`), in order with text.
- `FakeHa` answers the list and runs, records binary frames (`audio_frames()`), and scripts events (`run_event`).

## On screen

Holding the key brings up an aurora: light along the bottom edge and up the sides, fading as it climbs, coloured from
the input's accent and seeded per session (`view/aurora.rs`, drawn on the CPU at 120×200 and scaled up smooth). It
follows the voice: `app/meter.rs` measures each chunk's loudness and four bands (Goertzel), and the frame loop in
`view/voice.rs` (30 Hz, only while the overlay is up) eases toward them, quick up and slow down, so it breathes. It
enters with a short spike at the bottom centre that sinks as the light spreads along the edge, then climbs the sides a
little past its height and back; the colours run along the edge fast at first and slow to rest, quicker while the voice
is loud. The intro's curves are cubic Béziers (`view/ease.rs`), tuned in a browser prototype and copied as they are.
Behind it, the screen shrinks, dims and blurs, as the stage does under a menu (`Motion.recede-scale`, `recede-dim`).
Let
go and it settles behind a `Spinner`; the answer shows, then everything fades. A tap goes at once. The demo feeds a made-up voice (`app/demo/voice.rs`); `scripts/voice.steps` films it.

## Setting it up on the remote (once, the user's call)

1. Install the relay package.
2. Add its voice entity, turn the microphone on and make the relay the voice assistant
   (`PATCH /api/cfg/voice_control`).
3. Global control → Voice → Home Assistant, and pick the pipeline.

## To check on the device first

- The relay and r3cui are separate sandboxes. The relay must be able to listen on localhost and r3cui to connect
  to it. Custom integrations share the remote's network (the Kodi integration reaches Kodi), so this should hold.
- The core streams for a `voice_start` that r3cui sends, not only for its own voice-key handling.

## Later

- Spoken answers: HA's `tts-end` gives a URL; r3cui would fetch it and play it, which needs an audio path.
- Other destinations: anything that implements the voice port.
