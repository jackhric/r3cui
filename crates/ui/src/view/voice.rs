//! The voice session on screen.

use crate::view::stage::capture;
use crate::{AppWindow, Capture, Voice as Shown, VoiceStage};
use drivers::api::{Voice, VoiceFailure, VoicePhase};
use slint::ComponentHandle;

pub(crate) fn show(app: &AppWindow, voice: Option<&Voice>) {
    let v = app.global::<Shown>();
    let Some(voice) = voice else {
        v.set_stage(VoiceStage::Off);
        return;
    };
    let (stage, answer) = match voice.phase {
        VoicePhase::Listening => (VoiceStage::Listening, String::new()),
        VoicePhase::Thinking => (VoiceStage::Thinking, String::new()),
        VoicePhase::Answered { .. } => (VoiceStage::Answered, voice.answer.clone()),
        VoicePhase::Failed(f) => (VoiceStage::Failed, failure(f).to_string()),
    };
    // Opening, the screen under it is blurred as it is for every overlay.
    if v.get_stage() == VoiceStage::Off {
        capture(app, Capture::World);
    }
    v.set_stage(stage);
    v.set_heard(voice.heard.as_str().into());
    v.set_answer(answer.into());
}

fn failure(f: VoiceFailure) -> &'static str {
    match f {
        VoiceFailure::NoTextRecognized => "Didn't catch that",
        VoiceFailure::Timeout => "No answer in time",
        VoiceFailure::ServiceUnavailable => "Voice isn't reachable",
        VoiceFailure::InvalidAudio => "The audio didn't come through",
        VoiceFailure::Unexpected => "Something went wrong",
    }
}
