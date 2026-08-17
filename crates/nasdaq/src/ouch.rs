//! OUCH 4.2 — Nasdaq's native order-entry protocol.
//!
//! OUCH rides on SoupBinTCP: outbound (host → client) messages arrive as Sequenced Data and
//! are therefore guaranteed and resumable; inbound (client → host) messages go as
//! Unsequenced Data and are **not**.
//!
//! That asymmetry is the whole design. Quoting the specification: "all host-bound messages
//! are designed so that they can be benignly resent for robust recovery from connection and
//! application failures." Concretely:
//!
//! * an order is identified by (OUCH account, 14-byte day-unique `Order Token`) chosen by
//!   the client, not by the exchange;
//! * re-sending an Enter Order with a token already used is silently ignored rather than
//!   creating a second order;
//! * so after a socket failure the correct recovery is to resend every in-flight message,
//!   which is exactly what [`Order::enter`] is built to make cheap.
//!
//! Prices are `Price(4)`. The market-order sentinel for a cross is $214,748.3647
//! (`0x7FFFFFFF`); $200,000.00 and the maximum integer are also treated as market orders.

use exchange_core::wire::{Cursor, Writer};
use exchange_core::{Mpid4, Price, Symbol8, WireError, WireResult};

const PROTOCOL: &str = "OUCH 4.2";

/// Width of the client-assigned `Order Token` field.
pub const TOKEN_LEN: usize = 14;

/// Time-in-force sentinels from the specification's Data Types section.
pub mod time_in_force {
    /// Immediate or cancel: unexecuted shares are cancelled on entry.
    pub const IOC: u32 = 0;
    /// Live until the conclusion of the Extended Trading Close.
    pub const EXTENDED_TRADING_CLOSE: u32 = 99_996;
    /// Live until the primary market's close.
    pub const MARKET_HOURS: u32 = 99_998;
    /// Live until the end of the Nasdaq trading day.
    pub const SYSTEM_HOURS: u32 = 99_999;
    /// Values above this are invalid; such orders live only during system hours.
    pub const MAX_VALID: u32 = 99_999;
}

/// `Buy/Sell Indicator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
    /// `T` — sell short; the client affirms it can borrow for T+3 delivery.
    SellShort,
    /// `E` — sell short exempt.
    SellShortExempt,
}

impl Side {
    pub const fn code(self) -> char {
        match self {
            Self::Buy => 'B',
            Self::Sell => 'S',
            Self::SellShort => 'T',
            Self::SellShortExempt => 'E',
        }
    }

    pub fn parse(ch: char) -> WireResult<Self> {
        Ok(match ch {
            'B' => Self::Buy,
            'S' => Self::Sell,
            'T' => Self::SellShort,
            'E' => Self::SellShortExempt,
            other => {
                return Err(WireError::InvalidEnum {
                    protocol: PROTOCOL,
                    field: "Buy/Sell Indicator",
                    value: other,
                })
            }
        })
    }
}

/// `Display` — how the order is exposed, and which book it rests on.
///
/// Variant names track the spec's own labels (including `NonDisplay`) so a reader can map
/// them onto the Display table without a translation step.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    /// `A` — attributable, price to display.
    Attributable,
    /// `Y` — anonymous, price to comply.
    Anonymous,
    /// `N` — non-displayed.
    NonDisplay,
    /// `P` — post only.
    PostOnly,
    /// `I` — imbalance only (opening and closing cross).
    ImbalanceOnly,
    /// `M` — midpoint peg.
    MidpointPeg,
    /// `W` — midpoint peg, post only.
    MidpointPegPostOnly,
    /// `L` — post only and attributable, price to display.
    PostOnlyAttributable,
    /// `O` — retail order type 1.
    RetailType1,
    /// `T` — retail order type 2.
    RetailType2,
    /// `Q` — retail price improvement order.
    RetailPriceImprovement,
    /// `Z` — entered as displayed, changed to non-displayed (acknowledgements only).
    ChangedToNonDisplayed,
    /// A value not in the table above; carried through rather than rejected, because the
    /// spec adds display types over time and an unknown ack must not kill a session.
    Other(char),
}

impl Display {
    pub const fn code(self) -> char {
        match self {
            Self::Attributable => 'A',
            Self::Anonymous => 'Y',
            Self::NonDisplay => 'N',
            Self::PostOnly => 'P',
            Self::ImbalanceOnly => 'I',
            Self::MidpointPeg => 'M',
            Self::MidpointPegPostOnly => 'W',
            Self::PostOnlyAttributable => 'L',
            Self::RetailType1 => 'O',
            Self::RetailType2 => 'T',
            Self::RetailPriceImprovement => 'Q',
            Self::ChangedToNonDisplayed => 'Z',
            Self::Other(c) => c,
        }
    }

    pub const fn parse(ch: char) -> Self {
        match ch {
            'A' => Self::Attributable,
            'Y' => Self::Anonymous,
            'N' => Self::NonDisplay,
            'P' => Self::PostOnly,
            'I' => Self::ImbalanceOnly,
            'M' => Self::MidpointPeg,
            'W' => Self::MidpointPegPostOnly,
            'L' => Self::PostOnlyAttributable,
            'O' => Self::RetailType1,
            'T' => Self::RetailType2,
            'Q' => Self::RetailPriceImprovement,
            'Z' => Self::ChangedToNonDisplayed,
            other => Self::Other(other),
        }
    }
}

/// `Capacity`. Anything other than agency/principal/riskless is converted to `O` (other) by
/// Nasdaq on entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capacity {
    Agency,
    Principal,
    Riskless,
    Other,
}

impl Capacity {
    pub const fn code(self) -> char {
        match self {
            Self::Agency => 'A',
            Self::Principal => 'P',
            Self::Riskless => 'R',
            Self::Other => 'O',
        }
    }

    pub const fn parse(ch: char) -> Self {
        match ch {
            'A' => Self::Agency,
            'P' => Self::Principal,
            'R' => Self::Riskless,
            _ => Self::Other,
        }
    }
}

/// `Cross Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossType {
    /// `N` — continuous market.
    None,
    Opening,
    Closing,
    /// `H` — halt/IPO cross. Must be entered at the market price.
    HaltIpo,
    Supplemental,
    ExtendedLife,
    ExtendedTradingClose,
    Other(char),
}

impl CrossType {
    pub const fn code(self) -> char {
        match self {
            Self::None => 'N',
            Self::Opening => 'O',
            Self::Closing => 'C',
            Self::HaltIpo => 'H',
            Self::Supplemental => 'S',
            Self::ExtendedLife => 'E',
            Self::ExtendedTradingClose => 'A',
            Self::Other(c) => c,
        }
    }

    pub const fn parse(ch: char) -> Self {
        match ch {
            'N' => Self::None,
            'O' => Self::Opening,
            'C' => Self::Closing,
            'H' => Self::HaltIpo,
            'S' => Self::Supplemental,
            'E' => Self::ExtendedLife,
            'A' => Self::ExtendedTradingClose,
            other => Self::Other(other),
        }
    }
}

