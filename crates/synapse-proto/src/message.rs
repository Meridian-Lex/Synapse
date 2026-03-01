use crate::error::ProtoError;
use rmpv::Value;

pub const CONTENT_DIALOGUE: u8 = 0x01;
pub const CONTENT_WORK:     u8 = 0x02;

pub enum MsgPayload {
    Dialogue { channel_id: u64, timestamp_ms: u64, body: String },
    Work     { channel_id: u64, timestamp_ms: u64, body: Value  },
}

impl MsgPayload {
    pub fn encode(&self) -> Result<Vec<u8>, ProtoError> {
        match self {
            Self::Dialogue { channel_id, timestamp_ms, body } => {
                let b = body.as_bytes();
                let mut buf = Vec::with_capacity(17 + b.len());
                buf.push(CONTENT_DIALOGUE);
                buf.extend_from_slice(&channel_id.to_be_bytes());
                buf.extend_from_slice(&timestamp_ms.to_be_bytes());
                buf.extend_from_slice(b);
                Ok(buf)
            }
            Self::Work { channel_id, timestamp_ms, body } => {
                let mut mp = Vec::new();
                rmpv::encode::write_value(&mut mp, body)
                    .map_err(|e| ProtoError::CompressFailed(e.to_string()))?;
                let mut buf = Vec::with_capacity(17 + mp.len());
                buf.push(CONTENT_WORK);
                buf.extend_from_slice(&channel_id.to_be_bytes());
                buf.extend_from_slice(&timestamp_ms.to_be_bytes());
                buf.extend_from_slice(&mp);
                Ok(buf)
            }
        }
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ProtoError> {
        if buf.len() < 17 { return Err(ProtoError::Incomplete); }
        let channel_id   = u64::from_be_bytes(buf[1..9].try_into().unwrap());
        let timestamp_ms = u64::from_be_bytes(buf[9..17].try_into().unwrap());
        let body = &buf[17..];
        match buf[0] {
            CONTENT_DIALOGUE => Ok(Self::Dialogue {
                channel_id, timestamp_ms,
                body: String::from_utf8_lossy(body).into_owned(),
            }),
            CONTENT_WORK => {
                let val = rmpv::decode::read_value(&mut std::io::Cursor::new(body))
                    .map_err(|e| ProtoError::DecompressFailed(e.to_string()))?;
                Ok(Self::Work { channel_id, timestamp_ms, body: val })
            }
            b => Err(ProtoError::UnknownMsgType(b)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialogue_roundtrip() {
        let msg = MsgPayload::Dialogue { channel_id: 7, timestamp_ms: 1_700_000_000_000,
            body: "Hello Axiom.".to_string() };
        let decoded = MsgPayload::decode(&msg.encode().unwrap()).unwrap();
        match decoded {
            MsgPayload::Dialogue { channel_id, body, .. } => {
                assert_eq!(channel_id, 7);
                assert_eq!(body, "Hello Axiom.");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_work_roundtrip() {
        let body = Value::Map(vec![
            (Value::String("task".into()), Value::String("index".into())),
        ]);
        let msg = MsgPayload::Work { channel_id: 3, timestamp_ms: 0, body };
        let decoded = MsgPayload::decode(&msg.encode().unwrap()).unwrap();
        match decoded {
            MsgPayload::Work { channel_id, .. } => assert_eq!(channel_id, 3),
            _ => panic!("wrong variant"),
        }
    }
}
