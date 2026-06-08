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
            let mut fields: Vec<(u32, String)> = self.fields.iter().map(|(k, v)| (*k, v.clone())).collect();
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
            fields.sort_by_key(|(tag, _)| *tag);
            let mut out = String::new();
            for (tag, val) in fields {
                out.push_str(&format!("{}={}", tag, val));
            }
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
