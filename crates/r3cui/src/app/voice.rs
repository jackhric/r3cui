//! Hold the voice key to talk: the session the key starts, the audio the bridge brings to it, the answer reported
//! back. Pure logic: each step returns what to do, and the session does it.

use r3cui_sdk::api::{Voice, VoiceFailure, VoiceIntent, VoicePhase};
use std::time::{Duration, Instant};
use r3cui_bridge::link::{ErrorCode, LinkEvent, UiMsg};

/// Held this long, the core is asked to record. The remote powers its microphone about 650 ms into a press, and the
/// core refuses a recording asked for before that ("input device is not enabled") without trying again; the stock UI
/// waits as long. Let go sooner and it was only a tap: the core is never asked, and the session goes at once.
pub(crate) const HOLD: Duration = Duration::from_millis(800);
/// After the key is let go, how long the bridge has to bring the last audio before r3cui ends it itself.
pub(crate) const END_GRACE: Duration = Duration::from_millis(1500);
/// What the core records unless asked otherwise.
pub(crate) const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Do {
    /// To the driver Global control → Voice points at.
    Driver(VoiceIntent),
    /// Ask the core to start or end recording for the session.
    CoreStart(u32),
    /// Ask the core to record after this long, if the key is still held (`start_due`).
    StartAfter(u32, Duration),
    CoreEnd(u32),
    /// End the session's audio after this long, unless the bridge has by then.
    EndAfter(u32, Duration),
    /// Tell the bridge how the session ended, so it can tell the core.
    Report(UiMsg),
}

#[derive(Debug)]
struct Active {
    session: u32,
    pressed: Instant,
    ended: bool,
    reported: bool,
    /// The core was asked to record: the key was held past `HOLD`.
    recording: bool,
    /// Audio bytes the bridge brought, for the log.
    heard: usize,
}

#[derive(Debug, Default)]
pub(crate) struct VoiceKey {
    last: u32,
    active: Option<Active>,
    dismissed: Option<u32>,
    /// The press closed the overlay instead of talking, so its release is no one's.
    closing: bool,
}

impl VoiceKey {
    /// While the overlay shows a session, the key closes it rather than starting another.
    pub(crate) fn press(&mut self, now: Instant, view: Option<&Voice>) -> Vec<Do> {
        if self.shown(view).is_some_and(|v| v.phase != VoicePhase::Listening) {
            self.dismiss(view);
            self.closing = true;
            return Vec::new();
        }
        self.last += 1;
        let session = self.last;
        self.active = Some(Active { session, pressed: now, ended: false, reported: false, recording: false, heard: 0 });
        vec![Do::Driver(VoiceIntent::Start { session, sample_rate: SAMPLE_RATE }), Do::StartAfter(session, HOLD)]
    }

    /// `HOLD` has passed since the press: the core records, unless the key was let go first.
    pub(crate) fn start_due(&mut self, session: u32) -> Vec<Do> {
        match self.active.as_mut().filter(|a| a.session == session && !a.recording && !a.ended) {
            Some(a) => {
                a.recording = true;
                vec![Do::CoreStart(session)]
            }
            None => Vec::new(),
        }
    }

    /// `grace` is how long the bridge gets to end the audio: `END_GRACE` on the remote, none without a bridge.
    pub(crate) fn release(&mut self, now: Instant, grace: Duration) -> Vec<Do> {
        if std::mem::take(&mut self.closing) {
            return Vec::new();
        }
        let Some(a) = &self.active else { return Vec::new() };
        let session = a.session;
        if !a.recording || now.duration_since(a.pressed) < HOLD {
            self.active = None;
            return vec![Do::Driver(VoiceIntent::Cancel { session })];
        }
        let mut out = vec![Do::CoreEnd(session)];
        if grace.is_zero() {
            out.extend(self.end(session));
        } else {
            out.push(Do::EndAfter(session, grace));
        }
        out
    }

    pub(crate) fn bridge(&mut self, event: LinkEvent) -> Vec<Do> {
        match event {
            LinkEvent::Audio { session_id, pcm } if self.listening(session_id) => {
                if let Some(a) = self.active.as_mut() {
                    a.heard += pcm.len();
                }
                vec![Do::Driver(VoiceIntent::Audio { session: session_id, pcm: pcm.to_vec() })]
            }
            LinkEvent::Audio { session_id, pcm } => {
                tracing::debug!("voice: {} bytes for session {session_id}, which is not listening", pcm.len());
                Vec::new()
            }
            LinkEvent::End { session_id } => self.end(session_id),
            _ => Vec::new(),
        }
    }

    pub(crate) fn end_due(&mut self, session: u32) -> Vec<Do> {
        self.end(session)
    }

    /// Once the destination has answered or failed, the bridge hears how it went, once.
    pub(crate) fn answered(&mut self, view: Option<&Voice>) -> Vec<Do> {
        let (Some(a), Some(v)) = (self.active.as_mut(), view) else { return Vec::new() };
        if a.session != v.session || a.reported {
            return Vec::new();
        }
        let msg = match v.phase {
            VoicePhase::Answered { .. } => UiMsg::Finished { session_id: a.session },
            VoicePhase::Failed(f) => UiMsg::Error { session_id: a.session, code: code(f), message: format!("{f:?}") },
            VoicePhase::Listening | VoicePhase::Thinking => return Vec::new(),
        };
        a.reported = true;
        vec![Do::Report(msg)]
    }

    pub(crate) fn dismiss(&mut self, view: Option<&Voice>) {
        self.dismissed = view.map(|v| v.session);
    }

