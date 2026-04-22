// Bloomberg NeurIPS 2025: Interval Regression Learning
// Learn from interval targets where only bid/ask intervals are available

pub fn interval_loss(prediction: f64, lower: f64, upper: f64) -> f64 {
    if prediction < lower {
        (lower - prediction).powi(2)
    } else if prediction > upper {
        (prediction - upper).powi(2)
    } else {
        0.0
    }
}

pub fn interval_loss_grad(prediction: f64, lower: f64, upper: f64) -> f64 {
    if prediction < lower {
        -2.0 * (lower - prediction)
    } else if prediction > upper {
        2.0 * (prediction - upper)
    } else {
        0.0
    }
}

#[derive(Debug, Clone)]
pub struct IntervalSample {
    pub features: Vec<f64>,
    pub bid: f64,
    pub ask: f64,
    pub true_mid: Option<f64>,
}

impl IntervalSample {
    pub fn naive_mid(&self) -> f64 {
        (self.bid + self.ask) / 2.0
    }
}

#[derive(Debug, Clone)]
pub struct IntervalNet {
    pub input_size: usize,
    pub hidden_size: usize,
    pub w1: Vec<Vec<f64>>,
    pub b1: Vec<f64>,
    pub w2: Vec<f64>,
    pub b2: f64,
    pub feature_stats: Option<Vec<(f64, f64)>>,
    pub output_stats: Option<(f64, f64)>,
}

impl IntervalNet {
    pub fn new(input_size: usize, hidden_size: usize, seed: u64) -> Self {
        let mut rng = SimpleRng::new(seed);
        let scale1 = (2.0 / (input_size + hidden_size) as f64).sqrt();
        let scale2 = (2.0 / hidden_size as f64).sqrt();

        let w1 = (0..hidden_size)
            .map(|_| (0..input_size).map(|_| rng.randn() * scale1).collect())
            .collect();
        let b1 = vec![0.0; hidden_size];
        let w2 = (0..hidden_size).map(|_| rng.randn() * scale2).collect();

        Self {
            input_size,
            hidden_size,
            w1,
            b1,
            w2,
            b2: 0.0,
            feature_stats: None,
            output_stats: None,
        }
    }

    pub fn predict(&self, features: &[f64]) -> f64 {
        let x = self.normalize_features(features);

        let hidden: Vec<f64> = (0..self.hidden_size)
            .map(|j| {
                let z: f64 = x
                    .iter()
                    .enumerate()
                    .map(|(i, &xi)| self.w1[j][i] * xi)
                    .sum::<f64>()
                    + self.b1[j];
                z.max(0.0)
            })
            .collect();

        let raw: f64 = hidden
            .iter()
            .enumerate()
            .map(|(j, &hj)| self.w2[j] * hj)
            .sum::<f64>()
            + self.b2;

        self.denormalize_output(raw)
    }

    pub fn train_batch(&mut self, batch: &[IntervalSample], learning_rate: f64) -> f64 {
        let mut total_loss = 0.0;

        let mut dw1 = vec![vec![0.0; self.input_size]; self.hidden_size];
        let mut db1 = vec![0.0; self.hidden_size];
        let mut dw2 = vec![0.0; self.hidden_size];
        let mut db2 = 0.0;

        for sample in batch {
            let x = self.normalize_features(&sample.features);
            let bid_n = self.normalize_output(sample.bid);
            let ask_n = self.normalize_output(sample.ask);

            let pre_act: Vec<f64> = (0..self.hidden_size)
                .map(|j| {
                    x.iter()
                        .enumerate()
                        .map(|(i, &xi)| self.w1[j][i] * xi)
                        .sum::<f64>()
                        + self.b1[j]
                })
                .collect();
            let hidden: Vec<f64> = pre_act.iter().map(|&z| z.max(0.0)).collect();
            let raw_out: f64 = hidden
                .iter()
                .enumerate()
                .map(|(j, &hj)| self.w2[j] * hj)
                .sum::<f64>()
                + self.b2;

            let loss = interval_loss(raw_out, bid_n, ask_n);
            let d_out = interval_loss_grad(raw_out, bid_n, ask_n);
            total_loss += loss;

            for j in 0..self.hidden_size {
                dw2[j] += d_out * hidden[j];
            }
            db2 += d_out;

            for j in 0..self.hidden_size {
                let d_hidden = d_out * self.w2[j] * if pre_act[j] > 0.0 { 1.0 } else { 0.0 };
                for i in 0..self.input_size {
                    dw1[j][i] += d_hidden * x[i];
                }
                db1[j] += d_hidden;
            }
        }

        let n = batch.len() as f64;
        for j in 0..self.hidden_size {
            for i in 0..self.input_size {
                self.w1[j][i] -= learning_rate * dw1[j][i] / n;
            }
            self.b1[j] -= learning_rate * db1[j] / n;
            self.w2[j] -= learning_rate * dw2[j] / n;
        }
        self.b2 -= learning_rate * db2 / n;

        total_loss / n
    }

