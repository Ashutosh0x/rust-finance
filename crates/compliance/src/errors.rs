use thiserror::Error;

#[derive(Error, Debug)]
pub enum ComplianceError {
    #[error("Fat finger error: {0}")]
    FatFinger(String),
    #[error("Daily limit breached: {0}")]
    DailyLimitBreached(String),
    #[error("Rate limit breached: {0}")]
    RateLimitBreached(String),
}
