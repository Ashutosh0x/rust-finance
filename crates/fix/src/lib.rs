// crates/fix/src/lib.rs
//
// Root module for the FIX Engine layer.
pub mod session;

#[derive(Debug, thiserror::Error)]
pub enum FixError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error: {0}")]
    Parse(String),
}

// Stubs for parser interfaces since we only built the session layer in this phase
pub mod serializer {
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
        Unknown,
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
        pub fn msg_type(&self) -> MsgType {
            self.msg_type.clone()
        }
        pub fn set_field(&mut self, tag: u32, val: &str) {
            self.fields.insert(tag, val.to_string());
        }
        pub fn get_field(&self, tag: u32) -> Option<&String> {
            self.fields.get(&tag)
        }
        pub fn encode(&self) -> Vec<u8> {
            // Build body fields: all fields except the FIX framing tags 8, 9, 10.
            let mut body_fields: Vec<(u32, String)> = self
                .fields
                .iter()
                .filter(|(tag, _)| **tag != 8 && **tag != 9 && **tag != 10)
                .map(|(k, v)| (*k, v.clone()))
                .collect();

            // Synthesise tag 35 (MsgType) if not already present.
            if !body_fields.iter().any(|(tag, _)| *tag == 35) {
                let msg_type_str = match self.msg_type {
                    MsgType::Logon => "A",
                    MsgType::Logout => "5",
                    MsgType::Heartbeat => "0",
                    MsgType::TestRequest => "1",
                    MsgType::ResendRequest => "2",
                    MsgType::SequenceReset => "4",
                    MsgType::ExecutionReport => "8",
                    MsgType::OrderCancelReject => "9",
                    MsgType::Unknown => "?",
                };
                body_fields.push((35, msg_type_str.to_string()));
            }

            // FIX header ordering: tag 35 first in the body section, then all
            // remaining body fields sorted numerically.
            body_fields.sort_by_key(|(tag, _)| if *tag == 35 { 0 } else { *tag });

            // Serialise body to bytes using b'\x01' (SOH) as the field delimiter.
            let mut body: Vec<u8> = Vec::new();
            for (tag, val) in &body_fields {
                body.extend_from_slice(tag.to_string().as_bytes());
                body.push(b'=');
                body.extend_from_slice(val.as_bytes());
                body.push(b'\x01');
            }

            // BodyLength (tag 9) = number of bytes after "9=NNN\x01" up to (but not
            // including) the "10=" trailer -- i.e. the body byte count.
            let body_len = body.len();

            // Complete message: 8=FIX.4.4\x01 9=<len>\x01 <body> 10=<chk>\x01
            let mut out: Vec<u8> = Vec::new();
            out.extend_from_slice(b"8=FIX.4.4\x01");
            out.extend_from_slice(format!("9={}\x01", body_len).as_bytes());
            out.extend_from_slice(&body);

            // CheckSum (tag 10) = sum of all preceding bytes mod 256, zero-padded
            // to 3 decimal digits.
            let checksum: u32 = out.iter().map(|&b| b as u32).sum::<u32>() % 256;
            out.extend_from_slice(format!("10={:03}\x01", checksum).as_bytes());

            out
        }
    }

    pub struct FixParser;
    impl FixParser {
        pub fn new() -> Self {
            Self
        }
        pub fn push_bytes(&mut self, _bytes: &[u8]) {}
        pub fn next_message(&mut self) -> Option<FixMessage> {
            None
        }
    }
}
