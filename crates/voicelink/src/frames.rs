use serde::{Deserialize, Serialize};

/// Chosen by r3cui for `voice_start`; the relay passes it through.
pub type SessionId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SampleFormat {
    I8,
    I16,
    I32,
    U8,
    U16,
    U32,
    F32,
}

impl SampleFormat {
    pub fn bytes(self) -> usize {
        match self {
            Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::I32 | Self::U32 | Self::F32 => 4,
        }
    }
}

/// The `assistant_event` error codes of the Integration-API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    Timeout,
    NoTextRecognized,
    InvalidAudio,
    ServiceUnavailable,
    IntentFailed,
    TtsFailed,
    UnexpectedError,
}

/// Text frames from the relay. Audio follows as binary frames of PCM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayMsg {
    Begin {
        session_id: SessionId,
        sample_rate: u32,
        channels: u16,
        format: SampleFormat,
    },
    End {
        session_id: SessionId,
    },
}

/// Text frames from r3cui.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiMsg {
    Finished {
        session_id: SessionId,
    },
    Error {
        session_id: SessionId,
        code: ErrorCode,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both<T: Serialize + for<'a> Deserialize<'a> + PartialEq + std::fmt::Debug>(
        value: T,
        json: &str,
    ) {
        assert_eq!(serde_json::to_string(&value).unwrap(), json);
        assert_eq!(serde_json::from_str::<T>(json).unwrap(), value);
    }

    #[test]
    fn begin() {
        both(
            RelayMsg::Begin {
                session_id: 7,
                sample_rate: 16000,
                channels: 1,
                format: SampleFormat::I16,
            },
            r#"{"type":"begin","session_id":7,"sample_rate":16000,"channels":1,"format":"i16"}"#,
        );
    }

    #[test]
    fn end() {
        both(
            RelayMsg::End { session_id: 7 },
            r#"{"type":"end","session_id":7}"#,
        );
    }

    #[test]
    fn finished() {
        both(
            UiMsg::Finished { session_id: 7 },
            r#"{"type":"finished","session_id":7}"#,
        );
    }

    #[test]
    fn error() {
        both(
            UiMsg::Error {
                session_id: 7,
                code: ErrorCode::NoTextRecognized,
                message: "...".into(),
            },
            r#"{"type":"error","session_id":7,"code":"NO_TEXT_RECOGNIZED","message":"..."}"#,
        );
    }

    #[test]
    fn every_error_code_is_the_cores() {
        let names = [
            "TIMEOUT",
            "NO_TEXT_RECOGNIZED",
            "INVALID_AUDIO",
            "SERVICE_UNAVAILABLE",
            "INTENT_FAILED",
            "TTS_FAILED",
            "UNEXPECTED_ERROR",
        ];
        let codes = [
            ErrorCode::Timeout,
            ErrorCode::NoTextRecognized,
            ErrorCode::InvalidAudio,
            ErrorCode::ServiceUnavailable,
            ErrorCode::IntentFailed,
            ErrorCode::TtsFailed,
            ErrorCode::UnexpectedError,
        ];
        for (code, name) in codes.into_iter().zip(names) {
            both(code, &format!("\"{name}\""));
        }
    }

    #[test]
    fn sample_formats_are_lowercase() {
        both(SampleFormat::F32, "\"f32\"");
        both(SampleFormat::U8, "\"u8\"");
        assert_eq!(SampleFormat::I16.bytes(), 2);
    }
}
