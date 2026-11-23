mod office;
#[cfg(target_os = "macos")]
mod pdf;
#[cfg(any(not(target_os = "macos"), test))]
mod pdf_portable;

use crate::{
    briefing::{Briefing, Input},
    disclaimer,
    model::{self, Analysis},
    scenarios::{History, Replay, Scenario},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy)]
pub enum Format {
    Excel,
    Pdf,
    Word,
}

impl Format {
    pub const ALL: &'static [Self] = &[Self::Excel, Self::Pdf, Self::Word];

    pub fn label(self) -> &'static str {
        match self {
            Self::Excel => "Excel (.xlsx)",
            Self::Pdf => "PDF (.pdf)",
            Self::Word => "Word (.docx)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Excel => "xlsx",
            Self::Pdf => "pdf",
            Self::Word => "docx",
        }
    }
}

pub struct Report {
    title: String,
    kind: &'static str,
    ticker: String,
    sections: Vec<Section>,
}

struct Section {
    title: String,
    rows: Vec<(String, Cell)>,
}

enum Cell {
    Text(String),
    Number(f64),
    Percent(f64),
}

impl Cell {
    fn display(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Number(number) if number.fract() == 0. && number.abs() < 1e12 => {
                format!("{number:.0}")
            }
            Self::Number(number) => model::price(*number),
            Self::Percent(fraction) => format!("{:.2}%", fraction * 100.),
        }
    }
}

impl Section {
    fn text(&mut self, label: &str, value: impl Into<String>) {
        let value = value.into();
        if !value.trim().is_empty() {
            self.rows.push((label.into(), Cell::Text(value)));
        }
    }

    fn number(&mut self, label: &str, value: f64) {
        if value.is_finite() {
            self.rows.push((label.into(), Cell::Number(value)));
        }
    }

    fn percent(&mut self, label: &str, value: f64) {
        if value.is_finite() {
            self.rows.push((label.into(), Cell::Percent(value)));
        }
    }
}

impl Report {
    fn new(ticker: &str, title: &str, kind: &'static str) -> Self {
        Self {
            title: format!("{ticker} {title}"),
            kind,
            ticker: ticker.into(),
            sections: vec![],
        }
    }

    fn section(&mut self, title: &str) -> &mut Section {
        self.sections.push(Section {
            title: title.into(),
            rows: vec![],
        });
        self.sections.last_mut().unwrap()
    }

    pub fn research(analysis: &Analysis) -> Self {
        let mut report = Self::new(&analysis.symbol, "AI Research", "research");
        let info = report.section("Research Source");
        info.text("Provider", &analysis.provider);
        info.text("Model", &analysis.model);
        info.text("Generated", model::stamp(analysis.generated));
        info.text("Quote observed", model::stamp(analysis.context_time));
        info.text(
            "Price data",
            if analysis.demo {
                "Simulated sample data"
            } else {
                "Public market data"
            },
        );
        if let Some(price) = analysis.reference_price {
            info.number("Reference price", price);
            info.text("Currency", &analysis.reference_currency);
        }
        for (index, outlook) in analysis
            .horizons
            .iter()
            .take(model::HORIZONS.len())
            .enumerate()
        {
            let section = report.section(model::HORIZONS[index]);
            section.text("Window", model::PERIODS[index]);
            if let Some(decision) = outlook.decision {
                section.text("Model decision", decision.label());
            } else {
                section.text("Model outlook", &outlook.bias);
            }
            if let Some(confidence) = outlook.confidence_value() {
                section.percent("AI confidence", confidence / 100.);
            } else {
                section.text("AI confidence", "Percentage unavailable");
            }
            section.text("Rationale", &outlook.summary);
            section.text("Catalysts", outlook.catalysts.join("\n"));
            section.text("Risks", outlook.risks.join("\n"));
            section.text("Model suggestion", &outlook.suggestion);
        }
        let notes = report.section("Context and Limitations");
        notes.text(
            "Confidence",
            "AI confidence is the model’s own estimate of its judgment.",
        );
        for note in &analysis.context_notes {
            notes.text("Research context", note);
        }
        if analysis.horizons.is_empty() {
            report.section("Provider Response").text("", &analysis.raw);
        }
        report
    }

