//! Length-bounded wire contract for the experimental macOS input method.
//!
//! Arm binds a one-use helper lease to the active InputMethodKit client before
//! recording. Insert consumes that lease. A crossed, expired, or repeated frame
//! therefore cannot trigger a second native write.

pub(crate) const RESPONSE_LENGTH: usize = 24;
const REQUEST_HEADER_LENGTH: usize = 56;
const MAX_BUNDLE_IDENTIFIER_BYTES: usize = 512;
const MAX_TRANSCRIPT_BYTES: usize = 1024 * 1024;
const REQUEST_MAGIC: [u8; 4] = *b"SPK2";
const RESPONSE_MAGIC: [u8; 4] = *b"SPR2";
pub(crate) const PROTOCOL_VERSION: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum RequestOperation {
    Arm = 1,
    Insert = 2,
    Disarm = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputMethodResponseStatus {
    Confirmed,
    Dispatched,
    NoActiveClient,
    TargetMismatch,
    SelectionChanged,
    Unsupported,
    SecureInput,
    InvalidRequest,
    InternalError,
    Armed,
    Disarmed,
    LeaseExpired,
    RequestExpired,
    LeaseMissingOrConsumed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputMethodResponse {
    pub status: InputMethodResponseStatus,
    pub lease_id: u64,
}

pub(crate) fn encode_arm_request(
    request_id: u64,
    expires_at_milliseconds: u64,
    selection_location: usize,
    selection_length: usize,
    bundle_identifier: &str,
) -> Result<Vec<u8>, &'static str> {
    encode_request(
        RequestOperation::Arm,
        request_id,
        0,
        expires_at_milliseconds,
        selection_location,
        selection_length,
        bundle_identifier,
        "",
    )
}

pub(crate) fn encode_insert_request(
    request_id: u64,
    lease_id: u64,
    expires_at_milliseconds: u64,
    selection_location: usize,
    selection_length: usize,
    bundle_identifier: &str,
    text: &str,
) -> Result<Vec<u8>, &'static str> {
    encode_request(
        RequestOperation::Insert,
        request_id,
        lease_id,
        expires_at_milliseconds,
        selection_location,
        selection_length,
        bundle_identifier,
        text,
    )
}

pub(crate) fn encode_disarm_request(
    request_id: u64,
    lease_id: u64,
    expires_at_milliseconds: u64,
) -> Result<Vec<u8>, &'static str> {
    encode_request(
        RequestOperation::Disarm,
        request_id,
        lease_id,
        expires_at_milliseconds,
        0,
        0,
        "",
        "",
    )
}

#[allow(clippy::too_many_arguments)]
fn encode_request(
    operation: RequestOperation,
    request_id: u64,
    lease_id: u64,
    expires_at_milliseconds: u64,
    selection_location: usize,
    selection_length: usize,
    bundle_identifier: &str,
    text: &str,
) -> Result<Vec<u8>, &'static str> {
    if request_id == 0 || expires_at_milliseconds == 0 {
        return Err("the input-method request identity is invalid");
    }
    match operation {
        RequestOperation::Arm if lease_id != 0 => {
            return Err("an arm request cannot reuse a native lease");
        }
        RequestOperation::Insert | RequestOperation::Disarm if lease_id == 0 => {
            return Err("the native input-method lease is missing");
        }
        _ => {}
    }

    let bundle = bundle_identifier.as_bytes();
    let transcript = text.as_bytes();
    match operation {
        RequestOperation::Arm | RequestOperation::Insert => {
            if bundle.is_empty()
                || bundle.len() > MAX_BUNDLE_IDENTIFIER_BYTES
                || bundle_identifier.chars().any(char::is_control)
            {
                return Err("the target application identifier is invalid");
            }
        }
        RequestOperation::Disarm if !bundle.is_empty() => {
            return Err("a disarm request cannot carry an application identifier");
        }
        RequestOperation::Disarm => {}
    }
    match operation {
        RequestOperation::Insert => {
            if transcript.is_empty() || transcript.len() > MAX_TRANSCRIPT_BYTES {
                return Err("the transcript is too large for one input-method request");
            }
        }
        RequestOperation::Arm | RequestOperation::Disarm if !transcript.is_empty() => {
            return Err("this input-method request cannot carry transcript text");
        }
        _ => {}
    }
    selection_location
        .checked_add(selection_length)
        .ok_or("the target selection is invalid")?;
    if operation == RequestOperation::Disarm && (selection_location != 0 || selection_length != 0) {
        return Err("a disarm request cannot carry a text selection");
    }

    let frame_length = REQUEST_HEADER_LENGTH
        .checked_add(bundle.len())
        .and_then(|length| length.checked_add(transcript.len()))
        .ok_or("the input-method request is too large")?;
    let mut frame = Vec::with_capacity(frame_length);
    frame.extend_from_slice(&REQUEST_MAGIC);
    frame.extend_from_slice(&[PROTOCOL_VERSION, operation as u8, 0, 0]);
    frame.extend_from_slice(&request_id.to_be_bytes());
    frame.extend_from_slice(&lease_id.to_be_bytes());
    frame.extend_from_slice(&expires_at_milliseconds.to_be_bytes());
    frame.extend_from_slice(&(selection_location as u64).to_be_bytes());
    frame.extend_from_slice(&(selection_length as u64).to_be_bytes());
    frame.extend_from_slice(&(bundle.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(transcript.len() as u32).to_be_bytes());
    frame.extend_from_slice(bundle);
    frame.extend_from_slice(transcript);
    debug_assert_eq!(frame.len(), frame_length);
    Ok(frame)
}