/// `Intermarket Sweep Eligibility`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsoEligibility {
    Eligible,
    NotEligible,
    /// `y` — trade-at intermarket sweep order.
    TradeAtIso,
}

impl IsoEligibility {
    pub const fn code(self) -> char {
        match self {
            Self::Eligible => 'Y',
            Self::NotEligible => 'N',
            Self::TradeAtIso => 'y',
        }
    }

    pub const fn parse(ch: char) -> Self {
        match ch {
            'Y' => Self::Eligible,
            'y' => Self::TradeAtIso,
            _ => Self::NotEligible,
        }
    }
}

/// A 14-byte day-unique order token.
///
/// Tokens must be unique per OUCH account per day and are case sensitive. Reusing one is
/// not an error the exchange reports — the duplicate order is *silently ignored* — so
/// uniqueness is entirely the client's responsibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OrderToken([u8; TOKEN_LEN]);

impl OrderToken {
    /// Space-pad a string into a token. Longer input is truncated, which would silently
    /// collide, so [`Self::try_new`] is preferred on the order path.
    pub fn new(s: &str) -> Self {
        let mut out = [b' '; TOKEN_LEN];
        for (slot, byte) in out.iter_mut().zip(s.bytes()) {
            *slot = byte;
        }
        Self(out)
    }

    /// Reject anything that would not survive the 14-byte field intact.
    pub fn try_new(s: &str) -> Result<Self, String> {
        if s.len() > TOKEN_LEN {
            return Err(format!(
                "order token {s:?} is {} bytes; the OUCH field is {TOKEN_LEN}",
                s.len()
            ));
        }
        if !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ') {
            return Err(format!(
                "order token {s:?} must be alphanumeric or space (spec: Token fields)"
            ));
        }
        Ok(Self::new(s))
    }

    pub fn as_bytes(&self) -> &[u8; TOKEN_LEN] {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0)
            .unwrap_or("<non-ascii>")
            .trim_end()
    }
}

impl std::fmt::Display for OrderToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─── Inbound (client → host) ────────────────────────────────────────────────

/// An Enter Order message, `O`, 49 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnterOrder {
    pub token: OrderToken,
    pub side: Side,
    /// Must be greater than zero and less than 1,000,000.
    pub shares: u32,
    pub stock: Symbol8,
    pub price: Price,
    /// Seconds the order should live, or one of the [`time_in_force`] sentinels.
    pub time_in_force: u32,
    /// Blank uses the OUCH account's default firm. Non-blank requires a Service Bureau
    /// agreement covering that firm.
    pub firm: Mpid4,
    pub display: Display,
    pub capacity: Capacity,
    pub iso_eligibility: IsoEligibility,
    pub min_quantity: u32,
    pub cross_type: CrossType,
    /// `R` retail designated, `N` not, space to use the port default.
    pub customer_type: char,
}

impl EnterOrder {
    pub const LEN: usize = 49;

    /// A plain displayed day limit order — the common case.
    pub fn limit(token: OrderToken, side: Side, shares: u32, stock: &str, price: Price) -> Self {
        Self {
            token,
            side,
            shares,
            stock: Symbol8::new(stock),
            price,
            time_in_force: time_in_force::MARKET_HOURS,
            firm: Mpid4::BLANK,
            display: Display::Anonymous,
            capacity: Capacity::Agency,
            iso_eligibility: IsoEligibility::NotEligible,
            min_quantity: 0,
            cross_type: CrossType::None,
            customer_type: ' ',
        }
    }

    /// Reject an order the exchange would reject, before it costs a round trip.
    pub fn validate(&self) -> Result<(), String> {
        if self.shares == 0 || self.shares >= 1_000_000 {
            return Err(format!(
                "shares must be > 0 and < 1,000,000, got {}",
                self.shares
            ));
        }
        // The symbol and firm fields are fixed width by construction, so only emptiness
        // is still checkable here.
        if self.stock.is_empty() {
            return Err("stock symbol must not be blank".to_string());
        }
        if self.time_in_force > time_in_force::MAX_VALID {
            return Err(format!(
                "time in force {} exceeds {}",
                self.time_in_force,
                time_in_force::MAX_VALID
            ));
        }
        if self.min_quantity > self.shares {
            return Err(format!(
                "minimum quantity {} exceeds order quantity {}",
                self.min_quantity, self.shares
            ));
        }
        // A halt/IPO cross order must be priced at market.
        if matches!(self.cross_type, CrossType::HaltIpo)
            && self.price.to_price4() != Price::OUCH_MARKET_SENTINEL_RAW
        {
            return Err(
                "halt/IPO cross orders must use the market price sentinel ($214,748.3647)".into(),
            );
        }
        Ok(())
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::LEN);
        w.u8(b'O')
            .raw(self.token.as_bytes())
            .ascii_char(self.side.code())
            .be_u32(self.shares)
            .raw(self.stock.as_bytes())
            .be_u32(self.price.to_price4())
            .be_u32(self.time_in_force)
            .raw(self.firm.as_bytes())
            .ascii_char(self.display.code())
            .ascii_char(self.capacity.code())
            .ascii_char(self.iso_eligibility.code())
            .be_u32(self.min_quantity)
            .ascii_char(self.cross_type.code())
            .ascii_char(self.customer_type);
        w.into_vec()
    }

    pub fn decode(buf: &[u8]) -> WireResult<Self> {
        expect_len(buf, b'O', Self::LEN)?;
        let mut c = Cursor::at(buf, 1);
        Ok(Self {
            token: read_token(&mut c)?,
            side: Side::parse(c.ascii_char()?)?,
            shares: c.be_u32()?,
            stock: Symbol8::from_wire(c.take(8)?),
            price: Price::from_price4(c.be_u32()?),
            time_in_force: c.be_u32()?,
            firm: Mpid4::from_wire(c.take(4)?),
            display: Display::parse(c.ascii_char()?),
            capacity: Capacity::parse(c.ascii_char()?),
            iso_eligibility: IsoEligibility::parse(c.ascii_char()?),
            min_quantity: c.be_u32()?,
            cross_type: CrossType::parse(c.ascii_char()?),
            customer_type: c.ascii_char()?,
        })
    }
}

/// A Replace Order message, `U`, 47 bytes.
///
/// `shares` is the total liable for the whole order/replace chain, *inclusive of previous
/// executions*. After 100 of 500 shares fill, replacing with 500 leaves 400 exposed;
/// replacing with 600 exposes a fresh 500. This is what prevents double liability across a
/// chain, and it is the field most often got wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceOrder {
    pub existing_token: OrderToken,
    pub replacement_token: OrderToken,
    pub shares: u32,
    pub price: Price,
    pub time_in_force: u32,
    pub display: Display,
    pub iso_eligibility: IsoEligibility,
    pub min_quantity: u32,
}

