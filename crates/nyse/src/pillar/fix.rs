//! NYSE Pillar FIX 4.2 gateway profile.
//!
//! The Pillar FIX Gateway is the standards-based alternative to the binary gateway: slower
//! on the wire but far cheaper to certify, and the only option for firms whose OMS already
//! speaks FIX. It accepts a fixed set of FIX 4.2 application messages plus a large block of
//! NYSE-specific tags in the 5000–30000 range.
//!
//! This module builds and reads those messages. Two deliberate choices:
//!
//! * **Field order is explicit.** FIX requires header fields in a defined order and NYSE's
//!   spec lists body fields in tag order; an encoder that sorts a hash map produces output
//!   that happens to work with lenient engines and fails with strict ones. Every message
//!   here is emitted as an ordered list.
//! * **`BodyLength` and `CheckSum` are computed, never trusted.** `BodyLength` counts the
//!   bytes after `9=…<SOH>` up to and including the `<SOH>` before `10=`; `CheckSum` is the
//!   sum of every byte before `10=`, modulo 256, always three digits.

use std::fmt::Write as _;

use exchange_core::Price;

/// Field separator (`SOH`).
pub const SOH: u8 = 0x01;

/// Tags used by this profile. Standard FIX 4.2 tags plus the NYSE extensions the Pillar
/// gateway defines.
pub mod tag {
    // Standard header
    pub const BEGIN_STRING: u32 = 8;
    pub const BODY_LENGTH: u32 = 9;
    pub const MSG_TYPE: u32 = 35;
    pub const MSG_SEQ_NUM: u32 = 34;
    pub const POSS_DUP_FLAG: u32 = 43;
    pub const SENDER_COMP_ID: u32 = 49;
    pub const SENDER_SUB_ID: u32 = 50;
    pub const SENDING_TIME: u32 = 52;
    pub const TARGET_COMP_ID: u32 = 56;
    pub const TARGET_SUB_ID: u32 = 57;
    pub const POSS_RESEND: u32 = 97;
    pub const ON_BEHALF_OF_COMP_ID: u32 = 115;
    pub const ON_BEHALF_OF_SUB_ID: u32 = 116;
    pub const ORIG_SENDING_TIME: u32 = 122;
    pub const DELIVER_TO_COMP_ID: u32 = 128;
    pub const CHECKSUM: u32 = 10;

    // Session
    pub const BEGIN_SEQ_NO: u32 = 7;
    pub const END_SEQ_NO: u32 = 16;
    pub const NEW_SEQ_NO: u32 = 36;
    pub const REF_SEQ_NUM: u32 = 45;
    pub const TEXT: u32 = 58;
    pub const ENCRYPT_METHOD: u32 = 98;
    pub const HEART_BT_INT: u32 = 108;
    pub const TEST_REQ_ID: u32 = 112;
    pub const GAP_FILL_FLAG: u32 = 123;
    pub const RESET_SEQ_NUM_FLAG: u32 = 141;
    pub const REF_TAG_ID: u32 = 371;
    pub const REF_MSG_TYPE: u32 = 372;
    pub const SESSION_REJECT_REASON: u32 = 373;
    pub const USERNAME: u32 = 553;
    pub const PASSWORD: u32 = 554;
    pub const NEXT_EXPECTED_MSG_SEQ_NUM: u32 = 789;
    pub const SESSION_STATUS: u32 = 1409;

    // Application
    pub const ACCOUNT: u32 = 1;
    pub const CL_ORD_ID: u32 = 11;
    pub const CUM_QTY: u32 = 14;
    pub const EXEC_ID: u32 = 17;
    pub const EXEC_INST: u32 = 18;
    pub const EXEC_REF_ID: u32 = 19;
    pub const EXEC_TRANS_TYPE: u32 = 20;
    pub const LAST_MKT: u32 = 30;
    pub const LAST_PX: u32 = 31;
    pub const LAST_QTY: u32 = 32;
    pub const ORDER_ID: u32 = 37;
    pub const ORDER_QTY: u32 = 38;
    pub const ORD_STATUS: u32 = 39;
    pub const ORD_TYPE: u32 = 40;
    pub const ORIG_CL_ORD_ID: u32 = 41;
    pub const PRICE: u32 = 44;
    pub const SIDE: u32 = 54;
    pub const SYMBOL: u32 = 55;
    pub const TIME_IN_FORCE: u32 = 59;
    pub const TRANSACT_TIME: u32 = 60;
    pub const SETTLEMENT_TYPE: u32 = 63;
    pub const SYMBOL_SFX: u32 = 65;
    pub const CLIENT_ID: u32 = 109;
    pub const MIN_QTY: u32 = 110;
    pub const MAX_FLOOR: u32 = 111;
    pub const LOCATE_REQD: u32 = 114;
    pub const EXPIRE_TIME: u32 = 126;
    pub const EXEC_TYPE: u32 = 150;
    pub const LEAVES_QTY: u32 = 151;
    pub const EFFECTIVE_TIME: u32 = 168;
    pub const TRADING_SESSION_ID: u32 = 336;
    pub const NO_TRADING_SESSIONS: u32 = 386;
    pub const ORDER_CAPACITY: u32 = 528;
    pub const CXL_REJ_RESPONSE_TO: u32 = 434;