    pub fn outlook(
        history: &History,
        scenario: &Scenario,
        data_source: &str,
        basis: &str,
        trials: Option<&[Replay]>,
        revealed: Option<&Replay>,
    ) -> Self {
        let mut report = Self::new(&history.ticker, "Historical Outlook", "outlook");
        let input = report.section("Price History");
        input.text("Data source", data_source);
        input.text("Price basis", basis);
        input.text("Currency", &history.currency);
        input.text("History retrieved", model::stamp(history.fetched));
        input.text("Reference date", date(scenario.anchor.time));
        input.number("Reference close", scenario.anchor.price);
        input.number(
            "Horizon in trading sessions",
            (scenario.fan.len() - 1) as f64,
        );
        if trials.is_none() {
            input.text("View", "Historical replay");
        }

        let results = report.section("Historical Scenarios");
        let band = scenario.end();
        let cal = scenario.calibration;
        results.number("Comparable periods", scenario.matches.len() as f64);
        results.number(
            "Comparable periods ending higher",
            scenario.up_count() as f64,
        );
        results.percent("10th percentile return (calibrated)", band.low);
        results.percent("Median return (calibrated)", band.mid);
        results.percent("90th percentile return (calibrated)", band.high);
        results.text(
            "Self-calibrating ensemble weights",
            format!(
                "Kernel analogs {:.0}% ({:.1} effective) · Online QR {:.0}% · GARCH(1,1) volatility {:.0}% · Baseline {:.0}%",
                cal.weights.analogs * 100.,
                cal.effective_matches,
                cal.weights.online_qr * 100.,
                cal.weights.garch * 100.,
                cal.weights.baseline * 100.,
            ),
        );
        results.percent("Split-conformal interval adjustment", cal.conformal_delta);
        if let (Some(rs), Some(mv)) = (scenario.setup.rel_strength_20, scenario.setup.market_vol_20)
        {
            results.text(
                "Cross-asset S&P 500 regime",
                format!(
                    "20d relative strength {:+.1}% vs ^GSPC · S&P 500 daily vol {:.2}%",
                    rs * 100.,
                    mv * 100.
                ),
            );
        }
        if let Some(delta) = cal.sentiment_pinball_delta {
            results.text(
                "Walk-forward news sentiment ablation",
                format!(
                    "{:+.2} pp pinball delta across {} causal trials ({})",
                    delta * 100.,
                    cal.sentiment_ablation_trials,
                    if delta < 0. {
                        "stored headlines reduced error"
                    } else {
                        "stored headlines did not improve over price/regime features"
                    }
                ),
            );
        }
        results.text(
            "Method",
            "Self-calibrating 4-model walk-forward ensemble combining kernel/recency-weighted multivariable analogues (with S&P 500 regime), online L2-regularized quantile regression (with causal daily news history), mean-reverting GARCH(1,1) fat-tailed skew-adjusted volatility, and unconditional baseline with split-conformal interval calibration.",
        );

        if let Some(trials) = trials.filter(|trials| !trials.is_empty()) {
            let checks = report.section("Walk-Forward Evaluation");
            checks.number("Earlier dates evaluated", trials.len() as f64);
            checks.number(
                "Ends inside calibrated ensemble range",
                trials.iter().filter(|r| r.inside).count() as f64,
            );
            checks.number(
                "Ends inside pre-conformal ensemble range",
                trials.iter().filter(|r| r.raw_inside).count() as f64,
            );
            checks.number(
                "Ends inside unconditional baseline range",
                trials.iter().filter(|r| r.baseline_inside).count() as f64,
            );
            checks.percent(
                "Calibrated ensemble mean pinball loss",
                trials.iter().map(|r| r.pinball_loss).sum::<f64>() / trials.len() as f64,
            );
            checks.percent(
                "Unconditional baseline mean pinball loss",
                trials.iter().map(|r| r.baseline_pinball_loss).sum::<f64>() / trials.len() as f64,
            );
            checks.text(
                "Component model pinball losses",
                format!(
                    "Analogs {:.2} pp · Online QR {:.2} pp · GARCH(1,1) {:.2} pp · Baseline {:.2} pp (raw coverage {}/{}, baseline {}/{})",
                    cal.analog_pinball * 100.,
                    cal.online_qr_pinball * 100.,
                    cal.garch_pinball * 100.,
                    cal.baseline_pinball * 100.,
                    cal.raw_covered,
                    cal.trial_count,
                    cal.baseline_covered,
                    cal.trial_count,
                ),
            );
            checks.percent(
                "Median forecast average error",
                trials.iter().map(|r| r.absolute_error).sum::<f64>() / trials.len() as f64,
            );
            checks.percent(
                "All periods median average error",
                trials.iter().map(|r| r.baseline_error).sum::<f64>() / trials.len() as f64,
            );
        }
        if let Some(replay) = revealed {
            let result = report.section("Revealed Outcome");
            result.text("Outcome date", date(replay.through));
            result.percent("Realized return", replay.result);
            result.text(
                "Outcome relative to range",
                if replay.inside { "Inside" } else { "Outside" },
            );
        }

        let matches = report.section("Matched Historical Periods");
        for period in &scenario.matches {
            matches.percent(
                &format!(
                    "{} to {} (weight {:.1}%)",
                    date(period.start),
                    date(period.end),
                    period.weight * 100.
                ),
                period.realized,
            );
        }
        report
    }