impl ReplaceOrder {
    pub const LEN: usize = 47;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::LEN);
        w.u8(b'U')
            .raw(self.existing_token.as_bytes())
            .raw(self.replacement_token.as_bytes())
            .be_u32(self.shares)
            .be_u32(self.price.to_price4())
            .be_u32(self.time_in_force)
            .ascii_char(self.display.code())
            .ascii_char(self.iso_eligibility.code())
            .be_u32(self.min_quantity);
        w.into_vec()
    }

    pub fn decode(buf: &[u8]) -> WireResult<Self> {
        expect_len(buf, b'U', Self::LEN)?;
        let mut c = Cursor::at(buf, 1);
        Ok(Self {
            existing_token: read_token(&mut c)?,
            replacement_token: read_token(&mut c)?,
            shares: c.be_u32()?,
            price: Price::from_price4(c.be_u32()?),
            time_in_force: c.be_u32()?,
            display: Display::parse(c.ascii_char()?),
            iso_eligibility: IsoEligibility::parse(c.ascii_char()?),
            min_quantity: c.be_u32()?,
        })
    }
}

/// A Cancel Order message, `X`, 19 bytes.
///
/// `shares` is the new *intended order size*: the maximum that may execute in total after
/// the cancel is applied. Zero cancels the remaining balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancelOrder {
    pub token: OrderToken,
    pub shares: u32,
}

impl CancelOrder {
    pub const LEN: usize = 19;

    /// Cancel the entire remaining balance.
    pub fn full(token: OrderToken) -> Self {
        Self { token, shares: 0 }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::LEN);
        w.u8(b'X').raw(self.token.as_bytes()).be_u32(self.shares);
        w.into_vec()
    }

    pub fn decode(buf: &[u8]) -> WireResult<Self> {
        expect_len(buf, b'X', Self::LEN)?;
        let mut c = Cursor::at(buf, 1);
        Ok(Self {
            token: read_token(&mut c)?,
            shares: c.be_u32()?,
        })
    }
}

/// A Modify Order message, `M`, 20 bytes.
///
/// Modify keeps book priority unless the share count increases. Only short-sale marking
/// transitions are allowed on the side field (S↔T, S↔E, T↔E).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModifyOrder {
    pub token: OrderToken,
    pub side: Side,
    pub shares: u32,
}

impl ModifyOrder {
    pub const LEN: usize = 20;

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(Self::LEN);
        w.u8(b'M')
            .raw(self.token.as_bytes())
            .ascii_char(self.side.code())
            .be_u32(self.shares);
        w.into_vec()
    }

    pub fn decode(buf: &[u8]) -> WireResult<Self> {
        expect_len(buf, b'M', Self::LEN)?;
        let mut c = Cursor::at(buf, 1);
        Ok(Self {
            token: read_token(&mut c)?,
            side: Side::parse(c.ascii_char()?)?,
            shares: c.be_u32()?,
        })
    }
}

/// Any client → host message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inbound {
    Enter(EnterOrder),
    Replace(ReplaceOrder),
    Cancel(CancelOrder),
    Modify(ModifyOrder),
}

impl Inbound {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Enter(m) => m.encode(),
            Self::Replace(m) => m.encode(),
            Self::Cancel(m) => m.encode(),
            Self::Modify(m) => m.encode(),
        }
    }

    pub fn decode(buf: &[u8]) -> WireResult<Self> {
        match buf.first() {
            Some(b'O') => Ok(Self::Enter(EnterOrder::decode(buf)?)),
            Some(b'U') => Ok(Self::Replace(ReplaceOrder::decode(buf)?)),
            Some(b'X') => Ok(Self::Cancel(CancelOrder::decode(buf)?)),
            Some(b'M') => Ok(Self::Modify(ModifyOrder::decode(buf)?)),
            Some(other) => Err(WireError::UnknownMessageType {
                protocol: PROTOCOL,
                got: *other as u16,
                got_ascii: *other as char,
            }),
            None => Err(WireError::Truncated {
                at: 0,
                need: 1,
                have: 0,
            }),
        }
    }

    /// The token this message acts on, for in-flight bookkeeping.
    pub fn token(&self) -> OrderToken {
        match self {
            Self::Enter(m) => m.token,
            Self::Replace(m) => m.replacement_token,
            Self::Cancel(m) => m.token,
            Self::Modify(m) => m.token,
        }
    }
}

// ─── Outbound (host → client) ───────────────────────────────────────────────

/// `Order State` on an acknowledgement. `Dead` means the order was accepted and then
/// immediately cancelled; no further messages will follow for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    Live,
    Dead,
    Other(char),
}

impl OrderState {
    const fn parse(ch: char) -> Self {
        match ch {
            'L' => Self::Live,
            'D' => Self::Dead,
            other => Self::Other(other),
        }
    }

    pub const fn code(self) -> char {
        match self {
            Self::Live => 'L',
            Self::Dead => 'D',
            Self::Other(c) => c,
        }
    }
}

/// Reason an order was reduced or cancelled (spec §3.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// `U` — in response to a Cancel or Replace.
    UserRequested,
    /// `I` — IOC with no further match available.
    ImmediateOrCancel,
    /// `T` — time in force expired.
    Timeout,
    /// `S` — cancelled by a Nasdaq supervisory terminal.
    Supervisory,
    /// `D` — blocked by a regulatory restriction (e.g. trade-through).
    RegulatoryRestriction,
    /// `Q` — self-match prevention.
    SelfMatchPrevention,
    /// `Z` — cancelled by the system.
    SystemCancel,
    /// `C` — non-bookable cross order that did not execute in the cross.
    CrossCanceled,
    /// `K` — market collars.
    MarketCollars,
    /// `H` — on-open order cancelled because the symbol stayed halted.
    Halted,
    /// `X` — opening price protection threshold.
    OpenProtection,
    /// `E` — DAY order received after the closing cross completed.
    Closed,
    /// `F` — post-only order that would have been price slid for NMS.
    PostOnlyNmsSlide,
    /// `G` — post-only order that would have been slid by a contra displayed order.
    PostOnlyContraSlide,
    /// The spec instructs clients to accept any capital letter here.
    Other(char),
}

impl CancelReason {
    const fn parse(ch: char) -> Self {
        match ch {
            'U' => Self::UserRequested,
            'I' => Self::ImmediateOrCancel,
            'T' => Self::Timeout,
            'S' => Self::Supervisory,
            'D' => Self::RegulatoryRestriction,
            'Q' => Self::SelfMatchPrevention,
            'Z' => Self::SystemCancel,
            'C' => Self::CrossCanceled,
            'K' => Self::MarketCollars,
            'H' => Self::Halted,
            'X' => Self::OpenProtection,
            'E' => Self::Closed,
            'F' => Self::PostOnlyNmsSlide,
            'G' => Self::PostOnlyContraSlide,
            other => Self::Other(other),
        }
    }
}