    // NYSE extensions
    pub const LOCATE_BROKER: u32 = 5700;
    pub const SELF_TRADE_TYPE: u32 = 7928;
    pub const SPECIAL_ORD_TYPE: u32 = 9202;
    pub const ROUTING_INST: u32 = 9303;
    pub const OFFSET_PRICE: u32 = 9403;
    pub const EXTENDED_EXEC_INST: u32 = 9416;
    pub const DEAL_ID: u32 = 9483;
    pub const LIQUIDITY_INDICATOR: u32 = 9730;
    pub const ATTRIBUTED_QUOTE: u32 = 20001;
    pub const PROACTIVE_IF_LOCKED: u32 = 20002;
    pub const CANCEL_INSTEAD_OF_REPRICE: u32 = 20003;
    pub const WORKING_PRICE: u32 = 20004;
    pub const FLOW_INDICATOR: u32 = 20005;
    pub const WORKING_AWAY_FROM_DISPLAY: u32 = 20006;
    pub const UNSOLICITED_ACK: u32 = 20007;
    pub const PARTICIPANT_TYPE: u32 = 20008;
    pub const NANOSECOND_TRANSACT_TIME: u32 = 20009;
    pub const NANOSECOND_SENDING_TIME: u32 = 20010;
    pub const SUB_ID_INDICATOR: u32 = 20013;
}

/// `Side` (tag 54).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
    SellShort,
    SellShortExempt,
    /// NYSE Texas only.
    Cross,
    CrossShort,
    CrossShortExempt,
}

impl Side {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Buy => "1",
            Self::Sell => "2",
            Self::SellShort => "5",
            Self::SellShortExempt => "6",
            Self::Cross => "8",
            Self::CrossShort => "9",
            Self::CrossShortExempt => "A",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "1" => Self::Buy,
            "2" => Self::Sell,
            "5" => Self::SellShort,
            "6" => Self::SellShortExempt,
            "8" => Self::Cross,
            "9" => Self::CrossShort,
            "A" => Self::CrossShortExempt,
            _ => return None,
        })
    }

    /// Sides that must carry `LocateReqd = N` or be rejected.
    pub const fn is_short(self) -> bool {
        matches!(
            self,
            Self::SellShort | Self::SellShortExempt | Self::CrossShort | Self::CrossShortExempt
        )
    }
}

/// `OrdType` (tag 40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdType {
    Market,
    Limit,
    InsideLimit,
    Pegged,
}

impl OrdType {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Market => "1",
            Self::Limit => "2",
            Self::InsideLimit => "7",
            Self::Pegged => "P",
        }
    }
}

/// `TimeInForce` (tag 59). Pillar equities accept `0`, `2`, `3` and `7`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeInForce {
    Day,
    AtTheOpening,
    Ioc,
    OnClose,
}

impl TimeInForce {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Day => "0",
            Self::AtTheOpening => "2",
            Self::Ioc => "3",
            Self::OnClose => "7",
        }
    }
}

/// `TradingSessionID` (tag 336).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingSession {
    Overnight,
    Early,
    Core,
    Late,
    EarlyAndCore,
    CoreAndLate,
}

impl TradingSession {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Overnight => "0",
            Self::Early => "1",
            Self::Core => "2",
            Self::Late => "3",
            Self::EarlyAndCore => "4",
            Self::CoreAndLate => "5",
        }
    }
}

/// `OrderCapacity` (tag 528).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderCapacity {
    Agency,
    Principal,
    RisklessPrincipal,
}

impl OrderCapacity {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Agency => "A",
            Self::Principal => "P",
            Self::RisklessPrincipal => "R",
        }
    }
}

/// An ordered FIX message under construction.
///
/// Fields are appended in the order the caller writes them, which is the order they hit the
/// wire. `BeginString`, `BodyLength` and `CheckSum` are added by [`Self::encode`].
#[derive(Debug, Clone, Default)]
pub struct FixMessageBuilder {
    msg_type: String,
    fields: Vec<(u32, String)>,
}

impl FixMessageBuilder {
    pub fn new(msg_type: &str) -> Self {
        Self {
            msg_type: msg_type.to_string(),
            fields: Vec::with_capacity(24),
        }
    }

    pub fn set(&mut self, tag: u32, value: impl Into<String>) -> &mut Self {
        self.fields.push((tag, value.into()));
        self
    }

    pub fn set_opt(&mut self, tag: u32, value: Option<impl Into<String>>) -> &mut Self {
        if let Some(v) = value {
            self.fields.push((tag, v.into()));
        }
        self
    }

