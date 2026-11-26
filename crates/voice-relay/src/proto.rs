//! The core's binary voice frames: `proto/ucr_integration_voice.proto` (CC BY-SA 4.0, Unfolded Circle), written out as
//! prost structs so the build needs no protoc. `tests` pin them to bytes encoded by hand from the .proto.

use prost::Message;

#[derive(Clone, PartialEq, Message)]
pub struct IntegrationMessage {
    #[prost(oneof = "integration_message::Message", tags = "1, 2, 3")]
    pub message: Option<integration_message::Message>,
}

pub mod integration_message {
    use super::{RemoteVoiceBegin, RemoteVoiceData, RemoteVoiceEnd};

    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Message {
        #[prost(message, tag = "1")]
        VoiceBegin(RemoteVoiceBegin),
        #[prost(message, tag = "2")]
        VoiceData(RemoteVoiceData),
        #[prost(message, tag = "3")]
        VoiceEnd(RemoteVoiceEnd),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum SampleFormat {
    Unknown = 0,
    I8 = 1,
    I16 = 2,
    I32 = 3,
    U8 = 4,
    U16 = 5,
    U32 = 6,
    F32 = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum AudioFormat {
    Unknown = 0,
    Pcm = 1,
}

#[derive(Clone, PartialEq, Message)]
pub struct AudioConfiguration {
    #[prost(uint32, tag = "1")]
    pub channels: u32,
    #[prost(uint32, tag = "2")]
    pub sample_rate: u32,
    #[prost(enumeration = "SampleFormat", tag = "3")]
    pub sample_format: i32,
    #[prost(enumeration = "AudioFormat", tag = "4")]
    pub format: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct RemoteVoiceBegin {
    #[prost(uint32, tag = "1")]
    pub session_id: u32,
    #[prost(message, optional, tag = "2")]
    pub configuration: Option<AudioConfiguration>,
}

#[derive(Clone, PartialEq, Message)]
pub struct RemoteVoiceData {
    #[prost(uint32, tag = "1")]
    pub session_id: u32,
    #[prost(bytes = "vec", tag = "2")]
    pub samples: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
pub struct RemoteVoiceEnd {
    #[prost(uint32, tag = "1")]
    pub session_id: u32,
}

#[cfg(test)]
mod tests {
    use super::integration_message::Message as Frame;
    use super::*;

    fn wrap(message: Frame) -> Vec<u8> {
        IntegrationMessage {
            message: Some(message),
        }
        .encode_to_vec()
    }

    #[test]
    fn begin_is_the_bytes_the_proto_says() {
        let bytes = [
            0x0A, 0x0B, 0x08, 0x07, 0x12, 0x07, 0x08, 0x01, 0x10, 0x80, 0x7D, 0x18, 0x02,
        ];
        let begin = RemoteVoiceBegin {
            session_id: 7,
            configuration: Some(AudioConfiguration {
                channels: 1,
                sample_rate: 16000,
                sample_format: SampleFormat::I16 as i32,
                format: 0,
            }),
        };
        assert_eq!(wrap(Frame::VoiceBegin(begin.clone())), bytes);
        assert_eq!(
            IntegrationMessage::decode(&bytes[..]).unwrap().message,
            Some(Frame::VoiceBegin(begin))
        );
    }

    #[test]
    fn data_is_the_bytes_the_proto_says() {
        let bytes = [0x12, 0x07, 0x08, 0x07, 0x12, 0x03, 1, 2, 3];
        let data = RemoteVoiceData {
            session_id: 7,
            samples: vec![1, 2, 3],
        };
        assert_eq!(wrap(Frame::VoiceData(data.clone())), bytes);
        assert_eq!(
            IntegrationMessage::decode(&bytes[..]).unwrap().message,
            Some(Frame::VoiceData(data))
        );
    }

    #[test]
    fn end_is_the_bytes_the_proto_says() {
        let bytes = [0x1A, 0x02, 0x08, 0x07];
        assert_eq!(
            wrap(Frame::VoiceEnd(RemoteVoiceEnd { session_id: 7 })),
            bytes
        );
        assert_eq!(
            IntegrationMessage::decode(&bytes[..]).unwrap().message,
            Some(Frame::VoiceEnd(RemoteVoiceEnd { session_id: 7 }))
        );
    }
}