/// Reject reason (spec §3.10.1). Case matters: `C` is "Nasdaq is closed" while `c` is
/// "Risk: order type restricted".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    RiskRestrictedStock,
    RiskShortSellRestricted,
    RiskOrderTypeRestricted,
    NasdaqClosed,
    RiskExceedsAdvLimit,
    InvalidDisplayType,
    RiskFatFinger,
    Halted,
    FirmNotAuthorized,
    RiskMaxSharesExceeded,
    OutsidePermittedTimes,
    RiskMaxNotionalExceeded,
    InvalidMinimumQuantity,
    NoReferencePrice,
    Other,
    MidpointPegCrossedMarket,
    RiskMarketImpact,
    NotAllowedInThisCross,
    InvalidStock,
    TestMode,
    LooLocPricedTooAggressively,
    RiskAggregateExposureExceeded,
    RetailNotAllowed,
    RiskSymbolMessageRate,
    InvalidMidpointPostOnlyPrice,
    RiskPortMessageRate,
    InvalidPrice,
    RiskDuplicateMessageRate,
    SharesExceedSafetyThreshold,
    /// The spec instructs clients to accept any letter here.
    Unknown(char),
}

impl RejectReason {
    const fn parse(ch: char) -> Self {
        match ch {
            'a' => Self::RiskRestrictedStock,
            'b' => Self::RiskShortSellRestricted,
            'c' => Self::RiskOrderTypeRestricted,
            'C' => Self::NasdaqClosed,
            'd' => Self::RiskExceedsAdvLimit,
            'D' => Self::InvalidDisplayType,
            'e' => Self::RiskFatFinger,
            'H' => Self::Halted,
            'L' => Self::FirmNotAuthorized,
            'm' => Self::RiskMaxSharesExceeded,
            'M' => Self::OutsidePermittedTimes,
            'n' => Self::RiskMaxNotionalExceeded,
            'N' => Self::InvalidMinimumQuantity,
            'o' => Self::NoReferencePrice,
            'O' => Self::Other,
            'q' => Self::MidpointPegCrossedMarket,
            'r' => Self::RiskMarketImpact,
            'R' => Self::NotAllowedInThisCross,
            'S' => Self::InvalidStock,
            'T' => Self::TestMode,
            'u' => Self::LooLocPricedTooAggressively,
            'v' => Self::RiskAggregateExposureExceeded,
            'V' => Self::RetailNotAllowed,
            'w' => Self::RiskSymbolMessageRate,
            'W' => Self::InvalidMidpointPostOnlyPrice,
            'x' => Self::RiskPortMessageRate,
            'X' => Self::InvalidPrice,
            'y' => Self::RiskDuplicateMessageRate,
            'Z' => Self::SharesExceedSafetyThreshold,
            other => Self::Unknown(other),
        }
    }

    /// True when the reject came from Nasdaq's pre-trade risk checks rather than from the
    /// order itself. Worth separating: these signal a firm-level limit, not a bad order.
    pub const fn is_risk_control(self) -> bool {
        matches!(
            self,
            Self::RiskRestrictedStock
                | Self::RiskShortSellRestricted
                | Self::RiskOrderTypeRestricted
                | Self::RiskExceedsAdvLimit
                | Self::RiskFatFinger
                | Self::RiskMaxSharesExceeded
                | Self::RiskMaxNotionalExceeded
                | Self::RiskMarketImpact
                | Self::RiskAggregateExposureExceeded
                | Self::RiskSymbolMessageRate
                | Self::RiskPortMessageRate
                | Self::RiskDuplicateMessageRate
        )
    }
}

/// Reason a trade was broken (spec §3.8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakReason {
    /// `E` — clearly erroneous.
    Erroneous,
    /// `C` — both parties consented.
    Consent,
    /// `S` — broken by a Nasdaq supervisory terminal.
    Supervisory,
    /// `X` — broken by an external third party.
    External,
    Other(char),
}

impl BreakReason {
    const fn parse(ch: char) -> Self {
        match ch {
            'E' => Self::Erroneous,
            'C' => Self::Consent,
            'S' => Self::Supervisory,
            'X' => Self::External,
            other => Self::Other(other),
        }
    }
}

/// An order acknowledgement (`A` Accepted) or replacement acknowledgement (`U` Replaced).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Acknowledgement {
    pub timestamp: u64,
    pub token: OrderToken,
    pub side: Side,
    /// Accepted: total shares accepted. Replaced: shares left exposed after the replace.
    pub shares: u32,
    pub stock: Symbol8,
    /// May be better than entered, if Nasdaq repriced the order.
    pub price: Price,
    pub time_in_force: u32,
    pub firm: Mpid4,
    pub display: Display,
    /// Day-unique order reference number assigned by Nasdaq — the same value that appears
    /// in the ITCH Add Order message for this order.
    pub reference_number: u64,
    pub capacity: Capacity,
    pub iso_eligibility: IsoEligibility,
    pub min_quantity: u32,
    pub cross_type: CrossType,
    pub order_state: OrderState,
    /// Present only on a Replaced message: the token of the order that was replaced.
    pub previous_token: Option<OrderToken>,
    pub bbo_weight_indicator: char,
}

/// `C` — the order was reduced or cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Canceled {
    pub timestamp: u64,
    pub token: OrderToken,
    /// Shares just removed. Incremental, not cumulative.
    pub decrement_shares: u32,
    pub reason: CancelReason,
}

/// `D` — cancelled by self-match prevention, with the trade that was prevented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiqCanceled {
    pub timestamp: u64,
    pub token: OrderToken,
    pub decrement_shares: u32,
    pub reason: CancelReason,
    /// Shares that would have executed. May differ from `decrement_shares` depending on the
    /// AIQ strategy in use.
    pub quantity_prevented_from_trading: u32,
    pub execution_price: Price,
    pub liquidity_flag: char,
    pub aiq_strategy: char,
}

/// `E` — an execution, and `G` — an execution with a reference price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Executed {
    pub timestamp: u64,
    pub token: OrderToken,
    /// Incremental, not cumulative.
    pub executed_shares: u32,
    pub execution_price: Price,
    /// Determines the fee or rebate; see the spec's Liquidity Flag Values table.
    pub liquidity_flag: char,
    /// Shared by both sides of the match, and referenced by any later break.
    pub match_number: u64,
    /// Present only on the `G` variant.
    pub reference_price: Option<Price>,
    /// `I` = intraday indicative value, on the `G` variant.
    pub reference_price_type: Option<char>,
}

/// `B` — a previously reported execution was broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokenTrade {
    pub timestamp: u64,
    pub token: OrderToken,
    pub match_number: u64,
    pub reason: BreakReason,
}

/// `J` — the order or replace could not be accepted. The token is consumed and cannot be
/// reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejected {
    pub timestamp: u64,
    pub token: OrderToken,
    pub reason: RejectReason,
}

/// `T` — the system changed the order's book priority, assigning a new reference number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PriorityUpdate {
    pub timestamp: u64,
    pub token: OrderToken,
    pub price: Price,
    pub display: Display,
    pub reference_number: u64,
}

/// `M` — acknowledgement of a Modify Order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderModified {
    pub timestamp: u64,
    pub token: OrderToken,
    pub side: Side,
    pub shares: u32,
}

