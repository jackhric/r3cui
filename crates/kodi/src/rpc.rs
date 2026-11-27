//! JSON-RPC 2.0 as Kodi speaks it: a typed `Method` per call, the request envelope, and sorting an incoming frame
//! into a reply (matched to its request by id) or a notification.

use crate::error::{Error, Result};
use crate::ids::RequestId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// The envelope's `jsonrpc` member.
pub const JSONRPC_VERSION: &str = "2.0";

/// One of Kodi's methods: its name in the schema, its parameters (the struct itself), and what it answers.
pub trait Method: Serialize + Send + Sync + 'static {
    const NAME: &'static str;
    type Output: DeserializeOwned + Send + 'static;
}

/// A request ready to send: the frame's text and what to call it in errors.
#[derive(Debug, Clone)]
pub struct Request {
    pub id: RequestId,
    pub method: &'static str,
    pub body: String,
}

impl Request {
    pub fn new<M: Method>(id: RequestId, params: &M) -> Self {
        #[derive(Serialize)]
        struct Envelope<'a, P> {
            jsonrpc: &'static str,
            id: RequestId,
            method: &'static str,
            params: &'a P,
        }
        let body = serde_json::to_string(&Envelope { jsonrpc: JSONRPC_VERSION, id, method: M::NAME, params })
            .expect("method params always serialise");
        Self { id, method: M::NAME, body }
    }
}

/// A frame from Kodi.
#[derive(Debug)]
pub enum Incoming {
    Reply { id: RequestId, result: std::result::Result<Box<RawValue>, RpcError> },
    Notification { method: String, params: Box<RawValue> },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}

impl Incoming {
    pub fn parse(frame: &str) -> Result<Self> {
        #[derive(Deserialize)]
        struct Frame {
            id: Option<RequestId>,
            method: Option<String>,
            params: Option<Box<RawValue>>,
            result: Option<Box<RawValue>>,
            error: Option<RpcError>,
        }
        let f: Frame = serde_json::from_str(frame).map_err(|e| Error::Decode { method: "frame", detail: e.to_string().into() })?;
        match (f.id, f.method) {
            (Some(id), _) => Ok(Self::Reply {
                id,
                result: match (f.result, f.error) {
                    (_, Some(e)) => Err(e),
                    (Some(r), None) => Ok(r),
                    (None, None) => Err(RpcError { code: 0, message: "empty reply".into() }),
                },
            }),
            (None, Some(method)) => Ok(Self::Notification { method, params: f.params.unwrap_or_else(|| RawValue::from_string("null".into()).unwrap()) }),
            (None, None) => Err(Error::Decode { method: "frame", detail: "neither a reply nor a notification".into() }),
        }
    }
}

/// A reply's result as the method's output.
pub fn decode<M: Method>(result: std::result::Result<Box<RawValue>, RpcError>) -> Result<M::Output> {
    match result {
        Ok(raw) => serde_json::from_str(raw.get()).map_err(|e| Error::Decode { method: M::NAME, detail: e.to_string().into() }),
        Err(e) => Err(Error::Rpc { method: M::NAME, code: e.code, message: e.message.into() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methods::player::GetActivePlayers;

    #[test]
    fn envelope_and_frames() {
        let r = Request::new(RequestId(7), &GetActivePlayers {});
        assert_eq!(r.body, r#"{"jsonrpc":"2.0","id":7,"method":"Player.GetActivePlayers","params":{}}"#);
        let Incoming::Reply { id, result } = Incoming::parse(r#"{"id":7,"jsonrpc":"2.0","result":[{"playerid":1,"playertype":"internal","type":"video"}]}"#).unwrap() else { panic!() };
        assert_eq!(id, RequestId(7));
        let players = decode::<GetActivePlayers>(result).unwrap();
        assert_eq!(players[0].playerid.0, 1);
        let Incoming::Reply { result, .. } = Incoming::parse(r#"{"id":8,"jsonrpc":"2.0","error":{"code":-32100,"message":"Failed to execute method."}}"#).unwrap() else { panic!() };
        assert!(matches!(decode::<GetActivePlayers>(result), Err(Error::Rpc { code: -32100, .. })));
        let Incoming::Notification { method, .. } = Incoming::parse(r#"{"jsonrpc":"2.0","method":"Player.OnPause","params":{"data":{},"sender":"xbmc"}}"#).unwrap() else { panic!() };
        assert_eq!(method, "Player.OnPause");
    }
}
