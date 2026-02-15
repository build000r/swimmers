//! Binary frame codec for the Throngterm realtime protocol.
//!
//! Frame layouts (network byte order / big-endian):
//!
//! TERMINAL_INPUT  (0x10): u8 opcode | u16 session_id_len | session_id_utf8 | raw_input_bytes
//! TERMINAL_OUTPUT (0x11): u8 opcode | u16 session_id_len | session_id_utf8 | u64 seq | raw_output_bytes

use crate::types::opcodes;

/// Errors encountered when decoding a binary frame.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("frame too short: need at least {expected} bytes, got {actual}")]
    FrameTooShort { expected: usize, actual: usize },

    #[error("unknown opcode: 0x{0:02x}")]
    UnknownOpcode(u8),

    #[error("session_id is not valid UTF-8: {0}")]
    InvalidSessionId(#[from] std::string::FromUtf8Error),

    #[error("session_id_len ({len}) exceeds remaining frame data ({remaining})")]
    SessionIdOverflow { len: usize, remaining: usize },
}

/// Decode a `TERMINAL_INPUT` binary frame.
///
/// Returns `(session_id, input_bytes)`.
///
/// Expected layout: `0x10 | u16 session_id_len | session_id_utf8 | raw_input_bytes`
pub fn decode_input_frame(data: &[u8]) -> Result<(String, Vec<u8>), CodecError> {
    // Minimum: 1 (opcode) + 2 (session_id_len) = 3 bytes
    if data.len() < 3 {
        return Err(CodecError::FrameTooShort {
            expected: 3,
            actual: data.len(),
        });
    }

    let opcode = data[0];
    if opcode != opcodes::TERMINAL_INPUT {
        return Err(CodecError::UnknownOpcode(opcode));
    }

    let session_id_len = u16::from_be_bytes([data[1], data[2]]) as usize;
    let header_end = 3 + session_id_len;

    if data.len() < header_end {
        return Err(CodecError::SessionIdOverflow {
            len: session_id_len,
            remaining: data.len() - 3,
        });
    }

    let session_id = String::from_utf8(data[3..header_end].to_vec())?;
    let input_bytes = data[header_end..].to_vec();

    Ok((session_id, input_bytes))
}

/// Encode a `TERMINAL_OUTPUT` binary frame.
///
/// Layout: `0x11 | u16 session_id_len | session_id_utf8 | u64 seq | raw_output_bytes`
pub fn encode_output_frame(session_id: &str, seq: u64, data: &[u8]) -> Vec<u8> {
    let id_bytes = session_id.as_bytes();
    let id_len = id_bytes.len() as u16;

    // 1 (opcode) + 2 (id_len) + id_bytes.len() + 8 (seq) + data.len()
    let total = 1 + 2 + id_bytes.len() + 8 + data.len();
    let mut buf = Vec::with_capacity(total);

    buf.push(opcodes::TERMINAL_OUTPUT);
    buf.extend_from_slice(&id_len.to_be_bytes());
    buf.extend_from_slice(id_bytes);
    buf.extend_from_slice(&seq.to_be_bytes());
    buf.extend_from_slice(data);

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_output_frame() {
        let session_id = "test-session-42";
        let seq = 1234u64;
        let payload = b"hello terminal";

        let frame = encode_output_frame(session_id, seq, payload);

        // Verify opcode
        assert_eq!(frame[0], opcodes::TERMINAL_OUTPUT);

        // Verify session_id_len
        let id_len = u16::from_be_bytes([frame[1], frame[2]]) as usize;
        assert_eq!(id_len, session_id.len());

        // Verify session_id
        let id = std::str::from_utf8(&frame[3..3 + id_len]).unwrap();
        assert_eq!(id, session_id);

        // Verify seq
        let seq_offset = 3 + id_len;
        let decoded_seq = u64::from_be_bytes(
            frame[seq_offset..seq_offset + 8].try_into().unwrap(),
        );
        assert_eq!(decoded_seq, seq);

        // Verify payload
        let data_offset = seq_offset + 8;
        assert_eq!(&frame[data_offset..], payload);
    }

    #[test]
    fn roundtrip_input_frame() {
        let session_id = "my-session";
        let input = b"ls -la\n";

        // Build an input frame manually
        let id_bytes = session_id.as_bytes();
        let id_len = id_bytes.len() as u16;
        let mut frame = Vec::new();
        frame.push(opcodes::TERMINAL_INPUT);
        frame.extend_from_slice(&id_len.to_be_bytes());
        frame.extend_from_slice(id_bytes);
        frame.extend_from_slice(input);

        let (decoded_id, decoded_input) = decode_input_frame(&frame).unwrap();
        assert_eq!(decoded_id, session_id);
        assert_eq!(decoded_input, input);
    }

    #[test]
    fn decode_input_frame_too_short() {
        let result = decode_input_frame(&[0x10, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn decode_input_frame_wrong_opcode() {
        let result = decode_input_frame(&[0xFF, 0x00, 0x00]);
        assert!(matches!(result, Err(CodecError::UnknownOpcode(0xFF))));
    }

    #[test]
    fn decode_input_frame_session_id_overflow() {
        // Claims session_id is 100 bytes but only 2 bytes remain
        let frame = [0x10, 0x00, 0x64, 0x41, 0x42];
        let result = decode_input_frame(&frame);
        assert!(matches!(result, Err(CodecError::SessionIdOverflow { .. })));
    }

    #[test]
    fn decode_input_frame_empty_payload() {
        let session_id = "s1";
        let id_bytes = session_id.as_bytes();
        let id_len = id_bytes.len() as u16;
        let mut frame = Vec::new();
        frame.push(opcodes::TERMINAL_INPUT);
        frame.extend_from_slice(&id_len.to_be_bytes());
        frame.extend_from_slice(id_bytes);
        // No payload bytes

        let (decoded_id, decoded_input) = decode_input_frame(&frame).unwrap();
        assert_eq!(decoded_id, "s1");
        assert!(decoded_input.is_empty());
    }
}
