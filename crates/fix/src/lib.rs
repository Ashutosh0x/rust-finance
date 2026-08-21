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
    /// The longest BodyLength this parser will accept.
    ///
    /// FIX application messages are small; the largest realistic frames are
    /// mass-quote and security-list responses, and 1 MiB is far above any of
    /// them. The cap is not about tuning — the value comes off the wire and is
    /// used both in pointer arithmetic and in a "wait for this many bytes"
    /// decision, so without a bound an attacker controls a usize overflow and
    /// the buffer's growth at the same time.
    const MAX_BODY_LENGTH: usize = 1024 * 1024;

    /// Bytes after "8=" within which BodyLength must appear.
    const MAX_HEADER_SCAN: usize = 64;

    /// "10=XXX\x01"
    const CHECKSUM_FIELD_LEN: usize = 7;

    /// Shortest thing that could be a message: "8=FIX.X.X\x019=N\x0135=X\x0110=XXX\x01"
    const MIN_FRAME_LEN: usize = 20;

    /// Longest partial "8=FIX.4.4\x01" that could be split across two reads.
    const MAX_TAG8_PREFIX: usize = 16;

    /// What one parse attempt concluded.
    ///
    /// Three outcomes rather than `Option`, because "wait for more bytes" and
    /// "these bytes are not a message" need opposite handling and were
    /// previously both `None` — which is precisely how a malformed frame came
    /// to stall a session permanently.
    enum Frame {
        Message(FixMessage),
        /// Incomplete but plausible. Leave the buffer alone.
        NeedMore,
        /// Undecodable. Bytes have already been removed; try again.
        Discard,
    }

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

        /// Bytes still buffered, waiting to form a message.
        ///
        /// Exposed because "did this malformed frame get dropped" is not
        /// answerable from `next_message`'s return value — both a stall and a
        /// legitimate wait-for-more-bytes return `None`. A read loop can also
        /// use this to notice a peer that is filling the buffer without ever
        /// completing a message.
        pub fn buffer_len(&self) -> usize {
            self.buffer.len()
        }

        /// Extract the next complete FIX message from the buffer.
        ///
        /// Loops internally: a malformed frame is discarded and parsing
        /// continues, so `None` means only "not enough bytes yet".
        ///
        /// That distinction is the whole contract. Callers drain with
        /// `while let Some(msg) = parser.next_message()`, so a `None` that
        /// leaves undecodable bytes in place stops the loop forever — every
        /// later read appends bytes that are never looked at again. One
        /// malformed frame from the counterparty would silently kill the
        /// session. Discarding here rather than returning is what keeps that
        /// from being remotely triggerable.
        pub fn next_message(&mut self) -> Option<FixMessage> {
            loop {
                match self.try_next_frame() {
                    Frame::Message(msg) => return Some(msg),
                    Frame::NeedMore => return None,
                    // `resync` and `discard_through` always remove at least
                    // one byte, so this terminates.
                    Frame::Discard => continue,
                }
            }
        }

        /// One attempt at the front of the buffer.
        ///
        /// Algorithm:
        ///   1. Find "8=" prefix (BeginString start)
        ///   2. Find "9=<BodyLength>" field
        ///   3. Read exactly BodyLength bytes after the 9= field's SOH
        ///   4. Expect "10=<checksum>" immediately after
        ///   5. Validate checksum
        ///   6. Parse all tag=value pairs
        fn try_next_frame(&mut self) -> Frame {
            if self.buffer.len() < MIN_FRAME_LEN {
                return Frame::NeedMore;
            }

            // Step 1: Find start of message (tag 8)
            let Some(msg_start) = self.find_tag_start(8) else {
                // No BeginString anywhere. Everything buffered is noise, but
                // the tail may be a "8=FIX..." split across two reads, so keep
                // back enough bytes for the longest prefix we could be part
                // way through.
                let keep = MAX_TAG8_PREFIX.min(self.buffer.len());
                let drop_to = self.buffer.len() - keep;
                if drop_to > 0 {
                    self.buffer.drain(..drop_to);
                }
                return Frame::NeedMore;
            };

            // Anything before the BeginString is not part of a message.
            if msg_start > 0 {
                self.buffer.drain(..msg_start);
                return Frame::Discard;
            }

            // Step 2: Find BodyLength (tag 9)
            let Some(tag9_start) = self.find_tag_in_range(9, 0) else {
                return self.need_more_or_resync();
            };
            let Some(tag9_val_start) = self.skip_past_equals(tag9_start) else {
                return self.need_more_or_resync();
            };
            let Some(tag9_soh) = self.find_soh_after(tag9_val_start) else {
                return self.need_more_or_resync();
            };

            // A BodyLength that is not a number cannot be recovered by
            // waiting for more bytes.
            let Ok(body_len_str) = std::str::from_utf8(&self.buffer[tag9_val_start..tag9_soh])
            else {
                return self.resync();
            };
            let Ok(body_length) = body_len_str.parse::<usize>() else {
                return self.resync();
            };

            // Bounded BEFORE it is used in arithmetic. An unbounded value off
            // the wire is two separate faults: `body_start + body_length`
            // overflows usize — a remote panic in a debug build, a silent wrap
            // in release — and any value larger than a real message makes the
            // parser wait for bytes that will never arrive while the buffer
            // grows without limit.
            if body_length > MAX_BODY_LENGTH {
                return self.resync();
            }

            // Step 3: Body starts after "9=N\x01"
            let body_start = tag9_soh + 1;
            let Some(body_end) = body_start.checked_add(body_length) else {
                return self.resync();
            };
            let Some(needed) = body_end.checked_add(CHECKSUM_FIELD_LEN) else {
                return self.resync();
            };
            if self.buffer.len() < needed {
                return Frame::NeedMore; // incomplete message, wait for more bytes
            }

            // Step 4: Expect "10=" exactly at body_end
            let checksum_region_start = body_end;
            if self
                .buffer
                .get(checksum_region_start..checksum_region_start + 3)
                != Some(b"10=")
            {
                // BodyLength did not point at the checksum field, so the frame
                // is malformed however plausible it looked.
                return self.resync();
            }

            let cs_val_start = checksum_region_start + 3;
            let Some(cs_soh) = self.find_soh_after(cs_val_start) else {
                return Frame::NeedMore;
            };
            let msg_end = cs_soh + 1;

            // Step 5: Validate checksum
            let Ok(expected_checksum_str) = std::str::from_utf8(&self.buffer[cs_val_start..cs_soh])
            else {
                return self.discard_through(msg_end);
            };
            // A checksum that is not a number, or does not fit in a byte,
            // can never match. Discarding the frame is the only way forward;
            // returning early here is what stalled the session.
            let Ok(expected_checksum) = expected_checksum_str.parse::<u8>() else {
                return self.discard_through(msg_end);
            };

            let actual_checksum: u8 = {
                let sum: u32 = self.buffer[..checksum_region_start]
                    .iter()
                    .map(|b| *b as u32)
                    .sum();
                (sum % 256) as u8
            };

            if expected_checksum != actual_checksum {
                return self.discard_through(msg_end);
            }

            // Step 6: Extract the full message bytes and parse
            let msg_bytes: Vec<u8> = self.buffer.drain(..msg_end).collect();

            match FixMessage::from_tag_values(&msg_bytes) {
                Ok(msg) => Frame::Message(msg),
                // The bytes are already gone, so this cannot loop.
                Err(_) => Frame::Discard,
            }
        }

        /// Wait for more header bytes, unless we have already seen more than a
        /// FIX header could possibly be.
        ///
        /// BeginString is followed immediately by BodyLength in every FIX
        /// version. Scanning further would mean treating arbitrary data that
        /// happens to contain "8=" as a message header and waiting on it
        /// forever.
        fn need_more_or_resync(&mut self) -> Frame {
            if self.buffer.len() > MAX_HEADER_SCAN {
                self.resync()
            } else {
                Frame::NeedMore
            }
        }

        /// Drop one byte so the next "8=" can be found.
        ///
        /// Always removes at least one byte, which is what makes the loop in
        /// [`FixParser::next_message`] terminate.
        fn resync(&mut self) -> Frame {
            if self.buffer.is_empty() {
                return Frame::NeedMore;
            }
            self.buffer.drain(..1);
            Frame::Discard
        }

        /// Drop a whole framed message that turned out to be undecodable.
        fn discard_through(&mut self, end: usize) -> Frame {
            let end = end.clamp(1, self.buffer.len());
            self.buffer.drain(..end);
            Frame::Discard
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

        /// Build a valid FIX message from tag-value pairs.
        fn build_fix_message(fields: &[(u32, &str)]) -> Vec<u8> {
            let mut m = FixMessage::new(MsgType::Unknown);
            for (tag, val) in fields {
                m.set_field(*tag, val);
            }
            // Use encode() which correctly computes BodyLength and Checksum
            m.encode()
        }

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
            let msg = parser.next_message().expect("Should parse Logon");

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
            let msg = parser.next_message().expect("Should parse ExecReport");

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

            let parsed1 = parser.next_message().expect("Should get first message");
            assert_eq!(parsed1.msg_type(), MsgType::Heartbeat);

            let parsed2 = parser.next_message().expect("Should get second message");
            assert_eq!(parsed2.msg_type(), MsgType::Logout);
        }

        #[test]
        fn test_incomplete_message_returns_none() {
            let raw =
                build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "A"), (56, "B"), (34, "1")]);

            let mut parser = FixParser::new();
            // Push only half the bytes
            parser.push_bytes(&raw[..raw.len() / 2]);
            assert!(
                parser.next_message().is_none(),
                "Incomplete should return None"
            );

            // Push the rest
            parser.push_bytes(&raw[raw.len() / 2..]);
            assert!(parser.next_message().is_some(), "Complete should parse");
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
            let decoded = parser.next_message().expect("Roundtrip should work");

            assert_eq!(decoded.msg_type(), MsgType::NewOrderSingle);
            assert_eq!(decoded.get_field(55).map(|s| s.as_str()), Some("RELIANCE"));
            assert_eq!(decoded.get_field(38).map(|s| s.as_str()), Some("500"));
            assert_eq!(decoded.get_field(44).map(|s| s.as_str()), Some("2650.50"));
        }

        #[test]
        fn test_empty_buffer_returns_none() {
            let mut parser = FixParser::new();
            assert!(parser.next_message().is_none());
        }

        // ─── Malformed input (issue #43) ──────────────────────────
        //
        // A FIX session reads from a socket. Every one of these inputs can
        // arrive from the counterparty, so "the parser returns None" is not
        // sufficient — the bytes must also LEAVE the buffer. A None that
        // leaves the input in place makes the next call re-parse the same
        // bytes, which stalls the session forever and grows the buffer
        // without bound as more data arrives.
        //
        // `buffer_len()` is what these assert on, because the stall is
        // invisible from the return value alone.

        /// Feed one message after garbage and require the good one to arrive.
        ///
        /// This is the property that matters: a malformed message must not
        /// prevent the session from recovering.
        fn assert_recovers_after(garbage: &[u8]) {
            let good = build_fix_message(&[(8, "FIX.4.4"), (35, "0"), (49, "A"), (56, "B")]);

            let mut parser = FixParser::new();
            parser.push_bytes(garbage);
            parser.push_bytes(&good);

            // Drive the parser the way a read loop does: call until it either
            // yields a message or stops making progress.
            let mut last_len = parser.buffer_len();
            for _ in 0..64 {
                if let Some(msg) = parser.next_message() {
                    assert_eq!(msg.msg_type(), MsgType::Heartbeat);
                    return;
                }
                let now = parser.buffer_len();
                assert!(
                    now < last_len,
                    "parser stalled: buffer stuck at {now} bytes and no message produced"
                );
                last_len = now;
            }
            panic!("parser never recovered after malformed input");
        }

        #[test]
        fn non_numeric_body_length_does_not_stall() {
            // "9=abc" fails to parse as a number.
            assert_recovers_after(b"8=FIX.4.49=abc35=010=000");
        }

        #[test]
        fn oversized_body_length_does_not_stall() {
            // A BodyLength far larger than anything that will ever arrive.
            // The parser must not wait for it forever.
            assert_recovers_after(b"8=FIX.4.49=99999999935=010=000");
        }

        #[test]
        fn body_length_near_usize_max_does_not_overflow() {
            // body_start + body_length must not wrap. In a debug build an
            // overflow panics, which in a network parser is a remote crash.
            let huge = usize::MAX.to_string();
            let raw = format!("8=FIX.4.49={huge}35=010=000");
            assert_recovers_after(raw.as_bytes());
        }

        #[test]
        fn checksum_value_out_of_range_does_not_stall() {
            // "10=999" does not fit in a u8. The parse fails.
            let body = "35=049=A56=B";
            let raw = format!("8=FIX.4.49={}{body}10=999", body.len());
            assert_recovers_after(raw.as_bytes());
        }

        #[test]
        fn non_numeric_checksum_does_not_stall() {
            let body = "35=049=A56=B";
            let raw = format!("8=FIX.4.49={}{body}10=xyz", body.len());
            assert_recovers_after(raw.as_bytes());
        }

        #[test]
        fn wrong_checksum_is_rejected_and_recovers() {
            let body = "35=049=A56=B";
            // 000 is almost certainly not the real checksum.
            let raw = format!("8=FIX.4.49={}{body}10=000", body.len());
            assert_recovers_after(raw.as_bytes());
        }

        #[test]
        fn body_length_pointing_into_the_middle_of_a_field_recovers() {
            // BodyLength too short, so "10=" is not where it should be.
            let body = "35=049=A56=B";
            let raw = format!("8=FIX.4.49=4{body}10=000");
            assert_recovers_after(raw.as_bytes());
        }

        #[test]
        fn buffer_does_not_grow_without_bound_on_repeated_garbage() {
            // The DoS shape: a peer streaming malformed frames must not make
            // the parser accumulate them forever.
            let mut parser = FixParser::new();
            let junk = b"8=FIX.4.49=99999999935=010=000";

            for _ in 0..200 {
                parser.push_bytes(junk);
                while parser.next_message().is_some() {}
            }

            assert!(
                parser.buffer_len() < 64 * 1024,
                "buffer grew to {} bytes on malformed input",
                parser.buffer_len()
            );
        }
    }
}