/// Any host → client message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outbound {
    /// `S` — start (`S`) or end (`E`) of the OUCH trading day.
    SystemEvent {
        timestamp: u64,
        event: char,
    },
    Accepted(Acknowledgement),
    Replaced(Acknowledgement),
    Canceled(Canceled),
    AiqCanceled(AiqCanceled),
    Executed(Executed),
    BrokenTrade(BrokenTrade),
    Rejected(Rejected),
    /// `P` — a cross order's cancel is deferred until the cross completes.
    CancelPending {
        timestamp: u64,
        token: OrderToken,
    },
    /// `I` — a partial cancel of a cross order was refused during the late period.
    CancelReject {
        timestamp: u64,
        token: OrderToken,
    },
    PriorityUpdate(PriorityUpdate),
    OrderModified(OrderModified),
}

impl Outbound {
    /// The order token this message concerns, where it has one.
    pub fn token(&self) -> Option<OrderToken> {
        Some(match self {
            Self::SystemEvent { .. } => return None,
            Self::Accepted(a) | Self::Replaced(a) => a.token,
            Self::Canceled(c) => c.token,
            Self::AiqCanceled(c) => c.token,
            Self::Executed(e) => e.token,
            Self::BrokenTrade(b) => b.token,
            Self::Rejected(r) => r.token,
            Self::CancelPending { token, .. } | Self::CancelReject { token, .. } => *token,
            Self::PriorityUpdate(p) => p.token,
            Self::OrderModified(m) => m.token,
        })
    }

    /// Nanoseconds past midnight, as stamped by the OUCH host.
    pub fn timestamp(&self) -> u64 {
        match self {
            Self::SystemEvent { timestamp, .. } => *timestamp,
            Self::Accepted(a) | Self::Replaced(a) => a.timestamp,
            Self::Canceled(c) => c.timestamp,
            Self::AiqCanceled(c) => c.timestamp,
            Self::Executed(e) => e.timestamp,
            Self::BrokenTrade(b) => b.timestamp,
            Self::Rejected(r) => r.timestamp,
            Self::CancelPending { timestamp, .. } | Self::CancelReject { timestamp, .. } => {
                *timestamp
            }
            Self::PriorityUpdate(p) => p.timestamp,
            Self::OrderModified(m) => m.timestamp,
        }
    }

    /// Decode one host → client message.
    pub fn decode(buf: &[u8]) -> WireResult<Self> {
        let ty = *buf.first().ok_or(WireError::Truncated {
            at: 0,
            need: 1,
            have: 0,
        })?;

        let mut c = Cursor::at(buf, 1);
        Ok(match ty {
            b'S' => {
                expect_len(buf, ty, 10)?;
                Self::SystemEvent {
                    timestamp: c.be_u64()?,
                    event: c.ascii_char()?,
                }
            }

            b'A' => {
                expect_len(buf, ty, 66)?;
                Self::Accepted(read_ack(&mut c, false)?)
            }

            b'U' => {
                expect_len(buf, ty, 80)?;
                Self::Replaced(read_ack(&mut c, true)?)
            }

            b'C' => {
                expect_len(buf, ty, 28)?;
                Self::Canceled(Canceled {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                    decrement_shares: c.be_u32()?,
                    reason: CancelReason::parse(c.ascii_char()?),
                })
            }

            b'D' => {
                expect_len(buf, ty, 38)?;
                Self::AiqCanceled(AiqCanceled {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                    decrement_shares: c.be_u32()?,
                    reason: CancelReason::parse(c.ascii_char()?),
                    quantity_prevented_from_trading: c.be_u32()?,
                    execution_price: Price::from_price4(c.be_u32()?),
                    liquidity_flag: c.ascii_char()?,
                    aiq_strategy: c.ascii_char()?,
                })
            }

            b'E' => {
                expect_len(buf, ty, 40)?;
                Self::Executed(Executed {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                    executed_shares: c.be_u32()?,
                    execution_price: Price::from_price4(c.be_u32()?),
                    liquidity_flag: c.ascii_char()?,
                    match_number: c.be_u64()?,
                    reference_price: None,
                    reference_price_type: None,
                })
            }

            b'G' => {
                expect_len(buf, ty, 45)?;
                Self::Executed(Executed {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                    executed_shares: c.be_u32()?,
                    execution_price: Price::from_price4(c.be_u32()?),
                    liquidity_flag: c.ascii_char()?,
                    match_number: c.be_u64()?,
                    reference_price: Some(Price::from_price4(c.be_u32()?)),
                    reference_price_type: Some(c.ascii_char()?),
                })
            }

            b'B' => {
                expect_len(buf, ty, 32)?;
                Self::BrokenTrade(BrokenTrade {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                    match_number: c.be_u64()?,
                    reason: BreakReason::parse(c.ascii_char()?),
                })
            }

            b'J' => {
                expect_len(buf, ty, 24)?;
                Self::Rejected(Rejected {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                    reason: RejectReason::parse(c.ascii_char()?),
                })
            }

            b'P' => {
                expect_len(buf, ty, 23)?;
                Self::CancelPending {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                }
            }

            b'I' => {
                expect_len(buf, ty, 23)?;
                Self::CancelReject {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                }
            }

            b'T' => {
                expect_len(buf, ty, 36)?;
                Self::PriorityUpdate(PriorityUpdate {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                    price: Price::from_price4(c.be_u32()?),
                    display: Display::parse(c.ascii_char()?),
                    reference_number: c.be_u64()?,
                })
            }

            b'M' => {
                expect_len(buf, ty, 28)?;
                Self::OrderModified(OrderModified {
                    timestamp: c.be_u64()?,
                    token: read_token(&mut c)?,
                    side: Side::parse(c.ascii_char()?)?,
                    shares: c.be_u32()?,
                })
            }

            other => {
                return Err(WireError::UnknownMessageType {
                    protocol: PROTOCOL,
                    got: other as u16,
                    got_ascii: other as char,
                })
            }
        })
    }

