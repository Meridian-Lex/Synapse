use crate::error::ProtoError;

const COMPRESS_THRESHOLD: usize = 64;

pub fn should_compress(payload: &[u8]) -> bool { payload.len() > COMPRESS_THRESHOLD }

pub fn compress(data: &[u8]) -> Result<Vec<u8>, ProtoError> {
    zstd::encode_all(data, 3).map_err(|e| ProtoError::CompressFailed(e.to_string()))
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>, ProtoError> {
    zstd::decode_all(data).map_err(|e| ProtoError::DecompressFailed(e.to_string()))
}

/// Decompress at most `max_bytes` of output. Returns an error if the decompressed
/// size would exceed `max_bytes`, preventing zip-bomb memory exhaustion.
pub fn decompress_bounded(data: &[u8], max_bytes: usize) -> Result<Vec<u8>, ProtoError> {
    use std::io::Read;
    let decoder = zstd::Decoder::new(data)
        .map_err(|e| ProtoError::DecompressFailed(e.to_string()))?;
    let mut out = Vec::new();
    // Read at most max_bytes + 1 so we can detect overflow without full decompression.
    decoder
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| ProtoError::DecompressFailed(e.to_string()))?;
    if out.len() > max_bytes {
        return Err(ProtoError::DecompressFailed(format!(
            "decompressed size exceeds limit {} bytes",
            max_bytes
        )));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_roundtrip() {
        let data = b"the quick brown fox jumped over the lazy dog".repeat(10);
        let compressed = compress(&data).unwrap();
        assert!(compressed.len() < data.len());
        assert_eq!(decompress(&compressed).unwrap(), data);
    }

    #[test]
    fn test_threshold() {
        assert!(!should_compress(b"hi"));
        assert!(should_compress(&b"x".repeat(65)));
    }

    #[test]
    fn test_decompress_bounded_exact_limit() {
        let payload = b"x".repeat(100);
        let compressed = compress(&payload).unwrap();
        // Limit exactly equals decompressed size — should succeed.
        let result = decompress_bounded(&compressed, 100).unwrap();
        assert_eq!(result, payload);
    }

    #[test]
    fn test_decompress_bounded_overflow() {
        let payload = b"x".repeat(101);
        let compressed = compress(&payload).unwrap();
        // Limit is one byte short — must return an error.
        let err = decompress_bounded(&compressed, 100).unwrap_err();
        assert!(matches!(err, ProtoError::DecompressFailed(_)));
    }

    #[test]
    fn test_decompress_bounded_empty_input() {
        // Empty compressed input is invalid zstd — must return an error.
        let err = decompress_bounded(&[], 1024).unwrap_err();
        assert!(matches!(err, ProtoError::DecompressFailed(_)));
    }

    #[test]
    fn test_decompress_bounded_corrupt_input() {
        // Corrupt bytes are not valid zstd — must return an error.
        let err = decompress_bounded(b"\xff\xfe\xfd\xfc", 1024).unwrap_err();
        assert!(matches!(err, ProtoError::DecompressFailed(_)));
    }
}