    pub fn add_briefing(&mut self, assistant: &str, briefing: &Briefing, source: &Input) {
        let section = self.section("Assistant Briefing");
        section.text("Assistant", assistant);
        for (label, finding) in [
            ("Interpretation", &briefing.read),
            ("Countercase", &briefing.countercase),
            ("What would change it", &briefing.watch),
        ] {
            section.text(
                label,
                format!("{} [{}]", finding.text, finding.evidence.join(", ")),
            );
        }
        let evidence = self.section("Briefing Evidence");
        for item in &source.evidence {
            evidence.text(&format!("{} {}", item.id, item.label), &item.fact);
        }
    }

    pub fn add_news_signal(&mut self, signal: &crate::news_signal::NewsSignal) {
        if !signal.has_signal() {
            return;
        }
        let section = self.section("News and Event Signal");
        section.text("Summary", signal.summary());
        if !signal.event_tags.is_empty() {
            section.text("Detected event tags", signal.event_tags.join(", "));
        }
        if !signal.top_headlines.is_empty() {
            section.text("Recent headlines", signal.top_headlines.join("\n"));
        }
    }

    pub fn filename(&self, format: Format) -> String {
        let ticker: String = self
            .ticker
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("Tenerus-{ticker}-{}.{}", self.kind, format.extension())
    }

    pub fn write(&self, format: Format, path: &Path, replace: bool) -> Result<(), String> {
        let bytes = match format {
            Format::Excel => office::excel(self),
            Format::Word => office::word(self),
            #[cfg(target_os = "macos")]
            Format::Pdf => pdf::encode(self)?,
            #[cfg(not(target_os = "macos"))]
            Format::Pdf => pdf_portable::encode(self)?,
        };
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(&bytes).map_err(|e| e.to_string())?;
        if replace {
            temp.persist(path)
        } else {
            temp.persist_noclobber(path)
        }
        .map(|_| ())
        .map_err(|e| e.to_string())
    }
}

