/// Decode percent-encoded characters in a URI path (e.g. `%20` -> ` `).
pub(super) fn percent_decode(input: &str) -> String {
    let mut input_bytes = input.bytes();
    let decoded = std::iter::from_fn(|| next_decoded_byte(&mut input_bytes)).collect::<Vec<_>>();

    String::from_utf8_lossy(&decoded).into_owned()
}

fn next_decoded_byte(input_bytes: &mut impl Iterator<Item = u8>) -> Option<u8> {
    let byte = input_bytes.next()?;
    Some(match byte {
        b'%' => decode_percent_escape(input_bytes).unwrap_or(b'%'),
        byte => byte,
    })
}

fn decode_percent_escape(input_bytes: &mut impl Iterator<Item = u8>) -> Option<u8> {
    let high = input_bytes.next()?;
    let low = input_bytes.next()?;

    Some((hex_nibble(high)? << 4) | hex_nibble(low)?)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn percent_decode_decodes_valid_uppercase_and_lowercase_hex_sequences() {
        assert_eq!(percent_decode("/tmp/My%20Repo"), "/tmp/My Repo");
        assert_eq!(
            percent_decode("/tmp/slash%2fchild%2Fleaf"),
            "/tmp/slash/child/leaf"
        );
        assert_eq!(percent_decode("/tmp/caf%C3%A9"), "/tmp/caf\u{e9}");
    }

    #[test]
    fn percent_decode_preserves_invalid_percent_marker_and_consumes_attempted_escape() {
        assert_eq!(percent_decode("%ZZ/path"), "%/path");
        assert_eq!(percent_decode("%G0/path"), "%/path");
        assert_eq!(percent_decode("%0G/path"), "%/path");
    }

    #[test]
    fn percent_decode_preserves_partial_percent_marker_and_consumes_available_tail() {
        assert_eq!(percent_decode("/tmp/%"), "/tmp/%");
        assert_eq!(percent_decode("/tmp/%A"), "/tmp/%");
        assert_eq!(percent_decode("/tmp/%A/path"), "/tmp/%path");
    }

    #[test]
    fn percent_decode_handles_mixed_valid_invalid_and_plain_sequences_in_order() {
        assert_eq!(
            percent_decode("/tmp/%41-%ZZ-%42-%A-tail"),
            "/tmp/A-%-B-%tail"
        );
    }

    #[test]
    fn percent_decode_uses_lossy_utf8_for_decoded_non_utf8_bytes() {
        assert_eq!(percent_decode("/tmp/%FF"), "/tmp/\u{fffd}");
        assert_eq!(percent_decode("/tmp/%C3%28"), "/tmp/\u{fffd}(");
    }
}
