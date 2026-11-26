# Voice: hold to talk, sent where r3cui says

Status: design (2026-11-26). The Core-API calls and the `assistant_event` are built in `crates/coreapi`; the rest is not.

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
| UI | `crates/ui` | The voice key held starts a session, released ends it (a short press stays the input switcher); the voice overlay: listening, what was heard, the answer or an error. |

## The voice link

A WebSocket on `127.0.0.1:10790` (`voicelink::PORT`; custom integrations may not use ports below 1024 or 8000-9200).
The relay listens, r3cui connects and reconnects; a new connection replaces the old one.

| Direction | Frame | Meaning |
|---|---|---|
| relay → r3cui | text `{"type":"begin","session_id":7,"sample_rate":16000,"channels":1,"format":"i16"}` | audio for session 7 starts |
| relay → r3cui | binary | PCM samples of the session that began |
| relay → r3cui | text `{"type":"end","session_id":7}` | the user let go: no more audio |
| r3cui → relay | text `{"type":"finished","session_id":7}` | done; the relay tells the core |
| r3cui → relay | text `{"type":"error","session_id":7,"code":"NO_TEXT_RECOGNIZED","message":"..."}` | failed; `code` is one of the core's: `TIMEOUT`, `NO_TEXT_RECOGNIZED`, `INVALID_AUDIO`, `SERVICE_UNAVAILABLE`, `UNEXPECTED_ERROR` |

`session_id` is the one r3cui chose for `voice_start`; the relay passes it through.

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
