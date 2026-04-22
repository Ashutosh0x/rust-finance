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
        /// Algorithm:
        ///   1. Find "8=" prefix (BeginString start)
        ///   2. Find "9=<BodyLength>" field
        ///   3. Read exactly BodyLength bytes after the 9= field's SOH
        ///   4. Expect "10=<checksum>" immediately after
        ///   5. Validate checksum
        ///   6. Parse all tag=value pairs
        pub fn next_message(&mut self) -> Option<FixMessage> {
            // We need at minimum "8=FIX.X.X\x019=N\x0135=X\x0110=XXX\x01"
            if self.buffer.len() < 20 {
                return None;
            }

            // Step 1: Find start of message (tag 8)
            let msg_start = self.find_tag_start(8)?;

            // Step 2: Find BodyLength (tag 9)
            let tag9_start = self.find_tag_in_range(9, msg_start)?;
            let tag9_val_start = self.skip_past_equals(tag9_start)?;
            let tag9_soh = self.find_soh_after(tag9_val_start)?;

            let body_len_str = std::str::from_utf8(&self.buffer[tag9_val_start..tag9_soh]).ok()?;
            let body_length: usize = body_len_str.parse().ok()?;

            // Step 3: Body starts after "9=N\x01"
            let body_start = tag9_soh + 1;
            let body_end = body_start + body_length;

            // Do we have enough bytes? body + "10=XXX\x01" (minimum 8 bytes)
            if self.buffer.len() < body_end + 7 {
                return None; // incomplete message, wait for more bytes
            }

            // Step 4: Find checksum field "10=" after body
            let checksum_region_start = body_end;
            if checksum_region_start + 7 > self.buffer.len() {
                return None;
            }

            // Expect "10=" at body_end
            if self
                .buffer
                .get(checksum_region_start..checksum_region_start + 3)
                != Some(b"10=")
            {
                // Malformed — skip this message start and try again
                self.buffer.drain(..=msg_start);
                return None;
            }

            let cs_val_start = checksum_region_start + 3;
            let cs_soh = self.find_soh_after(cs_val_start)?;
            let msg_end = cs_soh + 1;

            // Step 5: Validate checksum
            let expected_checksum_str =
                std::str::from_utf8(&self.buffer[cs_val_start..cs_soh]).ok()?;
            let expected_checksum: u8 = expected_checksum_str.parse().ok()?;

            let actual_checksum: u8 = {
                let sum: u32 = self.buffer[msg_start..checksum_region_start]
                    .iter()
                    .map(|b| *b as u32)
                    .sum();
                (sum % 256) as u8
            };

            if expected_checksum != actual_checksum {
                // Malformed checksum - drain up to msg_end to continue
                self.buffer.drain(..msg_end);
                return None;
            }

            // Step 6: Extract the full message bytes and parse
            let msg_bytes: Vec<u8> = self.buffer.drain(..msg_end).collect();

            // Parse all tag-value pairs from the entire message
            FixMessage::from_tag_values(&msg_bytes).ok()
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
    }
}
