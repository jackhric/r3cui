//! Assist, Home Assistant's voice pipelines: which pipelines there are, and a voice run: audio in, then what was
//! heard, the answer and, if asked for, its speech, as typed `AssistEvent`s.

use crate::client::Client;
use crate::error::{Error, Result};
use crate::protocol::assist::{error_code, field, response_type, EventType, Stage};
use crate::protocol::field as frame;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;

const RUN_STARTS_WITHIN: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    pub language: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pipelines {
    pub list: Vec<Pipeline>,
    /// The id of the one Home Assistant uses when none is named.
    pub preferred: Option<String>,
}

/// Where a run stops: with the answer, or after speaking it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndStage {
    Intent,
    Tts,
}

impl EndStage {
    fn stage(self) -> Stage {
        match self {
            Self::Intent => Stage::Intent,
            Self::Tts => Stage::Tts,
        }
    }
}

/// Why a run failed, in the core's vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceError {
    Timeout,
    NoTextRecognized,
    InvalidAudio,
    ServiceUnavailable,
    Unexpected,
}

impl VoiceError {
    fn from_code(code: &str) -> Self {
        match code {
            error_code::TIMEOUT => Self::Timeout,
            error_code::NO_TEXT_RECOGNIZED => Self::NoTextRecognized,
            error_code::UNSUPPORTED_AUDIO => Self::InvalidAudio,
            error_code::WAKE_ENGINE_MISSING
            | error_code::WAKE_PROVIDER_MISSING
            | error_code::WAKE_STREAM_FAILED
            | error_code::WAKE_WORD_TIMEOUT
            | error_code::STT_PROVIDER_MISSING
            | error_code::INTENT_NOT_SUPPORTED
            | error_code::TTS_NOT_SUPPORTED => Self::ServiceUnavailable,
            _ => Self::Unexpected,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistEvent {
    /// Home Assistant is taking the audio.
    Listening,
    VadStart,
    VadEnd,
    /// What the speech-to-text made of the audio.
    Heard { text: String },
    /// `success` is false for answers like "I did not understand".
    Answer { text: String, success: bool },
    Speech { url: String, mime_type: String },
    Finished,
    Failed { code: VoiceError, message: String },
}

impl AssistEvent {
    fn ends_the_run(&self) -> bool {
        matches!(self, Self::Finished | Self::Failed { .. })
    }
}

/// One event of a run, or `None` for those that mean nothing to the caller (and for `run-start`, which `run_voice`
/// has consumed).
fn parse_event(event: &Value) -> Option<AssistEvent> {
    let kind = EventType::parse(event.get(frame::TYPE)?.as_str()?)?;
    let data = event.get(field::DATA).unwrap_or(&Value::Null);
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
    Some(match kind {
        EventType::SttStart => AssistEvent::Listening,
        EventType::SttVadStart => AssistEvent::VadStart,
        EventType::SttVadEnd => AssistEvent::VadEnd,
        EventType::SttEnd => AssistEvent::Heard { text: text(data.pointer(&format!("/{}/{}", field::STT_OUTPUT, field::TEXT)))? },
        EventType::IntentEnd => {
            let response = data.get(field::INTENT_OUTPUT)?.get(field::RESPONSE)?;
            let spoken = response.get(field::SPEECH)?.get(field::PLAIN)?.get(field::SPEECH);
            let success = matches!(response.get(field::RESPONSE_TYPE).and_then(Value::as_str), Some(response_type::ACTION_DONE | response_type::QUERY_ANSWER));
            AssistEvent::Answer { text: text(spoken)?, success }
        }
        EventType::TtsEnd => {
            let output = data.get(field::TTS_OUTPUT)?;
            AssistEvent::Speech { url: text(output.get(field::URL))?, mime_type: text(output.get(field::MIME_TYPE)).unwrap_or_default() }
        }
        EventType::RunEnd => AssistEvent::Finished,
        EventType::Error => AssistEvent::Failed {
            code: VoiceError::from_code(data.get(frame::CODE).and_then(Value::as_str).unwrap_or_default()),
            message: text(data.get(frame::MESSAGE)).unwrap_or_default(),
        },
        _ => return None,
    })
}

pub(crate) fn pipelines(result: &Value) -> Result<Pipelines> {
    let list = result.get(field::PIPELINES).and_then(Value::as_array).ok_or_else(|| Error::Decode("no pipelines in the list".into()))?;
    let text = |p: &Value, key: &str| p.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
    let list = list.iter().map(|p| Pipeline { id: text(p, frame::ID), name: text(p, field::NAME), language: text(p, field::LANGUAGE) }).collect();
    Ok(Pipelines { list, preferred: result.get(field::PREFERRED_PIPELINE).and_then(Value::as_str).map(str::to_owned) })
}

/// A voice run in progress: speak into it with `send_audio`, say you are done with `end_audio`, and read what
/// happens from `next`. Dropping it before it finishes cancels the run.
pub struct VoiceRun {
    client: Client,
    id: u32,
    handler_id: u8,
    events: mpsc::UnboundedReceiver<Value>,
    done: bool,
}

impl VoiceRun {
    pub(crate) async fn start(client: Client, pipeline: Option<&str>, sample_rate: u32, end: EndStage, timeout: Duration) -> Result<Self> {
        let mut fields = json!({
            field::START_STAGE: Stage::Stt.as_str(),
            field::END_STAGE: end.stage().as_str(),
            field::INPUT: { field::SAMPLE_RATE: sample_rate },
            field::TIMEOUT: timeout.as_secs(),
        });
        if let Some(pipeline) = pipeline {
            fields[field::PIPELINE] = json!(pipeline);
        }
        let (id, reply, mut events) = client.subscribe(crate::protocol::MessageType::AssistPipelineRun, fields);
        let previous = client.voice_run.lock().unwrap().replace(id);
        if let Some(previous) = previous {
            client.cancel(previous);
        }
        let handler_id = match tokio::time::timeout(RUN_STARTS_WITHIN, wait_for_run_start(reply, &mut events)).await {
            Ok(Ok(handler_id)) => handler_id,
            Ok(Err(e)) => {
                client.end_voice_run(id);
                return Err(e);
            }
            Err(_) => {
                client.end_voice_run(id);
                return Err(Error::Timeout);
            }
        };
        Ok(Self { client, id, handler_id, events, done: false })
    }

    /// Raw PCM from the microphone, in the format the run was started with. Empty audio is not sent: a frame without
    /// samples would end the stream.
    pub fn send_audio(&self, pcm: &[u8]) -> Result<()> {
        if pcm.is_empty() {
            return Ok(());
        }
        let mut frame = Vec::with_capacity(pcm.len() + 1);
        frame.push(self.handler_id);
        frame.extend_from_slice(pcm);
        self.client.send_binary(frame)
    }

    /// No more audio: Home Assistant goes on to the answer.
    pub fn end_audio(&self) -> Result<()> {
        self.client.send_binary(vec![self.handler_id])
    }

    /// The next event, in order; `None` once the run has finished or failed, or when the connection was lost first.
    pub async fn next(&mut self) -> Option<AssistEvent> {
        while !self.done {
            let event = self.events.recv().await?;
            let Some(event) = parse_event(&event) else { continue };
            if event.ends_the_run() {
                self.done = true;
                self.client.release(self.id);
            }
            return Some(event);
        }
        None
    }
}

impl Drop for VoiceRun {
    fn drop(&mut self) {
        self.client.end_voice_run(self.id);
    }
}

/// `run-start` may come before or after the reply to the request that started the run; the first failed reply or the
/// first `run-start` settles it.
async fn wait_for_run_start(reply: crate::client::Reply, events: &mut mpsc::UnboundedReceiver<Value>) -> Result<u8> {
    let mut reply = Box::pin(reply.wait(RUN_STARTS_WITHIN));
    let mut replied = false;
    loop {
        tokio::select! {
            outcome = &mut reply, if !replied => {
                outcome?;
                replied = true;
            }
            event = events.recv() => {
                let event = event.ok_or(Error::Closed)?;
                if event.get(frame::TYPE).and_then(Value::as_str) == Some(EventType::RunStart.as_str()) {
                    return handler_id(&event);
                }
            }
        }
    }
}

fn handler_id(run_start: &Value) -> Result<u8> {
    let id = run_start.pointer(&format!("/{}/{}/{}", field::DATA, field::RUNNER_DATA, field::STT_BINARY_HANDLER_ID)).and_then(Value::as_u64);
    id.and_then(|id| u8::try_from(id).ok()).ok_or_else(|| Error::Decode("run-start without an stt_binary_handler_id".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: EventType, data: Value) -> Value {
        json!({ "type": kind.as_str(), "data": data, "timestamp": "2024-11-26T10:00:00" })
    }

    fn intent_end(response_type: &str) -> Value {
        event(EventType::IntentEnd, json!({ "intent_output": { "response": { "response_type": response_type, "speech": { "plain": { "speech": "Done", "extra_data": null } } } } }))
    }

    #[test]
    fn events_map_to_typed_ones() {
        let stt_end = event(EventType::SttEnd, json!({ "stt_output": { "text": "turn on the light" } }));
        assert_eq!(parse_event(&stt_end), Some(AssistEvent::Heard { text: "turn on the light".into() }));
        assert_eq!(parse_event(&event(EventType::SttStart, json!({}))), Some(AssistEvent::Listening));
        assert_eq!(parse_event(&event(EventType::SttVadStart, json!({}))), Some(AssistEvent::VadStart));
        assert_eq!(parse_event(&event(EventType::SttVadEnd, json!({}))), Some(AssistEvent::VadEnd));
        let tts_end = event(EventType::TtsEnd, json!({ "tts_output": { "url": "/api/tts_proxy/x.mp3", "mime_type": "audio/mpeg" } }));
        assert_eq!(parse_event(&tts_end), Some(AssistEvent::Speech { url: "/api/tts_proxy/x.mp3".into(), mime_type: "audio/mpeg".into() }));
        assert_eq!(parse_event(&event(EventType::RunEnd, json!({}))), Some(AssistEvent::Finished));
        assert_eq!(parse_event(&event(EventType::IntentStart, json!({}))), None);
        assert_eq!(parse_event(&event(EventType::TtsStart, json!({}))), None);
    }

    #[test]
    fn an_answer_succeeds_when_something_was_done_or_answered() {
        let answer = |kind: &str| parse_event(&intent_end(kind));
        let said = |success| Some(AssistEvent::Answer { text: "Done".into(), success });
        assert_eq!(answer("action_done"), said(true));
        assert_eq!(answer("query_answer"), said(true));
        assert_eq!(answer("error"), said(false));
    }

    #[test]
    fn an_answer_without_words_is_skipped() {
        let silent = event(EventType::IntentEnd, json!({ "intent_output": { "response": { "response_type": "action_done", "speech": {} } } }));
        assert_eq!(parse_event(&silent), None);
    }

    #[test]
    fn error_codes_map_to_the_cores() {
        let code = |c: &str| match parse_event(&event(EventType::Error, json!({ "code": c, "message": "m" }))) {
            Some(AssistEvent::Failed { code, message }) => {
                assert_eq!(message, "m");
                code
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(code("timeout"), VoiceError::Timeout);
        assert_eq!(code("stt-no-text-recognized"), VoiceError::NoTextRecognized);
        assert_eq!(code("stt-provider-unsupported-metadata"), VoiceError::InvalidAudio);
        assert_eq!(code("stt-provider-missing"), VoiceError::ServiceUnavailable);
        assert_eq!(code("wake-word-timeout"), VoiceError::ServiceUnavailable);
        assert_eq!(code("intent-failed"), VoiceError::Unexpected);
        assert_eq!(code("something-new"), VoiceError::Unexpected);
    }

    #[test]
    fn the_pipeline_list_keeps_what_a_picker_needs() {
        let result = json!({
            "pipelines": [
                { "id": "01abc", "name": "Home Assistant", "language": "en", "stt_engine": "stt.faster_whisper" },
                { "id": "01def", "name": "Cloud", "language": "de", "stt_engine": null }
            ],
            "preferred_pipeline": "01def"
        });
        let got = pipelines(&result).unwrap();
        assert_eq!(got.list[0], Pipeline { id: "01abc".into(), name: "Home Assistant".into(), language: "en".into() });
        assert_eq!((got.list.len(), got.preferred.as_deref()), (2, Some("01def")));
        assert!(pipelines(&json!({})).is_err());
    }
}
