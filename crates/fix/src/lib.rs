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
    pub enum MsgType { Logon, Logout, Heartbeat, TestRequest, ResendRequest, SequenceReset, ExecutionReport, OrderCancelReject, Unknown }
    
    pub struct FixMessage {
        msg_type: MsgType,
        fields: std::collections::HashMap<u32, String>
    }
    
    impl FixMessage {
        pub fn new(msg_type: MsgType) -> Self { Self { msg_type, fields: std::collections::HashMap::new() } }
        pub fn msg_type(&self) -> MsgType { self.msg_type.clone() }
        pub fn set_field(&mut self, tag: u32, val: &str) { self.fields.insert(tag, val.to_string()); }
        pub fn get_field(&self, tag: u32) -> Option<&String> { self.fields.get(&tag) }
        pub fn encode(&self) -> Vec<u8> {
            // Collect fields from the internal map.
            let mut fields: Vec<(u32, String)> = self
                .fields
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect();

            // Ensure MsgType (35) is present; derive it from self.msg_type if missing.
            if !fields.iter().any(|(tag, _)| *tag == 35) {
                let msg_type = match self.msg_type {
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
                fields.push((35, msg_type.to_string()));
            }

            // Extract BeginString (8) if present; it must precede BodyLength (9).
            let mut begin_string: Option<String> = None;
            let mut msg_type_val: Option<String> = None;
            let mut other_fields: Vec<(u32, String)> = Vec::new();

            for (tag, val) in fields.into_iter() {
                match tag {
                    8 => begin_string = Some(val),
                    35 => msg_type_val = Some(val),
                    // Ignore any user-specified BodyLength (9) or CheckSum (10);
                    // they will be recomputed according to FIX framing rules.
                    9 | 10 => { /* skip */ }
                    _ => other_fields.push((tag, val)),
                }
            }

            // Sort remaining fields (excluding 8, 9, 10, 35) by tag for determinism.
            other_fields.sort_by_key(|(tag, _)| *tag);

            // Build the body portion starting with MsgType (35), followed by the rest.
            let mut body_part = String::new();
            if let Some(mt) = msg_type_val {
                body_part.push_str(&format!("35={}", mt));
            }
            for (tag, val) in other_fields {
                body_part.push_str(&format!("{}={}", tag, val));
            }

            // BodyLength (9) is the length in bytes of the message after 9=...<SOH>,
            // i.e., the length of body_part.
            let body_length = body_part.as_bytes().len();

            // Construct the full message: optional 8=, then 9=BodyLength, then body_part.
            let mut out = String::new();
            if let Some(begin) = begin_string {
                out.push_str(&format!("8={}", begin));
            }
            out.push_str(&format!("9={}", body_length));
            out.push_str(&body_part);

            // Compute CheckSum (10): sum of all bytes modulo 256, formatted as 3 digits.
            let sum: u32 = out.as_bytes().iter().map(|b| *b as u32).sum();
            let checksum = (sum % 256) as u8;
            out.push_str(&format!("10={:03}", checksum));

            out.into_bytes()
        }
    }
    
    pub struct FixParser;
    impl FixParser {
        pub fn new() -> Self { Self }
        pub fn push_bytes(&mut self, _bytes: &[u8]) {}
        pub fn next_message(&mut self) -> Option<FixMessage> { None }
    }
}
