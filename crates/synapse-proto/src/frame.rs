use crate::error::ProtoError;

pub const HEADER_LEN: usize = 16;
pub const MAX_PAYLOAD: u32  = 4 * 1024 * 1024;
pub const PROTOCOL_VERSION: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    Hello = 0x01, Challenge = 0x02, HelloResp = 0x03, HelloAck = 0x04, HelloErr = 0x05,
    Msg = 0x10, MsgAck = 0x11, MsgEdit = 0x12, MsgDelete = 0x13,
    Subscribe = 0x20, Unsubscribe = 0x21, ChanCreate = 0x22,
    ChanInfo = 0x23, ChanList = 0x24, ChanHistory = 0x25, SubscribeAck = 0x26,
    Presence = 0x30, PresenceReq = 0x31, Typing = 0x32,
    Ping = 0x40, Pong = 0x41, Sys = 0x50, Error = 0x51, Bye = 0x60,
}

impl TryFrom<u8> for MsgType {
    type Error = ProtoError;
    fn try_from(v: u8) -> Result<Self, <Self as TryFrom<u8>>::Error> {
        match v {
            0x01 => Ok(Self::Hello),     0x02 => Ok(Self::Challenge),
            0x03 => Ok(Self::HelloResp), 0x04 => Ok(Self::HelloAck),
            0x05 => Ok(Self::HelloErr),  0x10 => Ok(Self::Msg),
            0x11 => Ok(Self::MsgAck),    0x12 => Ok(Self::MsgEdit),
            0x13 => Ok(Self::MsgDelete), 0x20 => Ok(Self::Subscribe),
            0x21 => Ok(Self::Unsubscribe), 0x22 => Ok(Self::ChanCreate),
            0x23 => Ok(Self::ChanInfo),  0x24 => Ok(Self::ChanList),
            0x25 => Ok(Self::ChanHistory), 0x26 => Ok(Self::SubscribeAck),
            0x30 => Ok(Self::Presence),
            0x31 => Ok(Self::PresenceReq), 0x32 => Ok(Self::Typing),
            0x40 => Ok(Self::Ping),      0x41 => Ok(Self::Pong),
            0x50 => Ok(Self::Sys),       0x51 => Ok(Self::Error),
            0x60 => Ok(Self::Bye),
            b => Err(ProtoError::UnknownMsgType(b)),
        }
    }
}

impl From<MsgType> for u8 { fn from(t: MsgType) -> u8 { t as u8 } }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority { Normal = 0, High = 1, Urgent = 2, System = 3 }

impl Priority {
    fn from_bits(b: u8) -> Self {
        match b & 0b11 { 1 => Self::High, 2 => Self::Urgent, 3 => Self::System, _ => Self::Normal }
    }
    fn to_bits(self) -> u8 { self as u8 }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flags {
    pub compressed: bool, pub e2e_reserved: bool, pub ack_required: bool,
    pub is_reply: bool, pub has_expiry: bool, pub edited: bool, pub priority: Priority,
}

impl Flags {
    pub fn to_byte(self) -> u8 {
        (self.compressed   as u8)
        | ((self.e2e_reserved as u8) << 1)
        | ((self.ack_required as u8) << 2)
        | ((self.is_reply    as u8) << 3)
        | ((self.has_expiry  as u8) << 4)
        | ((self.edited      as u8) << 5)
        | (self.priority.to_bits()  << 6)
    }
    pub fn from_byte(b: u8) -> Self {
        Self {
            compressed:   (b & 0x01) != 0, e2e_reserved: (b & 0x02) != 0,
            ack_required: (b & 0x04) != 0, is_reply:     (b & 0x08) != 0,
            has_expiry:   (b & 0x10) != 0, edited:       (b & 0x20) != 0,
            priority: Priority::from_bits(b >> 6),
        }
    }
}

impl Default for Flags {
    fn default() -> Self {
        Self { compressed: false, e2e_reserved: false, ack_required: false,
               is_reply: false, has_expiry: false, edited: false, priority: Priority::Normal }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding { Raw = 0x00, Zstd = 0x01 }

impl TryFrom<u8> for Encoding {
    type Error = ProtoError;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v { 0x00 => Ok(Self::Raw), 0x01 => Ok(Self::Zstd), b => Err(ProtoError::UnknownEncoding(b)) }
    }
}

#[derive(Debug, Clone)]
pub struct FrameHeader {
    pub version: u8, pub flags: Flags, pub msg_type: MsgType,
    pub encoding: Encoding, pub payload_len: u32, pub message_id: u64,
}

impl FrameHeader {
    pub fn new(msg_type: MsgType, message_id: u64, payload_len: u32) -> Self {
        Self { version: PROTOCOL_VERSION, flags: Flags::default(),
               msg_type, encoding: Encoding::Raw, payload_len, message_id }
    }

    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut buf = [0u8; HEADER_LEN];
        buf[0] = self.version;
        buf[1] = self.flags.to_byte();
        buf[2] = self.msg_type.into();
        buf[3] = self.encoding as u8;
        buf[4..8].copy_from_slice(&self.payload_len.to_be_bytes());
        buf[8..16].copy_from_slice(&self.message_id.to_be_bytes());
        buf
    }

