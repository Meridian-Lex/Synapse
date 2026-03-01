use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtoError {
    #[error("frame too large: {0} bytes")]
    FrameTooLarge(u32),
    #[error("unknown message type: 0x{0:02x}")]
    UnknownMsgType(u8),
    #[error("unknown encoding: 0x{0:02x}")]
    UnknownEncoding(u8),
    #[error("payload decompression failed: {0}")]
    DecompressFailed(String),
    #[error("payload compression failed: {0}")]
    CompressFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("insufficient data")]
    Incomplete,
}
