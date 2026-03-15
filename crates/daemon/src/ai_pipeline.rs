use common::events::BotEvent;

pub enum ModelTier {
    Opus46,
    Sonnet46,
    NoModel,
}

#[derive(Default, Debug, Clone)]
pub struct SignalContext {
    pub is_tick_level: bool,
    pub signal_confidence: f64,
    pub is_drift_check: bool,
    pub is_earnings_event: bool,
    pub is_fomc_window: bool,
    pub gex_flipped_sign: bool,
    pub notional_usd: f64,
    pub is_daily_brief_time: bool,
    pub is_param_tune_window: bool,
}

pub fn route_event(event: &BotEvent, context: &SignalContext) -> ModelTier {
    // Pure Rust — never call an LLM
    if context.is_tick_level {
        return ModelTier::NoModel;
    }

    // Sonnet for fast, routine work (runs continuously)
    // Here we map BotEvent to logic assuming it's a proxy for MarketEvent::News
    if let BotEvent::Feed(feed) = event {
        if feed.contains("NEWS") {
            return ModelTier::Sonnet46;
        }
    }

    if context.signal_confidence < 0.70 {
        return ModelTier::Sonnet46;
    }
    if context.is_drift_check {
        return ModelTier::Sonnet46;
    }

    // Opus 4.6 for high-stakes decisions only
    if context.is_earnings_event {
        return ModelTier::Opus46;
    }
    if context.is_fomc_window {
        return ModelTier::Opus46;
    }
    if context.gex_flipped_sign {
        return ModelTier::Opus46;
    }
    if context.signal_confidence > 0.85 {
        return ModelTier::Opus46;
    }
    if context.notional_usd > 5_000.0 {
        return ModelTier::Opus46;
    } // risk veto
    if context.is_daily_brief_time {
        return ModelTier::Opus46;
    }
    if context.is_param_tune_window {
        return ModelTier::Opus46;
    }

    ModelTier::Sonnet46 // default to cheap
}
