use pricing::hull_white::{
    HullWhiteParams, HwBondOptionInput, TreeParams, YieldCurve, hw_bond_option, hw_bond_price,
    trinomial_tree_price,
};

#[test]
fn test_hw_bond_discount_consistency() {
    let p = HullWhiteParams {
        mean_reversion: 0.10,
        volatility: 0.01,
    };

    // Flat 5% yield curve
    let curve = YieldCurve {
        points: vec![(0.0, 0.05), (1.0, 0.05), (5.0, 0.05), (10.0, 0.05)],
    };

    // Forward bond price should roughly match the ratio of discount factors P(0, T)/P(0, t)
    // with convexity adjustment zeroed out if vol = 0
    let price_5y = hw_bond_price(&p, &curve, 0.05, 0.0, 5.0);
    let expected_5y = (-0.05 * 5.0_f64).exp();
    assert!(
        (price_5y - expected_5y).abs() < 1e-4,
        "HW zero-time bond price violated flat curve expectation"
    );
}

#[test]
fn test_hw_european_swaption_approx() {
    let p = HullWhiteParams {
        mean_reversion: 0.05,
        volatility: 0.01,
    };
    let curve = YieldCurve {
        points: vec![(0.0, 0.03), (10.0, 0.03)],
    };

    let input = HwBondOptionInput {
        option_maturity: 1.0,
        bond_maturity: 2.0,
        strike: 0.96, // roughly ATM for 1Y into 1Y bond at 3%
        face_value: 1.0,
        is_call: true,
        current_rate: 0.03,
    };

    let opt_price = hw_bond_option(&p, &curve, &input);
    // Non-zero time value expected
    assert!(opt_price > 0.001);
}

#[test]
fn test_hw_trinomial_tree_bsm_convergence() {
    let p = TreeParams {
        spot: 100.0,
        strike: 100.0,
        rate: 0.05,
        dividend_yield: 0.0,
        volatility: 0.20,
        time_to_expiry: 1.0,
        steps: 100,
        is_call: true,
        is_american: false, // European
    };

    let tree_price = trinomial_tree_price(&p);

    // BSM known answer for these params is ~10.4505
    assert!(
        (tree_price - 10.4505).abs() < 0.05,
        "Tree price {} deviates too much from BSM 10.4505",
        tree_price
    );
}