pub(crate) fn decode_response(
    response: &[u8],
    expected_request_id: u64,
) -> Result<InputMethodResponse, &'static str> {
    if response.len() != RESPONSE_LENGTH {
        return Err("the input-method helper returned a truncated response");
    }
    if response[..4] != RESPONSE_MAGIC
        || response[4] != PROTOCOL_VERSION
        || response[6] != 0
        || response[7] != 0
    {
        return Err("the input-method helper returned an invalid response");
    }
    let request_id = u64::from_be_bytes(
        response[8..16]
            .try_into()
            .expect("a fixed response always has eight identifier bytes"),
    );
    if request_id != expected_request_id {
        return Err("the input-method helper returned a stale response");
    }
    let lease_id = u64::from_be_bytes(
        response[16..24]
            .try_into()
            .expect("a fixed response always has eight lease bytes"),
    );
    let status = match response[5] {
        1 => InputMethodResponseStatus::Confirmed,
        2 => InputMethodResponseStatus::Dispatched,
        3 => InputMethodResponseStatus::NoActiveClient,
        4 => InputMethodResponseStatus::TargetMismatch,
        5 => InputMethodResponseStatus::SelectionChanged,
        6 => InputMethodResponseStatus::Unsupported,
        7 => InputMethodResponseStatus::SecureInput,
        8 => InputMethodResponseStatus::InvalidRequest,
        9 => InputMethodResponseStatus::InternalError,
        10 => InputMethodResponseStatus::Armed,
        11 => InputMethodResponseStatus::Disarmed,
        12 => InputMethodResponseStatus::LeaseExpired,
        13 => InputMethodResponseStatus::RequestExpired,
        14 => InputMethodResponseStatus::LeaseMissingOrConsumed,
        _ => return Err("the input-method helper returned an unknown status"),
    };
    if (status == InputMethodResponseStatus::Armed) != (lease_id != 0) {
        return Err("the input-method helper returned an invalid lease");
    }
    Ok(InputMethodResponse { status, lease_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u8, request_id: u64, lease_id: u64) -> [u8; RESPONSE_LENGTH] {
        let mut response = [0_u8; RESPONSE_LENGTH];
        response[..4].copy_from_slice(&RESPONSE_MAGIC);
        response[4] = PROTOCOL_VERSION;
        response[5] = status;
        response[8..16].copy_from_slice(&request_id.to_be_bytes());
        response[16..24].copy_from_slice(&lease_id.to_be_bytes());
        response
    }

    #[test]
    fn insert_frame_matches_the_native_big_endian_contract() {
        let text = "नमस्ते 👋 — مرحباً";
        let frame =
            encode_insert_request(42, 99, 123_456, 12, 3, "com.example.Editor", text).unwrap();
        assert_eq!(&frame[..4], b"SPK2");
        assert_eq!(&frame[4..8], &[2, 2, 0, 0]);
        assert_eq!(u64::from_be_bytes(frame[8..16].try_into().unwrap()), 42);
        assert_eq!(u64::from_be_bytes(frame[16..24].try_into().unwrap()), 99);
        assert_eq!(
            u64::from_be_bytes(frame[24..32].try_into().unwrap()),
            123_456
        );
        assert_eq!(u64::from_be_bytes(frame[32..40].try_into().unwrap()), 12);
        assert_eq!(u64::from_be_bytes(frame[40..48].try_into().unwrap()), 3);
        let bundle_length = u32::from_be_bytes(frame[48..52].try_into().unwrap()) as usize;
        let text_length = u32::from_be_bytes(frame[52..56].try_into().unwrap()) as usize;
        assert_eq!(bundle_length, "com.example.Editor".len());
        assert_eq!(text_length, text.len());
        assert_eq!(
            &frame[REQUEST_HEADER_LENGTH..REQUEST_HEADER_LENGTH + bundle_length],
            b"com.example.Editor"
        );
        assert_eq!(
            &frame[REQUEST_HEADER_LENGTH + bundle_length..],
            text.as_bytes()
        );
    }

    #[test]
    fn insert_frame_matches_the_shared_native_golden_bytes() {
        let frame =
            encode_insert_request(42, 99, 123_456, 12, 3, "com.example.Editor", "Hi").unwrap();
        let expected = [
            0x53, 0x50, 0x4b, 0x32, 0x02, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x2a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x63, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x01, 0xe2, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0c, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x12, 0x00, 0x00, 0x00, 0x02,
            0x63, 0x6f, 0x6d, 0x2e, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e, 0x45, 0x64,
            0x69, 0x74, 0x6f, 0x72, 0x48, 0x69,
        ];
        assert_eq!(frame, expected);
    }

    #[test]
    fn arm_and_disarm_frames_never_carry_transcript_text() {
        let arm = encode_arm_request(42, 123_456, 12, 3, "com.example.Editor").unwrap();
        assert_eq!(arm[5], RequestOperation::Arm as u8);
        assert_eq!(u32::from_be_bytes(arm[52..56].try_into().unwrap()), 0);

        let disarm = encode_disarm_request(42, 99, 123_456).unwrap();
        assert_eq!(disarm[5], RequestOperation::Disarm as u8);
        assert_eq!(u32::from_be_bytes(disarm[48..52].try_into().unwrap()), 0);
        assert_eq!(u32::from_be_bytes(disarm[52..56].try_into().unwrap()), 0);
    }

    #[test]
    fn frames_reject_unbounded_or_ambiguous_inputs() {
        assert!(encode_arm_request(0, 1, 0, 0, "com.example.Editor").is_err());
        assert!(encode_arm_request(1, 0, 0, 0, "com.example.Editor").is_err());
        assert!(encode_arm_request(1, 1, 0, 0, "bad\nidentifier").is_err());
        assert!(encode_insert_request(1, 0, 1, 0, 0, "com.example.Editor", "hello").is_err());
        assert!(encode_insert_request(1, 2, 1, 0, 0, "com.example.Editor", "").is_err());
        assert!(encode_insert_request(
            1,
            2,
            1,
            0,
            0,
            "com.example.Editor",
            &"x".repeat(MAX_TRANSCRIPT_BYTES + 1),
        )
        .is_err());
        assert!(encode_arm_request(1, 1, usize::MAX, 1, "com.example.Editor").is_err());
        assert!(encode_disarm_request(1, 0, 1).is_err());

        let maximum_bundle = "x".repeat(MAX_BUNDLE_IDENTIFIER_BYTES);
        let maximum_text = "x".repeat(MAX_TRANSCRIPT_BYTES);
        assert!(encode_arm_request(1, 1, 0, 0, &maximum_bundle).is_ok());
        assert!(encode_insert_request(1, 2, 1, 0, 0, "com.example.Editor", &maximum_text).is_ok());
        assert!(encode_arm_request(1, 1, 0, 0, &format!("{maximum_bundle}x")).is_err());
    }

    #[test]
    fn every_helper_status_is_explicit() {
        let statuses = [
            InputMethodResponseStatus::Confirmed,
            InputMethodResponseStatus::Dispatched,
            InputMethodResponseStatus::NoActiveClient,
            InputMethodResponseStatus::TargetMismatch,
            InputMethodResponseStatus::SelectionChanged,
            InputMethodResponseStatus::Unsupported,
            InputMethodResponseStatus::SecureInput,
            InputMethodResponseStatus::InvalidRequest,
            InputMethodResponseStatus::InternalError,
            InputMethodResponseStatus::Armed,
            InputMethodResponseStatus::Disarmed,
            InputMethodResponseStatus::LeaseExpired,
            InputMethodResponseStatus::RequestExpired,
            InputMethodResponseStatus::LeaseMissingOrConsumed,
        ];
        for (index, expected) in statuses.into_iter().enumerate() {
            let lease_id = if expected == InputMethodResponseStatus::Armed {
                99
            } else {
                0
            };
            assert_eq!(
                decode_response(&response(index as u8 + 1, 42, lease_id), 42)
                    .unwrap()
                    .status,
                expected
            );
        }
    }

    #[test]
    fn response_must_match_the_current_request_and_lease_shape() {
        assert!(decode_response(&response(1, 41, 0), 42).is_err());
        assert!(decode_response(&response(0, 42, 0), 42).is_err());
        assert!(decode_response(&response(10, 42, 0), 42).is_err());
        assert!(decode_response(&response(1, 42, 99), 42).is_err());
        let mut malformed = response(1, 42, 0);
        malformed[0] = b'X';
        assert!(decode_response(&malformed, 42).is_err());
        let mut wrong_version = response(1, 42, 0);
        wrong_version[4] = 3;
        assert!(decode_response(&wrong_version, 42).is_err());
        let mut nonzero_reserved = response(1, 42, 0);
        nonzero_reserved[7] = 1;
        assert!(decode_response(&nonzero_reserved, 42).is_err());
        assert!(decode_response(&malformed[..23], 42).is_err());
    }
}
