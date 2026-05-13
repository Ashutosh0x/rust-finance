#![forbid(unsafe_code)]
// crates/fix/src/lib.rs
//
// FIX 4.2/4.4 protocol engine — parser, serializer, session layer.
//
// v0.3: Replaced stub parser with production-grade tag-value parser.
// Zero external dependencies — hand-rolled for maximum control.
//
// Parser design:
//   1. Read tag 8 (BeginString) and tag 9 (BodyLength) from the buffer
//   2. Use BodyLength to determine exact message boundary
//   3. Extract tag=value pairs from the body
//   4. Validate checksum (tag 10)
//   5. Derive MsgType from tag 35
pub mod session;

#[derive(Debug, thiserror::Error)]
pub enum FixError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: u8, actual: u8 },
    /// Tag 9 (BodyLength) framing is inconsistent with the bytes actually
    /// present: the declared length lands inside the buffer but does not
    /// point at the CheckSum field. (Distinct from "not enough bytes yet",
    /// which is `Ok(None)`, not an error.)
    #[error("BodyLength framing error: {0}")]
    BodyLengthFraming(String),
    /// The frame's CheckSum field (tag 10) is missing where BodyLength said
    /// it would be, or is not a parseable numeric value.
    #[error("missing or malformed CheckSum field: {0}")]
    MissingChecksumField(String),
    /// Catch-all for a structurally malformed frame at the current `8=`
    /// start (e.g. no `9=` after `8=`, non-numeric BodyLength).
    #[error("malformed FIX frame: {reason}")]
    MalformedFrame { reason: String },
}

pub mod serializer {
    use super::FixError;

    #[derive(Debug, Clone, PartialEq)]
    pub enum MsgType {
        Logon,
        Logout,
        Heartbeat,
        TestRequest,
        ResendRequest,
        SequenceReset,
        ExecutionReport,
        OrderCancelReject,
        NewOrderSingle,
        OrderCancelRequest,
        Unknown,
    }

    impl MsgType {
        /// Parse MsgType from FIX tag 35 value.
        pub fn from_fix_value(val: &str) -> Self {
            match val {
                "A" => Self::Logon,
                "5" => Self::Logout,
                "0" => Self::Heartbeat,
                "1" => Self::TestRequest,
                "2" => Self::ResendRequest,
                "4" => Self::SequenceReset,
                "8" => Self::ExecutionReport,
                "9" => Self::OrderCancelReject,
                "D" => Self::NewOrderSingle,
                "F" => Self::OrderCancelRequest,
                _ => Self::Unknown,
            }
        }

