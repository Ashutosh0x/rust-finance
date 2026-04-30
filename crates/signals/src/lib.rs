#![forbid(unsafe_code)]

// ─── Existing modules ────────────────────────────────────────────
pub mod gex;
pub mod indicators;
pub mod microstructure;

// ─── 2026 Quant Alpha Library ────────────────────────────────────
// Based on: Oxford MLOFI 2019, Avellaneda-Stoikov 2008, RegimeFolio 2025,
//           Barzykin 2025, PolySwarm Apr 2026, Chain-of-Alpha 2025,
//           AlphaForgeBench 2026, Cont-Kukanov-Stoikov 2014,
//           Easley-López de Prado-O'Hara 2012, Almgren-Chriss 2001

/// Multi-level microprice fair value (MLOFI, Oxford 2019).
pub mod microprice_ml;

/// Adverse selection & toxicity detection (Barzykin 2025, Crypto 2026).
pub mod adverse_selection;

/// Two-state volatility regime classifier (RegimeFolio 2025).
pub mod regime;

/// Kelly Criterion position sizing with Bayesian shrinkage (PolySwarm 2026).
pub mod kelly;

/// Alpha signal health / IC decay monitor (AlphaForgeBench 2026).
pub mod alpha_health;

/// IC-weighted composite signal generator (Chain-of-Alpha 2025).
pub mod composite;

/// 2026-enhanced Avellaneda-Stoikov quoting engine (all modules integrated).
pub mod quoting_engine;
