use pricing::sabr::{SabrParams, implied_vol};

#[test]
fn test_sabr_atm_implied_vol() {
    let p = SabrParams {
        alpha: 0.20,
        beta: 0.5,
        rho: -0.5,
        nu: 0.4,
    };

    let forward = 100.0;
    let strike = 100.0;
    let tte = 1.0;

    // SABR formula for ATM (f=k) is roughly alpha / f^(1-beta) * (1 + ... * tte)
    let iv = implied_vol(&p, forward, strike, tte);

    // For alpha=0.2, beta=0.5, f=100 -> f^(0.5) = 10. alpha / 10 = 0.02.
    // So ATM normal-ish vol is very low, or lognormal vol depends on beta. Wait, the implementation returns lognormal IV relative to forward.
    assert!(iv > 0.01 && iv < 0.5, "SABR ATM vol out of bounds: {}", iv);
}

#[test]
fn test_sabr_smile_shape() {
    let p = SabrParams {
        alpha: 0.30,
        beta: 1.0, // Lognormal SABR
        rho: -0.6, // Negative skew
        nu: 0.5,   // High vol of vol -> pronounced smile
    };

    let forward = 100.0;
    let tte = 1.0;

    let atm_vol = implied_vol(&p, forward, 100.0, tte);
    let otm_put_vol = implied_vol(&p, forward, 90.0, tte);
    let otm_call_vol = implied_vol(&p, forward, 110.0, tte);

    // Negative rho implies downward sloping skew (OTM puts have higher IV than OTM calls)
    assert!(
        otm_put_vol > atm_vol,
        "OTM put vol should be > ATM vol for negative rho"
    );
    assert!(
        otm_put_vol > otm_call_vol,
        "Negative skew means put vol > call vol"
    );
}
