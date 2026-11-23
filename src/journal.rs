use crate::model::*;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DAY: i64 = 86_400;
pub const REVIEW_DAYS: [i64; 3] = [28, 180, 365];
pub const REVIEW_LABELS: [&str; 3] = ["4 weeks", "6 months", "1 year"];
pub const MOVE_THRESHOLD: f64 = 2.0;

fn decision_correct(decision: Decision, reference: f64, price: f64) -> bool {
    match decision {
        Decision::Buy => price > reference * (1. + MOVE_THRESHOLD / 100.),
        Decision::Sell => price < reference * (1. - MOVE_THRESHOLD / 100.),
        Decision::DoNotBuy => price <= reference * (1. + MOVE_THRESHOLD / 100.),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Benchmark {
    pub reviewed: usize,
    pub correct: usize,
    pub always_buy: usize,
    pub always_sell: usize,
    pub always_wait: usize,
    pub repeated: usize,
}
impl Benchmark {
    pub fn rate(&self, correct: usize) -> String {
        if self.reviewed == 0 {
            "—".into()
        } else {
            format!("{:.0}%", 100. * correct as f64 / self.reviewed as f64)
        }
    }
}

pub struct StockSplit {
    pub time: i64,
    pub factor: f64,
}
pub struct ReviewData {
    pub market: Market,
    pub splits: Vec<StockSplit>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Realization {
    pub price: f64,
    pub comparison_reference_price: f64,
    pub price_time: i64,
    pub checked_at: i64,
    pub change_percent: f64,
    pub correct: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Prediction {
    pub id: u64,
    pub symbol: String,
    pub horizon: usize,
    pub generated: i64,
    pub quote_time: i64,
    pub review_at: i64,
    pub provider: String,
    pub model: String,
    pub endpoint: String,
    pub decision: Decision,
    pub confidence_percent: Option<f64>,
    pub reason: String,
    pub risks: Vec<String>,
    pub reference_price: Option<f64>,
    pub currency: String,
    pub demo: bool,
    pub outcome: Option<Realization>,
}
impl Prediction {
    fn has_reference(&self) -> bool {
        self.reference_price
            .is_some_and(|v| v.is_finite() && v > 0.)
            && valid_currency(&self.currency)
    }

    fn valid_quote_time(&self) -> bool {
        self.quote_time > 0
            && self.quote_time <= self.generated
            && self.generated - self.quote_time <= 7 * DAY
    }

    fn valid_checkpoint(&self) -> bool {
        REVIEW_DAYS
            .get(self.horizon)
            .is_some_and(|days| self.generated.checked_add(days * DAY) == Some(self.review_at))
    }

    pub fn verified_outcome(&self, at: i64) -> Option<&Realization> {
        self.outcome.as_ref().filter(|o| {
            !self.demo
                && self.has_reference()
                && self.valid_quote_time()
                && self.valid_checkpoint()
                && o.price.is_finite()
                && o.price > 0.
                && o.comparison_reference_price.is_finite()
                && o.comparison_reference_price > 0.
                && o.change_percent.is_finite()
                && (self.review_at..=self.review_at.saturating_add(7 * DAY)).contains(&o.price_time)
                && o.price_time.saturating_add(DAY) <= o.checked_at
                && o.checked_at <= at
        })
    }

    pub fn can_review(&self, at: i64) -> bool {
        !self.demo
            && self.outcome.is_none()
            && self.review_at <= at
            && self.has_reference()
            && self.valid_quote_time()
            && self.valid_checkpoint()
    }
    pub fn status(&self, at: i64) -> &'static str {
        if self.demo {
            "Sample · excluded from results"
        } else if let Some(outcome) = self.verified_outcome(at) {
            if outcome.correct {
                "Correct"
            } else {
                "Incorrect"
            }
        } else if !self.has_reference() {
            "Unscored · reference price unavailable"
        } else if !self.valid_quote_time() {
            "Unscored · reference quote too old or invalid"
        } else if !self.valid_checkpoint() || self.outcome.is_some() {
            "Unscored · saved review data is invalid"
        } else if self.review_at > at {
            "Pending · review date not reached"
        } else {
            "Awaiting historical closing price"
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Journal {
    pub entries: Vec<Prediction>,
}

#[derive(Clone, Serialize, Debug)]
pub struct LearningExample {
    pub ticker: String,
    pub horizon: String,
    pub decision: Decision,
    pub confidence_percent: Option<f64>,
    pub original_reason: String,
    pub original_risks: Vec<String>,
    pub decided_at: i64,
    pub reviewed_price_at: i64,
    pub realized_change_percent: f64,
    pub correct: bool,
}
#[derive(Clone, Serialize, Debug)]
pub struct LearningContext {
    pub window_days: u16,
    pub reviewed: usize,
    pub correct: usize,
    pub examples: Vec<LearningExample>,
}

impl Journal {
    pub fn load(path: &Path) -> Result<Self, String> {
        let journal: Self = read_saved_json(path)
            .map_err(|e| {
                format!("Cannot read prediction logbook: {e}. The saved file has been kept.")
            })?
            .unwrap_or_default();
        let mut ids = std::collections::HashSet::new();
        if journal.entries.iter().any(|e| {
            e.horizon >= HORIZONS.len()
                || !valid_ticker(&e.symbol)
                || e.id == 0
                || !ids.insert(e.id)
        }) {
            return Err(
                "The prediction logbook contains an invalid entry. The saved file has been kept."
                    .into(),
            );
        }
        Ok(journal)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        atomic_json(path, self)
    }
    pub fn record(&mut self, analysis: &Analysis) -> usize {
        if !valid_ticker(&analysis.symbol) {
            return 0;
        }
        let start = self.entries.iter().map(|e| e.id).max().unwrap_or(0);
        let before = self.entries.len();
        for (horizon, outlook) in analysis.horizons.iter().enumerate().take(3) {
            let Some(decision) = outlook.decision else {
                continue;
            };
            let id = start
                .checked_add(1 + (self.entries.len() - before) as u64)
                .unwrap_or_else(|| {
                    (1..)
                        .find(|&id| self.entries.iter().all(|e| e.id != id))
                        .unwrap()
                });
            self.entries.push(Prediction {
                id,
                symbol: analysis.symbol.clone(),
                horizon,
                generated: analysis.generated,
                quote_time: analysis.context_time,
                review_at: analysis
                    .generated
                    .saturating_add(REVIEW_DAYS[horizon] * DAY),
                provider: analysis.provider.clone(),
                model: analysis.model.clone(),
                endpoint: analysis.endpoint.clone(),
                decision,
                confidence_percent: outlook.confidence_value(),
                reason: outlook.summary.clone(),
                risks: outlook.risks.clone(),
                reference_price: analysis
                    .reference_price
                    .filter(|v| v.is_finite() && *v > 0.),
                currency: analysis.reference_currency.clone(),
                demo: analysis.demo,
                outcome: None,
            });
        }
        self.entries.len() - before
    }
    pub fn resolve(&mut self, ids: &[u64], data: &ReviewData, at: i64) -> usize {
        let market = &data.market;
        if market.quote.demo {
            return 0;
        }
        let ids = ids
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let mut resolved = 0;
        for entry in &mut self.entries {
            if !ids.contains(&entry.id)
                || !entry.can_review(at)
                || entry.symbol != market.quote.symbol
                || entry.currency != market.quote.currency
            {
                continue;
            }
            // A daily bar's timestamp is its session start, not its closing time.
            // Waiting 24 hours avoids scoring an in-progress daily candle.
            let point = market
                .points
                .iter()
                .filter(|p| {
                    p.time >= entry.review_at
                        && p.time <= entry.review_at.saturating_add(7 * DAY)
                        && p.time.saturating_add(DAY) <= at
                        && p.close.is_finite()
                        && p.close > 0.
                })
                .min_by_key(|p| p.time);
            let Some(point) = point else { continue };
            // Historical close prices are split-adjusted by the feed. Put the
            // saved spot quote on the same basis, including later splits.
            let factor = data
                .splits
                .iter()
                .filter(|s| s.time > entry.quote_time && s.time <= at)
                .map(|s| s.factor)
                .product::<f64>();
            if !factor.is_finite() || factor <= 0. {
                continue;
            }
            let reference = entry.reference_price.unwrap() / factor;
            let change = (point.close - reference) / reference * 100.;
            if !change.is_finite() {
                continue;
            }
            let correct = decision_correct(entry.decision, reference, point.close);
            entry.outcome = Some(Realization {
                price: point.close,
                comparison_reference_price: reference,
                price_time: point.time,
                checked_at: at,
                change_percent: change,
                correct,
            });
            resolved += 1;
        }
        resolved
    }
    /// Score the AI and fixed decisions on exactly the same reviewed calls.
    /// Keep the first reviewed call per ticker/horizon/UTC day so repeatedly
    /// generating a result cannot give that day's outcome extra weight.
    pub fn benchmarks(&self, provider: &Provider, symbol: Option<&str>, at: i64) -> [Benchmark; 3] {
        let mut eligible = self
            .entries
            .iter()
            .filter(|e| {
                e.provider == provider.name
                    && e.model == provider.model
                    && e.endpoint == provider.endpoint
                    && symbol.is_none_or(|s| s == e.symbol)
                    && e.verified_outcome(at).is_some()
            })
            .collect::<Vec<_>>();
        eligible.sort_by_key(|e| (e.generated, e.id));
        let mut seen = std::collections::HashSet::new();
        let mut scores = [Benchmark::default(); 3];
        for e in eligible {
            let score = &mut scores[e.horizon];
            if !seen.insert((&e.symbol, e.horizon, e.generated.div_euclid(DAY))) {
                score.repeated += 1;
                continue;
            }
            let o = e.outcome.as_ref().unwrap();
            score.reviewed += 1;
            score.correct += usize::from(decision_correct(
                e.decision,
                o.comparison_reference_price,
                o.price,
            ));
            score.always_buy += usize::from(decision_correct(
                Decision::Buy,
                o.comparison_reference_price,
                o.price,
            ));
            score.always_sell += usize::from(decision_correct(
                Decision::Sell,
                o.comparison_reference_price,
                o.price,
            ));
            score.always_wait += usize::from(decision_correct(
                Decision::DoNotBuy,
                o.comparison_reference_price,
                o.price,
            ));
        }
        scores
    }
    pub fn learning_context(&self, provider: &Provider, days: u16, at: i64) -> LearningContext {
        let mut eligible = self
            .entries
            .iter()
            .filter(|e| {
                days > 0
                    && e.provider == provider.name
                    && e.model == provider.model
                    && e.endpoint == provider.endpoint
                    && e.verified_outcome(at)
                        .is_some_and(|o| o.price_time >= at.saturating_sub(i64::from(days) * DAY))
            })
            .collect::<Vec<_>>();
        eligible.sort_by_key(|e| std::cmp::Reverse(e.outcome.as_ref().unwrap().price_time));
        LearningContext {
            window_days: days,
            reviewed: eligible.len(),
            correct: eligible
                .iter()
                .filter(|e| e.outcome.as_ref().unwrap().correct)
                .count(),
            examples: eligible
                .into_iter()
                .take(24)
                .map(|e| {
                    let outcome = e.outcome.as_ref().unwrap();
                    LearningExample {
                        ticker: e.symbol.clone(),
                        horizon: HORIZONS[e.horizon].into(),
                        decision: e.decision,
                        confidence_percent: e.confidence_percent,
                        original_reason: e.reason.chars().take(600).collect(),
                        original_risks: e
                            .risks
                            .iter()
                            .take(2)
                            .map(|s| s.chars().take(200).collect())
                            .collect(),
                        decided_at: e.generated,
                        reviewed_price_at: outcome.price_time,
                        realized_change_percent: outcome.change_percent,
                        correct: outcome.correct,
                    }
                })
                .collect(),
        }
    }
}

fn valid_currency(currency: &str) -> bool {
    (3..=8).contains(&currency.len()) && currency.chars().all(|c| c.is_ascii_alphabetic())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn analysis() -> Analysis {
        Analysis {
            symbol: "NVDA".into(),
            provider: "Meta".into(),
            model: META_MODEL.into(),
            endpoint: META_ENDPOINT.into(),
            generated: 1_800_000_000,
            context_time: 1_800_000_000 - 60,
            reference_price: Some(100.),
            reference_currency: "USD".into(),
            demo: false,
            horizons: [Decision::Buy, Decision::Sell, Decision::DoNotBuy]
                .into_iter()
                .map(|decision| Outlook {
                    decision: Some(decision),
                    confidence_percent: Some(68.),
                    summary: "Fixture: earnings evidence is limited.".into(),
                    risks: vec!["Fixture: valuation uncertainty".into()],
                    ..Default::default()
                })
                .collect(),
            raw: "Fixture only, not provider output".into(),
            context_notes: vec![],
            usage: None,
        }
    }
    pub fn prices(at: i64, close: f64) -> ReviewData {
        let mut market = demo_market("NVDA", 6).unwrap();
        market.quote.demo = false;
        market.quote.currency = "USD".into();
        market.points = vec![Point {
            time: at,
            close,
            volume: Some(1.),
        }];
        ReviewData {
            market,
            splits: vec![],
        }
    }
    fn reviewed_entry(
        horizon: usize,
        decision: Decision,
        close: f64,
        generated: i64,
    ) -> Prediction {
        let mut a = analysis();
        a.generated = generated;
        a.context_time = generated - 60;
        a.horizons[horizon].decision = Some(decision);
        let mut journal = Journal::default();
        journal.record(&a);
        let id = journal.entries[horizon].id;
        let due = journal.entries[horizon].review_at;
        assert_eq!(journal.resolve(&[id], &prices(due, close), due + DAY), 1);
        journal.entries.remove(horizon)
    }
    #[test]
    fn benchmarks_compare_fixed_actions_on_the_exact_same_calls() {
        let generated = analysis().generated;
        let journal = Journal {
            entries: vec![
                reviewed_entry(0, Decision::Buy, 110., generated),
                reviewed_entry(0, Decision::Buy, 95., generated + DAY),
                reviewed_entry(0, Decision::Sell, 100., generated + 2 * DAY),
            ],
        };
        let scores = journal.benchmarks(&Provider::meta(), None, generated + 400 * DAY);
        assert_eq!(
            scores[0],
            Benchmark {
                reviewed: 3,
                correct: 1,
                always_buy: 1,
                always_sell: 1,
                always_wait: 2,
                repeated: 0
            }
        );
        assert_eq!(scores[1], Benchmark::default());
        assert_eq!(scores[0].rate(scores[0].correct), "33%");
        assert_eq!(scores[0].rate(scores[0].always_wait), "67%");
        assert_eq!(scores[1].rate(0), "—");
    }
    #[test]
    fn benchmark_boundaries_and_split_adjustments_match_outcome_scoring() {
        let generated = analysis().generated;
        let mut split = reviewed_entry(0, Decision::Buy, 110., generated);
        let o = split.outcome.as_mut().unwrap();
        o.price = 27.5;
        o.comparison_reference_price = 25.;
        let journal = Journal {
            entries: vec![
                split,
                reviewed_entry(0, Decision::DoNotBuy, 102., generated + DAY),
                reviewed_entry(0, Decision::Sell, 98., generated + 2 * DAY),
            ],
        };
        let scores = journal.benchmarks(&Provider::meta(), None, generated + 400 * DAY);
        assert_eq!(scores[0].reviewed, 3);
        assert_eq!(scores[0].correct, 2);
        assert_eq!(scores[0].always_buy, 1);
        assert_eq!(scores[0].always_sell, 0);
        assert_eq!(scores[0].always_wait, 2);
    }
    #[test]
    fn benchmark_deduplication_keeps_first_reviewed_call_without_rewriting_history() {
        let generated = analysis().generated;
        let first = reviewed_entry(0, Decision::Sell, 110., generated);
        let mut later = reviewed_entry(0, Decision::Buy, 110., generated + 60);
        later.id = 100;
        let journal = Journal {
            entries: vec![later, first],
        };
        let at = generated + 400 * DAY;
        let score = journal.benchmarks(&Provider::meta(), None, at)[0];
        assert_eq!(score.reviewed, 1);
        assert_eq!(score.repeated, 1);
        assert_eq!(
            score.correct, 0,
            "The later correct retry must not replace the first call"
        );
        assert_eq!(journal.entries.len(), 2);
        assert_eq!(
            journal.entries[0].id, 100,
            "Comparison must not reorder saved calls"
        );
        assert_eq!(
            journal
                .learning_context(&Provider::meta(), 365, at)
                .window_days,
            365
        );
    }
    #[test]
    fn benchmarks_keep_tickers_horizons_and_connections_separate() {
        let generated = analysis().generated;
        let base = reviewed_entry(0, Decision::Buy, 110., generated);
        let mut other_symbol = base.clone();
        other_symbol.symbol = "AAPL".into();
        let mut entries = vec![
            base.clone(),
            other_symbol,
            reviewed_entry(1, Decision::Buy, 110., generated),
            reviewed_entry(2, Decision::Buy, 110., generated),
        ];
        for field in ["provider", "model", "endpoint"] {
            let mut other = base.clone();
            match field {
                "provider" => other.provider = "Google".into(),
                "model" => other.model = "other-model".into(),
                _ => other.endpoint = "http://localhost:8000".into(),
            }
            entries.push(other);
        }
        let journal = Journal { entries };
        let at = generated + 400 * DAY;
        let all = journal.benchmarks(&Provider::meta(), None, at);
        assert_eq!(all.map(|s| s.reviewed), [2, 1, 1]);
        assert_eq!(
            journal
                .benchmarks(&Provider::meta(), Some("NVDA"), at)
                .map(|s| s.reviewed),
            [1, 1, 1]
        );
        assert_eq!(
            journal.benchmarks(&Provider::meta(), Some("MISSING"), at),
            [Benchmark::default(); 3]
        );
    }
    #[test]
    fn benchmarks_exclude_pending_sample_future_and_invalid_reviews() {
        let generated = analysis().generated;
        let valid = reviewed_entry(0, Decision::Buy, 110., generated);
        let at = generated + 40 * DAY;
        for case in [
            "pending",
            "sample",
            "future-check",
            "incomplete-session",
            "stale-reference",
            "unknown-currency",
            "nonpositive",
            "nonfinite",
            "late-price",
            "early-price",
            "changed-checkpoint",
            "future-generation",
            "invalid-horizon",
        ] {
            let mut entry = valid.clone();
            match case {
                "pending" => entry.outcome = None,
                "sample" => entry.demo = true,
                "future-check" => entry.outcome.as_mut().unwrap().checked_at = at + 1,
                "incomplete-session" => {
                    let o = entry.outcome.as_mut().unwrap();
                    o.checked_at = o.price_time + DAY - 1;
                }
                "stale-reference" => entry.quote_time = generated - 8 * DAY,
                "unknown-currency" => entry.currency = "?".into(),
                "nonpositive" => entry.outcome.as_mut().unwrap().comparison_reference_price = 0.,
                "nonfinite" => entry.outcome.as_mut().unwrap().price = f64::NAN,
                "late-price" => {
                    entry.outcome.as_mut().unwrap().price_time = entry.review_at + 8 * DAY
                }
                "early-price" => entry.outcome.as_mut().unwrap().price_time = entry.review_at - DAY,
                "changed-checkpoint" => entry.review_at -= DAY,
                "future-generation" => entry.generated = at + 1,
                _ => entry.horizon = 9,
            }
            let journal = Journal {
                entries: vec![entry],
            };
            assert_eq!(
                journal.benchmarks(&Provider::meta(), None, at),
                [Benchmark::default(); 3],
                "{case}"
            );
            assert_eq!(
                journal
                    .learning_context(&Provider::meta(), 180, at)
                    .reviewed,
                0,
                "{case}"
            );
        }
    }
    #[test]
    fn regenerated_predictions_survive_restart_without_rewriting_originals() {
        let mut journal = Journal::default();
        let mut a = analysis();
        assert_eq!(journal.record(&a), 3);
        let due = journal.entries[0].review_at;
        a.generated += 60;
        a.horizons[0].decision = Some(Decision::Sell);
        a.horizons[0].confidence_percent = Some(51.);
        journal.record(&a);
        assert_eq!(journal.entries.len(), 6);
        assert_eq!(journal.entries[0].decision, Decision::Buy);
        assert_eq!(journal.entries[0].confidence_percent, Some(68.));
        assert_eq!(journal.entries[0].review_at, due);
        assert_eq!(journal.entries[3].decision, Decision::Sell);
        let path = std::env::temp_dir().join(format!(
            "tenerus-journal-restart-{}.json",
            std::process::id()
        ));
        journal.save(&path).unwrap();
        let restored = Journal::load(&path).unwrap();
        assert_eq!(restored.entries.len(), 6);
        assert_eq!(restored.entries[5].id, 6);
        assert_eq!(restored.entries[0].reason, a.horizons[0].summary);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn review_uses_the_first_completed_checkpoint_session_and_freezes_the_result() {
        let mut journal = Journal::default();
        journal.record(&analysis());
        let due = journal.entries[0].review_at;
        let mut data = prices(due + DAY, 110.);
        data.market.points.insert(
            0,
            Point {
                time: due - DAY,
                close: 50.,
                volume: Some(0.),
            },
        );
        data.market.points.push(Point {
            time: due + 3 * DAY,
            close: 60.,
            volume: Some(0.),
        });
        assert_eq!(journal.resolve(&[1, 2, 3], &data, due - 1), 0);
        assert_eq!(journal.resolve(&[1, 2, 3], &data, due + 2 * DAY), 1);
        let o = journal.entries[0].outcome.as_ref().unwrap();
        assert_eq!(o.price, 110.);
        assert_eq!(o.change_percent, 10.);
        assert!(o.correct);
        assert!(journal.entries[1].outcome.is_none());
        assert_eq!(journal.entries[0].reference_price, Some(100.));
        assert_eq!(
            journal.resolve(&[1], &prices(due + DAY, 20.), due + 10 * DAY),
            0
        );
        assert_eq!(journal.entries[0].outcome.as_ref().unwrap().price, 110.);
    }
    #[test]
    fn missing_unfinished_wrong_currency_and_sample_prices_never_become_results() {
        for case in [
            "unfinished",
            "missing",
            "too_late",
            "currency",
            "symbol",
            "demo",
            "stale_reference",
            "missing_reference",
        ] {
            let mut journal = Journal::default();
            journal.record(&analysis());
            let due = journal.entries[0].review_at;
            let mut data = prices(due, 110.);
            let mut at = due + 10 * DAY;
            match case {
                "unfinished" => at = due + DAY - 1,
                "missing" => data.market.points.clear(),
                "too_late" => data.market.points[0].time = due + 8 * DAY,
                "currency" => data.market.quote.currency = "EUR".into(),
                "symbol" => data.market.quote.symbol = "AAPL".into(),
                "demo" => data.market.quote.demo = true,
                "stale_reference" => journal.entries[0].quote_time -= 8 * DAY,
                "missing_reference" => journal.entries[0].reference_price = None,
                _ => unreachable!(),
            }
            assert_eq!(journal.resolve(&[1], &data, at), 0, "{case}");
            assert!(journal.entries[0].outcome.is_none(), "{case}");
        }
    }
    #[test]
    fn action_scoring_has_explicit_thresholds_including_the_boundaries() {
        for (decision, expected) in [
            (Decision::Buy, [false, false, false, false, true]),
            (Decision::Sell, [true, false, false, false, false]),
            (Decision::DoNotBuy, [true, true, true, true, false]),
        ] {
            for (price, correct) in [95., 98., 100., 102., 105.].into_iter().zip(expected) {
                let mut journal = Journal::default();
                let mut a = analysis();
                a.horizons[0].decision = Some(decision);
                journal.record(&a);
                let due = journal.entries[0].review_at;
                assert_eq!(journal.resolve(&[1], &prices(due, price), due + DAY), 1);
                assert_eq!(
                    journal.entries[0].outcome.as_ref().unwrap().correct,
                    correct,
                    "{decision:?} at {price}"
                );
            }
        }
    }
    #[test]
    fn reported_splits_keep_reference_and_historical_close_on_the_same_basis() {
        let mut journal = Journal::default();
        journal.record(&analysis());
        let due = journal.entries[0].review_at;
        let mut data = prices(due, 27.5);
        data.splits = vec![
            StockSplit {
                time: due - DAY,
                factor: 2.,
            },
            StockSplit {
                time: due + 3 * DAY,
                factor: 2.,
            },
            StockSplit {
                time: due + 90 * DAY,
                factor: 10.,
            },
        ];
        assert_eq!(journal.resolve(&[1], &data, due + 5 * DAY), 1);
        let o = journal.entries[0].outcome.as_ref().unwrap();
        assert_eq!(o.comparison_reference_price, 25.);
        assert_eq!(o.change_percent, 10.);
        assert!(o.correct);
        assert_eq!(journal.entries[0].reference_price, Some(100.));
    }
    #[test]
    fn learning_uses_recent_resolved_outcomes_for_the_same_connection_only() {
        let mut journal = Journal::default();
        journal.record(&analysis());
        let due = journal.entries[0].review_at;
        journal.resolve(&[1], &prices(due, 95.), due + DAY);
        let provider = Provider::meta();
        let at = due + 30 * DAY;
        let memory = journal.learning_context(&provider, 180, at);
        assert_eq!(memory.reviewed, 1);
        assert_eq!(memory.correct, 0);
        assert_eq!(memory.examples[0].decision, Decision::Buy);
        assert_eq!(memory.examples[0].realized_change_percent, -5.);
        assert_eq!(
            memory.examples[0].original_reason,
            analysis().horizons[0].summary
        );
        assert_eq!(journal.learning_context(&provider, 0, at).reviewed, 0);
        assert_eq!(
            journal.learning_context(&provider, 180, due - 1).reviewed,
            0
        );
        assert_eq!(
            journal
                .learning_context(&provider, 180, due + 181 * DAY)
                .reviewed,
            0
        );
        let mut other = provider.clone();
        other.model = "different-model".into();
        assert_eq!(journal.learning_context(&other, 180, at).reviewed, 0);
        other = provider.clone();
        other.endpoint = "https://other.example/v1".into();
        assert_eq!(journal.learning_context(&other, 180, at).reviewed, 0);
        journal.entries[0].demo = true;
        assert_eq!(journal.learning_context(&provider, 180, at).reviewed, 0);
        assert_eq!(
            journal.entries.len(),
            3,
            "Expired context must not delete the logbook"
        );
    }
    #[test]
    fn memory_is_bounded_but_counts_every_eligible_review() {
        let mut journal = Journal::default();
        journal.record(&analysis());
        let due = journal.entries[0].review_at;
        journal.resolve(&[1], &prices(due, 110.), due + DAY);
        let entry = journal.entries[0].clone();
        for i in 0..30 {
            let mut entry = entry.clone();
            entry.id = i + 100;
            entry.reason = "long reason ".repeat(200);
            journal.entries.push(entry);
        }
        let memory = journal.learning_context(&Provider::meta(), 180, due + 2 * DAY);
        assert_eq!(memory.reviewed, 31);
        assert_eq!(memory.examples.len(), 24);
        assert!(
            memory
                .examples
                .iter()
                .all(|e| e.original_reason.chars().count() <= 600)
        );
    }
    #[test]
    fn damaged_journal_is_reported_without_touching_the_saved_file() {
        let path = std::env::temp_dir().join(format!(
            "tenerus-journal-damaged-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, b"broken saved file").unwrap();
        assert!(Journal::load(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"broken saved file");
        std::fs::remove_file(path).unwrap();
    }
}