    pub fn from_bytes(buf: &[u8; HEADER_LEN]) -> Result<Self, ProtoError> {
        let payload_len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        if payload_len > MAX_PAYLOAD { return Err(ProtoError::FrameTooLarge(payload_len)); }
        Ok(Self {
            version:     buf[0],
            flags:       Flags::from_byte(buf[1]),
            msg_type:    MsgType::try_from(buf[2])?,
            encoding:    Encoding::try_from(buf[3])?,
            payload_len,
            message_id:  u64::from_be_bytes(buf[8..16].try_into().unwrap()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_header_size() { assert_eq!(HEADER_LEN, 16); }

    #[test]
    fn test_msg_type_roundtrip() {
        let types = [
            MsgType::Hello, MsgType::Challenge, MsgType::HelloResp,
            MsgType::HelloAck, MsgType::HelloErr, MsgType::Msg,
            MsgType::MsgAck, MsgType::MsgEdit, MsgType::MsgDelete,
            MsgType::Subscribe, MsgType::Unsubscribe, MsgType::ChanCreate,
            MsgType::ChanInfo, MsgType::ChanList, MsgType::ChanHistory, MsgType::SubscribeAck,
            MsgType::Presence, MsgType::PresenceReq, MsgType::Typing,
            MsgType::Ping, MsgType::Pong, MsgType::Sys, MsgType::Error, MsgType::Bye,
        ];
        for t in types {
            let byte: u8 = t.into();
            assert_eq!(MsgType::try_from(byte).unwrap(), t);
        }
    }

    #[test]
    fn test_flags_roundtrip() {
        let flags = Flags { compressed: true, e2e_reserved: false, ack_required: true,
            is_reply: false, has_expiry: true, edited: false, priority: Priority::High };
        assert_eq!(Flags::from_byte(flags.to_byte()), flags);
    }

    #[test]
    fn test_encoding_roundtrip() {
        assert_eq!(Encoding::try_from(0x00u8).unwrap(), Encoding::Raw);
        assert_eq!(Encoding::try_from(0x01u8).unwrap(), Encoding::Zstd);
        assert!(Encoding::try_from(0x02u8).is_err());
    }

    #[test]
    fn test_frame_header_roundtrip() {
        let header = FrameHeader::new(MsgType::Msg, 0xdeadbeefcafe, 1234);
        let back = FrameHeader::from_bytes(&header.to_bytes()).unwrap();
        assert_eq!(back.msg_type, MsgType::Msg);
        assert_eq!(back.message_id, 0xdeadbeefcafe);
        assert_eq!(back.payload_len, 1234);
    }

    #[test]
    fn test_frame_too_large_rejected() {
        let mut bytes = [0u8; 16];
        bytes[0] = PROTOCOL_VERSION;
        bytes[2] = MsgType::Msg as u8;
        bytes[4..8].copy_from_slice(&(MAX_PAYLOAD + 1).to_be_bytes());
        assert!(matches!(FrameHeader::from_bytes(&bytes), Err(ProtoError::FrameTooLarge(_))));
    }
}
