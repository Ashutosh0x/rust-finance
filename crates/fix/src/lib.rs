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
        pub fn encode(&self) -> Vec<u8> { vec![] }
    }
    
    pub struct FixParser;
    impl FixParser {
        pub fn new() -> Self { Self }
        pub fn push_bytes(&mut self, _bytes: &[u8]) {}
        pub fn next_message(&mut self) -> Option<FixMessage> { None }
    }
}
