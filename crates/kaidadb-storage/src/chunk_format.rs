use bytes::Bytes;
use kaidadb_common::{
    KaidaDbError, CHUNK_FORMAT_VERSION, CHUNK_HEADER_SIZE, CHUNK_MAGIC,
};

/// Encode a chunk payload into the .kdc on-disk format.
///
/// Format:
/// ```text
/// [0-3]   Magic: 0x4B444243 ("KDBC")
/// [4]     Version: u8
/// [5]     Flags: u8
/// [6-9]   CRC32 of payload
/// [10-17] Payload length: u64 (little-endian)
/// [18..]  Payload bytes
/// ```
pub fn encode_chunk(data: &[u8]) -> Vec<u8> {
    let crc = crc32fast::hash(data);
    let len = data.len() as u64;

    let mut buf = Vec::with_capacity(CHUNK_HEADER_SIZE + data.len());
    buf.extend_from_slice(&CHUNK_MAGIC);
    buf.push(CHUNK_FORMAT_VERSION);
    buf.push(0u8); // flags
    buf.extend_from_slice(&crc.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(data);
    buf
}

/// Decode and verify a chunk from on-disk format, returning the payload.
pub fn decode_chunk(raw: &[u8]) -> kaidadb_common::Result<Bytes> {
    if raw.len() < CHUNK_HEADER_SIZE {
        return Err(KaidaDbError::InvalidChunkFormat(
            "data too short for header".into(),
        ));
    }

    if raw[0..4] != CHUNK_MAGIC {
        return Err(KaidaDbError::InvalidChunkFormat("bad magic bytes".into()));
    }

    let version = raw[4];
    if version != CHUNK_FORMAT_VERSION {
        return Err(KaidaDbError::InvalidChunkFormat(format!(
            "unsupported version: {version}"
        )));
    }

    let stored_crc = u32::from_le_bytes(raw[6..10].try_into().unwrap());
    let payload_len = u64::from_le_bytes(raw[10..18].try_into().unwrap()) as usize;

    if raw.len() < CHUNK_HEADER_SIZE + payload_len {
        return Err(KaidaDbError::InvalidChunkFormat(format!(
            "expected {} payload bytes, got {}",
            payload_len,
            raw.len() - CHUNK_HEADER_SIZE
        )));
    }

    let payload = &raw[CHUNK_HEADER_SIZE..CHUNK_HEADER_SIZE + payload_len];
    let computed_crc = crc32fast::hash(payload);
    if stored_crc != computed_crc {
        return Err(KaidaDbError::ChunkIntegrity {
            chunk_id: "unknown".into(),
            detail: format!("CRC mismatch: stored={stored_crc:#x}, computed={computed_crc:#x}"),
        });
    }

    Ok(Bytes::copy_from_slice(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let data = b"hello, kaidadb chunk!";
        let encoded = encode_chunk(data);
        let decoded = decode_chunk(&encoded).unwrap();
        assert_eq!(&decoded[..], data);
    }

    #[test]
    fn test_decode_bad_magic() {
        let mut encoded = encode_chunk(b"data");
        encoded[0] = 0xFF;
        assert!(decode_chunk(&encoded).is_err());
    }

    #[test]
    fn test_decode_corrupted_payload() {
        let mut encoded = encode_chunk(b"data");
        let last = encoded.len() - 1;
        encoded[last] ^= 0xFF;
        assert!(decode_chunk(&encoded).is_err());
    }

    #[test]
    fn test_decode_truncated() {
        let encoded = encode_chunk(b"data");
        assert!(decode_chunk(&encoded[..10]).is_err());
    }
}
