//! Price scenarios from completed daily sessions, with a forward-only replay.
use crate::model::{demo_market, valid_ticker};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

const DAY: i64 = 86_400;
const LOOKBACK: usize = 60;
const MATCH_TARGET: usize = 24;
const MATCH_MINIMUM: usize = 10;
const REPLAY_LIMIT: usize = 18;
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
                Close {
                    time,
                    price: log_price.exp(),
                }
            })
            .collect::<Vec<_>>();
        let ratio = quote.price / closes.last()?.price;
        for bar in &mut closes {
            bar.price *= ratio;
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
    pub momentum: f64,
    pub daily_volatility: f64,
    pub drawdown: f64,
}
impl Setup {
    fn at(closes: &[Close], i: usize) -> Self {
        let window = &closes[i - 20..=i];
        let momentum = change(window[0].price, closes[i].price);
        let average_log_return = (closes[i].price / window[0].price).ln() / 20.;
        let variance = window
            .windows(2)
            .map(|p| ((p[1].price / p[0].price).ln() - average_log_return).powi(2))
            .sum::<f64>()
            / 19.;
        let high = closes[i + 1 - LOOKBACK..=i]
            .iter()
            .map(|bar| bar.price)
            .fold(0., f64::max);
        Self {
            momentum,
            daily_volatility: variance.sqrt(),
            drawdown: change(high, closes[i].price),
        }
    }

    fn distance(self, other: Self) -> f64 {
        let vol = self.daily_volatility.max(0.002);
        let other_vol = other.daily_volatility.max(0.002);
        ((self.momentum / vol - other.momentum / other_vol) / 20_f64.sqrt()).powi(2)
            + (vol / other_vol).ln().powi(2)
            + ((self.drawdown / vol - other.drawdown / other_vol) / 60_f64.sqrt()).powi(2)
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

    fn contains(self, realized: f64) -> bool {
        realized >= self.low && realized <= self.high
    }
}

#[derive(Clone, Debug)]
pub struct Analog {
    pub start: i64,
    pub end: i64,
    pub realized: f64,
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub anchor: Close,
    pub setup: Setup,
    pub past: Vec<Close>,
    pub fan: Vec<Band>,
    pub matches: Vec<Analog>,
    pub baseline_returns: Vec<f64>,
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
    pub baseline_inside: bool,
    pub absolute_error: f64,
    pub baseline_error: f64,
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
    let current = snapshot(closes, &setups, last, horizon).ok_or(
        "There are too few non-overlapping historical periods for this horizon. Try a shorter horizon.",
    )?;
    let trials = (0..=last.saturating_sub(horizon))
        .rev()
        .step_by(horizon)
        .take(REPLAY_LIMIT)
        .filter_map(|i| {
            let (selected, baseline) = candidates(&setups, i, horizon)?;
            let actual = change(closes[i].price, closes[i + horizon].price);
            let end_band = |indices: &[usize]| {
                Band::from_returns(
                    &mut indices
                        .iter()
                        .map(|&j| change(closes[j].price, closes[j + horizon].price))
                        .collect::<Vec<_>>(),
                )
            };
            let analog = end_band(&selected);
            let historical = end_band(&baseline);
            Some(Replay {
                at: closes[i].time,
                through: closes[i + horizon].time,
                result: actual,
                inside: analog.contains(actual),
                baseline_inside: historical.contains(actual),
                absolute_error: (actual - analog.mid).abs(),
                baseline_error: (actual - historical.mid).abs(),
            })
        })
        .collect();
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
    Some((snapshot(closes, &setups(closes), i, horizon)?, realized))
}

fn setups(closes: &[Close]) -> Vec<Setup> {
    (LOOKBACK..closes.len())
        .map(|i| Setup::at(closes, i))
        .collect()
}

fn candidates(setups: &[Setup], i: usize, horizon: usize) -> Option<(Vec<usize>, Vec<usize>)> {
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
    let mut selected: Vec<usize> = Vec::with_capacity(MATCH_TARGET);
    for &(j, _) in &ranked {
        if selected.iter().all(|&k| j.abs_diff(k) >= horizon) {
            selected.push(j);
            if selected.len() == MATCH_TARGET {
                break;
            }
        }
    }
    if selected.len() < MATCH_MINIMUM {
        // The nearest setups can block two non-overlapping windows at once.
        // Use the best matches on a complete, regularly spaced set in that case.
        selected = ranked
            .into_iter()
            .map(|(j, _)| j)
            .filter(|j| (end - j).is_multiple_of(horizon))
            .take(MATCH_TARGET)
            .collect();
    }
    Some((selected, baseline))
}

fn snapshot(closes: &[Close], setups: &[Setup], i: usize, horizon: usize) -> Option<Scenario> {
    let (selected, baseline) = candidates(setups, i, horizon)?;
    let end_return = |j: usize| change(closes[j].price, closes[j + horizon].price);
    Some(Scenario {
        anchor: closes[i],
        setup: setups[i - LOOKBACK],
        past: closes[i.saturating_sub(40)..=i].to_vec(),
        fan: (0..=horizon)
            .map(|step| {
                Band::from_returns(
                    &mut selected
                        .iter()
                        .map(|&j| change(closes[j].price, closes[j + step].price))
                        .collect::<Vec<_>>(),
                )
            })
            .collect(),
        matches: selected
            .into_iter()
            .map(|j| Analog {
                start: closes[j].time,
                end: closes[j + horizon].time,
                realized: end_return(j),
            })
            .collect(),
        baseline_returns: baseline.into_iter().map(end_return).collect(),
    })
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = p * (sorted.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    sorted[lo] + (sorted[hi] - sorted[lo]) * rank.fract()
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
            .map(|i| Close {
                time: i + 1,
                price: 100.,
            })
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
            momentum: 0.1,
            daily_volatility: 0.02,
            drawdown: -0.03,
        };
        let mut history = vec![setup; last - LOOKBACK + 1];
        history[2].momentum = 0.;
        history[last - LOOKBACK].momentum = 0.;
        let (selected, baseline) = candidates(&history, last, horizon).unwrap();
        assert_eq!(selected.len(), MATCH_MINIMUM);
        assert_eq!(baseline.len(), MATCH_MINIMUM);
        assert!(selected.iter().enumerate().all(|(i, &j)| {
            j + horizon < last && selected[..i].iter().all(|&k| j.abs_diff(k) >= horizon)
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
        }
        history.closes.truncate(150);
        assert!(report(&history, 20).is_err());
    }
}