    /// Encode a host → client message. Test and capture-replay tooling only.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(80);
        match self {
            Self::SystemEvent { timestamp, event } => {
                w.u8(b'S').be_u64(*timestamp).ascii_char(*event);
            }
            Self::Accepted(a) => {
                w.u8(b'A');
                write_ack(&mut w, a, false);
            }
            Self::Replaced(a) => {
                w.u8(b'U');
                write_ack(&mut w, a, true);
            }
            Self::Canceled(c) => {
                w.u8(b'C')
                    .be_u64(c.timestamp)
                    .raw(c.token.as_bytes())
                    .be_u32(c.decrement_shares)
                    .ascii_char(cancel_reason_code(c.reason));
            }
            Self::AiqCanceled(c) => {
                w.u8(b'D')
                    .be_u64(c.timestamp)
                    .raw(c.token.as_bytes())
                    .be_u32(c.decrement_shares)
                    .ascii_char('Q')
                    .be_u32(c.quantity_prevented_from_trading)
                    .be_u32(c.execution_price.to_price4())
                    .ascii_char(c.liquidity_flag)
                    .ascii_char(c.aiq_strategy);
            }
            Self::Executed(e) => {
                w.u8(if e.reference_price.is_some() {
                    b'G'
                } else {
                    b'E'
                })
                .be_u64(e.timestamp)
                .raw(e.token.as_bytes())
                .be_u32(e.executed_shares)
                .be_u32(e.execution_price.to_price4())
                .ascii_char(e.liquidity_flag)
                .be_u64(e.match_number);
                if let Some(rp) = e.reference_price {
                    w.be_u32(rp.to_price4())
                        .ascii_char(e.reference_price_type.unwrap_or('I'));
                }
            }
            Self::BrokenTrade(b) => {
                w.u8(b'B')
                    .be_u64(b.timestamp)
                    .raw(b.token.as_bytes())
                    .be_u64(b.match_number)
                    .ascii_char(break_reason_code(b.reason));
            }
            Self::Rejected(r) => {
                w.u8(b'J')
                    .be_u64(r.timestamp)
                    .raw(r.token.as_bytes())
                    .ascii_char(reject_reason_code(r.reason));
            }
            Self::CancelPending { timestamp, token } => {
                w.u8(b'P').be_u64(*timestamp).raw(token.as_bytes());
            }
            Self::CancelReject { timestamp, token } => {
                w.u8(b'I').be_u64(*timestamp).raw(token.as_bytes());
            }
            Self::PriorityUpdate(p) => {
                w.u8(b'T')
                    .be_u64(p.timestamp)
                    .raw(p.token.as_bytes())
                    .be_u32(p.price.to_price4())
                    .ascii_char(p.display.code())
                    .be_u64(p.reference_number);
            }
            Self::OrderModified(m) => {
                w.u8(b'M')
                    .be_u64(m.timestamp)
                    .raw(m.token.as_bytes())
                    .ascii_char(m.side.code())
                    .be_u32(m.shares);
            }
        }
        w.into_vec()
    }
}

// ─── shared helpers ─────────────────────────────────────────────────────────

fn expect_len(buf: &[u8], msg_type: u8, expected: usize) -> WireResult<()> {
    if buf.len() != expected {
        return Err(WireError::LengthMismatch {
            protocol: PROTOCOL,
            msg_type: msg_type as u16,
            declared: buf.len(),
            expected,
        });
    }
    Ok(())
}

fn read_token(c: &mut Cursor<'_>) -> WireResult<OrderToken> {
    let raw = c.take(TOKEN_LEN)?;
    let mut out = [b' '; TOKEN_LEN];
    out.copy_from_slice(raw);
    Ok(OrderToken(out))
}

fn read_ack(c: &mut Cursor<'_>, replaced: bool) -> WireResult<Acknowledgement> {
    let timestamp = c.be_u64()?;
    let token = read_token(c)?;
    let side = Side::parse(c.ascii_char()?)?;
    let shares = c.be_u32()?;
    let stock = Symbol8::from_wire(c.take(8)?);
    let price = Price::from_price4(c.be_u32()?);
    let time_in_force = c.be_u32()?;
    let firm = Mpid4::from_wire(c.take(4)?);
    let display = Display::parse(c.ascii_char()?);
    let reference_number = c.be_u64()?;
    let capacity = Capacity::parse(c.ascii_char()?);
    let iso_eligibility = IsoEligibility::parse(c.ascii_char()?);
    let min_quantity = c.be_u32()?;
    let cross_type = CrossType::parse(c.ascii_char()?);
    let order_state = OrderState::parse(c.ascii_char()?);
    let previous_token = if replaced { Some(read_token(c)?) } else { None };
    let bbo_weight_indicator = c.ascii_char()?;

    Ok(Acknowledgement {
        timestamp,
        token,
        side,
        shares,
        stock,
        price,
        time_in_force,
        firm,
        display,
        reference_number,
        capacity,
        iso_eligibility,
        min_quantity,
        cross_type,
        order_state,
        previous_token,
        bbo_weight_indicator,
    })
}

fn write_ack(w: &mut Writer, a: &Acknowledgement, replaced: bool) {
    w.be_u64(a.timestamp)
        .raw(a.token.as_bytes())
        .ascii_char(a.side.code())
        .be_u32(a.shares)
        .raw(a.stock.as_bytes())
        .be_u32(a.price.to_price4())
        .be_u32(a.time_in_force)
        .raw(a.firm.as_bytes())
        .ascii_char(a.display.code())
        .be_u64(a.reference_number)
        .ascii_char(a.capacity.code())
        .ascii_char(a.iso_eligibility.code())
        .be_u32(a.min_quantity)
        .ascii_char(a.cross_type.code())
        .ascii_char(a.order_state.code());
    if replaced {
        w.raw(a.previous_token.unwrap_or(OrderToken::new("")).as_bytes());
    }
    w.ascii_char(a.bbo_weight_indicator);
}

const fn cancel_reason_code(r: CancelReason) -> char {
    match r {
        CancelReason::UserRequested => 'U',
        CancelReason::ImmediateOrCancel => 'I',
        CancelReason::Timeout => 'T',
        CancelReason::Supervisory => 'S',
        CancelReason::RegulatoryRestriction => 'D',
        CancelReason::SelfMatchPrevention => 'Q',
        CancelReason::SystemCancel => 'Z',
        CancelReason::CrossCanceled => 'C',
        CancelReason::MarketCollars => 'K',
        CancelReason::Halted => 'H',
        CancelReason::OpenProtection => 'X',
        CancelReason::Closed => 'E',
        CancelReason::PostOnlyNmsSlide => 'F',
        CancelReason::PostOnlyContraSlide => 'G',
        CancelReason::Other(c) => c,
    }
}

const fn break_reason_code(r: BreakReason) -> char {
    match r {
        BreakReason::Erroneous => 'E',
        BreakReason::Consent => 'C',
        BreakReason::Supervisory => 'S',
        BreakReason::External => 'X',
        BreakReason::Other(c) => c,
    }
}