    pub fn fit(
        &mut self,
        data: &[IntervalSample],
        epochs: usize,
        learning_rate: f64,
        batch_size: usize,
    ) -> Vec<f64> {
        self.compute_normalization_stats(data);

        let mut epoch_losses = Vec::with_capacity(epochs);

        for _epoch in 0..epochs {
            let mut epoch_loss = 0.0;
            let mut n_batches = 0;

            for chunk in data.chunks(batch_size) {
                epoch_loss += self.train_batch(chunk, learning_rate);
                n_batches += 1;
            }

            let avg_loss = epoch_loss / n_batches as f64;
            epoch_losses.push(avg_loss);
        }

        epoch_losses
    }

    pub fn evaluate(&self, test: &[IntervalSample]) -> EvalMetrics {
        let mut mae = 0.0;
        let mut inside_count = 0;
        let mut n_with_truth = 0;

        for s in test {
            let pred = self.predict(&s.features);
            let in_interval = pred >= s.bid && pred <= s.ask;
            if in_interval {
                inside_count += 1;
            }

            if let Some(true_mid) = s.true_mid {
                mae += (pred - true_mid).abs();
                n_with_truth += 1;
            }
        }

        EvalMetrics {
            interval_coverage_pct: inside_count as f64 / test.len() as f64 * 100.0,
            mae_vs_true_mid: if n_with_truth > 0 {
                Some(mae / n_with_truth as f64)
            } else {
                None
            },
            n_samples: test.len(),
        }
    }

    fn compute_normalization_stats(&mut self, data: &[IntervalSample]) {
        if data.is_empty() {
            return;
        }
        let n = data.len() as f64;
        let dim = self.input_size;

        let means: Vec<f64> = (0..dim)
            .map(|i| data.iter().map(|s| s.features[i]).sum::<f64>() / n)
            .collect();
        let stds: Vec<f64> = (0..dim)
            .map(|i| {
                let m = means[i];
                let var = data
                    .iter()
                    .map(|s| (s.features[i] - m).powi(2))
                    .sum::<f64>()
                    / n;
                var.sqrt().max(1e-8)
            })
            .collect();

        self.feature_stats = Some(means.into_iter().zip(stds).collect());

        let mids: Vec<f64> = data.iter().map(|s| s.naive_mid()).collect();
        let out_mean = mids.iter().sum::<f64>() / n;
        let out_std = (mids.iter().map(|m| (m - out_mean).powi(2)).sum::<f64>() / n)
            .sqrt()
            .max(1e-8);
        self.output_stats = Some((out_mean, out_std));
    }

    fn normalize_features(&self, features: &[f64]) -> Vec<f64> {
        if let Some(stats) = &self.feature_stats {
            features
                .iter()
                .zip(stats.iter())
                .map(|(&f, (m, s))| (f - m) / s)
                .collect()
        } else {
            features.to_vec()
        }
    }

    fn normalize_output(&self, price: f64) -> f64 {
        if let Some((m, s)) = self.output_stats {
            (price - m) / s
        } else {
            price
        }
    }

    fn denormalize_output(&self, normalized: f64) -> f64 {
        if let Some((m, s)) = self.output_stats {
            normalized * s + m
        } else {
            normalized
        }
    }
}

pub struct EvalMetrics {
    pub interval_coverage_pct: f64,
    pub mae_vs_true_mid: Option<f64>,
    pub n_samples: usize,
}

pub fn extract_bond_features(
    duration: f64,
    convexity: f64,
    credit_spread_bps: f64,
    yield_to_maturity: f64,
    time_to_maturity: f64,
    coupon_rate: f64,
    credit_rating_numeric: f64,
    sector_encoding: f64,
    bid_ask_spread_bps: f64,
    implied_liquidity_score: f64,
) -> Vec<f64> {
    vec![
        duration,
        convexity / 100.0,
        credit_spread_bps / 100.0,
        yield_to_maturity,
        time_to_maturity,
        coupon_rate,
        credit_rating_numeric / 7.0,
        sector_encoding,
        bid_ask_spread_bps / 100.0,
        implied_liquidity_score,
    ]
}

pub struct SimpleRng {
    pub state: u64,
}
impl SimpleRng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }
    pub fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }
    pub fn randn(&mut self) -> f64 {
        let u1 = (self.next_u64() as f64 + 1.0) / (u64::MAX as f64 + 2.0);
        let u2 = (self.next_u64() as f64 + 1.0) / (u64::MAX as f64 + 2.0);
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}
