// crates/ingestion/src/fix/mod.rs
// Financial Information eXchange (FIX) 4.4 Protocol Handler
// Used for direct market access (DMA) to institutional venues

use std::collections::HashMap;
use std::convert::TryFrom;
use tracing::{error, info, debug, warn};

#[derive(Debug, Clone, PartialEq)]
pub enum FixMsgType {
    Heartbeat,           // 0
    TestRequest,         // 1
    ResendRequest,       // 2
    Reject,              // 3
    SequenceReset,       // 4
    Logout,              // 5
    ExecutionReport,     // 8
    OrderCancelReject,   // 9
    Logon,               // A
    NewOrderSingle,      // D
    OrderCancelRequest,  // F
    OrderCancelReplace,  // G
    OrderStatusRequest,  // H
    MarketDataRequest,   // V
    MarketDataSnapshot,  // W
    MarketDataIncremental,// X
}

impl TryFrom<&str> for FixMsgType {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "0" => Ok(FixMsgType::Heartbeat),
            "1" => Ok(FixMsgType::TestRequest),
            "2" => Ok(FixMsgType::ResendRequest),
            "3" => Ok(FixMsgType::Reject),
            "4" => Ok(FixMsgType::SequenceReset),
            "5" => Ok(FixMsgType::Logout),
            "8" => Ok(FixMsgType::ExecutionReport),
            "9" => Ok(FixMsgType::OrderCancelReject),
            "A" => Ok(FixMsgType::Logon),
            "D" => Ok(FixMsgType::NewOrderSingle),
            "F" => Ok(FixMsgType::OrderCancelRequest),
            "G" => Ok(FixMsgType::OrderCancelReplace),
            "H" => Ok(FixMsgType::OrderStatusRequest),
            "V" => Ok(FixMsgType::MarketDataRequest),
            "W" => Ok(FixMsgType::MarketDataSnapshot),
            "X" => Ok(FixMsgType::MarketDataIncremental),
            _ => Err(format!("Unknown MsgType: {}", s)),
        }
    }
}

impl FixMsgType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FixMsgType::Heartbeat => "0",
            FixMsgType::TestRequest => "1",
            FixMsgType::ResendRequest => "2",
            FixMsgType::Reject => "3",
            FixMsgType::SequenceReset => "4",
            FixMsgType::Logout => "5",
            FixMsgType::ExecutionReport => "8",
            FixMsgType::OrderCancelReject => "9",
            FixMsgType::Logon => "A",
            FixMsgType::NewOrderSingle => "D",
            FixMsgType::OrderCancelRequest => "F",
            FixMsgType::OrderCancelReplace => "G",
            FixMsgType::OrderStatusRequest => "H",
            FixMsgType::MarketDataRequest => "V",
            FixMsgType::MarketDataSnapshot => "W",
            FixMsgType::MarketDataIncremental => "X",
        }
    }
}

/// A parsed FIX message representation
#[derive(Debug, Clone)]
pub struct FixMessage {
    pub msg_type: FixMsgType,
    pub seq_num: u64,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub sending_time: String,
    pub fields: HashMap<u32, String>,
}

impl FixMessage {
    /// Parse a raw FIX byte stream (separated by SOH character \x01)
    pub fn parse(raw: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(raw).map_err(|e| e.to_string())?;
        let kvs: Vec<&str> = text.split('\x01').filter(|s| !s.is_empty()).collect();

        let mut fields = HashMap::new();
        for kv in &kvs {
            let mut parts = kv.splitn(2, '=');
            if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                if let Ok(tag) = k.parse::<u32>() {
                    fields.insert(tag, v.to_string());
                }
            }
        }

        // Validate basic FIX header
        if fields.get(&8).map(|v| v.as_str()) != Some("FIX.4.4") {
            return Err("Not FIX.4.4".into());
        }

        let msg_type_str = fields.get(&35).ok_or("Missing MsgType (35)")?;
        let msg_type = FixMsgType::try_from(msg_type_str.as_str())?;

        let seq_num = fields.get(&34).ok_or("Missing MsgSeqNum (34)")?
            .parse::<u64>().map_err(|_| "Invalid MsgSeqNum")?;

        let sender_comp_id = fields.get(&49).ok_or("Missing SenderCompID (49)")?.clone();
        let target_comp_id = fields.get(&56).ok_or("Missing TargetCompID (56)")?.clone();
        let sending_time = fields.get(&52).ok_or("Missing SendingTime (52)")?.clone();

        Ok(Self {
            msg_type,
            seq_num,
            sender_comp_id,
            target_comp_id,
            sending_time,
            fields,
        })
    }

    /// Serialize to raw FIX format with recalculation of BodyLength(9) and CheckSum(10)
    pub fn serialize(&mut self) -> Vec<u8> {
        self.fields.insert(8, "FIX.4.4".to_string());
        self.fields.insert(35, self.msg_type.as_str().to_string());
        self.fields.insert(34, self.seq_num.to_string());
        self.fields.insert(49, self.sender_comp_id.clone());
        self.fields.insert(56, self.target_comp_id.clone());
        self.fields.insert(52, self.sending_time.clone());

        // Remove length and checksum if they exist so we can compute them cleanly
        self.fields.remove(&9);
        self.fields.remove(&10);

        // Required ordering: 8, 9, 35 ... others ... 10
        let mut body = String::new();
        
        // Put MsgType (35) first in body so length calculation is correct
        body.push_str(&format!("35={}\x01", self.fields.get(&35).unwrap()));
        
        // Add all other fields (skip 8, 9, 10, 35)
        for (tag, value) in &self.fields {
            if *tag != 8 && *tag != 9 && *tag != 10 && *tag != 35 {
                body.push_str(&format!("{}={}\x01", tag, value));
            }
        }

        // Calculate BodyLength
        let body_length = body.len();
        
        // Construct everything up to checksum
        let header = format!("8=FIX.4.4\x019={}\x01", body_length);
        let mut full_msg = header;
        full_msg.push_str(&body);

        // Calculate CheckSum
        let sum: u32 = full_msg.bytes().map(|b| b as u32).sum();
        let checksum = sum % 256;
        full_msg.push_str(&format!("10={:03}\x01", checksum));

        full_msg.into_bytes()
    }

    // Helper to extract ExecutionReport fields safely
    pub fn exec_report_details(&self) -> Option<(String, String, f64, f64)> {
        if self.msg_type != FixMsgType::ExecutionReport { return None; }
        
        // 37=OrderID, 39=OrdStatus, 14=CumQty, 6=AvgPx
        let order_id = self.fields.get(&37)?.clone();
        let status = self.fields.get(&39)?.clone();
        let cum_qty = self.fields.get(&14)?.parse::<f64>().ok()?;
        let avg_px = self.fields.get(&6)?.parse::<f64>().ok()?;
        
        Some((order_id, status, cum_qty, avg_px))
    }
}