const fn reject_reason_code(r: RejectReason) -> char {
    match r {
        RejectReason::RiskRestrictedStock => 'a',
        RejectReason::RiskShortSellRestricted => 'b',
        RejectReason::RiskOrderTypeRestricted => 'c',
        RejectReason::NasdaqClosed => 'C',
        RejectReason::RiskExceedsAdvLimit => 'd',
        RejectReason::InvalidDisplayType => 'D',
        RejectReason::RiskFatFinger => 'e',
        RejectReason::Halted => 'H',
        RejectReason::FirmNotAuthorized => 'L',
        RejectReason::RiskMaxSharesExceeded => 'm',
        RejectReason::OutsidePermittedTimes => 'M',
        RejectReason::RiskMaxNotionalExceeded => 'n',
        RejectReason::InvalidMinimumQuantity => 'N',
        RejectReason::NoReferencePrice => 'o',
        RejectReason::Other => 'O',
        RejectReason::MidpointPegCrossedMarket => 'q',
        RejectReason::RiskMarketImpact => 'r',
        RejectReason::NotAllowedInThisCross => 'R',
        RejectReason::InvalidStock => 'S',
        RejectReason::TestMode => 'T',
        RejectReason::LooLocPricedTooAggressively => 'u',
        RejectReason::RiskAggregateExposureExceeded => 'v',
        RejectReason::RetailNotAllowed => 'V',
        RejectReason::RiskSymbolMessageRate => 'w',
        RejectReason::InvalidMidpointPostOnlyPrice => 'W',
        RejectReason::RiskPortMessageRate => 'x',
        RejectReason::InvalidPrice => 'X',
        RejectReason::RiskDuplicateMessageRate => 'y',
        RejectReason::SharesExceedSafetyThreshold => 'Z',
        RejectReason::Unknown(c) => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str) -> OrderToken {
        OrderToken::new(s)
    }

    #[test]
    fn enter_order_is_forty_nine_bytes_with_documented_offsets() {
        let o = EnterOrder::limit(
            tok("ORD0000001"),
            Side::Buy,
            500,
            "AAPL",
            Price::from_price4(1_234_500),
        );
        let bytes = o.encode();
        assert_eq!(bytes.len(), EnterOrder::LEN);
        assert_eq!(bytes[0], b'O');
        assert_eq!(&bytes[1..15], b"ORD0000001    ");
        assert_eq!(bytes[15], b'B');
        assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 500);
        assert_eq!(&bytes[20..28], b"AAPL    ");
        assert_eq!(
            u32::from_be_bytes(bytes[28..32].try_into().unwrap()),
            1_234_500
        );
        assert_eq!(
            u32::from_be_bytes(bytes[32..36].try_into().unwrap()),
            time_in_force::MARKET_HOURS
        );
    }

    #[test]
    fn every_inbound_message_round_trips() {
        let cases = vec![
            Inbound::Enter(EnterOrder::limit(
                tok("T1"),
                Side::SellShort,
                100,
                "MSFT",
                Price::from_price4(4_205_000),
            )),
            Inbound::Replace(ReplaceOrder {
                existing_token: tok("T1"),
                replacement_token: tok("T2"),
                shares: 600,
                price: Price::from_price4(4_210_000),
                time_in_force: time_in_force::SYSTEM_HOURS,
                display: Display::PostOnly,
                iso_eligibility: IsoEligibility::TradeAtIso,
                min_quantity: 100,
            }),
            Inbound::Cancel(CancelOrder::full(tok("T2"))),
            Inbound::Modify(ModifyOrder {
                token: tok("T2"),
                side: Side::SellShortExempt,
                shares: 300,
            }),
        ];
        for case in cases {
            let bytes = case.encode();
            assert_eq!(Inbound::decode(&bytes).unwrap(), case);
        }
    }

    #[test]
    fn inbound_message_lengths_match_the_specification() {
        assert_eq!(EnterOrder::LEN, 49);
        assert_eq!(ReplaceOrder::LEN, 47);
        assert_eq!(CancelOrder::LEN, 19);
        assert_eq!(ModifyOrder::LEN, 20);
        assert_eq!(CancelOrder::full(tok("x")).encode().len(), 19);
        assert_eq!(
            ModifyOrder {
                token: tok("x"),
                side: Side::Buy,
                shares: 1
            }
            .encode()
            .len(),
            20
        );
    }

    #[test]
    fn every_outbound_message_round_trips_at_its_documented_length() {
        let ack = Acknowledgement {
            timestamp: 34_200_000_000_000,
            token: tok("ORD1"),
            side: Side::Buy,
            shares: 500,
            stock: Symbol8::new("AAPL"),
            price: Price::from_price4(1_234_500),
            time_in_force: time_in_force::MARKET_HOURS,
            firm: Mpid4::new("ABCD"),
            display: Display::Anonymous,
            reference_number: 987_654_321,
            capacity: Capacity::Principal,
            iso_eligibility: IsoEligibility::NotEligible,
            min_quantity: 0,
            cross_type: CrossType::None,
            order_state: OrderState::Live,
            previous_token: None,
            bbo_weight_indicator: '0',
        };
        let mut replaced = ack;
        replaced.previous_token = Some(tok("ORD0"));

        let cases: Vec<(Outbound, usize)> = vec![
            (
                Outbound::SystemEvent {
                    timestamp: 1,
                    event: 'S',
                },
                10,
            ),
            (Outbound::Accepted(ack), 66),
            (Outbound::Replaced(replaced), 80),
            (
                Outbound::Canceled(Canceled {
                    timestamp: 2,
                    token: tok("ORD1"),
                    decrement_shares: 100,
                    reason: CancelReason::UserRequested,
                }),
                28,
            ),
            (
                Outbound::AiqCanceled(AiqCanceled {
                    timestamp: 3,
                    token: tok("ORD1"),
                    decrement_shares: 50,
                    reason: CancelReason::SelfMatchPrevention,
                    quantity_prevented_from_trading: 50,
                    execution_price: Price::from_price4(1_234_500),
                    liquidity_flag: 'A',
                    aiq_strategy: 'D',
                }),
                38,
            ),
            (
                Outbound::Executed(Executed {
                    timestamp: 4,
                    token: tok("ORD1"),
                    executed_shares: 200,
                    execution_price: Price::from_price4(1_234_500),
                    liquidity_flag: 'R',
                    match_number: 55_555,
                    reference_price: None,
                    reference_price_type: None,
                }),
                40,
            ),
            (
                Outbound::Executed(Executed {
                    timestamp: 5,
                    token: tok("ORD1"),
                    executed_shares: 200,
                    execution_price: Price::from_price4(1_234_500),
                    liquidity_flag: 'R',
                    match_number: 55_556,
                    reference_price: Some(Price::from_price4(1_234_000)),
                    reference_price_type: Some('I'),
                }),
                45,
            ),
            (
                Outbound::BrokenTrade(BrokenTrade {
                    timestamp: 6,
                    token: tok("ORD1"),
                    match_number: 55_555,
                    reason: BreakReason::Erroneous,
                }),
                32,
            ),
            (
                Outbound::Rejected(Rejected {
                    timestamp: 7,
                    token: tok("ORD1"),
                    reason: RejectReason::RiskFatFinger,
                }),
                24,
            ),
            (
                Outbound::CancelPending {
                    timestamp: 8,
                    token: tok("ORD1"),
                },
                23,
            ),
            (
                Outbound::CancelReject {
                    timestamp: 9,
                    token: tok("ORD1"),
                },
                23,
            ),
            (
                Outbound::PriorityUpdate(PriorityUpdate {
                    timestamp: 10,
                    token: tok("ORD1"),
                    price: Price::from_price4(1_234_600),
                    display: Display::Anonymous,
                    reference_number: 42,
                }),
                36,
            ),
            (
                Outbound::OrderModified(OrderModified {
                    timestamp: 11,
                    token: tok("ORD1"),
                    side: Side::SellShort,
                    shares: 300,
                }),
                28,
            ),
        ];

        for (msg, len) in cases {
            let bytes = msg.encode();
            assert_eq!(bytes.len(), len, "wire length for {msg:?}");
            assert_eq!(Outbound::decode(&bytes).unwrap(), msg);
        }
    }

    #[test]
    fn accepted_carries_the_reference_number_that_links_to_itch() {
        let bytes = Outbound::Accepted(Acknowledgement {
            timestamp: 1,
            token: tok("A"),
            side: Side::Buy,
            shares: 1,
            stock: Symbol8::new("A"),
            price: Price::ZERO,
            time_in_force: 0,
            firm: Mpid4::BLANK,
            display: Display::Anonymous,
            reference_number: 0x0102_0304_0506_0708,
            capacity: Capacity::Agency,
            iso_eligibility: IsoEligibility::NotEligible,
            min_quantity: 0,
            cross_type: CrossType::None,
            order_state: OrderState::Live,
            previous_token: None,
            bbo_weight_indicator: ' ',
        })
        .encode();
        // Reference Number sits at offset 49 in the Accepted message.
        assert_eq!(
            u64::from_be_bytes(bytes[49..57].try_into().unwrap()),
            0x0102_0304_0506_0708
        );
    }

    #[test]
    fn order_dead_state_means_no_further_messages() {
        let mut ack = Acknowledgement {
            timestamp: 1,
            token: tok("A"),
            side: Side::Buy,
            shares: 100,
            stock: Symbol8::new("AAPL"),
            price: Price::ZERO,
            time_in_force: time_in_force::IOC,
            firm: Mpid4::BLANK,
            display: Display::Anonymous,
            reference_number: 1,
            capacity: Capacity::Agency,
            iso_eligibility: IsoEligibility::NotEligible,
            min_quantity: 0,
            cross_type: CrossType::None,
            order_state: OrderState::Dead,
            previous_token: None,
            bbo_weight_indicator: ' ',
        };
        let bytes = Outbound::Accepted(ack).encode();
        let Outbound::Accepted(decoded) = Outbound::decode(&bytes).unwrap() else {
            panic!()
        };
        assert_eq!(decoded.order_state, OrderState::Dead);
        ack.order_state = OrderState::Live;
        assert_ne!(decoded.order_state, ack.order_state);
    }

    #[test]
    fn validation_catches_orders_nasdaq_would_reject() {
        let base = || {
            EnterOrder::limit(
                tok("T"),
                Side::Buy,
                100,
                "AAPL",
                Price::from_price4(1_000_000),
            )
        };

        let mut o = base();
        o.shares = 1_000_000;
        assert!(o.validate().is_err(), "shares must be < 1,000,000");

        let mut o = base();
        o.shares = 0;
        assert!(o.validate().is_err(), "shares must be > 0");

        // Symbol width is now enforced where a caller-supplied string enters the field: the
        // stored form is fixed width, so it cannot be over-long by the time validate() runs.
        assert!(
            Symbol8::try_new("TOOLONGSYM").is_err(),
            "symbol exceeds the 8-byte field"
        );
        let mut o = base();
        o.stock = Symbol8::BLANK;
        assert!(o.validate().is_err(), "blank symbol");

        let mut o = base();
        o.time_in_force = 100_000;
        assert!(o.validate().is_err(), "time in force above 99,999");

        let mut o = base();
        o.min_quantity = 500;
        assert!(o.validate().is_err(), "min quantity above order quantity");

        let mut o = base();
        o.cross_type = CrossType::HaltIpo;
        assert!(
            o.validate().is_err(),
            "halt cross needs the market sentinel"
        );
        o.price = Price::from_price4(Price::OUCH_MARKET_SENTINEL_RAW);
        assert!(o.validate().is_ok());

        assert!(base().validate().is_ok());
    }

    #[test]
    fn tokens_longer_than_the_field_are_rejected_not_silently_truncated() {
        assert!(OrderToken::try_new("123456789012345").is_err());
        assert!(OrderToken::try_new("12345678901234").is_ok());
        // The lenient constructor truncates, which is exactly the collision risk that
        // makes try_new the right choice on the order path.
        assert_eq!(
            OrderToken::new("123456789012345").as_str(),
            "12345678901234"
        );
    }

    #[test]
    fn tokens_reject_characters_outside_the_documented_set() {
        assert!(OrderToken::try_new("ORD-001").is_err());
        assert!(OrderToken::try_new("ORD 001").is_ok());
    }

    #[test]
    fn reject_reasons_distinguish_case() {
        // 'C' is "Nasdaq is closed"; 'c' is a risk-control rejection.
        let closed = Outbound::Rejected(Rejected {
            timestamp: 1,
            token: tok("T"),
            reason: RejectReason::NasdaqClosed,
        });
        let risk = Outbound::Rejected(Rejected {
            timestamp: 1,
            token: tok("T"),
            reason: RejectReason::RiskOrderTypeRestricted,
        });
        assert_ne!(closed.encode(), risk.encode());
        assert!(!RejectReason::NasdaqClosed.is_risk_control());
        assert!(RejectReason::RiskOrderTypeRestricted.is_risk_control());
        assert!(RejectReason::RiskFatFinger.is_risk_control());
    }

    #[test]
    fn unknown_cancel_reasons_are_carried_through_rather_than_failing() {
        // The spec tells clients to expect additions and support every capital letter.
        let mut bytes = Outbound::Canceled(Canceled {
            timestamp: 1,
            token: tok("T"),
            decrement_shares: 1,
            reason: CancelReason::UserRequested,
        })
        .encode();
        bytes[27] = b'W';
        let Outbound::Canceled(c) = Outbound::decode(&bytes).unwrap() else {
            panic!()
        };
        assert_eq!(c.reason, CancelReason::Other('W'));
    }

    #[test]
    fn a_wrong_length_outbound_message_is_rejected() {
        let mut bytes = Outbound::CancelPending {
            timestamp: 1,
            token: tok("T"),
        }
        .encode();
        bytes.push(0);
        assert!(matches!(
            Outbound::decode(&bytes),
            Err(WireError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn market_orders_for_a_cross_use_the_sentinel_price() {
        let mut o = EnterOrder::limit(
            tok("X"),
            Side::Buy,
            100,
            "AAPL",
            Price::from_price4(Price::OUCH_MARKET_SENTINEL_RAW),
        );
        o.cross_type = CrossType::Opening;
        let bytes = o.encode();
        assert_eq!(
            u32::from_be_bytes(bytes[28..32].try_into().unwrap()),
            0x7FFF_FFFF
        );
        assert_eq!(EnterOrder::decode(&bytes).unwrap(), o);
    }

    #[test]
    fn ioc_is_a_time_in_force_of_zero() {
        let mut o = EnterOrder::limit(
            tok("X"),
            Side::Buy,
            100,
            "AAPL",
            Price::from_price4(1_000_000),
        );
        o.time_in_force = time_in_force::IOC;
        let bytes = o.encode();
        assert_eq!(u32::from_be_bytes(bytes[32..36].try_into().unwrap()), 0);
        assert!(o.validate().is_ok());
    }
}
