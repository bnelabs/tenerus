//! Deterministic local headline sentiment and event-risk analyzer.
//! Treats all external headline text strictly as untrusted input data.
use crate::model::Article;
use serde::Serialize;

const DAY: i64 = 86_400;

#[derive(Clone, Debug, Default, Serialize)]
pub struct NewsSignal {
    pub article_count: usize,
    /// Mean headline polarity in [-1.0, +1.0], or 0.0 when no live headlines exist.
    pub sentiment_score: f64,
    pub sentiment_label: String,
    pub positive_count: usize,
    pub negative_count: usize,
    pub neutral_count: usize,
    pub event_tags: Vec<String>,
    pub earnings_in_horizon: bool,
    pub days_to_earnings: Option<i64>,
    pub top_headlines: Vec<String>,
}

impl NewsSignal {
    pub fn has_signal(&self) -> bool {
        self.article_count > 0 || self.earnings_in_horizon
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.article_count > 0 {
            parts.push(format!(
                "{} live headline{} · {} ({:+.2}; +{} / −{} / {} neutral)",
                self.article_count,
                if self.article_count == 1 { "" } else { "s" },
                self.sentiment_label,
                self.sentiment_score,
                self.positive_count,
                self.negative_count,
                self.neutral_count,
            ));
        } else {
            parts.push("No live headlines supplied".into());
        }
        if !self.event_tags.is_empty() {
            parts.push(format!("Events: {}", self.event_tags.join(", ")));
        }
        if self.earnings_in_horizon {
            if let Some(days) = self.days_to_earnings {
                parts.push(format!(
                    "Expected earnings in ~{days} day{} (inside forecast window — higher gap risk)",
                    if days == 1 { "" } else { "s" }
                ));
            } else {
                parts.push("Expected earnings falls inside forecast window".into());
            }
        }
        parts.join(". ")
    }
}

const POSITIVE_TERMS: &[(&str, f64)] = &[
    ("beats", 1.0),
    ("beat", 0.9),
    ("tops", 0.8),
    ("surges", 1.0),
    ("surge", 0.9),
    ("soars", 1.0),
    ("rallies", 0.9),
    ("rally", 0.8),
    ("jumps", 0.8),
    ("gains", 0.7),
    ("outperforms", 1.0),
    ("outperform", 0.9),
    ("upgrade", 1.0),
    ("upgraded", 1.0),
    ("upgrades", 1.0),
    ("raises", 0.9),
    ("raised", 0.8),
    ("record", 0.6),
    ("strong", 0.7),
    ("growth", 0.6),
    ("breakout", 0.8),
    ("buyback", 0.7),
    ("repurchase", 0.7),
    ("approval", 0.9),
    ("approved", 0.9),
    ("partnership", 0.6),
    ("expands", 0.6),
    ("profit", 0.6),
    ("bullish", 0.9),
    ("boom", 0.8),
    ("accelerate", 0.7),
    ("accelerates", 0.7),
];

const NEGATIVE_TERMS: &[(&str, f64)] = &[
    ("misses", 1.0),
    ("miss", 0.9),
    ("plunges", 1.0),
    ("plunge", 1.0),
    ("tumbles", 1.0),
    ("slumps", 1.0),
    ("drops", 0.8),
    ("falls", 0.7),
    ("sinks", 0.9),
    ("crashes", 1.0),
    ("downgrade", 1.0),
    ("downgraded", 1.0),
    ("downgrades", 1.0),
    ("cuts", 0.8),
    ("cut", 0.7),
    ("slashed", 1.0),
    ("slashes", 1.0),
    ("warning", 0.9),
    ("warns", 0.9),
    ("weak", 0.8),
    ("loss", 0.8),
    ("losses", 0.8),
    ("lawsuit", 0.9),
    ("sued", 0.9),
    ("probe", 0.9),
    ("investigation", 0.9),
    ("subpoena", 1.0),
    ("recall", 0.9),
    ("layoffs", 0.8),
    ("layoff", 0.8),
    ("antitrust", 0.8),
    ("tariff", 0.6),
    ("tariffs", 0.6),
    ("sanctions", 0.8),
    ("bearish", 0.9),
    ("default", 1.0),
    ("bankruptcy", 1.0),
    ("delay", 0.7),
    ("delays", 0.7),
    ("decline", 0.7),
    ("declines", 0.7),
    ("shortfall", 0.9),
];

