use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageType {
    Dialogue,
    Work,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BufferedMessage {
    pub seq: u64,
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub text: Option<String>,
    pub payload: Option<serde_json::Value>,
    pub received_at: u64, // unix ms
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelStatus {
    pub channel: String,
    pub connected: bool,
    pub buffered: usize,
}

// HTTP request types
#[derive(Debug, Deserialize)]
pub struct SubscribeReq {
    pub channel: String,
}

#[derive(Debug, Deserialize)]
pub struct LeaveReq {
    pub channel: String,
}

#[derive(Debug, Deserialize)]
pub struct SendReq {
    pub channel: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct SendWorkReq {
    pub channel: String,
    pub payload: serde_json::Value,
}

// HTTP response types
#[derive(Debug, Serialize)]
pub struct OkResp {
    pub ok: bool,
}

#[derive(Debug, Serialize)]
pub struct ErrorResp {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PollResp {
    pub messages: Vec<BufferedMessage>,
    pub next_seq: u64,
}

#[derive(Debug, Serialize)]
pub struct WaitResp {
    pub messages: Vec<BufferedMessage>,
    pub next_seq: u64,
    pub timed_out: bool,
}

#[derive(Debug, Serialize)]
pub struct ChannelsResp {
    pub channels: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct UsersResp {
    pub users: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct StatusResp {
    pub channels: Vec<ChannelStatus>,
}
