//! Wire-decoding errors.

/// Errors raised while decoding a binary exchange message.
///
/// These are all *data* errors, not I/O errors: a malformed or truncated message means the
/// caller must resynchronise (usually by dropping the packet and requesting a
/// retransmission), never that it should silently continue with a partial value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    #[error("truncated message: need {need} bytes at offset {at}, only {have} available")]
    Truncated { at: usize, need: usize, have: usize },

    #[error("unknown message type {got:#04x} ({got_ascii:?}) for {protocol}")]
    UnknownMessageType {
        protocol: &'static str,
        got: u16,
        got_ascii: char,
    },

    #[error("{protocol} message type {msg_type} declares length {declared}, expected {expected}")]
    LengthMismatch {
        protocol: &'static str,
        msg_type: u16,
        declared: usize,
        expected: usize,
    },

    #[error("invalid {field} value {value:?} in {protocol} message")]
    InvalidEnum {
        protocol: &'static str,
        field: &'static str,
        value: char,
    },

    #[error("field {field} is not valid ASCII")]
    NotAscii { field: &'static str },

    #[error("numeric field {field} out of range: {value}")]
    OutOfRange { field: &'static str, value: i128 },
}

pub type WireResult<T> = Result<T, WireError>;