        /// Convert to FIX tag 35 value.
        pub fn to_fix_value(&self) -> &'static str {
            match self {
                Self::Logon => "A",
                Self::Logout => "5",
                Self::Heartbeat => "0",
                Self::TestRequest => "1",
                Self::ResendRequest => "2",
                Self::SequenceReset => "4",
                Self::ExecutionReport => "8",
                Self::OrderCancelReject => "9",
                Self::NewOrderSingle => "D",
                Self::OrderCancelRequest => "F",
                Self::Unknown => "?",
            }
        }
    }

    #[derive(Debug)]
    pub struct FixMessage {
        msg_type: MsgType,
        fields: std::collections::HashMap<u32, String>,
    }

    impl FixMessage {
        pub fn new(msg_type: MsgType) -> Self {
            Self {
                msg_type,
                fields: std::collections::HashMap::new(),
            }
        }

        /// Parse a FixMessage from raw tag=value pairs (SOH-delimited bytes).
        /// This is used internally by FixParser after framing.
        pub fn from_tag_values(raw: &[u8]) -> Result<Self, FixError> {
            let text = std::str::from_utf8(raw)
                .map_err(|e| FixError::Parse(format!("Invalid UTF-8: {}", e)))?;

            let mut fields = std::collections::HashMap::new();
            let mut msg_type = MsgType::Unknown;

            for pair in text.split('\x01') {
                if pair.is_empty() {
                    continue;
                }
                let eq_pos = pair
                    .find('=')
                    .ok_or_else(|| FixError::Parse(format!("Missing '=' in field: {}", pair)))?;
                let tag: u32 = pair[..eq_pos]
                    .parse()
                    .map_err(|_| FixError::Parse(format!("Invalid tag: {}", &pair[..eq_pos])))?;
                let val = &pair[eq_pos + 1..];

                // Tag 35 = MsgType
                if tag == 35 {
                    msg_type = MsgType::from_fix_value(val);
                }

                fields.insert(tag, val.to_string());
            }

            Ok(Self { msg_type, fields })
        }

        pub fn msg_type(&self) -> MsgType {
            self.msg_type.clone()
        }
        pub fn set_field(&mut self, tag: u32, val: &str) {
            self.fields.insert(tag, val.to_string());
        }
        pub fn get_field(&self, tag: u32) -> Option<&String> {
            self.fields.get(&tag)
        }

        /// Get all fields (for debugging/logging).
        pub fn fields(&self) -> &std::collections::HashMap<u32, String> {
            &self.fields
        }

        pub fn encode(&self) -> Vec<u8> {
            // Collect fields from the internal map.
            let mut fields: Vec<(u32, String)> =
                self.fields.iter().map(|(k, v)| (*k, v.clone())).collect();

            // Ensure MsgType (35) is present; derive it from self.msg_type if missing.
            if !fields.iter().any(|(tag, _)| *tag == 35) {
                fields.push((35, self.msg_type.to_fix_value().to_string()));
            }

            // Extract BeginString (8) if present; it must precede BodyLength (9).
            let mut begin_string: Option<String> = None;
            let mut msg_type_val: Option<String> = None;
            let mut other_fields: Vec<(u32, String)> = Vec::new();

            for (tag, val) in fields.into_iter() {
                match tag {
                    8 => begin_string = Some(val),
                    35 => msg_type_val = Some(val),
                    9 | 10 => { /* skip */ }
                    _ => other_fields.push((tag, val)),
                }
            }

            other_fields.sort_by_key(|(tag, _)| *tag);

            // Build the body portion starting with MsgType (35), followed by the rest.
            let mut body_part = String::new();
            if let Some(mt) = msg_type_val {
                body_part.push_str(&format!("35={}\x01", mt));
            }
            for (tag, val) in other_fields {
                body_part.push_str(&format!("{}={}\x01", tag, val));
            }

            // BodyLength (9) is the length in bytes of the message after 9=...<SOH>,
            // i.e., the length of body_part.
            let body_length = body_part.len();

            // Construct the full message: optional 8=, then 9=BodyLength, then body_part.
            let mut out = String::new();
            if let Some(begin) = begin_string {
                out.push_str(&format!("8={}\x01", begin));
            }
            out.push_str(&format!("9={}\x01", body_length));
            out.push_str(&body_part);

            // Compute CheckSum (10): sum of all bytes modulo 256, formatted as 3 digits.
            let sum: u32 = out.as_bytes().iter().map(|b| *b as u32).sum();
            let checksum = (sum % 256) as u8;
            out.push_str(&format!("10={:03}\x01", checksum));

            out.into_bytes()
        }
    }

    // ─── Production FIX Parser ───────────────────────────────────

    /// Length-delimited FIX parser.
    ///
    /// Accumulates bytes via `push_bytes()`, then call `next_message()`
    /// to extract complete messages using the BodyLength (tag 9) framing.
    ///
    /// This replaces the v0.2 stub that returned dummy Heartbeats.
    pub struct FixParser {
        buffer: Vec<u8>,
    }

    impl Default for FixParser {
        fn default() -> Self {
            Self::new()
        }
    }

    impl FixParser {
        pub fn new() -> Self {
            Self {
                buffer: Vec::with_capacity(4096),
            }
        }

        pub fn push_bytes(&mut self, bytes: &[u8]) {
            self.buffer.extend_from_slice(bytes);
        }

        /// Extract the next complete FIX message from the buffer.
        ///
        /// Three-way result — the whole point of this signature is to *not*
        /// conflate "wait for more bytes" with "this frame is garbage":
        ///   * `Ok(Some(msg))` — a complete, checksum-valid message was parsed
        ///     and consumed from the buffer.
        ///   * `Ok(None)`      — not enough bytes yet (including: tag 9's
        ///     declared BodyLength runs past what's buffered). Buffer is left
        ///     untouched; call again after more bytes arrive.
        ///   * `Err(..)`       — the frame at the current `8=` start is
        ///     malformed (bad BodyLength framing, missing/bad CheckSum, bad
        ///     checksum value). Before returning, the parser **resynchronizes**:
        ///     it discards exactly the corrupt frame and leaves any bytes that
        ///     follow it intact, so a subsequent valid message in the same TCP
        ///     read is still recoverable on the next call. This is the
        ///     "recover-open" invariant: one bad frame must not desync the
        ///     stream or silently swallow good messages behind it.
        ///
        /// Algorithm:
        ///   1. Find "8=" prefix (BeginString start)
        ///   2. Find "9=<BodyLength>" field
        ///   3. Read exactly BodyLength bytes after the 9= field's SOH
        ///   4. Expect "10=<checksum>" immediately after
        ///   5. Validate checksum
        ///   6. Parse all tag=value pairs
        pub fn next_message(&mut self) -> Result<Option<FixMessage>, FixError> {
            // We need at minimum "8=FIX.X.X\x019=N\x0135=X\x0110=XXX\x01".
            // Too few bytes to even hold a header => definitely incomplete.
            if self.buffer.len() < 20 {
                return Ok(None);
            }

            // Step 1: Find start of message (tag 8). No `8=` anywhere yet =>
            // nothing actionable; treat as incomplete (more bytes may bring
            // the start of a frame).
            let msg_start = match self.find_tag_start(8) {
                Some(s) => s,
                None => return Ok(None),
            };

            // Step 2: Find BodyLength (tag 9). It must appear after `8=`.
            // If there's no `9=` but there *is* a later `8=`, the current
            // frame's header is structurally broken => malformed, resync.
            let tag9_start = match self.find_tag_in_range(9, msg_start) {
                Some(s) => s,
                None => {
                    return self.fail_resync(
                        msg_start,
                        FixError::MalformedFrame {
                            reason: "no BodyLength (tag 9) after BeginString".to_string(),
                        },
                    );
                }
            };
            // We just located `9=`; the `=` and a following SOH must exist.
            let tag9_val_start = match self.skip_past_equals(tag9_start) {
                Some(p) => p,
                // `9` with no `=` yet — could still be mid-write of the field.
                None => return Ok(None),
            };
            let tag9_soh = match self.find_soh_after(tag9_val_start) {
                Some(p) => p,
                // BodyLength value not terminated yet — incomplete.
                None => return Ok(None),
            };

            let body_len_str = match std::str::from_utf8(&self.buffer[tag9_val_start..tag9_soh]) {
                Ok(s) => s,
                Err(_) => {
                    return self.fail_resync(
                        msg_start,
                        FixError::BodyLengthFraming(
                            "BodyLength field is not valid UTF-8".to_string(),
                        ),
                    );
                }
            };
            let body_length: usize = match body_len_str.parse() {
                Ok(n) => n,
                Err(_) => {
                    return self.fail_resync(
                        msg_start,
                        FixError::BodyLengthFraming(format!(
                            "BodyLength is not a number: {body_len_str:?}"
                        )),
                    );
                }
            };

            // Step 3: Body starts after "9=N\x01"
            let body_start = tag9_soh + 1;
            let body_end = body_start + body_length;

            // Do we have enough bytes? body + at least "10=N\x01" (5 bytes).
            // If not, this is *incomplete*, NOT malformed — the declared body
            // simply hasn't fully arrived. Preserve the buffer and wait.
            if self.buffer.len() < body_end + 5 {
                return Ok(None);
            }

            // Step 4: BodyLength is supposed to land us exactly on "10=".
            // If it doesn't, the framing is wrong: BodyLength under/over-counts
            // and the CheckSum tag isn't where it claimed to be.
            let checksum_region_start = body_end;
            if self
                .buffer
                .get(checksum_region_start..checksum_region_start + 3)
                != Some(b"10=")
            {
                return self.fail_resync(
                    msg_start,
                    FixError::BodyLengthFraming(format!(
                        "BodyLength={body_length} does not point at the CheckSum field (no `10=` there)"
                    )),
                );
            }

            let cs_val_start = checksum_region_start + 3;
            let cs_soh = match self.find_soh_after(cs_val_start) {
                Some(p) => p,
                // We saw `10=` but its value isn't SOH-terminated yet.
                None => return Ok(None),
            };
            let msg_end = cs_soh + 1;

            // Step 5: Validate checksum
            let expected_checksum_str =
                match std::str::from_utf8(&self.buffer[cs_val_start..cs_soh]) {
                    Ok(s) => s,
                    Err(_) => {
                        return self.fail_resync(
                            msg_start,
                            FixError::MissingChecksumField(
                                "CheckSum value is not valid UTF-8".to_string(),
                            ),
                        );
                    }
                };
            let expected_checksum: u8 = match expected_checksum_str.parse() {
                Ok(n) => n,
                Err(_) => {
                    return self.fail_resync(
                        msg_start,
                        FixError::MissingChecksumField(format!(
                            "CheckSum is not a number: {expected_checksum_str:?}"
                        )),
                    );
                }
            };

            let actual_checksum: u8 = {
                let sum: u32 = self.buffer[msg_start..checksum_region_start]
                    .iter()
                    .map(|b| *b as u32)
                    .sum();
                (sum % 256) as u8
            };

            if expected_checksum != actual_checksum {
                return self.fail_resync(
                    msg_start,
                    FixError::ChecksumMismatch {
                        expected: expected_checksum,
                        actual: actual_checksum,
                    },
                );
            }

            // Step 6: Extract the full message bytes and parse. Note we drop
            // any leading garbage before `msg_start` here too (the bytes
            // before the `8=` we locked onto).
            let msg_bytes: Vec<u8> = self.buffer.drain(..msg_end).collect();
            let msg_bytes = &msg_bytes[msg_start..];

            // Parse all tag-value pairs from the message body. A failure here
            // is a *field-level* parse error (bad tag, bad UTF-8 in a value)
            // on an otherwise well-framed, checksum-valid frame — that frame is
            // already consumed, so we just surface the error; the next call
            // resumes cleanly from whatever follows.
            FixMessage::from_tag_values(msg_bytes).map(Some)
        }

        /// Discard the corrupt frame starting at `bad_start` and resynchronize:
        /// scan forward for the next plausible `8=` (either at buffer start or
        /// immediately after a SOH) and drop everything before it. If no further
        /// `8=` exists, drain the whole buffer. Returns the supplied error.
        ///
        /// This is what makes a single malformed frame *non-fatal*: bytes that
        /// belong to the next, well-formed message survive the drain.
        fn fail_resync(
            &mut self,
            bad_start: usize,
            err: FixError,
        ) -> Result<Option<FixMessage>, FixError> {
            // Look for the next "8=" strictly after the bad frame's start.
            let mut resync_at = None;
            let search_from = bad_start + 1;
            let mut i = search_from;
            while i + 2 <= self.buffer.len() {
                let at_boundary = i == 0 || self.buffer[i - 1] == 0x01;
                if at_boundary && self.buffer[i..].starts_with(b"8=") {
                    resync_at = Some(i);
                    break;
                }
                i += 1;
            }
            match resync_at {
                Some(pos) => {
                    self.buffer.drain(..pos);
                }
                None => {
                    self.buffer.clear();
                }
            }
            Err(err)
        }

        // ── Helper methods ───────────────────────────────────────

        fn find_tag_start(&self, tag: u32) -> Option<usize> {
            let prefix = format!("{}=", tag);
            let prefix_bytes = prefix.as_bytes();

            // At position 0 (start of buffer)
            if self.buffer.starts_with(prefix_bytes) {
                return Some(0);
            }

            // After a SOH
            for i in 0..self.buffer.len().saturating_sub(prefix_bytes.len()) {
                if self.buffer[i] == 0x01 && self.buffer[i + 1..].starts_with(prefix_bytes) {
                    return Some(i + 1);
                }
            }

            None
        }

        fn find_tag_in_range(&self, tag: u32, start: usize) -> Option<usize> {
            let prefix = format!("{}=", tag);
            let prefix_bytes = prefix.as_bytes();

            for i in start..self.buffer.len().saturating_sub(prefix_bytes.len()) {
                if (i == 0 || self.buffer[i - 1] == 0x01)
                    && self.buffer[i..].starts_with(prefix_bytes)
                {
                    return Some(i);
                }
            }

            None
        }

        fn skip_past_equals(&self, pos: usize) -> Option<usize> {
            self.buffer[pos..]
                .iter()
                .position(|&b| b == b'=')
                .map(|p| pos + p + 1)
        }

        fn find_soh_after(&self, pos: usize) -> Option<usize> {
            self.buffer[pos..]
                .iter()
                .position(|&b| b == 0x01)
                .map(|p| pos + p)
        }
    }

    // ─── Tests ───────────────────────────────────────────────────

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Build a valid FIX message from tag-value pairs (correct BodyLength
        /// and CheckSum, courtesy of `encode()`).
        fn build_fix_message(fields: &[(u32, &str)]) -> Vec<u8> {
            let mut m = FixMessage::new(MsgType::Unknown);
            for (tag, val) in fields {
                m.set_field(*tag, val);
            }
            m.encode()
        }

        /// A small, always-valid heartbeat we can append after a corrupt frame
        /// to prove resync recovered it.
        fn good_heartbeat() -> Vec<u8> {
            build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "A"), (56, "B"), (34, "99")])
        }

        /// Hand-build a frame from *exactly* the given body fields (no auto
        /// MsgType injection, unlike `FixMessage::encode()`), with a correct
        /// BodyLength and CheckSum. Lets us construct a wire-valid frame that
        /// genuinely lacks tag 35.
        ///
        /// `begin_string` becomes `8=...`, then `9=<len>`, then the body, then
        /// `10=<sum>`. BodyLength counts the bytes after the `9=N\x01` field up
        /// to and including the SOH before `10=` (i.e. the body part).
        fn build_raw_frame(begin_string: &str, body_fields: &[(u32, &str)]) -> Vec<u8> {
            let mut body = String::new();
            for (tag, val) in body_fields {
                body.push_str(&format!("{tag}={val}\x01"));
            }
            let mut out = String::new();
            out.push_str(&format!("8={begin_string}\x01"));
            out.push_str(&format!("9={}\x01", body.len()));
            out.push_str(&body);
            let sum: u32 = out.as_bytes().iter().map(|b| *b as u32).sum();
            out.push_str(&format!("10={:03}\x01", (sum % 256) as u8));
            out.into_bytes()
        }

        /// Rewrite the value of the first SOH-delimited `9=...` field in an
        /// otherwise-valid encoded frame, *without* recomputing the checksum.
        /// Used to manufacture BodyLength-framing corruption.
        fn corrupt_body_length(frame: &[u8], new_len: &str) -> Vec<u8> {
            let s = std::str::from_utf8(frame).unwrap();
            let nine = s.find("\x019=").map(|i| i + 1).unwrap();
            let val_start = nine + 2; // past "9="
            let soh = s[val_start..].find('\x01').unwrap() + val_start;
            let mut out = Vec::new();
            out.extend_from_slice(&frame[..val_start]);
            out.extend_from_slice(new_len.as_bytes());
            out.extend_from_slice(&frame[soh..]);
            out
        }

        // ── Happy-path / pre-existing behavior (call sites updated for the
        //    new `Result<Option<_>, _>` signature) ──────────────────────────

        #[test]
        fn test_parse_logon_message() {
            let raw = build_fix_message(&[
                (8, "FIX.4.4"),
                (35, "A"),
                (49, "CLIENT"),
                (56, "SERVER"),
                (34, "1"),
                (98, "0"),
                (108, "30"),
            ]);

            let mut parser = FixParser::new();
            parser.push_bytes(&raw);
            let msg = parser
                .next_message()
                .expect("well-formed frame must not error")
                .expect("buffer holds a complete message");

            assert_eq!(msg.msg_type(), MsgType::Logon);
            assert_eq!(msg.get_field(49).map(|s| s.as_str()), Some("CLIENT"));
            assert_eq!(msg.get_field(56).map(|s| s.as_str()), Some("SERVER"));
            assert_eq!(msg.get_field(34).map(|s| s.as_str()), Some("1"));
        }

        #[test]
        fn test_parse_execution_report() {
            let raw = build_fix_message(&[
                (8, "FIX.4.4"),
                (35, "8"),
                (49, "EXCHANGE"),
                (56, "ALGO"),
                (34, "42"),
                (17, "EXEC-001"),
                (150, "F"),
                (39, "2"),
                (55, "AAPL"),
                (54, "1"),
                (32, "100"),
                (31, "175.50"),
            ]);

            let mut parser = FixParser::new();
            parser.push_bytes(&raw);
            let msg = parser
                .next_message()
                .expect("well-formed frame must not error")
                .expect("buffer holds a complete message");

            assert_eq!(msg.msg_type(), MsgType::ExecutionReport);
            assert_eq!(msg.get_field(55).map(|s| s.as_str()), Some("AAPL"));
            assert_eq!(msg.get_field(32).map(|s| s.as_str()), Some("100"));
            assert_eq!(msg.get_field(31).map(|s| s.as_str()), Some("175.50"));
        }

        #[test]
        fn test_multiple_messages() {
            let msg1 =
                build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "A"), (56, "B"), (34, "1")]);
            let msg2 =
                build_fix_message(&[(8, "FIX.4.4"), (35, "5"), (49, "A"), (56, "B"), (34, "2")]);

            let mut parser = FixParser::new();
            parser.push_bytes(&msg1);
            parser.push_bytes(&msg2);

            let parsed1 = parser.next_message().unwrap().expect("first message");
            assert_eq!(parsed1.msg_type(), MsgType::Heartbeat);

            let parsed2 = parser.next_message().unwrap().expect("second message");
            assert_eq!(parsed2.msg_type(), MsgType::Logout);
        }

        #[test]
        fn test_roundtrip_encode_parse() {
            let mut original = FixMessage::new(MsgType::NewOrderSingle);
            original.set_field(8, "FIX.4.4");
            original.set_field(49, "RUSTFORGE");
            original.set_field(56, "NSE");
            original.set_field(55, "RELIANCE");
            original.set_field(54, "1"); // Buy
            original.set_field(38, "500"); // Qty
            original.set_field(44, "2650.50"); // Price

            let encoded = original.encode();

            let mut parser = FixParser::new();
            parser.push_bytes(&encoded);
            let decoded = parser
                .next_message()
                .expect("roundtrip frame must not error")
                .expect("roundtrip frame is complete");

            assert_eq!(decoded.msg_type(), MsgType::NewOrderSingle);
            assert_eq!(decoded.get_field(55).map(|s| s.as_str()), Some("RELIANCE"));
            assert_eq!(decoded.get_field(38).map(|s| s.as_str()), Some("500"));
            assert_eq!(decoded.get_field(44).map(|s| s.as_str()), Some("2650.50"));
        }

        #[test]
        fn test_empty_buffer_returns_none() {
            // WHY: an empty buffer is "wait for more bytes", never an error.
            let mut parser = FixParser::new();
            assert!(matches!(parser.next_message(), Ok(None)));
        }

        // ── Regression: incomplete vs malformed must NOT be conflated ───────

        #[test]
        fn test_incomplete_message_returns_none() {
            // WHY: a truncated TCP read is normal; the parser must report
            // `Ok(None)` and leave the buffer untouched so the rest of the
            // frame can be appended and parsed on a later call. Regressing
            // this back to "drain + None" (or, worse, `Err`) would either lose
            // the message or kill the session over a non-event.
            let raw =
                build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "A"), (56, "B"), (34, "1")]);

            let mut parser = FixParser::new();
            parser.push_bytes(&raw[..raw.len() / 2]);
            assert!(
                matches!(parser.next_message(), Ok(None)),
                "half a frame is incomplete, not malformed"
            );

            // Now complete the frame; the *same* parser must parse it.
            parser.push_bytes(&raw[raw.len() / 2..]);
            let msg = parser
                .next_message()
                .expect("completed frame must not error")
                .expect("completed frame is now whole");
            assert_eq!(msg.msg_type(), MsgType::Heartbeat);
        }

        #[test]
        fn test_body_length_exceeds_buffer_is_incomplete_not_malformed() {
            // WHY: tag 9 saying "200 more bytes" when only 40 are buffered is
            // the textbook *incomplete* case — the body just hasn't arrived.
            // It MUST be `Ok(None)` (preserve the buffer), never `Err`. If we
            // returned `Err` here a peer that simply writes a large message in
            // two TCP segments would have its session torn down.
            let frame =
                build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "A"), (56, "B"), (34, "1")]);
            // Inflate BodyLength far past what's buffered.
            let lying = corrupt_body_length(&frame, "9999");

            let mut parser = FixParser::new();
            parser.push_bytes(&lying);
            assert!(
                matches!(parser.next_message(), Ok(None)),
                "BodyLength > buffered bytes is incomplete"
            );
        }

        #[test]
        fn test_truncated_then_completed_roundtrips() {
            // WHY: same as the incomplete-message test but split at an
            // adversarial spot (one byte before the closing SOH) — proves the
            // buffer is genuinely preserved across calls, not just "close
            // enough".
            let frame =
                build_fix_message(&[(8, "FIX.4.4"), (35, "8"), (49, "EX"), (56, "AL"), (34, "7")]);
            let cut = frame.len() - 1;

            let mut parser = FixParser::new();
            parser.push_bytes(&frame[..cut]);
            assert!(matches!(parser.next_message(), Ok(None)));
            parser.push_bytes(&frame[cut..]);
            let msg = parser.next_message().unwrap().expect("now complete");
            assert_eq!(msg.msg_type(), MsgType::ExecutionReport);
        }

        // ── Malformed framing → Err + resync (recover-open) ────────────────

        #[test]
        fn test_body_length_too_short_errs_and_resyncs() {
            // WHY: this is the stream-desync hazard the PR exists to kill. A
            // garbled BodyLength on frame A used to make the parser drain the
            // whole buffer and return `None`, *silently eating* a perfectly
            // good frame B that arrived in the same TCP read. Now: frame A is
            // an `Err`, the parser resyncs to frame B's `8=`, and the very next
            // call returns frame B. No data loss.
            let mut bad =
                build_fix_message(&[(8, "FIX.4.4"), (35, "5"), (49, "A"), (56, "B"), (34, "1")]);
            bad = corrupt_body_length(&bad, "3"); // far too small → `10=` not at body_end
            let good = good_heartbeat();

            let mut parser = FixParser::new();
            parser.push_bytes(&bad);
            parser.push_bytes(&good);

            let err = parser.next_message().expect_err("frame A is malformed");
            assert!(matches!(err, FixError::BodyLengthFraming(_)));

            let recovered = parser
                .next_message()
                .expect("frame B is well-formed")
                .expect("frame B was recovered after resync");
            assert_eq!(recovered.msg_type(), MsgType::Heartbeat);
            assert_eq!(recovered.get_field(34).map(|s| s.as_str()), Some("99"));
        }

        #[test]
        fn test_body_length_lands_but_no_checksum_there_errs_and_resyncs() {
            // WHY: BodyLength can also over-count and land *inside* a field
            // instead of on `10=`. Same failure class, same recovery contract.
            let mut bad =
                build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "AA"), (56, "BB"), (34, "2")]);
            // Push BodyLength out by a couple of bytes so body_end falls in the
            // middle of the trailing field rather than on `10=`.
            let real_len: usize = {
                let s = std::str::from_utf8(&bad).unwrap();
                let n = s.find("\x019=").map(|i| i + 3).unwrap();
                let soh = s[n..].find('\x01').unwrap() + n;
                s[n..soh].parse().unwrap()
            };
            bad = corrupt_body_length(&bad, &(real_len + 2).to_string());
            let good = good_heartbeat();

            let mut parser = FixParser::new();
            parser.push_bytes(&bad);
            parser.push_bytes(&good);

            let err = parser.next_message().expect_err("no `10=` at declared body_end");
            assert!(matches!(err, FixError::BodyLengthFraming(_)));

            let recovered = parser.next_message().unwrap().expect("resynced to frame B");
            assert_eq!(recovered.msg_type(), MsgType::Heartbeat);
        }

        // ── Checksum failures → Err + resync ───────────────────────────────

        #[test]
        fn test_invalid_checksum_value_errs_and_resyncs() {
            // WHY: a checksum mismatch means line corruption (or a buggy peer).
            // The frame is unusable, but the bytes *after* it might be a clean
            // message — so it's `Err(ChecksumMismatch)` + resync, not a buffer
            // wipe. Pre-fix this drained to msg_end and returned `None`, which
            // happened to keep the next frame, but the caller couldn't tell a
            // corruption event from "nothing to do yet".
            //
            // Construction: take a valid frame and mutate a single body byte by
            // +1 (length unchanged → framing still fine), leaving tag 10 stale.
            // The real sum is now off by exactly 1 (mod 256) → guaranteed
            // mismatch, no flakiness.
            let mut bad =
                build_fix_message(&[(8, "FIX.4.4"), (35, "8"), (49, "X"), (56, "Y"), (34, "3")]);
            let pos = {
                let s = std::str::from_utf8(&bad).unwrap();
                s.find("49=X").map(|i| i + 3).unwrap()
            };
            bad[pos] = b'Y'; // 'X' (0x58) -> 'Y' (0x59): sum shifts by 1
            let good = good_heartbeat();

            let mut parser = FixParser::new();
            parser.push_bytes(&bad);
            parser.push_bytes(&good);

            let err = parser.next_message().expect_err("checksum is wrong");
            assert!(matches!(err, FixError::ChecksumMismatch { .. }));

            let recovered = parser.next_message().unwrap().expect("resynced to frame B");
            assert_eq!(recovered.msg_type(), MsgType::Heartbeat);
        }

        #[test]
        fn test_non_numeric_checksum_errs() {
            // WHY: `10=ABC\x01` is structurally broken — there's no integer to
            // compare against. Must surface as a structured error
            // (`MissingChecksumField`), never a panic on `.parse()`.
            let mut bad =
                build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "P"), (56, "Q"), (34, "4")]);
            let s = std::str::from_utf8(&bad).unwrap();
            let ten = s.rfind("\x0110=").map(|i| i + 4).unwrap();
            bad[ten] = b'A';
            bad[ten + 1] = b'B';
            bad[ten + 2] = b'C';

            let mut parser = FixParser::new();
            parser.push_bytes(&bad);
            let err = parser.next_message().expect_err("non-numeric checksum");
            assert!(matches!(err, FixError::MissingChecksumField(_)));
        }

        // ── from_tag_values: field-level parsing (runs AFTER framing+checksum,
        //    so we exercise it directly) ─────────────────────────────────────

        #[test]
        fn test_from_tag_values_missing_equals() {
            // WHY: a field with no `=` (`FOO\x01`) is unparseable. Structured
            // error, no panic.
            let raw = b"35=0\x01FOO\x0149=A\x01";
            let err = FixMessage::from_tag_values(raw).expect_err("missing '=' in a field");
            assert!(matches!(err, FixError::Parse(_)));
        }

        #[test]
        fn test_from_tag_values_non_numeric_tag() {
            // WHY: tags are integers; `abc=1` can't index anything. Structured
            // error, no panic.
            let raw = b"35=0\x01abc=1\x01";
            let err = FixMessage::from_tag_values(raw).expect_err("non-numeric tag");
            assert!(matches!(err, FixError::Parse(_)));
        }

        #[test]
        fn test_from_tag_values_invalid_utf8() {
            // WHY: FIX is byte-oriented; a value with invalid UTF-8 must not
            // crash the parser. (Tightening this to byte-slice fields instead
            // of `str` is a possible future change, but today it must at least
            // be a graceful error.)
            let raw = b"35=0\x0158=\xFF\xFE\x01";
            let err = FixMessage::from_tag_values(raw).expect_err("invalid UTF-8 body");
            assert!(matches!(err, FixError::Parse(_)));
        }

        // ── Semantic vs wire validation ────────────────────────────────────

        #[test]
        fn test_missing_msgtype_field_parses_as_unknown() {
            // WHY: a frame with valid framing + checksum but no tag 35 is
            // *well-formed on the wire* — the parser should accept it and
            // report `MsgType::Unknown`. Rejecting it here would conflate two
            // layers: wire parsing (this crate) vs. session/business rules
            // ("this MsgType is required"), which belong upstream in
            // `session.rs` / the OMS. This test pins the current behavior so a
            // future change to it is deliberate.
            let frame = build_raw_frame("FIX.4.4", &[(49, "A"), (56, "B"), (34, "1")]);
            let mut parser = FixParser::new();
            parser.push_bytes(&frame);
            let msg = parser
                .next_message()
                .expect("frame is well-formed on the wire")
                .expect("frame is complete");
            assert_eq!(
                msg.msg_type(),
                MsgType::Unknown,
                "no tag 35 => Unknown; semantic 'MsgType required' is a separate layer"
            );
            assert!(
                msg.get_field(35).is_none(),
                "this frame genuinely carries no MsgType field"
            );
        }

        // ── Flagged limitations (documented, not fixed in this PR) ──────────

        #[test]
        fn test_soh_inside_length_delimited_data_field_is_lossy() {
            // LIMITATION: FIX length-delimited data fields — tag 95 RawDataLength
            // immediately followed by tag 96 RawData — may legally contain raw
            // `\x01` bytes inside the RawData value. This parser splits the body
            // *naively* on `\x01` and does not honor the length prefix, so an
            // embedded SOH is misread as a field separator and the data field is
            // truncated/garbled. This test documents the current (lossy) behavior;
            // honoring length-delimited fields (95→96, and the other
            // Length/Data pairs) is follow-up work, intentionally out of scope
            // for this PR.
            //
            // Frame: 95=4\x0196=A\x01BC\x01  — RawDataLength says 4 bytes
            // ("A\x01BC"), but the naive splitter sees `96=A`, then a stray
            // `BC` chunk with no `=`.
            let raw = b"35=0\x0149=S\x0156=T\x0134=1\x0195=4\x0196=A\x01BC\x01";
            let result = FixMessage::from_tag_values(raw);
            // Today: the `BC\x01` fragment has no `=` → parse error. We assert
            // it so a future fix that *correctly* handles length-delimited data
            // (and thus makes this `Ok`) trips this test on purpose.
            assert!(
                result.is_err(),
                "naive SOH-splitting currently mishandles length-delimited data fields"
            );
        }

        #[test]
        fn test_checksum_collision_is_accepted() {
            // LIMITATION: FIX's CheckSum (tag 10) is the sum of all preceding
            // bytes mod 256 — a *transmission-integrity* check, not tamper
            // detection. Two different bodies can share the same mod-256 sum, so
            // a payload mutated in a sum-preserving way (e.g. swap one byte +1
            // and another -1) sails through. The parser accepts it; surfacing
            // (not "fixing") this is the point. Real tamper resistance would
            // need a MAC/signature layer above FIX, which is out of scope.
            let original =
                build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "AB"), (56, "CD"), (34, "1")]);
            // Find the two-byte sender comp id "AB" inside the body and turn it
            // into "BA" (same byte multiset → same sum → checksum still valid).
            let s = std::str::from_utf8(&original).unwrap();
            let pos = s.find("49=AB").map(|i| i + 3).unwrap();
            let mut mutated = original.clone();
            mutated[pos] = b'B';
            mutated[pos + 1] = b'A';
            assert_ne!(original, mutated, "we actually changed the bytes");

            let mut parser = FixParser::new();
            parser.push_bytes(&mutated);
            let msg = parser
                .next_message()
                .expect("mod-256 sum still matches, so the parser accepts it")
                .expect("frame is complete");
            // The mutation went through undetected — that's the documented gap.
            assert_eq!(msg.get_field(49).map(|s| s.as_str()), Some("BA"));
        }
    }
}
