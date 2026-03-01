use crate::error::ProtoError;

const COMPRESS_THRESHOLD: usize = 64;

pub fn should_compress(payload: &[u8]) -> bool { payload.len() > COMPRESS_THRESHOLD }

pub fn compress(data: &[u8]) -> Result<Vec<u8>, ProtoError> {
    zstd::encode_all(data, 3).map_err(|e| ProtoError::CompressFailed(e.to_string()))
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>, ProtoError> {
    zstd::decode_all(data).map_err(|e| ProtoError::DecompressFailed(e.to_string()))
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
}