    /// Price fields go out with six decimal places, the precision the Pillar FIX gateway
    /// documents (`0.000001`–`999,999,999.999999`).
    pub fn set_price(&mut self, tag: u32, price: Price) -> &mut Self {
        self.set(tag, format!("{:.6}", price.as_f64()))
    }

    /// Serialise with `BeginString`, `BodyLength` and `CheckSum`.
    pub fn encode(&self, begin_string: &str) -> Vec<u8> {
        let mut body = String::with_capacity(256);
        let _ = write!(body, "35={}\u{1}", self.msg_type);
        for (tag, value) in &self.fields {
            let _ = write!(body, "{tag}={value}\u{1}");
        }

        let mut out = String::with_capacity(body.len() + 32);
        let _ = write!(out, "8={begin_string}\u{1}9={}\u{1}", body.len());
        out.push_str(&body);

        let checksum: u32 = out.bytes().map(u32::from).sum();
        let _ = write!(out, "10={:03}\u{1}", checksum % 256);
        out.into_bytes()
    }
}

/// A parsed FIX message: the tag/value pairs in the order they arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixMessage {
    pub msg_type: String,
    pub fields: Vec<(u32, String)>,
}

impl FixMessage {
    /// First value for `tag`.
    pub fn get(&self, tag: u32) -> Option<&str> {
        self.fields
            .iter()
            .find(|(t, _)| *t == tag)
            .map(|(_, v)| v.as_str())
    }

    pub fn get_u64(&self, tag: u32) -> Option<u64> {
        self.get(tag)?.trim().parse().ok()
    }

    pub fn get_f64(&self, tag: u32) -> Option<f64> {
        self.get(tag)?.trim().parse().ok()
    }

    pub fn get_price(&self, tag: u32) -> Option<Price> {
        self.get_f64(tag).map(Price::from_f64)
    }

