-- ============================================================
-- RustForge TimescaleDB Schema Migration v001
-- ============================================================

CREATE EXTENSION IF NOT EXISTS timescaledb CASCADE;

-- ── Market Events (Hypertable) ───────────────────────────────
CREATE TABLE IF NOT EXISTS market_events (
    time        TIMESTAMPTZ     NOT NULL,
    symbol      TEXT            NOT NULL,
    price       DOUBLE PRECISION NOT NULL,
    volume      DOUBLE PRECISION NOT NULL,
    bid         DOUBLE PRECISION,
    ask         DOUBLE PRECISION,
    source      TEXT            NOT NULL  -- 'finnhub' | 'alpaca'
);

SELECT create_hypertable('market_events', 'time', if_not_exists => TRUE);
SELECT add_retention_policy('market_events', INTERVAL '90 days', if_not_exists => TRUE);
SELECT add_compression_policy('market_events', INTERVAL '7 days', if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_market_events_symbol_time
    ON market_events (symbol, time DESC);

-- ── AI Signals ───────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS ai_signals (
    time        TIMESTAMPTZ     NOT NULL,
    signal_id   UUID            DEFAULT gen_random_uuid(),
    analyst     TEXT            NOT NULL,  -- 'dexter' | 'mirofish'
    symbol      TEXT            NOT NULL,
    action      TEXT            NOT NULL,  -- 'BUY' | 'SELL' | 'HOLD'
    confidence  DOUBLE PRECISION NOT NULL CHECK (confidence BETWEEN 0.0 AND 1.0),
    rationale   TEXT,
    raw_tokens  INTEGER
);

SELECT create_hypertable('ai_signals', 'time', if_not_exists => TRUE);

CREATE INDEX IF NOT EXISTS idx_ai_signals_symbol_time
    ON ai_signals (symbol, time DESC);

-- ── Orders ───────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS orders (
    time            TIMESTAMPTZ     NOT NULL,
    order_id        UUID            DEFAULT gen_random_uuid() PRIMARY KEY,
    symbol          TEXT            NOT NULL,
    side            TEXT            NOT NULL CHECK (side IN ('BUY', 'SELL')),
    quantity        DOUBLE PRECISION NOT NULL,
    price           DOUBLE PRECISION,
    order_type      TEXT            NOT NULL,  -- 'MARKET' | 'LIMIT'
    status          TEXT            NOT NULL DEFAULT 'PENDING',
    fill_price      DOUBLE PRECISION,
    fill_qty        DOUBLE PRECISION,
    solana_sig      TEXT,
    rpc_node        TEXT,
    latency_us      BIGINT,
    created_at      TIMESTAMPTZ     DEFAULT NOW(),
    updated_at      TIMESTAMPTZ     DEFAULT NOW()
);

SELECT create_hypertable('orders', 'time', if_not_exists => TRUE);

-- ── Portfolio Snapshots ──────────────────────────────────────
CREATE TABLE IF NOT EXISTS portfolio_snapshots (
    time        TIMESTAMPTZ     NOT NULL,
    total_value DOUBLE PRECISION NOT NULL,
    cash        DOUBLE PRECISION NOT NULL,
    pnl_day     DOUBLE PRECISION,
    pnl_total   DOUBLE PRECISION,
    drawdown    DOUBLE PRECISION,
    var_95      DOUBLE PRECISION,
    positions   JSONB
);

SELECT create_hypertable('portfolio_snapshots', 'time', if_not_exists => TRUE);

-- ── Risk Events ──────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS risk_events (
    time        TIMESTAMPTZ     NOT NULL,
    event_type  TEXT            NOT NULL,  -- 'VAR_BREACH' | 'DRAWDOWN_HALT' | 'KILL_SWITCH'
    symbol      TEXT,
    threshold   DOUBLE PRECISION,
    actual      DOUBLE PRECISION,
    message     TEXT,
    resolved_at TIMESTAMPTZ
);

SELECT create_hypertable('risk_events', 'time', if_not_exists => TRUE);

-- ── Relay Latency Log ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS relay_latency (
    time        TIMESTAMPTZ     NOT NULL,
    rpc_node    TEXT            NOT NULL,
    latency_us  BIGINT          NOT NULL,
    success     BOOLEAN         NOT NULL,
    error_msg   TEXT
);

SELECT create_hypertable('relay_latency', 'time', if_not_exists => TRUE);
SELECT add_retention_policy('relay_latency', INTERVAL '7 days', if_not_exists => TRUE);

-- ── Continuous Aggregate: 1-min OHLCV ────────────────────────
CREATE MATERIALIZED VIEW IF NOT EXISTS ohlcv_1min
WITH (timescaledb.continuous) AS
SELECT
    time_bucket('1 minute', time) AS bucket,
    symbol,
    FIRST(price, time)  AS open,
    MAX(price)          AS high,
    MIN(price)          AS low,
    LAST(price, time)   AS close,
    SUM(volume)         AS volume
FROM market_events
GROUP BY bucket, symbol
WITH NO DATA;

SELECT add_continuous_aggregate_policy('ohlcv_1min',
    start_offset => INTERVAL '1 hour',
    end_offset   => INTERVAL '1 minute',
    schedule_interval => INTERVAL '1 minute',
    if_not_exists => TRUE
);
