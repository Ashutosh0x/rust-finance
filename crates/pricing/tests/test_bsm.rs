use pricing::bsm::{BsmParams, implied_vol, price};

#[test]
fn test_bsm_call_put_parity() {
    let params = BsmParams {
        spot: 100.0,
        strike: 100.0,
        rate: 0.05,
        dividend_yield: 0.0,
        volatility: 0.2,
        time_to_expiry: 1.0,
    };

    let result = price(&params).expect("Failed to price BSM");

    // Known answer for Call: ~10.4505
    // Known answer for Put: ~5.5735
    assert!(
        (result.call_price - 10.4505).abs() < 1e-3,
        "Call price deviation: {}",
        result.call_price
    );
    assert!(
        (result.put_price - 5.5735).abs() < 1e-3,
        "Put price deviation: {}",
        result.put_price
    );

    // Call-Put Parity: C - P = S - K * exp(-rT)
    let c_minus_p = result.call_price - result.put_price;
    let s_minus_k_disc = params.spot - params.strike * (-params.rate * params.time_to_expiry).exp();

    assert!(
        (c_minus_p - s_minus_k_disc).abs() < 1e-5,
        "Call-Put parity violated"
    );
}

#[test]
fn test_bsm_implied_volatility() {
    let target_vol = 0.25;
    let params = BsmParams {
        spot: 150.0,
        strike: 145.0,
        rate: 0.02,
        dividend_yield: 0.01,
        volatility: target_vol,
        time_to_expiry: 0.5,
    };

    // Forward pricing
    let result = price(&params).expect("Failed forward pricing");
    let call_mkt = result.call_price;

    // Reverse IV
    let iv = implied_vol(
        call_mkt,
        params.spot,
        params.strike,
        params.rate,
        params.dividend_yield,
        params.time_to_expiry,
        true,
        100,
        1e-4,
    )
    .expect("IV failed to converge");

    // Must round trip within 1 bb
    assert!(
        (iv - target_vol).abs() < 1e-3,
        "IV failed to match target. Got {}, expected {}",
        iv,
        target_vol
    );
}