fn date(time: i64) -> String {
    model::stamp(time)
        .split_whitespace()
        .next()
        .unwrap_or("Unavailable")
        .into()
}

pub fn with_extension(path: PathBuf, format: Format) -> (PathBuf, bool) {
    if path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(format.extension()))
    {
        return (path, false);
    }
    let mut name = path.into_os_string();
    name.push(".");
    name.push(format.extension());
    (name.into(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenarios;

    #[test]
    fn research_exports_keep_numeric_confidence_and_do_not_include_connection_details() {
        let mut analysis = crate::journal::tests::analysis();
        analysis.demo = true;
        analysis.endpoint = "https://private.invalid/path".into();
        analysis.horizons[0].summary = "İstanbul & Zürich <test> =example 😃".into();
        let report = Report::research(&analysis);
        let excel = String::from_utf8_lossy(&office::excel(&report)).into_owned();
        let word = String::from_utf8_lossy(&office::word(&report)).into_owned();
        assert!(excel.contains("Simulated sample data"));
        assert!(excel.contains(">0.68</v>"));
        for content in [excel, word] {
            assert!(content.contains("İstanbul &amp; Zürich &lt;test&gt; =example 😃"));
            assert!(content.contains(disclaimer::NOTICE));
            assert!(!content.contains("private.invalid"));
        }
    }

    #[test]
    fn replay_exports_only_include_outcome_after_reveal() {
        let history = History::sample("NVDA").unwrap();
        let report = scenarios::report(&history, 20).unwrap();
        let replay = &report.trials[0];
        let scenario = scenarios::replay(&history, replay.at, 20).unwrap().0;
        let export = Report::outlook(
            &history,
            &scenario,
            "Simulated sample data",
            "Daily closes",
            None,
            None,
        );
        assert!(
            export
                .sections
                .iter()
                .all(|section| section.title != "Revealed Outcome")
        );
        let export = Report::outlook(
            &history,
            &scenario,
            "Simulated sample data",
            "Daily closes",
            None,
            Some(replay),
        );
        assert!(
            export
                .sections
                .iter()
                .any(|section| section.title == "Revealed Outcome")
        );
    }

    // Set the directory when reviewing output from the built report writers.
    #[test]
    fn all_report_formats_write_to_disk() {
        let scratch = tempfile::tempdir().unwrap();
        let output = std::env::var_os("TENERUS_EXPORT_FIXTURES")
            .map(PathBuf::from)
            .unwrap_or_else(|| scratch.path().to_path_buf());
        let mut analysis = crate::journal::tests::analysis();
        analysis.symbol = "NVDA".into();
        analysis.demo = true;
        analysis.horizons[0].summary =
            "Mixed momentum leaves room for either direction. İstanbul, Zürich, São Paulo, 東京."
                .into();
        analysis.horizons[0].risks =
            vec!["A sudden earnings revision could break the historical comparison.".into()];
        analysis.context_notes = vec!["Fixture based on simulated prices.".into()];
        let history = History::sample("NVDA").unwrap();
        let scenario = scenarios::report(&history, 20).unwrap();
        for report in [
            Report::research(&analysis),
            Report::outlook(
                &history,
                &scenario.current,
                "Simulated sample data",
                "Simulated daily closes",
                Some(&scenario.trials),
                None,
            ),
        ] {
            for &format in Format::ALL {
                let file = output.join(report.filename(format));
                report.write(format, &file, true).unwrap();
                assert!(file.metadata().unwrap().len() > 1000);
                assert!(report.write(format, &file, false).is_err());
            }
            let portable = pdf_portable::encode(&report).unwrap();
            assert!(portable.starts_with(b"%PDF-1.4\n"));
            assert!(portable.ends_with(b"%%EOF\n"));
            let text = String::from_utf8_lossy(&portable);
            assert!(text.contains("Tenerus  /  1"));
            assert!(text.contains(disclaimer::NOTICE));
        }
    }
}
