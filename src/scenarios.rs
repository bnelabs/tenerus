//! Self-calibrating multivariable price scenario engine with forward-only walk-forward replay.
//!
//! Combines four strictly causal models using out-of-sample pinball-loss Hedge weighting
//! and split-conformal prediction interval calibration:
//!   1. Kernel- and recency-weighted historical analogues
//!   2. Online L2-regularized quantile regression (multivariable)
//!   3. RiskMetrics EWMA fat-tailed parametric volatility fan
//!   4. Unconditional non-overlapping historical baseline
use crate::model::{demo_market, valid_ticker};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

const DAY: i64 = 86_400;
const LOOKBACK: usize = 60;
const MATCH_TARGET: usize = 24;
const MATCH_MINIMUM: usize = 10;
const REPLAY_LIMIT: usize = 36;
pub const HORIZONS: [usize; 3] = [5, 20, 60];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Depth {
    #[default]
    Beginner,
    Expert,
    MarketGod,
}
impl Depth {
    pub const ALL: [Self; 3] = [Self::Beginner, Self::Expert, Self::MarketGod];
    pub fn label(self) -> &'static str {
        match self {
            Self::Beginner => "Beginner",
            Self::Expert => "Expert",
            Self::MarketGod => "Market god",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Close {
    pub time: i64,
    pub price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
}
impl Close {
    pub fn new(time: i64, price: f64) -> Self {
        Self {
            time,
            price,
            high: None,
            low: None,
            volume: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct History {
    pub ticker: String,
    pub currency: String,
    pub fetched: i64,
    pub sample: bool,
    pub closes: Vec<Close>,
}
impl History {
    pub fn valid(&self) -> bool {
        valid_ticker(&self.ticker)
            && (1..=4).contains(&self.currency.len())
            && self.currency.bytes().all(|c| c.is_ascii_alphabetic())
            && self.fetched > 0
            && self.closes.len() <= 4_000
            && self.closes.iter().all(|bar| {
                bar.time > 0 && bar.time <= self.fetched && bar.price.is_finite() && bar.price > 0.
            })
            && self.closes.windows(2).all(|w| w[0].time < w[1].time)
    }

    pub fn sample(ticker: &str) -> Option<Self> {
        let quote = demo_market(ticker, 0)?.quote;
        let mut dates = (0..3_650)
            .map(|age| quote.market_time - age * DAY)
            .filter(|t| ticker.ends_with("-USD") || (t.div_euclid(DAY) + 3).rem_euclid(7) < 5)
            .collect::<Vec<_>>();
        dates.reverse();
        let mut noise = ticker
            .bytes()
            .fold(17u64, |n, c| n.wrapping_mul(31).wrapping_add(u64::from(c)));
        let mut log_price: f64 = 3.;
        let len = dates.len();
        let mut closes = dates
            .into_iter()
            .enumerate()
            .map(|(i, time)| {
                noise = noise.wrapping_mul(6364136223846793005).wrapping_add(1);
                let shock = ((noise >> 32) as f64 / u32::MAX as f64 - 0.5) * 2.;
                let phase = i as f64 / 58.;
                let (trend, swing) = if i + 38 >= len {
                    (0.0027, 0.018)
                } else {
                    (
                        0.00065 + 0.0019 * phase.sin(),
                        0.018 + 0.010 * (phase * 0.6).cos(),
                    )
                };
                log_price += trend + swing * shock;
                let price = log_price.exp();
                let spread = (swing * (0.45 + shock.abs() * 0.65)).max(0.004);
                let volume = 25_000_000.0 * (0.75 + 0.55 * shock.abs() + 0.2 * phase.sin().abs());
                Close {
                    time,
                    price,
                    high: Some(price * (1. + spread)),
                    low: Some((price * (1. - spread)).max(0.0001)),
                    volume: Some(volume),
                }
            })
            .collect::<Vec<_>>();
        let ratio = quote.price / closes.last()?.price;
        for bar in &mut closes {
            bar.price *= ratio;
            if let Some(h) = &mut bar.high {
                *h *= ratio;
            }
            if let Some(l) = &mut bar.low {
                *l *= ratio;
            }
        }
        Some(Self {
            ticker: ticker.into(),
            currency: quote.currency,
            fetched: quote.fetched,
            sample: true,
            closes,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Setup {
    pub momentum_5: f64,
    pub momentum: f64,
    pub momentum_60: f64,
    pub daily_volatility: f64,
    pub vol_ratio: f64,
    pub drawdown: f64,
    pub volume_ratio: Option<f64>,
    pub range_ratio: Option<f64>,
    pub ewma_vol: f64,
    pub ewma_drift: f64,
}
impl Setup {
    fn at(closes: &[Close], i: usize) -> Self {
        let window20 = &closes[i - 20..=i];
        let momentum_5 = change(closes[i - 5].price, closes[i].price);
        let momentum = change(window20[0].price, closes[i].price);
        let momentum_60 = change(closes[i - LOOKBACK].price, closes[i].price);

        let mean_log20 = (closes[i].price / window20[0].price).ln() / 20.;
        let var20 = window20
            .windows(2)
            .map(|p| ((p[1].price / p[0].price).ln() - mean_log20).powi(2))
            .sum::<f64>()
            / 19.;
        let daily_volatility = var20.sqrt();

        let window5 = &closes[i - 5..=i];
        let mean_log5 = (closes[i].price / window5[0].price).ln() / 5.;
        let var5 = window5
            .windows(2)
            .map(|p| ((p[1].price / p[0].price).ln() - mean_log5).powi(2))
            .sum::<f64>()
            / 4.;
        let vol_ratio = if daily_volatility > 1e-6 {
            (var5.sqrt() / daily_volatility).clamp(0.25, 4.0)
        } else {
            1.0
        };

        let high = closes[i + 1 - LOOKBACK..=i]
            .iter()
            .map(|bar| bar.price)
            .fold(0., f64::max);
        let drawdown = change(high, closes[i].price);

        // RiskMetrics EWMA volatility (lambda = 0.94) strictly causal over trailing 60 bars.
        let mut ewma_var = var20.max(1e-6);
        let lambda = 0.94;
        for pair in closes[i - LOOKBACK..=i].windows(2) {
            let r = (pair[1].price / pair[0].price).ln();
            if r.is_finite() {
                ewma_var = lambda * ewma_var + (1.0 - lambda) * r * r;
            }
        }
        let ewma_vol = ewma_var.sqrt().max(0.002);
        // Strong shrinkage on drift to avoid extrapolating short-term trends indefinitely.
        let mean_log60 = (closes[i].price / closes[i - LOOKBACK].price).ln() / LOOKBACK as f64;
        let ewma_drift = (mean_log60 * 0.25).clamp(-0.0025, 0.0025);

        let volume_ratio = {
            let v20: Vec<f64> = closes[i + 1 - 20..=i]
                .iter()
                .filter_map(|b| b.volume.filter(|v| v.is_finite() && *v > 0.))
                .collect();
            let v5: Vec<f64> = closes[i + 1 - 5..=i]
                .iter()
                .filter_map(|b| b.volume.filter(|v| v.is_finite() && *v > 0.))
                .collect();
            if v20.len() >= 15 && v5.len() >= 4 {
                let avg20 = v20.iter().sum::<f64>() / v20.len() as f64;
                let avg5 = v5.iter().sum::<f64>() / v5.len() as f64;
                (avg20 > 0.).then(|| (avg5 / avg20).clamp(0.2, 5.0))
            } else {
                None
            }
        };

        let range_ratio = {
            let bar_range = |b: &Close| -> Option<f64> {
                let h = b.high.filter(|v| v.is_finite() && *v >= b.price)?;
                let l = b
                    .low
                    .filter(|v| v.is_finite() && *v > 0. && *v <= b.price)?;
                Some(((h - l) / b.price).max(1e-5))
            };
            let r20: Vec<f64> = closes[i + 1 - 20..=i]
                .iter()
                .filter_map(bar_range)
                .collect();
            let r5: Vec<f64> = closes[i + 1 - 5..=i].iter().filter_map(bar_range).collect();
            if r20.len() >= 15 && r5.len() >= 4 {
                let avg20 = r20.iter().sum::<f64>() / r20.len() as f64;
                let avg5 = r5.iter().sum::<f64>() / r5.len() as f64;
                (avg20 > 0.).then(|| (avg5 / avg20).clamp(0.25, 4.0))
            } else {
                None
            }
        };

        Self {
            momentum_5,
            momentum,
            momentum_60,
            daily_volatility,
            vol_ratio,
            drawdown,
            volume_ratio,
            range_ratio,
            ewma_vol,
            ewma_drift,
        }
    }

    fn distance(self, other: Self) -> f64 {
        let vol = self.daily_volatility.max(0.002);
        let other_vol = other.daily_volatility.max(0.002);
        let mut d = 0.55
            * ((self.momentum_5 / vol - other.momentum_5 / other_vol) / 5_f64.sqrt()).powi(2)
            + ((self.momentum / vol - other.momentum / other_vol) / 20_f64.sqrt()).powi(2)
            + 0.70
                * ((self.momentum_60 / vol - other.momentum_60 / other_vol) / 60_f64.sqrt())
                    .powi(2)
            + (vol / other_vol).ln().powi(2)
            + 0.45 * (self.vol_ratio / other.vol_ratio).ln().powi(2)
            + ((self.drawdown / vol - other.drawdown / other_vol) / 60_f64.sqrt()).powi(2);
        if let (Some(a), Some(b)) = (self.volume_ratio, other.volume_ratio) {
            d += 0.35 * (a / b).ln().powi(2);
        }
        if let (Some(a), Some(b)) = (self.range_ratio, other.range_ratio) {
            d += 0.25 * (a / b).ln().powi(2);
        }
        d
    }

    fn feature_vec(self) -> [f64; 7] {
        [
            self.momentum_5.clamp(-0.6, 0.6),
            self.momentum.clamp(-1.0, 1.0),
            self.momentum_60.clamp(-1.5, 2.0),
            self.daily_volatility.max(0.002).ln(),
            self.vol_ratio.ln(),
            self.drawdown.clamp(-0.9, 0.0),
            self.volume_ratio.unwrap_or(1.0).ln(),
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
}
impl Band {
    fn from_returns(returns: &mut [f64]) -> Self {
        returns.sort_by(f64::total_cmp);
        Self {
            low: percentile(returns, 0.1),
            mid: percentile(returns, 0.5),
            high: percentile(returns, 0.9),
        }
    }

    fn from_weighted_returns(pairs: &mut [(f64, f64)]) -> Self {
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self {
            low: weighted_percentile(pairs, 0.1),
            mid: weighted_percentile(pairs, 0.5),
            high: weighted_percentile(pairs, 0.9),
        }
    }

    fn ordered(low: f64, mid: f64, high: f64) -> Self {
        let mut lo = low.min(mid).min(high).max(-0.99);
        let mut hi = high.max(mid).max(low);
        let m = mid.clamp(lo, hi);
        if !lo.is_finite() || !m.is_finite() || !hi.is_finite() {
            lo = 0.0;
            hi = 0.0;
        }
        Self {
            low: lo,
            mid: m.clamp(lo, hi),
            high: hi,
        }
    }

    fn blend(bands: [Band; 4], weights: [f64; 4]) -> Self {
        let mut low = 0.0;
        let mut mid = 0.0;
        let mut high = 0.0;
        for i in 0..4 {
            low += weights[i] * bands[i].low;
            mid += weights[i] * bands[i].mid;
            high += weights[i] * bands[i].high;
        }
        Self::ordered(low, mid, high)
    }

    fn calibrate(self, delta: f64) -> Self {
        let half_width = (self.high - self.low) * 0.5;
        let eff_delta = if delta < 0.0 {
            delta.max(-half_width * 0.35)
        } else {
            delta
        };
        let low = (self.low - eff_delta).min(self.mid).max(-0.99);
        let high = (self.high + eff_delta).max(self.mid);
        Self::ordered(low, self.mid, high)
    }

    pub fn contains(self, realized: f64) -> bool {
        realized >= self.low && realized <= self.high
    }

    pub fn pinball_loss(self, realized: f64) -> f64 {
        (pinball(realized, self.low, 0.1)
            + pinball(realized, self.mid, 0.5)
            + pinball(realized, self.high, 0.9))
            / 3.0
    }
}

fn pinball(y: f64, q: f64, alpha: f64) -> f64 {
    let diff = y - q;
    if diff >= 0.0 {
        alpha * diff
    } else {
        (alpha - 1.0) * diff
    }
}

#[derive(Clone, Debug)]
pub struct Analog {
    pub start: i64,
    pub end: i64,
    pub realized: f64,
    pub weight: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct ModelWeights {
    pub analogs: f64,
    pub online_qr: f64,
    pub ewma: f64,
    pub baseline: f64,
}
impl Default for ModelWeights {
    fn default() -> Self {
        Self {
            analogs: 0.25,
            online_qr: 0.25,
            ewma: 0.25,
            baseline: 0.25,
        }
    }
}
impl ModelWeights {
    fn as_array(self) -> [f64; 4] {
        [self.analogs, self.online_qr, self.ewma, self.baseline]
    }
    fn from_array(a: [f64; 4]) -> Self {
        Self {
            analogs: a[0],
            online_qr: a[1],
            ewma: a[2],
            baseline: a[3],
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Calibration {
    pub weights: ModelWeights,
    pub conformal_delta: f64,
    pub effective_matches: f64,
    pub ensemble_pinball: f64,
    pub baseline_pinball: f64,
    pub analog_pinball: f64,
    pub online_qr_pinball: f64,
    pub ewma_pinball: f64,
    pub trial_count: usize,
    pub raw_covered: usize,
    pub calibrated_covered: usize,
    pub baseline_covered: usize,
}
impl Calibration {
    /// Positive skill means the calibrated ensemble beat the unconditional baseline's pinball loss.
    pub fn pinball_skill(&self) -> f64 {
        if self.baseline_pinball > 1e-9 && self.trial_count > 0 {
            1.0 - self.ensemble_pinball / self.baseline_pinball
        } else {
            0.0
        }
    }
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub anchor: Close,
    pub setup: Setup,
    pub past: Vec<Close>,
    pub fan: Vec<Band>,
    pub matches: Vec<Analog>,
    pub baseline_returns: Vec<f64>,
    pub calibration: Calibration,
}
impl Scenario {
    pub fn end(&self) -> Band {
        *self
            .fan
            .last()
            .expect("The fan includes the anchor and horizon")
    }
    pub fn up_count(&self) -> usize {
        self.matches.iter().filter(|a| a.realized > 0.).count()
    }
}

#[derive(Clone, Debug)]
pub struct Replay {
    pub at: i64,
    pub through: i64,
    pub result: f64,
    pub inside: bool,
    pub raw_inside: bool,
    pub baseline_inside: bool,
    pub absolute_error: f64,
    pub baseline_error: f64,
    pub pinball_loss: f64,
    pub baseline_pinball_loss: f64,
}

#[derive(Debug)]
pub struct Report {
    pub current: Scenario,
    pub trials: Vec<Replay>,
}

pub fn report(history: &History, horizon: usize) -> Result<Report, &'static str> {
    if !HORIZONS.contains(&horizon) {
        return Err("Choose a supported scenario horizon.");
    }
    if !history.valid() {
        return Err("The daily price history contains invalid or out-of-order data.");
    }
    let closes = &history.closes;
    let Some(last) = closes.len().checked_sub(1) else {
        return Err("The feed returned no completed daily closes.");
    };
    let setups = setups(closes);
    let (trials, cal) = walk_forward(closes, &setups, last, horizon);
    let current = snapshot(closes, &setups, last, horizon, cal).ok_or(
        "There are too few non-overlapping historical periods for this horizon. Try a shorter horizon.",
    )?;
    Ok(Report { current, trials })
}

pub fn replay(history: &History, at: i64, horizon: usize) -> Option<(Scenario, Vec<f64>)> {
    if !HORIZONS.contains(&horizon) || !history.valid() {
        return None;
    }
    let closes = &history.closes;
    let i = closes.binary_search_by_key(&at, |bar| bar.time).ok()?;
    let realized = closes
        .get(i..=i.checked_add(horizon)?)?
        .iter()
        .map(|bar| change(closes[i].price, bar.price))
        .collect();
    let all_setups = setups(closes);
    // Strictly causal: calibration at replay date `i` uses ONLY trials whose outcomes
    // finished strictly before `i` (never touching closes after `i`).
    let (_, cal) = walk_forward(closes, &all_setups, i, horizon);
    Some((snapshot(closes, &all_setups, i, horizon, cal)?, realized))
}

fn setups(closes: &[Close]) -> Vec<Setup> {
    (LOOKBACK..closes.len())
        .map(|i| Setup::at(closes, i))
        .collect()
}

fn candidates(
    setups: &[Setup],
    i: usize,
    horizon: usize,
) -> Option<(Vec<(usize, f64)>, Vec<usize>)> {
    let end = i.checked_sub(horizon + 1)?;
    let current = *setups.get(i.checked_sub(LOOKBACK)?)?;
    if end < LOOKBACK {
        return None;
    }
    let baseline = (LOOKBACK..=end).rev().step_by(horizon).collect::<Vec<_>>();
    if baseline.len() < MATCH_MINIMUM {
        return None;
    }
    let mut ranked = (LOOKBACK..=end)
        .map(|j| (j, current.distance(setups[j - LOOKBACK])))
        .collect::<Vec<_>>();
    ranked.sort_unstable_by(|&(a, da), &(b, db)| match da.total_cmp(&db) {
        Ordering::Equal => b.cmp(&a),
        order => order,
    });
    let mut selected: Vec<(usize, f64)> = Vec::with_capacity(MATCH_TARGET);
    for &(j, dist) in &ranked {
        if selected.iter().all(|&(k, _)| j.abs_diff(k) >= horizon) {
            selected.push((j, dist));
            if selected.len() == MATCH_TARGET {
                break;
            }
        }
    }
    if selected.len() < MATCH_MINIMUM {
        selected = ranked
            .into_iter()
            .filter(|&(j, _)| (end - j).is_multiple_of(horizon))
            .take(MATCH_TARGET)
            .collect();
    }
    Some((selected, baseline))
}

fn match_weights(selected: &[(usize, f64)], i: usize) -> (Vec<f64>, f64) {
    if selected.is_empty() {
        return (vec![], 0.0);
    }
    let mut dists: Vec<f64> = selected.iter().map(|&(_, d)| d).collect();
    dists.sort_by(f64::total_cmp);
    let tau = percentile(&dists, 0.5).max(0.25);
    let mut raw: Vec<f64> = selected
        .iter()
        .map(|&(j, dist)| {
            let age_years = i.saturating_sub(j) as f64 / 252.0;
            ((-dist / tau).exp() * (-0.08 * age_years).exp()).max(1e-6)
        })
        .collect();
    let sum: f64 = raw.iter().sum();
    if sum > 0.0 {
        for w in &mut raw {
            *w /= sum;
        }
    }
    let sum_sq: f64 = raw.iter().map(|w| w * w).sum();
    let n_eff = if sum_sq > 0.0 { 1.0 / sum_sq } else { 0.0 };
    (raw, n_eff)
}

fn ewma_band(setup: Setup, step: usize) -> Band {
    if step == 0 {
        return Band {
            low: 0.0,
            mid: 0.0,
            high: 0.0,
        };
    }
    let h = step as f64;
    let drift = setup.ewma_drift * h;
    // Fat-tailed empirical 10th/90th multiplier (~1.34 vs Gaussian 1.2816) for equity returns.
    let spread = 1.34 * setup.ewma_vol * h.sqrt();
    let low = (drift - spread).exp() - 1.0;
    let mid = drift.exp() - 1.0;
    let high = (drift + spread).exp() - 1.0;
    Band::ordered(low, mid, high)
}

/// Strictly causal online L2-regularized quantile regression trained sequentially over
/// completed non-overlapping historical periods prior to index `i`.
fn online_qr_band(
    closes: &[Close],
    setups: &[Setup],
    baseline: &[usize],
    current: Setup,
    horizon: usize,
    baseline_band: Band,
) -> Band {
    if baseline.len() < MATCH_MINIMUM {
        return baseline_band;
    }
    // `baseline` is newest-first; iterate chronologically oldest-first so AdaGrad updates
    // adapt online in time order.
    let n = baseline.len();
    const D: usize = 7;
    let mut mean = [0.0; D];
    let mut min_ret = f64::INFINITY;
    let mut max_ret = f64::NEG_INFINITY;
    let mut samples: Vec<([f64; D], f64)> = Vec::with_capacity(n);
    for &j in baseline.iter().rev() {
        let f = setups[j - LOOKBACK].feature_vec();
        let y = change(closes[j].price, closes[j + horizon].price);
        if !y.is_finite() {
            continue;
        }
        for k in 0..D {
            mean[k] += f[k];
        }
        min_ret = min_ret.min(y);
        max_ret = max_ret.max(y);
        samples.push((f, y));
    }
    if samples.len() < MATCH_MINIMUM {
        return baseline_band;
    }
    let nf = samples.len() as f64;
    for m in &mut mean {
        *m /= nf;
    }
    let mut std = [0.0; D];
    for (f, _) in &samples {
        for k in 0..D {
            std[k] += (f[k] - mean[k]).powi(2);
        }
    }
    for s in &mut std {
        *s = (*s / nf).sqrt().max(1e-4);
    }

    let standardize = |raw: [f64; D]| -> [f64; D + 1] {
        let mut x = [1.0; D + 1];
        for k in 0..D {
            x[k + 1] = ((raw[k] - mean[k]) / std[k]).clamp(-3.0, 3.0);
        }
        x
    };

    let xs: Vec<([f64; D + 1], f64)> = samples
        .into_iter()
        .map(|(f, y)| (standardize(f), y))
        .collect();
    let cur_x = standardize(current.feature_vec());
    let init = [baseline_band.low, baseline_band.mid, baseline_band.high];
    let alphas = [0.10, 0.50, 0.90];
    let l2 = 0.08;
    let lr = 0.045;
    let mut preds = [0.0; 3];

    for q_idx in 0..3 {
        let alpha = alphas[q_idx];
        let mut w = [0.0; D + 1];
        w[0] = init[q_idx];
        let mut g2 = [1e-4; D + 1];
        // Three sequential causal passes with AdaGrad so online weights stabilize smoothly.
        for _epoch in 0..3 {
            for (x, y) in &xs {
                let mut pred = 0.0;
                for k in 0..=D {
                    pred += w[k] * x[k];
                }
                let err = *y - pred;
                // Subgradient of pinball loss wrt prediction: -alpha if y > pred else (1 - alpha)
                let d_loss = if err > 0.0 {
                    -alpha
                } else if err < 0.0 {
                    1.0 - alpha
                } else {
                    0.0
                };
                for k in 0..=D {
                    let reg = if k == 0 { 0.0 } else { l2 * w[k] };
                    let grad = d_loss * x[k] + reg;
                    g2[k] += grad * grad;
                    w[k] -= (lr / g2[k].sqrt()) * grad;
                }
            }
        }
        let mut pred = 0.0;
        for k in 0..=D {
            pred += w[k] * cur_x[k];
        }
        let span = (max_ret - min_ret).max(0.05);
        preds[q_idx] = pred.clamp((min_ret - 0.35 * span).max(-0.98), max_ret + 0.35 * span);
    }
    Band::ordered(preds[0], preds[1], preds[2])
}

fn component_end_bands(
    closes: &[Close],
    setups: &[Setup],
    i: usize,
    horizon: usize,
) -> Option<([Band; 4], Vec<(usize, f64)>, Vec<f64>, f64, Vec<usize>)> {
    let (selected, baseline) = candidates(setups, i, horizon)?;
    let (weights, n_eff) = match_weights(&selected, i);
    let mut analog_pairs: Vec<(f64, f64)> = selected
        .iter()
        .zip(&weights)
        .map(|(&(j, _), &w)| (change(closes[j].price, closes[j + horizon].price), w))
        .collect();
    let analog = Band::from_weighted_returns(&mut analog_pairs);

    let mut base_rets: Vec<f64> = baseline
        .iter()
        .map(|&j| change(closes[j].price, closes[j + horizon].price))
        .collect();
    let baseline_band = Band::from_returns(&mut base_rets);
    let current = setups[i - LOOKBACK];
    let qr = online_qr_band(closes, setups, &baseline, current, horizon, baseline_band);
    let ewma = ewma_band(current, horizon);
    Some((
        [analog, qr, ewma, baseline_band],
        selected,
        weights,
        n_eff,
        baseline,
    ))
}

fn hedge_weights(losses: [f64; 4], count: usize) -> ModelWeights {
    if count < 3 {
        return ModelWeights::default();
    }
    let scale = (losses[3] / count as f64).max(0.005);
    let eta = 3.2;
    let avg = losses.map(|l| l / count as f64);
    let min_l = avg.into_iter().fold(f64::INFINITY, f64::min);
    let mut w = [0.0; 4];
    let mut sum = 0.0;
    for m in 0..4 {
        w[m] = (-eta * (avg[m] - min_l) / scale).exp().max(0.03);
        sum += w[m];
    }
    for x in &mut w {
        *x /= sum;
    }
    ModelWeights::from_array(w)
}

fn conformal_adjustment(scores: &[f64]) -> f64 {
    if scores.len() < 5 {
        return 0.0;
    }
    let mut sorted = scores.to_vec();
    sorted.sort_by(f64::total_cmp);
    // Finite-sample split-conformal 80% quantile: ceil((n + 1) * 0.80) / n
    let n = sorted.len();
    let rank = (((n + 1) as f64 * 0.80).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    sorted[rank].clamp(-0.04, 0.15)
}

/// Strictly causal walk-forward evaluation ending before `at_index`.
/// Every trial uses only outcomes that finished strictly before its start date,
/// and the returned `Calibration` uses only trials whose outcome completed strictly before `at_index`.
fn walk_forward(
    closes: &[Close],
    setups: &[Setup],
    at_index: usize,
    horizon: usize,
) -> (Vec<Replay>, Calibration) {
    let Some(last_start) = at_index.checked_sub(horizon) else {
        return (vec![], Calibration::default());
    };
    let mut indices: Vec<usize> = (0..=last_start)
        .rev()
        .step_by(horizon)
        .take(REPLAY_LIMIT)
        .collect();
    // Run chronologically oldest-first so Hedge weights and conformal residuals update causally!
    indices.reverse();

    let mut cumulative_losses = [0.0; 4];
    let mut prior_trials = 0usize;
    let mut conformal_scores: Vec<f64> = Vec::with_capacity(indices.len());
    let mut ens_pinball_sum = 0.0;
    let mut raw_covered = 0usize;
    let mut calibrated_covered = 0usize;
    let mut baseline_covered = 0usize;
    let mut trials: Vec<Replay> = Vec::with_capacity(indices.len());

    for idx in indices {
        let Some((bands, _, _, _, _)) = component_end_bands(closes, setups, idx, horizon) else {
            continue;
        };
        let actual = change(closes[idx].price, closes[idx + horizon].price);
        let weights = hedge_weights(cumulative_losses, prior_trials);
        let raw_ens = Band::blend(bands, weights.as_array());
        let delta = conformal_adjustment(&conformal_scores);
        let cal_ens = raw_ens.calibrate(delta);

        let ens_pb = cal_ens.pinball_loss(actual);
        let base_pb = bands[3].pinball_loss(actual);
        let inside = cal_ens.contains(actual);
        let raw_inside = raw_ens.contains(actual);
        let baseline_inside = bands[3].contains(actual);

        // Update causal trackers strictly AFTER observing `actual`.
        for m in 0..4 {
            cumulative_losses[m] += bands[m].pinball_loss(actual);
        }
        conformal_scores.push((raw_ens.low - actual).max(actual - raw_ens.high));
        prior_trials += 1;
        ens_pinball_sum += ens_pb;
        raw_covered += usize::from(raw_inside);
        calibrated_covered += usize::from(inside);
        baseline_covered += usize::from(baseline_inside);

        trials.push(Replay {
            at: closes[idx].time,
            through: closes[idx + horizon].time,
            result: actual,
            inside,
            raw_inside,
            baseline_inside,
            absolute_error: (actual - cal_ens.mid).abs(),
            baseline_error: (actual - bands[3].mid).abs(),
            pinball_loss: ens_pb,
            baseline_pinball_loss: base_pb,
        });
    }

    // UI lists replay dates newest-first.
    trials.reverse();

    let cal = if prior_trials > 0 {
        let nf = prior_trials as f64;
        Calibration {
            weights: hedge_weights(cumulative_losses, prior_trials),
            conformal_delta: conformal_adjustment(&conformal_scores),
            effective_matches: 0.0,
            ensemble_pinball: ens_pinball_sum / nf,
            baseline_pinball: cumulative_losses[3] / nf,
            analog_pinball: cumulative_losses[0] / nf,
            online_qr_pinball: cumulative_losses[1] / nf,
            ewma_pinball: cumulative_losses[2] / nf,
            trial_count: prior_trials,
            raw_covered,
            calibrated_covered,
            baseline_covered,
        }
    } else {
        Calibration::default()
    };
    (trials, cal)
}

fn snapshot(
    closes: &[Close],
    setups: &[Setup],
    i: usize,
    horizon: usize,
    mut cal: Calibration,
) -> Option<Scenario> {
    let (end_bands, selected, weights, n_eff, baseline) =
        component_end_bands(closes, setups, i, horizon)?;
    cal.effective_matches = n_eff;
    let w_arr = cal.weights.as_array();
    let current = setups[i - LOOKBACK];
    let qr_end = end_bands[1];
    let end_return = |j: usize| change(closes[j].price, closes[j + horizon].price);

    let fan = (0..=horizon)
        .map(|step| {
            if step == 0 {
                return Band {
                    low: 0.0,
                    mid: 0.0,
                    high: 0.0,
                };
            }
            let frac = (step as f64 / horizon as f64).sqrt();
            let mut analog_pairs: Vec<(f64, f64)> = selected
                .iter()
                .zip(&weights)
                .map(|(&(j, _), &w)| (change(closes[j].price, closes[j + step].price), w))
                .collect();
            let analog_step = Band::from_weighted_returns(&mut analog_pairs);
            let qr_step = Band::ordered(
                qr_end.low * frac,
                qr_end.mid * (step as f64 / horizon as f64),
                qr_end.high * frac,
            );
            let ewma_step = ewma_band(current, step);
            let mut base_step_rets: Vec<f64> = baseline
                .iter()
                .map(|&j| change(closes[j].price, closes[j + step].price))
                .collect();
            let base_step = Band::from_returns(&mut base_step_rets);
            let blended = Band::blend([analog_step, qr_step, ewma_step, base_step], w_arr);
            blended.calibrate(cal.conformal_delta * frac)
        })
        .collect();

    Some(Scenario {
        anchor: closes[i],
        setup: current,
        past: closes[i.saturating_sub(40)..=i].to_vec(),
        fan,
        matches: selected
            .into_iter()
            .zip(weights)
            .map(|((j, _), weight)| Analog {
                start: closes[j].time,
                end: closes[j + horizon].time,
                realized: end_return(j),
                weight,
            })
            .collect(),
        baseline_returns: baseline.into_iter().map(end_return).collect(),
        calibration: cal,
    })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = p * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    sorted[lo] + (sorted[hi] - sorted[lo]) * rank.fract()
}

fn weighted_percentile(sorted: &[(f64, f64)], p: f64) -> f64 {
    if sorted.len() == 1 {
        return sorted[0].0;
    }
    let total: f64 = sorted.iter().map(|&(_, w)| w).sum();
    if total <= 0.0 {
        let vals: Vec<f64> = sorted.iter().map(|&(v, _)| v).collect();
        return percentile(&vals, p);
    }
    let target = p * total;
    let mut acc = 0.0;
    let mut prev_v = sorted[0].0;
    let mut prev_c = 0.0;
    for &(v, w) in sorted {
        let mid_c = acc + 0.5 * w;
        if mid_c >= target {
            if mid_c <= prev_c + 1e-12 {
                return v;
            }
            let t = ((target - prev_c) / (mid_c - prev_c)).clamp(0.0, 1.0);
            return prev_v + (v - prev_v) * t;
        }
        prev_v = v;
        prev_c = mid_c;
        acc += w;
    }
    sorted.last().unwrap().0
}

pub fn change(from: f64, to: f64) -> f64 {
    (to - from) / from
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_uses_sixty_closes_and_rejects_unsupported_horizons() {
        let mut closes = (0..=60)
            .map(|i| Close::new(i + 1, 100.))
            .collect::<Vec<_>>();
        closes[0].price = 400.;
        let setup = Setup::at(&closes, 60);
        assert_eq!(setup.drawdown, 0.);
        assert_eq!(setup.momentum, 0.);
        assert_eq!(setup.daily_volatility, 0.);
        let mut history = History::sample("NVDA").unwrap();
        assert!(report(&history, 0).is_err());
        assert!(replay(&history, history.closes[0].time, usize::MAX).is_none());
        history.closes[1].time = history.closes[0].time;
        assert!(report(&history, 20).is_err());
    }

    #[test]
    fn a_close_match_cannot_hide_ten_available_non_overlapping_periods() {
        let horizon = 5;
        let last = LOOKBACK + MATCH_MINIMUM * horizon + 1;
        let setup = Setup {
            momentum_5: 0.03,
            momentum: 0.1,
            momentum_60: 0.2,
            daily_volatility: 0.02,
            vol_ratio: 1.0,
            drawdown: -0.03,
            volume_ratio: None,
            range_ratio: None,
            ewma_vol: 0.02,
            ewma_drift: 0.0,
        };
        let mut history = vec![setup; last - LOOKBACK + 1];
        history[2].momentum = 0.;
        history[last - LOOKBACK].momentum = 0.;
        let (selected, baseline) = candidates(&history, last, horizon).unwrap();
        assert_eq!(selected.len(), MATCH_MINIMUM);
        assert_eq!(baseline.len(), MATCH_MINIMUM);
        assert!(selected.iter().enumerate().all(|(i, &(j, _))| {
            j + horizon < last && selected[..i].iter().all(|&(k, _)| j.abs_diff(k) >= horizon)
        }));
        assert!(candidates(&history, last - 1, horizon).is_none());
    }

    #[test]
    fn replay_never_learns_from_the_hidden_result_or_overlaps_match_returns() {
        let mut history = History::sample("NVDA").unwrap();
        let trials = report(&history, 20).unwrap().trials;
        let at = trials[3].at;
        let (before, actual) = replay(&history, at, 20).unwrap();
        let n = history
            .closes
            .binary_search_by_key(&at, |p| p.time)
            .unwrap();
        assert!(before.matches.iter().all(|a| a.end < at));
        for (i, a) in before.matches.iter().enumerate() {
            assert!(
                before.matches[i + 1..]
                    .iter()
                    .all(|b| a.end <= b.start || b.end <= a.start)
            );
        }
        for point in &mut history.closes[n + 1..] {
            point.price *= 2.;
        }
        let (after, changed) = replay(&history, at, 20).unwrap();
        assert_eq!(before.end().mid, after.end().mid);
        assert_eq!(before.end().low, after.end().low);
        assert_eq!(before.end().high, after.end().high);
        assert_eq!(
            before.calibration.conformal_delta,
            after.calibration.conformal_delta
        );
        assert_ne!(actual, changed);
    }

    #[test]
    fn horizon_reports_have_an_ordered_band_and_clear_minimum_sample() {
        let mut history = History::sample("NVDA").unwrap();
        assert!(history.valid());
        for horizon in HORIZONS {
            let report = report(&history, horizon).unwrap();
            let end = report.current.end();
            assert!(end.low <= end.mid && end.mid <= end.high);
            assert_eq!(report.current.fan.len(), horizon + 1);
            assert!(!report.trials.is_empty());
            let w = report.current.calibration.weights;
            let total = w.analogs + w.online_qr + w.ewma + w.baseline;
            assert!((total - 1.0).abs() < 1e-9);
            assert!(report.current.calibration.effective_matches >= 1.0);
        }
        history.closes.truncate(150);
        assert!(report(&history, 20).is_err());
    }

    #[test]
    fn hedge_weights_favor_lower_pinball_loss_and_conformal_widens_undercovering_bands() {
        // Model 1 (online_qr) has half the pinball loss of others -> should get highest weight.
        let weights = hedge_weights([0.20, 0.08, 0.22, 0.20], 10);
        assert!(weights.online_qr > weights.analogs);
        assert!(weights.online_qr > weights.ewma);
        assert!(weights.online_qr > weights.baseline);

        // Positive residuals (outcomes outside raw interval) produce positive conformal widening.
        let delta = conformal_adjustment(&[0.02, 0.03, 0.04, 0.05, 0.06, 0.07, -0.01, 0.01]);
        assert!(delta > 0.04);
        let raw = Band::ordered(-0.05, 0.01, 0.06);
        let calibrated = raw.calibrate(delta);
        assert!(calibrated.low < raw.low && calibrated.high > raw.high);
        assert_eq!(calibrated.mid, raw.mid);
    }

    #[test]
    fn weighted_quantiles_emphasize_closer_matches_over_distant_matches() {
        let mut pairs = vec![(-0.20, 0.05), (0.08, 0.90), (0.25, 0.05)];
        let band = Band::from_weighted_returns(&mut pairs);
        assert!(band.mid > 0.05 && band.mid < 0.11);
        assert!(band.low <= band.mid && band.mid <= band.high);
    }
}