const NEGATORS: &[&str] = &["not", "no", "never", "without", "fails", "failed", "below"];

const EVENT_RULES: &[(&str, &[&str])] = &[
    (
        "Earnings",
        &[
            "earnings",
            "eps",
            "quarter",
            "quarterly",
            "q1",
            "q2",
            "q3",
            "q4",
            "revenue",
            "results",
            "profit",
        ],
    ),
    (
        "Guidance",
        &[
            "guidance", "outlook", "forecast", "raises", "cuts", "targets",
        ],
    ),
    (
        "Analyst",
        &[
            "upgrade",
            "upgraded",
            "downgrade",
            "downgraded",
            "price target",
            "rating",
            "outperform",
            "underperform",
            "overweight",
            "underweight",
            "analyst",
        ],
    ),
    (
        "M&A / Capital",
        &[
            "acquire",
            "acquisition",
            "merger",
            "buyout",
            "buyback",
            "repurchase",
            "dividend",
            "split",
            "offering",
            "ipo",
            "spin-off",
        ],
    ),
    (
        "Legal / Regulatory",
        &[
            "sec",
            "ftc",
            "doj",
            "fda",
            "antitrust",
            "lawsuit",
            "sued",
            "probe",
            "investigation",
            "regulator",
            "subpoena",
            "recall",
            "fine",
            "settlement",
        ],
    ),
    (
        "Macro / Policy",
        &[
            "fed",
            "inflation",
            "cpi",
            "interest rate",
            "rates",
            "tariff",
            "tariffs",
            "sanctions",
            "recession",
            "treasury",
            "jobs report",
        ],
    ),
];

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '\'')
        .filter(|t| !t.is_empty())
        .map(|t| t.trim_matches('\'').to_ascii_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

pub fn score_headline(title: &str) -> f64 {
    let tokens = tokenize(title);
    if tokens.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0;
    let mut hits = 0.0;
    for (idx, token) in tokens.iter().enumerate() {
        let negated = idx > 0
            && (NEGATORS.contains(&tokens[idx - 1].as_str())
                || (idx > 1 && NEGATORS.contains(&tokens[idx - 2].as_str())));
        let sign = if negated { -1.0 } else { 1.0 };
        if let Some((_, w)) = POSITIVE_TERMS.iter().find(|(term, _)| *term == token) {
            sum += sign * *w;
            hits += *w;
        } else if let Some((_, w)) = NEGATIVE_TERMS.iter().find(|(term, _)| *term == token) {
            sum -= sign * *w;
            hits += *w;
        }
    }
    if hits == 0.0 {
        0.0
    } else {
        (sum / (hits + 0.5)).clamp(-1.0, 1.0)
    }
}

pub fn analyze_headlines(
    articles: &[Article],
    earnings_start: Option<i64>,
    reference_time: i64,
    horizon_sessions: usize,
) -> NewsSignal {
    let live: Vec<&Article> = articles
        .iter()
        .filter(|a| !a.demo && !a.title.trim().is_empty())
        .take(15)
        .collect();
    let mut total = 0.0;
    let mut positive_count = 0usize;
    let mut negative_count = 0usize;
    let mut neutral_count = 0usize;
    let mut tag_set = std::collections::BTreeSet::new();

    for article in &live {
        let score = score_headline(&article.title);
        total += score;
        if score >= 0.15 {
            positive_count += 1;
        } else if score <= -0.15 {
            negative_count += 1;
        } else {
            neutral_count += 1;
        }
        let lower = article.title.to_ascii_lowercase();
        let tokens = tokenize(&article.title);
        for (tag, keywords) in EVENT_RULES {
            if keywords.iter().any(|kw| {
                if kw.contains(' ') {
                    lower.contains(kw)
                } else {
                    tokens.iter().any(|t| t == kw)
                }
            }) {
                tag_set.insert((*tag).to_string());
            }
        }
    }

    let article_count = live.len();
    let sentiment_score = if article_count == 0 {
        0.0
    } else {
        (total / article_count as f64).clamp(-1.0, 1.0)
    };
    let sentiment_label = if article_count == 0 {
        "No live headlines"
    } else if sentiment_score >= 0.18 {
        "Positive tone"
    } else if sentiment_score <= -0.18 {
        "Negative tone"
    } else if positive_count > 0 && negative_count > 0 {
        "Mixed tone"
    } else {
        "Neutral tone"
    }
    .to_string();

    // Convert trading sessions (~5 per 7 calendar days) to a calendar lookahead window.
    let calendar_window = ((horizon_sessions as i64) * 7 / 5 + 3).max(7) * DAY;
    let (earnings_in_horizon, days_to_earnings) = earnings_start
        .filter(|&ts| ts > 0 && reference_time > 0 && ts >= reference_time - DAY)
        .map(|ts| {
            let delta = ts - reference_time;
            let days = (delta.max(0) + DAY / 2) / DAY;
            (delta <= calendar_window, Some(days))
        })
        .unwrap_or((false, None));

    let top_headlines = live
        .iter()
        .take(5)
        .map(|a| {
            let title: String = a.title.chars().take(140).collect();
            if a.publisher.trim().is_empty() {
                title
            } else {
                format!("{title} ({})", a.publisher.trim())
            }
        })
        .collect();

    NewsSignal {
        article_count,
        sentiment_score,
        sentiment_label,
        positive_count,
        negative_count,
        neutral_count,
        event_tags: tag_set.into_iter().collect(),
        earnings_in_horizon,
        days_to_earnings,
        top_headlines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn article(title: &str, demo: bool) -> Article {
        Article {
            title: title.into(),
            publisher: "Wire".into(),
            link: "https://example.com/news".into(),
            date: "Today".into(),
            demo,
        }
    }

    #[test]
    fn scores_positive_negative_and_negated_financial_headlines() {
        assert!(score_headline("Company beats Q3 earnings and raises full-year guidance") > 0.4);
        assert!(score_headline("Stock plunges after SEC probe and analyst downgrade") < -0.4);
        assert!(score_headline("Company fails to beat earnings expectations") < 0.0);
        assert_eq!(
            score_headline("Annual shareholder meeting scheduled for May"),
            0.0
        );
    }

    #[test]
    fn excludes_demo_articles_tags_events_and_flags_earnings_in_horizon() {
        let now_ts = 1_800_000_000;
        let articles = vec![
            article("Demo article surges record profit", true),
            article(
                "Chipmaker beats quarterly revenue and raises guidance",
                false,
            ),
            article("Regulator opens antitrust probe into cloud unit", false),
        ];
        let signal = analyze_headlines(&articles, Some(now_ts + 10 * DAY), now_ts, 20);
        assert_eq!(signal.article_count, 2);
        assert_eq!(signal.positive_count, 1);
        assert_eq!(signal.negative_count, 1);
        assert!(signal.event_tags.iter().any(|t| t == "Earnings"));
        assert!(signal.event_tags.iter().any(|t| t == "Guidance"));
        assert!(signal.event_tags.iter().any(|t| t == "Legal / Regulatory"));
        assert!(signal.earnings_in_horizon);
        assert_eq!(signal.days_to_earnings, Some(10));

        let far = analyze_headlines(&articles, Some(now_ts + 45 * DAY), now_ts, 5);
        assert!(!far.earnings_in_horizon);
    }
}