    /// Parse a complete message, verifying the checksum.
    ///
    /// A bad checksum is an error rather than a warning: it means the bytes were framed
    /// wrongly, and every field read out of them is suspect.
    pub fn parse(raw: &[u8]) -> Result<Self, FixParseError> {
        let checksum_pos = find_subsequence(raw, b"\x0110=")
            .map(|p| p + 1)
            .ok_or(FixParseError::MissingCheckSum)?;

        let expected: u32 = std::str::from_utf8(&raw[checksum_pos + 3..])
            .ok()
            .and_then(|s| s.split('\u{1}').next())
            .and_then(|s| s.trim().parse().ok())
            .ok_or(FixParseError::MissingCheckSum)?;

        let actual: u32 = raw[..checksum_pos]
            .iter()
            .map(|b| u32::from(*b))
            .sum::<u32>()
            % 256;
        if actual != expected {
            return Err(FixParseError::ChecksumMismatch { expected, actual });
        }

        let text =
            std::str::from_utf8(&raw[..checksum_pos]).map_err(|_| FixParseError::NotAscii)?;
        let mut fields = Vec::with_capacity(24);
        let mut msg_type = String::new();
        for pair in text.split('\u{1}') {
            if pair.is_empty() {
                continue;
            }
            let (tag_s, value) = pair.split_once('=').ok_or(FixParseError::MalformedField)?;
            let tag: u32 = tag_s.parse().map_err(|_| FixParseError::MalformedField)?;
            if tag == tag::MSG_TYPE {
                msg_type = value.to_string();
            }
            fields.push((tag, value.to_string()));
        }
        if msg_type.is_empty() {
            return Err(FixParseError::MissingMsgType);
        }
        Ok(Self { msg_type, fields })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum FixParseError {
    #[error("message has no CheckSum (tag 10)")]
    MissingCheckSum,
    #[error("checksum mismatch: expected {expected:03}, computed {actual:03}")]
    ChecksumMismatch { expected: u32, actual: u32 },
    #[error("message has no MsgType (tag 35)")]
    MissingMsgType,
    #[error("malformed tag=value pair")]
    MalformedField,
    #[error("message is not ASCII")]
    NotAscii,
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Session and firm identity for a Pillar FIX session.
#[derive(Debug, Clone)]
pub struct FixSessionConfig {
    pub begin_string: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    /// The firm's MPID, sent as `OnBehalfOfCompID`. Pillar scopes `ClOrdID` uniqueness to
    /// (SenderCompID + MPID), so this is part of the order's identity, not decoration.
    pub mpid: String,
    pub sender_sub_id: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub heartbeat_interval_secs: u32,
}

impl FixSessionConfig {
    pub fn new(
        sender_comp_id: impl Into<String>,
        target_comp_id: impl Into<String>,
        mpid: impl Into<String>,
    ) -> Self {
        Self {
            begin_string: "FIX.4.2".into(),
            sender_comp_id: sender_comp_id.into(),
            target_comp_id: target_comp_id.into(),
            mpid: mpid.into(),
            sender_sub_id: None,
            username: None,
            password: None,
            heartbeat_interval_secs: 30,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.sender_comp_id.trim().is_empty() {
            return Err("SenderCompID is empty".into());
        }
        if self.target_comp_id.trim().is_empty() {
            return Err("TargetCompID is empty".into());
        }
        if self.mpid.trim().is_empty() {
            return Err("MPID is empty; Pillar scopes ClOrdID uniqueness to it".into());
        }
        if self.mpid.len() > 4 {
            return Err(format!("MPID {:?} exceeds 4 characters", self.mpid));
        }
        Ok(())
    }

    /// Write the standard header in FIX's required order: `35`, `34`, `49`, `50`, `52`,
    /// `56`, then the `OnBehalfOf` block.
    fn write_header(&self, b: &mut FixMessageBuilder, seq_num: u64, sending_time: &str) {
        b.set(tag::MSG_SEQ_NUM, seq_num.to_string())
            .set(tag::SENDER_COMP_ID, self.sender_comp_id.clone());
        if let Some(sub) = &self.sender_sub_id {
            b.set(tag::SENDER_SUB_ID, sub.clone());
        }
        b.set(tag::SENDING_TIME, sending_time.to_string())
            .set(tag::TARGET_COMP_ID, self.target_comp_id.clone())
            .set(tag::ON_BEHALF_OF_COMP_ID, self.mpid.clone());
    }
}

/// A New Order Single for the Pillar FIX gateway.
#[derive(Debug, Clone)]
pub struct NewOrderSingle {
    pub cl_ord_id: String,
    pub symbol: String,
    pub side: Side,
    pub order_qty: u64,
    pub ord_type: OrdType,
    /// Required for every order type except Market.
    pub price: Option<Price>,
    pub time_in_force: TimeInForce,
    pub trading_session: TradingSession,
    pub capacity: OrderCapacity,
    pub account: Option<String>,
    pub min_qty: Option<u64>,
    pub max_floor: Option<u64>,
    /// `ExecInst` (tag 18): `f` ISO, `M` midpoint liquidity, `R` primary peg, …
    pub exec_inst: Option<String>,
    /// `ExtendedExecInst` (tag 9416).
    pub extended_exec_inst: Option<String>,
    /// `RoutingInst` (tag 9303).
    pub routing_inst: Option<String>,
    /// `SelfTradeType` (tag 7928).
    pub self_trade_type: Option<String>,
    /// Required for short sides; must be `N`.
    pub locate_reqd: Option<bool>,
    pub locate_broker: Option<String>,
}

impl NewOrderSingle {
    /// A plain day limit order.
    pub fn limit(
        cl_ord_id: impl Into<String>,
        symbol: impl Into<String>,
        side: Side,
        qty: u64,
        price: Price,
    ) -> Self {
        Self {
            cl_ord_id: cl_ord_id.into(),
            symbol: symbol.into(),
            side,
            order_qty: qty,
            ord_type: OrdType::Limit,
            price: Some(price),
            time_in_force: TimeInForce::Day,
            trading_session: TradingSession::Core,
            capacity: OrderCapacity::Agency,
            account: None,
            min_qty: None,
            max_floor: None,
            exec_inst: None,
            extended_exec_inst: None,
            routing_inst: None,
            self_trade_type: None,
            locate_reqd: None,
            locate_broker: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.cl_ord_id.is_empty() || self.cl_ord_id.len() > 20 {
            return Err(format!(
                "ClOrdID {:?} must be 1..=20 characters",
                self.cl_ord_id
            ));
        }
        if self.symbol.is_empty() || self.symbol.len() > 16 {
            return Err(format!(
                "Symbol {:?} must be 1..=16 characters",
                self.symbol
            ));
        }
        if self.order_qty == 0 || self.order_qty > 999_999_999 {
            return Err(format!(
                "OrderQty must be 1..=999,999,999, got {}",
                self.order_qty
            ));
        }
        if !matches!(self.ord_type, OrdType::Market) && self.price.is_none() {
            return Err("non-market orders require a price".into());
        }
        if self.side.is_short() && self.locate_reqd != Some(false) {
            return Err(
                "short sides must send LocateReqd=N; Pillar rejects Y or an absent tag".into(),
            );
        }
        if let Some(min) = self.min_qty {
            if min > self.order_qty {
                return Err(format!("MinQty {min} exceeds OrderQty {}", self.order_qty));
            }
        }
        for (name, value) in [
            ("ClOrdID", &self.cl_ord_id),
            ("Account", self.account.as_ref().unwrap_or(&String::new())),
        ] {
            if let Some(bad) = value
                .chars()
                .find(|c| matches!(c, ',' | ';' | '|' | '@' | '<' | '>' | '&' | '"' | '\''))
            {
                return Err(format!("{name} contains the disallowed character {bad:?}"));
            }
        }
        Ok(())
    }

    /// Encode as FIX `35=D`.
    ///
    /// `transact_time` is the caller's application time as `YYYYMMDD-HH:MM:SS.mmm`; the
    /// gateway echoes its own time back on the acknowledgement.
    pub fn encode(
        &self,
        session: &FixSessionConfig,
        seq_num: u64,
        sending_time: &str,
        transact_time: &str,
    ) -> Vec<u8> {
        let mut b = FixMessageBuilder::new("D");
        session.write_header(&mut b, seq_num, sending_time);

        b.set_opt(tag::ACCOUNT, self.account.clone())
            .set(tag::CL_ORD_ID, self.cl_ord_id.clone())
            .set_opt(tag::EXEC_INST, self.exec_inst.clone())
            .set(tag::ORDER_QTY, self.order_qty.to_string())
            .set(tag::ORD_TYPE, self.ord_type.code());
        if let Some(p) = self.price {
            b.set_price(tag::PRICE, p);
        }
        b.set(tag::SIDE, self.side.code())
            .set(tag::SYMBOL, self.symbol.clone())
            .set(tag::TIME_IN_FORCE, self.time_in_force.code())
            .set(tag::TRANSACT_TIME, transact_time.to_string())
            .set_opt(tag::MIN_QTY, self.min_qty.map(|q| q.to_string()))
            .set_opt(tag::MAX_FLOOR, self.max_floor.map(|q| q.to_string()))
            .set_opt(
                tag::LOCATE_REQD,
                self.locate_reqd.map(|v| if v { "Y" } else { "N" }),
            )
            // NoTradingSessions is a repeating group of one, so its count precedes the id.
            .set(tag::NO_TRADING_SESSIONS, "1")
            .set(tag::TRADING_SESSION_ID, self.trading_session.code())
            .set(tag::ORDER_CAPACITY, self.capacity.code())
            .set_opt(tag::LOCATE_BROKER, self.locate_broker.clone())
            .set_opt(tag::SELF_TRADE_TYPE, self.self_trade_type.clone())
            .set_opt(tag::ROUTING_INST, self.routing_inst.clone())
            .set_opt(tag::EXTENDED_EXEC_INST, self.extended_exec_inst.clone());

        b.encode(&session.begin_string)
    }
}

/// Encode an Order Cancel Request (`35=F`).
///
/// FIX requires the cancel to restate the original order's symbol, side and quantity so the
/// gateway can validate it against the order it names, which is why this takes so many
/// arguments rather than just the two order ids.
#[allow(clippy::too_many_arguments)]
pub fn encode_cancel_request(
    session: &FixSessionConfig,
    seq_num: u64,
    sending_time: &str,
    transact_time: &str,
    cl_ord_id: &str,
    orig_cl_ord_id: &str,
    symbol: &str,
    side: Side,
    order_qty: u64,
) -> Vec<u8> {
    let mut b = FixMessageBuilder::new("F");
    session.write_header(&mut b, seq_num, sending_time);
    b.set(tag::CL_ORD_ID, cl_ord_id)
        .set(tag::ORDER_QTY, order_qty.to_string())
        .set(tag::ORIG_CL_ORD_ID, orig_cl_ord_id)
        .set(tag::SIDE, side.code())
        .set(tag::SYMBOL, symbol)
        .set(tag::TRANSACT_TIME, transact_time);
    b.encode(&session.begin_string)
}

/// Encode a Logon (`35=A`).
pub fn encode_logon(
    session: &FixSessionConfig,
    seq_num: u64,
    sending_time: &str,
    reset_seq_num: bool,
    next_expected_seq_num: Option<u64>,
) -> Vec<u8> {
    let mut b = FixMessageBuilder::new("A");
    session.write_header(&mut b, seq_num, sending_time);
    b.set(tag::ENCRYPT_METHOD, "0")
        .set(
            tag::HEART_BT_INT,
            session.heartbeat_interval_secs.to_string(),
        )
        .set(
            tag::RESET_SEQ_NUM_FLAG,
            if reset_seq_num { "Y" } else { "N" },
        )
        .set_opt(tag::USERNAME, session.username.clone())
        .set_opt(tag::PASSWORD, session.password.clone())
        .set_opt(
            tag::NEXT_EXPECTED_MSG_SEQ_NUM,
            next_expected_seq_num.map(|n| n.to_string()),
        );
    b.encode(&session.begin_string)
}

/// Encode a Logout (`35=5`).
pub fn encode_logout(
    session: &FixSessionConfig,
    seq_num: u64,
    sending_time: &str,
    text: Option<&str>,
) -> Vec<u8> {
    let mut b = FixMessageBuilder::new("5");
    session.write_header(&mut b, seq_num, sending_time);
    b.set_opt(tag::TEXT, text.map(str::to_string));
    b.encode(&session.begin_string)
}

/// Encode a Heartbeat (`35=0`), optionally answering a Test Request.
pub fn encode_heartbeat(
    session: &FixSessionConfig,
    seq_num: u64,
    sending_time: &str,
    test_req_id: Option<&str>,
) -> Vec<u8> {
    let mut b = FixMessageBuilder::new("0");
    session.write_header(&mut b, seq_num, sending_time);
    b.set_opt(tag::TEST_REQ_ID, test_req_id.map(str::to_string));
    b.encode(&session.begin_string)
}

/// Encode a Resend Request (`35=2`). `end` of 0 means "everything from `begin`".
pub fn encode_resend_request(
    session: &FixSessionConfig,
    seq_num: u64,
    sending_time: &str,
    begin: u64,
    end: u64,
) -> Vec<u8> {
    let mut b = FixMessageBuilder::new("2");
    session.write_header(&mut b, seq_num, sending_time);
    b.set(tag::BEGIN_SEQ_NO, begin.to_string())
        .set(tag::END_SEQ_NO, end.to_string());
    b.encode(&session.begin_string)
}

/// `ExecType` (tag 150) on an Execution Report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecType {
    New,
    PartialFill,
    Fill,
    DoneForDay,
    Canceled,
    Replaced,
    PendingCancel,
    Rejected,
    Suspended,
    Restated,
    Trade,
    TradeCorrect,
    TradeCancel,
    Other(char),
}

impl ExecType {
    pub fn parse(s: &str) -> Self {
        match s {
            "0" => Self::New,
            "1" => Self::PartialFill,
            "2" => Self::Fill,
            "3" => Self::DoneForDay,
            "4" => Self::Canceled,
            "5" => Self::Replaced,
            "6" => Self::PendingCancel,
            "8" => Self::Rejected,
            "9" => Self::Suspended,
            "D" => Self::Restated,
            "F" => Self::Trade,
            "G" => Self::TradeCorrect,
            "H" => Self::TradeCancel,
            other => Self::Other(other.chars().next().unwrap_or('?')),
        }
    }

    /// True when the order is finished and will produce no further reports.
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Fill | Self::DoneForDay | Self::Canceled | Self::Rejected
        )
    }

    /// True when this report carries a fill.
    pub const fn is_fill(self) -> bool {
        matches!(self, Self::PartialFill | Self::Fill | Self::Trade)
    }
}

/// The fields of an Execution Report worth pulling out by name.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionReport {
    pub cl_ord_id: String,
    pub orig_cl_ord_id: Option<String>,
    pub order_id: String,
    pub exec_id: String,
    pub exec_type: ExecType,
    pub ord_status: String,
    pub symbol: String,
    pub side: Option<Side>,
    pub last_px: Option<Price>,
    pub last_qty: Option<u64>,
    pub cum_qty: Option<u64>,
    pub leaves_qty: Option<u64>,
    pub deal_id: Option<String>,
    pub liquidity_indicator: Option<String>,
    pub text: Option<String>,
}

impl ExecutionReport {
    /// Extract from a parsed `35=8`.
    pub fn from_message(m: &FixMessage) -> Option<Self> {
        if m.msg_type != "8" {
            return None;
        }
        Some(Self {
            cl_ord_id: m.get(tag::CL_ORD_ID).unwrap_or_default().to_string(),
            orig_cl_ord_id: m.get(tag::ORIG_CL_ORD_ID).map(str::to_string),
            order_id: m.get(tag::ORDER_ID).unwrap_or_default().to_string(),
            exec_id: m.get(tag::EXEC_ID).unwrap_or_default().to_string(),
            exec_type: ExecType::parse(m.get(tag::EXEC_TYPE).unwrap_or_default()),
            ord_status: m.get(tag::ORD_STATUS).unwrap_or_default().to_string(),
            symbol: m.get(tag::SYMBOL).unwrap_or_default().to_string(),
            side: m.get(tag::SIDE).and_then(Side::parse),
            last_px: m.get_price(tag::LAST_PX),
            last_qty: m.get_u64(tag::LAST_QTY),
            cum_qty: m.get_u64(tag::CUM_QTY),
            leaves_qty: m.get_u64(tag::LEAVES_QTY),
            deal_id: m.get(tag::DEAL_ID).map(str::to_string),
            liquidity_indicator: m.get(tag::LIQUIDITY_INDICATOR).map(str::to_string),
            text: m.get(tag::TEXT).map(str::to_string),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> FixSessionConfig {
        FixSessionConfig::new("FIRMFIX", "NYSE", "ABCD")
    }

    fn as_text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace('\u{1}', "|")
    }

    #[test]
    fn encoded_messages_have_a_valid_body_length_and_checksum() {
        let msg = NewOrderSingle::limit("ORD1", "IBM", Side::Buy, 500, Price::from_f64(180.50))
            .encode(
                &session(),
                1,
                "20260817-14:30:00.000",
                "20260817-14:30:00.000",
            );
        // Round tripping through the parser verifies the checksum.
        let parsed = FixMessage::parse(&msg).unwrap();
        assert_eq!(parsed.msg_type, "D");

        // BodyLength must equal the byte count between 9=...<SOH> and 10=.
        let text = as_text(&msg);
        let declared: usize = parsed.get(tag::BODY_LENGTH).unwrap().parse().unwrap();
        let body_start = text.find("|9=").unwrap();
        let after_9 = text[body_start + 1..].find('|').unwrap() + body_start + 2;
        let checksum_start = text.rfind("|10=").unwrap() + 1;
        assert_eq!(checksum_start - after_9, declared);
    }

    #[test]
    fn a_tampered_message_fails_the_checksum() {
        let mut msg = encode_heartbeat(&session(), 1, "20260817-14:30:00.000", None);
        // Flip a byte in the middle of the body.
        let pos = msg.len() / 2;
        msg[pos] = if msg[pos] == b'X' { b'Y' } else { b'X' };
        assert!(matches!(
            FixMessage::parse(&msg),
            Err(FixParseError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn header_fields_are_emitted_in_the_required_order() {
        let msg = encode_heartbeat(&session(), 7, "20260817-14:30:00.000", None);
        let text = as_text(&msg);
        let order = ["8=", "9=", "35=", "34=", "49=", "52=", "56=", "115="];
        let mut last = 0usize;
        for needle in order {
            let at = text
                .find(needle)
                .unwrap_or_else(|| panic!("missing {needle}"));
            assert!(at >= last, "{needle} appears out of order in {text}");
            last = at;
        }
        assert!(text.ends_with('|'), "message must end with SOH");
        assert!(text.contains("|10="), "checksum must be last");
    }

    #[test]
    fn new_order_single_carries_the_documented_tags() {
        let mut o = NewOrderSingle::limit("ORD1", "IBM", Side::Buy, 500, Price::from_f64(180.50));
        o.account = Some("ACCT1".into());
        o.min_qty = Some(100);
        let text = as_text(&o.encode(&session(), 1, "T", "T"));
        for expect in [
            "|35=D|",
            "|1=ACCT1|",
            "|11=ORD1|",
            "|38=500|",
            "|40=2|",
            "|44=180.500000|",
            "|54=1|",
            "|55=IBM|",
            "|59=0|",
            "|60=T|",
            "|110=100|",
            "|386=1|",
            "|336=2|",
            "|528=A|",
        ] {
            assert!(text.contains(expect), "missing {expect} in {text}");
        }
    }

    #[test]
    fn the_trading_session_group_count_precedes_its_id() {
        let text = as_text(
            &NewOrderSingle::limit("O", "IBM", Side::Buy, 1, Price::from_f64(1.0)).encode(
                &session(),
                1,
                "T",
                "T",
            ),
        );
        let count_at = text.find("|386=").unwrap();
        let id_at = text.find("|336=").unwrap();
        assert!(
            count_at < id_at,
            "NoTradingSessions must precede TradingSessionID"
        );
    }

    #[test]
    fn on_behalf_of_comp_id_carries_the_mpid() {
        let text = as_text(&encode_heartbeat(&session(), 1, "T", None));
        assert!(text.contains("|115=ABCD|"));
    }

    #[test]
    fn short_sales_must_declare_locate_reqd_n() {
        let mut o = NewOrderSingle::limit("O", "IBM", Side::SellShort, 100, Price::from_f64(180.0));
        assert!(o.validate().is_err(), "absent LocateReqd is rejected");
        o.locate_reqd = Some(true);
        assert!(o.validate().is_err(), "LocateReqd=Y is rejected");
        o.locate_reqd = Some(false);
        assert!(o.validate().is_ok());
        assert!(as_text(&o.encode(&session(), 1, "T", "T")).contains("|114=N|"));
    }

    #[test]
    fn order_validation_catches_field_width_and_range_violations() {
        let base = || NewOrderSingle::limit("O", "IBM", Side::Buy, 100, Price::from_f64(180.0));

        let mut o = base();
        o.cl_ord_id = "X".repeat(21);
        assert!(o.validate().is_err(), "ClOrdID over 20 characters");

        o = base();
        o.symbol = "X".repeat(17);
        assert!(o.validate().is_err(), "Symbol over 16 characters");

        o = base();
        o.order_qty = 0;
        assert!(o.validate().is_err(), "zero quantity");

        o = base();
        o.price = None;
        assert!(o.validate().is_err(), "limit order without a price");
        o.ord_type = OrdType::Market;
        assert!(o.validate().is_ok(), "market orders need no price");

        o = base();
        o.min_qty = Some(500);
        assert!(o.validate().is_err(), "MinQty above OrderQty");

        o = base();
        o.cl_ord_id = "ORD@1".into();
        assert!(o.validate().is_err(), "disallowed character in ClOrdID");

        assert!(base().validate().is_ok());
    }

    #[test]
    fn logon_carries_the_session_parameters() {
        let mut s = session();
        s.username = Some("user".into());
        s.password = Some("pass".into());
        let text = as_text(&encode_logon(&s, 1, "T", true, Some(5)));
        assert!(text.contains("|35=A|"));
        assert!(text.contains("|98=0|"));
        assert!(text.contains("|108=30|"));
        assert!(text.contains("|141=Y|"));
        assert!(text.contains("|553=user|"));
        assert!(text.contains("|554=pass|"));
        assert!(text.contains("|789=5|"));
    }

    #[test]
    fn cancel_request_carries_both_order_ids() {
        let text = as_text(&encode_cancel_request(
            &session(),
            2,
            "T",
            "T",
            "ORD2",
            "ORD1",
            "IBM",
            Side::Buy,
            500,
        ));
        assert!(text.contains("|35=F|"));
        assert!(text.contains("|11=ORD2|"));
        assert!(text.contains("|41=ORD1|"));
    }

    #[test]
    fn resend_request_uses_zero_for_an_open_ended_range() {
        let text = as_text(&encode_resend_request(&session(), 3, "T", 10, 0));
        assert!(text.contains("|35=2|"));
        assert!(text.contains("|7=10|"));
        assert!(text.contains("|16=0|"));
    }

    #[test]
    fn execution_report_fields_are_extracted_by_name() {
        let mut b = FixMessageBuilder::new("8");
        b.set(tag::MSG_SEQ_NUM, "5")
            .set(tag::SENDER_COMP_ID, "NYSE")
            .set(tag::SENDING_TIME, "T")
            .set(tag::TARGET_COMP_ID, "FIRMFIX")
            .set(tag::CL_ORD_ID, "ORD1")
            .set(tag::ORDER_ID, "9876543210")
            .set(tag::EXEC_ID, "EX1")
            .set(tag::EXEC_TYPE, "1")
            .set(tag::ORD_STATUS, "1")
            .set(tag::SYMBOL, "IBM")
            .set(tag::SIDE, "1")
            .set(tag::LAST_PX, "180.500000")
            .set(tag::LAST_QTY, "200")
            .set(tag::CUM_QTY, "200")
            .set(tag::LEAVES_QTY, "300")
            .set(tag::DEAL_ID, "DEAL42")
            .set(tag::LIQUIDITY_INDICATOR, "R");
        let raw = b.encode("FIX.4.2");

        let parsed = FixMessage::parse(&raw).unwrap();
        let er = ExecutionReport::from_message(&parsed).unwrap();
        assert_eq!(er.cl_ord_id, "ORD1");
        assert_eq!(er.order_id, "9876543210");
        assert_eq!(er.exec_type, ExecType::PartialFill);
        assert_eq!(er.side, Some(Side::Buy));
        assert_eq!(er.last_px.unwrap().to_string(), "180.50");
        assert_eq!(er.last_qty, Some(200));
        assert_eq!(er.leaves_qty, Some(300));
        assert_eq!(er.deal_id.as_deref(), Some("DEAL42"));
        assert!(er.exec_type.is_fill());
        assert!(!er.exec_type.is_terminal());
    }

    #[test]
    fn exec_types_classify_terminal_and_fill_states() {
        assert!(ExecType::parse("2").is_terminal());
        assert!(ExecType::parse("4").is_terminal());
        assert!(ExecType::parse("8").is_terminal());
        assert!(!ExecType::parse("0").is_terminal());
        assert!(ExecType::parse("F").is_fill());
        assert!(!ExecType::parse("4").is_fill());
    }

    #[test]
    fn a_non_execution_report_yields_none() {
        let raw = encode_heartbeat(&session(), 1, "T", None);
        let parsed = FixMessage::parse(&raw).unwrap();
        assert!(ExecutionReport::from_message(&parsed).is_none());
    }

    #[test]
    fn session_config_validation_catches_identity_mistakes() {
        let mut s = FixSessionConfig::new("", "NYSE", "ABCD");
        assert!(s.validate().is_err());
        s = FixSessionConfig::new("FIRM", "NYSE", "");
        assert!(s.validate().is_err());
        s = FixSessionConfig::new("FIRM", "NYSE", "TOOLONG");
        assert!(s.validate().is_err());
        assert!(FixSessionConfig::new("FIRM", "NYSE", "ABCD")
            .validate()
            .is_ok());
    }

    #[test]
    fn prices_are_emitted_with_six_decimals() {
        let mut b = FixMessageBuilder::new("D");
        b.set_price(tag::PRICE, Price::from_f64(0.000001));
        let text = as_text(&b.encode("FIX.4.2"));
        assert!(text.contains("|44=0.000001|"), "{text}");
    }
}
