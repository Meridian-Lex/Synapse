use crate::{error::ProtoError, frame::{FrameHeader, HEADER_LEN}};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<(FrameHeader, Vec<u8>), ProtoError> {
    let mut header_buf = [0u8; HEADER_LEN];
    reader.read_exact(&mut header_buf).await?;
    let header = FrameHeader::from_bytes(&header_buf)?;
    let mut payload = vec![0u8; header.payload_len as usize];
    if !payload.is_empty() { reader.read_exact(&mut payload).await?; }
    Ok((header, payload))
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    header: &FrameHeader,
    payload: &[u8],
) -> Result<(), ProtoError> {
    if header.payload_len as usize != payload.len() {
        return Err(ProtoError::PayloadLengthMismatch {
            expected: header.payload_len,
            actual: payload.len() as u32,
        });
    }
    writer.write_all(&header.to_bytes()).await?;
    if !payload.is_empty() { writer.write_all(payload).await?; }
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{FrameHeader, MsgType};

    #[tokio::test]
    async fn test_codec_roundtrip() {
        let payload = b"hello synapse".to_vec();
        let header = FrameHeader::new(MsgType::Ping, 42, payload.len() as u32);
        let mut buf = Vec::new();
        write_frame(&mut buf, &header, &payload).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let (h, p) = read_frame(&mut cursor).await.unwrap();
        assert_eq!(h.msg_type, MsgType::Ping);
        assert_eq!(h.message_id, 42);
        assert_eq!(p, payload);
    }

    #[tokio::test]
    async fn test_codec_empty_payload() {
        let header = FrameHeader::new(MsgType::Pong, 1, 0);
        let mut buf = Vec::new();
        write_frame(&mut buf, &header, &[]).await.unwrap();
        let mut cursor = std::io::Cursor::new(buf);
        let (h, p) = read_frame(&mut cursor).await.unwrap();
        assert_eq!(h.msg_type, MsgType::Pong);
        assert!(p.is_empty());
    }

    #[tokio::test]
    async fn test_codec_length_mismatch_returns_error() {
        let header = FrameHeader::new(MsgType::Ping, 1, 10);
        let payload = b"short".to_vec(); // only 5 bytes, header says 10
        let mut buf = Vec::new();
        let result = write_frame(&mut buf, &header, &payload).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProtoError::PayloadLengthMismatch { expected, actual } => {
                assert_eq!(expected, 10);
                assert_eq!(actual, 5);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