    /// The overlay shows a session until it is dismissed.
    pub(crate) fn shown<'a>(&self, view: Option<&'a Voice>) -> Option<&'a Voice> {
        view.filter(|v| Some(v.session) != self.dismissed)
    }

    fn listening(&self, session: u32) -> bool {
        self.active.as_ref().is_some_and(|a| a.session == session && !a.ended)
    }

    fn end(&mut self, session: u32) -> Vec<Do> {
        match self.active.as_mut().filter(|a| a.session == session && !a.ended) {
            Some(a) => {
                a.ended = true;
                tracing::info!("voice: session {session} ended with {} bytes of audio from the bridge", a.heard);
                vec![Do::Driver(VoiceIntent::End { session })]
            }
            None => Vec::new(),
        }
    }
}

fn code(f: VoiceFailure) -> ErrorCode {
    match f {
        VoiceFailure::Timeout => ErrorCode::Timeout,
        VoiceFailure::NoTextRecognized => ErrorCode::NoTextRecognized,
        VoiceFailure::InvalidAudio => ErrorCode::InvalidAudio,
        VoiceFailure::ServiceUnavailable => ErrorCode::ServiceUnavailable,
        VoiceFailure::Unexpected => ErrorCode::UnexpectedError,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(session_id: u32, pcm: &[u8]) -> LinkEvent {
        LinkEvent::Audio { session_id, pcm: pcm.to_vec().into() }
    }

    #[test]
    fn a_hold_talks_and_reports_once() {
        let (mut key, t) = (VoiceKey::default(), Instant::now());
        assert_eq!(key.press(t, None), [Do::Driver(VoiceIntent::Start { session: 1, sample_rate: SAMPLE_RATE }), Do::StartAfter(1, HOLD)]);
        assert_eq!(key.start_due(1), [Do::CoreStart(1)], "held past HOLD: the core records");
        assert!(key.start_due(1).is_empty(), "asked once");
        assert_eq!(key.bridge(audio(1, &[1, 2])), [Do::Driver(VoiceIntent::Audio { session: 1, pcm: vec![1, 2] })]);
        assert!(key.bridge(audio(9, &[9])).is_empty(), "another session's audio");
        assert_eq!(key.release(t + Duration::from_secs(2), END_GRACE), [Do::CoreEnd(1), Do::EndAfter(1, END_GRACE)]);
        assert_eq!(key.bridge(audio(1, &[3])), [Do::Driver(VoiceIntent::Audio { session: 1, pcm: vec![3] })], "late audio still counts");
        assert_eq!(key.bridge(LinkEvent::End { session_id: 1 }), [Do::Driver(VoiceIntent::End { session: 1 })]);
        assert!(key.end_due(1).is_empty(), "ended once");
        assert!(key.bridge(audio(1, &[4])).is_empty(), "nothing after the end");

        let mut v = Voice { session: 1, ..Voice::default() };
        assert!(key.answered(Some(&v)).is_empty(), "still listening");
        v.phase = VoicePhase::Answered { success: true };
        assert_eq!(key.answered(Some(&v)), [Do::Report(UiMsg::Finished { session_id: 1 })]);
        assert!(key.answered(Some(&v)).is_empty(), "reported once");
        assert!(key.shown(Some(&v)).is_some());
        key.dismiss(Some(&v));
        assert!(key.shown(Some(&v)).is_none());
    }

    #[test]
    fn the_key_closes_a_shown_answer_instead_of_talking_again() {
        let (mut key, t) = (VoiceKey::default(), Instant::now());
        key.press(t, None);
        key.start_due(1);
        key.release(t + Duration::from_secs(1), Duration::ZERO);
        for phase in [VoicePhase::Thinking, VoicePhase::Answered { success: true }] {
            let v = Voice { session: 1, phase, ..Voice::default() };
            let mut key = VoiceKey { last: key.last, ..VoiceKey::default() };
            assert!(key.press(t + Duration::from_secs(2), Some(&v)).is_empty(), "no new session");
            assert!(key.shown(Some(&v)).is_none(), "the overlay goes");
            assert!(key.release(t + Duration::from_secs(3), END_GRACE).is_empty(), "its release ends nothing");
            assert_eq!(key.press(t + Duration::from_secs(4), Some(&v))[1], Do::StartAfter(2, HOLD), "the next press talks");
        }
    }

    #[test]
    fn a_tap_sends_nothing() {
        let (mut key, t) = (VoiceKey::default(), Instant::now());
        key.press(t, None);
        assert_eq!(key.release(t + Duration::from_millis(500), END_GRACE), [Do::Driver(VoiceIntent::Cancel { session: 1 })], "the core was never asked");
        assert!(key.start_due(1).is_empty(), "let go before HOLD: nothing to record");
        assert!(key.bridge(audio(1, &[1])).is_empty());
    }

    #[test]
    fn without_a_relay_the_audio_ends_with_the_key_and_failures_are_reported() {
        let (mut key, t) = (VoiceKey::default(), Instant::now());
        key.press(t, None);
        key.start_due(1);
        assert_eq!(key.release(t + Duration::from_secs(1), Duration::ZERO), [Do::CoreEnd(1), Do::Driver(VoiceIntent::End { session: 1 })]);
        let v = Voice { session: 1, phase: VoicePhase::Failed(VoiceFailure::NoTextRecognized), ..Voice::default() };
        assert!(matches!(key.answered(Some(&v)).as_slice(), [Do::Report(UiMsg::Error { session_id: 1, code: ErrorCode::NoTextRecognized, .. })]));
    }
}
