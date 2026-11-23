use super::*;
use crate::{
    briefing::{self, Briefing, Engine, Finding, Input},
    market_data::{Imports, Source},
    report_export,
    scenarios::{self, Depth, HORIZONS, History, Replay, Report, Scenario},
};
use std::{
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const HISTORY_REFRESH: Duration = Duration::from_secs(30 * 60);
const HISTORY_CACHE_SIZE: usize = 12;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct BriefKey {
    ticker: String,
    engine: Engine,
    horizon: usize,
    at: i64,
    sample: bool,
    evidence: u64,
}

fn briefing_key(history: &History, scenario: &Scenario, input: &Input, engine: Engine) -> BriefKey {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for item in &input.evidence {
        item.fact.hash(&mut hash);
    }
    BriefKey {
        ticker: history.ticker.clone(),
        engine,
        horizon: scenario.fan.len() - 1,
        at: scenario.anchor.time,
        sample: history.sample,
        evidence: hash.finish(),
    }
}

type ReportCache = (Arc<History>, usize, Result<Arc<Report>, &'static str>);

struct Playback {
    point: Replay,
    scenario: Arc<Scenario>,
    realized: Vec<f64>,
    revealed: bool,
}

pub(super) struct Desk {
    pub active: bool,
    pub imports: Imports,
    horizon: usize,
    history_file: Option<PathBuf>,
    histories: HashMap<String, Arc<History>>,
    sample: Option<(String, Option<Arc<History>>)>,
    inflight: HashSet<String>,
    attempted: HashMap<String, Instant>,
    errors: HashMap<String, String>,
    report: Option<ReportCache>,
    playback: Option<Playback>,
    stress: f64,
    engines: Vec<Engine>,
    running: Option<(BriefKey, Arc<AtomicBool>)>,
    briefings: VecDeque<(BriefKey, Briefing)>,
    briefing_errors: Option<(BriefKey, String)>,
}
impl Desk {
    pub(super) fn new(active: bool, history_file: Option<PathBuf>) -> Self {
        let cached = history_file
            .as_ref()
            .filter(|p| p.metadata().is_ok_and(|m| m.len() < 5_000_000))
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<Vec<History>>(&b).ok())
            .unwrap_or_default();
        Self {
            active,
            horizon: 20,
            imports: Imports::load(
                history_file
                    .as_ref()
                    .map(|path| path.with_file_name("imported-history.json")),
            ),
            history_file,
            histories: cached
                .into_iter()
                .filter(|h| !h.sample && h.valid())
                .take(HISTORY_CACHE_SIZE)
                .map(|h| (h.ticker.clone(), Arc::new(h)))
                .collect(),
            sample: None,
            inflight: HashSet::new(),
            attempted: HashMap::new(),
            errors: HashMap::new(),
            report: None,
            playback: None,
            stress: -8.,
            engines: Engine::ALL
                .into_iter()
                .filter(|e| e.executable().is_some())
                .collect(),
            running: None,
            briefings: VecDeque::new(),
            briefing_errors: None,
        }
    }

    pub(super) fn clear_replay(&mut self) {
        self.playback = None;
    }

    pub(super) fn history(&mut self, ticker: &str, demo: bool) -> Option<Arc<History>> {
        if let Some(imported) = self.imports.get(ticker) {
            return Some(imported.history.clone());
        }
        if demo {
            if self.sample.as_ref().is_none_or(|(t, _)| t != ticker) {
                self.sample = Some((ticker.into(), History::sample(ticker).map(Arc::new)));
            }
            return self.sample.as_ref()?.1.clone();
        }
        self.histories.get(ticker).cloned()
    }

    fn report(&mut self, history: &Arc<History>) -> Result<Arc<Report>, &'static str> {
        let horizon = self.horizon;
        if self
            .report
            .as_ref()
            .is_none_or(|(h, n, _)| !Arc::ptr_eq(h, history) || *n != horizon)
        {
            self.report = Some((
                history.clone(),
                horizon,
                scenarios::report(history, horizon).map(Arc::new),
            ));
        }
        self.report.as_ref().unwrap().2.clone()
    }

    pub(super) fn finish_history(
        &mut self,
        ticker: String,
        result: Result<History, String>,
        offline: bool,
    ) {
        self.inflight.remove(&ticker);
        if offline {
            self.attempted.remove(&ticker);
            return;
        }
        match result {
            Ok(history) => {
                if !history.valid()
                    || history.closes.is_empty()
                    || self.histories.get(&ticker).is_some_and(|previous| {
                        history.closes.last().map(|b| b.time)
                            < previous.closes.last().map(|b| b.time)
                            || (history.closes.len() < 60
                                && history.closes.len() < previous.closes.len())
                    })
                {
                    self.errors.insert(
                        ticker,
                        "The data feed returned an incomplete history update.".into(),
                    );
                    return;
                }
                self.errors.remove(&ticker);
                self.histories.insert(ticker, Arc::new(history));
                let mut newest = self.histories.values().map(Arc::as_ref).collect::<Vec<_>>();
                newest.sort_unstable_by_key(|h| (std::cmp::Reverse(h.fetched), &h.ticker));
                newest.truncate(HISTORY_CACHE_SIZE);
                if let Some(path) = &self.history_file {
                    let _ = atomic_json(path, &newest);
                }
                let keep = newest
                    .iter()
                    .map(|h| h.ticker.clone())
                    .collect::<HashSet<_>>();
                self.histories.retain(|ticker, _| keep.contains(ticker));
            }
            Err(err) => {
                self.errors.insert(ticker, err);
            }
        }
    }

    pub(super) fn finish_briefing(&mut self, key: BriefKey, result: Result<Briefing, String>) {
        let Some((current, cancel)) = self.running.take() else {
            return;
        };
        if current != key {
            self.running = Some((current, cancel));
            return;
        }
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        match result {
            Ok(briefing) => {
                self.briefings.retain(|(k, _)| k != &key);
                self.briefings.push_front((key, briefing));
                self.briefings.truncate(24);
            }
            Err(err) => self.briefing_errors = Some((key, err)),
        }
    }
}
impl Drop for Desk {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.running {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl MarketApp {
    pub(super) fn using_outlook(&self) -> bool {
        self.desk.active && !self.saved_only && !self.business_news
    }

    pub(super) fn ensure_outlook_history(&mut self, ctx: &egui::Context) {
        if self.prefs.auto_refresh || !self.desk.attempted.contains_key(&self.prefs.selected) {
            self.load_outlook_history(ctx, false);
        }
    }

    pub(super) fn load_outlook_history(&mut self, ctx: &egui::Context, force: bool) {
        if !self.using_outlook() || self.prefs.offline || self.desk.inflight.len() >= 3 {
            return;
        }
        let ticker = self.prefs.selected.clone();
        if self.desk.imports.get(&ticker).is_some()
            || self.desk.inflight.contains(&ticker)
            || (!force
                && self
                    .desk
                    .attempted
                    .get(&ticker)
                    .is_some_and(|t| t.elapsed() < HISTORY_REFRESH))
        {
            return;
        }
        self.desk.inflight.insert(ticker.clone());
        self.desk.attempted.insert(ticker.clone(), Instant::now());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = network::fetch_scenario_history(&ticker);
            let _ = tx.send(Event::ScenarioHistory(ticker, result));
            ctx.request_repaint();
        });
    }

    pub(super) fn workspace_tabs(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        export_available: bool,
    ) -> Option<report_export::Format> {
        let action = ui
            .horizontal(|ui| {
                for (value, label) in [(true, "Outlook"), (false, "Price & news")] {
                    let selected = self.desk.active == value;
                    let response = ui.add(
                        egui::Button::new(
                            RichText::new(label)
                                .size(15.)
                                .color(if selected { p.text } else { p.muted })
                                .strong(),
                        )
                        .frame(false)
                        .min_size(vec2(0., 35.)),
                    );
                    if response.clicked() {
                        self.desk.active = value;
                        self.desk.clear_replay();
                        self.load_outlook_history(ui.ctx(), false);
                    }
                    if selected {
                        let r = response.rect.shrink2(vec2(5., 1.));
                        ui.painter()
                            .hline(r.x_range(), r.bottom(), Stroke::new(2.5, p.blue));
                    }
                }
                if export_available && self.desk.active {
                    ui.with_layout(Layout::right_to_left(Align::Center), support::export_menu)
                        .inner
                } else {
                    None
                }
            })
            .inner;
        ui.separator();
        action
    }

    pub(super) fn outlook(&mut self, root: &mut egui::Ui, p: Palette) {
        let ticker = self.prefs.selected.clone();
        let symbol = self.prefs.symbol(&ticker);
        let source = self.desk.imports.get(&ticker).map(|entry| entry.source);
        let quote = source.is_none().then(|| self.quote(&ticker)).flatten();
        let history = self.desk.history(&ticker, self.prefs.offline);
        let report = history.as_ref().map(|h| self.desk.report(h));
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(egui::Margin::symmetric(24, 19)),
            )
            .show(root, |ui| {
                self.outlook_header(ui, p, &symbol, quote.as_ref(), history.as_deref(), source);
                ui.add_space(6.);
                let ready = history
                    .as_deref()
                    .zip(report.as_ref().and_then(|result| result.as_ref().ok()));
                if let Some((format, (history, report))) =
                    self.workspace_tabs(ui, p, ready.is_some()).zip(ready)
                {
                    self.export_outlook(ui.ctx(), history, report, source, format);
                }
                ui.add_space(11.);
                let right_edge = ui.max_rect().right();
                egui::ScrollArea::vertical()
                    .id_salt("outlook_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let full = (right_edge - ui.cursor().left()).min(ui.available_width());
                        if full >= 765. {
                            let rail = (full * 0.26).clamp(237., 290.);
                            let gap = 19.;
                            let row = ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = gap * 2.;
                                let main = full - rail - gap * 2.;
                                let chart = ui.allocate_ui_with_layout(
                                    vec2(main, 0.),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        ui.set_width(main);
                                        self.outlook_panel(
                                            ui,
                                            p,
                                            history.as_deref(),
                                            report.as_ref(),
                                        );
                                    },
                                );
                                let available = (right_edge - ui.cursor().left()).clamp(0., rail);
                                ui.allocate_ui_with_layout(
                                    vec2(available, 0.),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        ui.set_width(available);
                                        self.outlook_rail(
                                            ui,
                                            p,
                                            history.as_deref(),
                                            report.as_ref(),
                                        );
                                    },
                                );
                                chart.response.rect
                            });
                            let x = row.inner.right() + gap;
                            ui.painter().vline(
                                x,
                                row.response.rect.y_range(),
                                Stroke::new(1., p.line),
                            );
                        } else {
                            self.outlook_panel(ui, p, history.as_deref(), report.as_ref());
                            ui.add_space(13.);
                            self.outlook_rail(ui, p, history.as_deref(), report.as_ref());
                        }
                        ui.add_space(13.);
                    });
            });
    }

    fn export_outlook(
        &mut self,
        ctx: &egui::Context,
        history: &History,
        report: &Report,
        source: Option<Source>,
        format: report_export::Format,
    ) {
        let playback = self.desk.playback.as_ref();
        let scenario = playback.map_or(&report.current, |replay| replay.scenario.as_ref());
        let trials = playback.is_none().then_some(report.trials.as_slice());
        let revealed = playback
            .filter(|replay| replay.revealed)
            .map(|replay| &replay.point);
        let (source, basis) = source
            .map(|entry| (entry.label(), entry.price_basis()))
            .unwrap_or_else(|| {
                if history.sample {
                    ("Simulated sample data", "Simulated daily closes")
                } else {
                    ("Yahoo Finance", "Published daily closes")
                }
            });
        let mut export =
            report_export::Report::outlook(history, scenario, source, basis, trials, revealed);
        let news_sig = playback.is_none().then(|| {
            self.outlook_news_signal(&history.ticker, scenario.fan.len().saturating_sub(1))
        });
        if let Some(sig) = &news_sig {
            export.add_news_signal(sig);
        }
        let mut input = Input::from_scenario(
            &history.ticker,
            &history.currency,
            history.sample,
            scenario,
            trials,
        );
        if let Some(sig) = &news_sig {
            input = input.with_news_signal(sig);
        }
        let engine = self.prefs.briefing_engine;
        let key = briefing_key(history, scenario, &input, engine);
        if let Some((_, result)) = self
            .desk
            .briefings
            .iter()
            .find(|(candidate, _)| *candidate == key)
        {
            export.add_briefing(engine.label(), result, &input);
        }
        self.save_report(ctx, export, format);
    }

    fn outlook_header(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        symbol: &Symbol,
        quote: Option<&Quote>,
        history: Option<&History>,
        source: Option<Source>,
    ) {
        let wide = ui.available_width() >= 825.;
        let heading = |ui: &mut egui::Ui| {
            ui.add(
                egui::Label::new(display_text(
                    &symbol.name,
                    if wide { 43. } else { 34. },
                    p.text,
                ))
                .truncate(),
            );
            ui.horizontal_wrapped(|ui| {
                let latest =
                    history.and_then(|h| h.closes.last().map(|bar| (bar, h.currency.as_str())));
                let quote = quote
                    .filter(|q| latest.is_none_or(|(bar, _)| q.demo || q.market_time >= bar.time));
                let last = quote
                    .map(|q| (q.price, q.currency.as_str()))
                    .or(latest.map(|(bar, currency)| (bar.price, currency)));
                if let Some((value, currency)) = last {
                    ui.label(display_text(quote_price(value, currency), 29., p.text));
                }
                if let Some(quote) = quote {
                    let delta = quote.change();
                    let change = delta
                        .zip(quote.percent())
                        .map(|(v, percent)| format!("{v:+.2} ({percent:+.2}%)"))
                        .or_else(|| delta.map(|v| format!("{v:+.2}")))
                        .or_else(|| quote.percent().map(|v| format!("{v:+.2}%")));
                    if let Some(change) = change {
                        text(
                            ui,
                            change,
                            16.,
                            if delta.or_else(|| quote.percent()).is_some_and(|v| v < 0.) {
                                p.red
                            } else {
                                p.green
                            },
                        );
                    }
                }
                let detail = if quote.is_some_and(|q| q.demo) || history.is_some_and(|h| h.sample) {
                    format!("{} · Sample data", symbol.ticker)
                } else if let Some(q) = quote {
                    format!("{} · {}", symbol.ticker, q.observation_time())
                } else if let Some(bar) = history.and_then(|h| h.closes.last()) {
                    format!("{} · Close {}", symbol.ticker, date(bar.time))
                } else {
                    symbol.ticker.clone()
                };
                text(ui, detail, 11.5, p.muted);
                if let Some(source) = source {
                    text(
                        ui,
                        format!("{} · {}", source.label(), source.price_basis()),
                        11.5,
                        p.muted,
                    );
                }
                if let Some(q) = quote.filter(|q| !q.demo) {
                    text(ui, format!("Retrieved {}", stamp(q.fetched)), 11.5, p.muted);
                }
            });
        };
        if wide {
            let controls_width = 232.;
            let heading_width = ui.available_width() - controls_width - ui.spacing().item_spacing.x;
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    vec2(heading_width, 78.),
                    Layout::top_down(Align::Min),
                    |ui| {
                        ui.set_width(heading_width);
                        heading(ui);
                    },
                );
                ui.allocate_ui_with_layout(
                    vec2(controls_width, 54.),
                    Layout::left_to_right(Align::Center),
                    |ui| self.depth_control(ui, p),
                );
            });
        } else {
            heading(ui);
            self.depth_control(ui, p);
        }
    }

    fn depth_control(&mut self, ui: &mut egui::Ui, p: Palette) {
        let active = self.prefs.outlook_depth;
        let mut depth = active;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.;
            for option in Depth::ALL {
                if segment(ui, option.label(), option == active, p).clicked() {
                    depth = option;
                }
            }
        });
        if active != depth {
            self.prefs.outlook_depth = depth;
            self.desk.clear_replay();
            self.save();
        }
    }

    fn outlook_panel(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        history: Option<&History>,
        report: Option<&Result<Arc<Report>, &'static str>>,
    ) {
        let width = ui.available_width();
        Frame::new().fill(p.panel).stroke(Stroke::new(1., p.line)).corner_radius(10)
            .inner_margin(18.)
            .show(ui, |ui| {
                ui.set_width((width - 38.).max(0.));
                ui.horizontal_wrapped(|ui| {
                    let title = ui.label(display_text("What could happen next?", 23., p.text));
                    let gap = width - 38. - title.rect.width() - 248.;
                    if gap > 10. {
                        ui.add_space(gap);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.;
                        for horizon in HORIZONS {
                            if segment(ui, &format!("{horizon} sessions"), horizon == self.desk.horizon, p).clicked()
                                && horizon != self.desk.horizon
                            {
                                self.desk.horizon = horizon;
                                self.desk.clear_replay();
                            }
                        }
                    });
                });
                ui.add_space(2.);
                let Some((history, report)) = history.zip(report) else {
                    self.outlook_empty(ui, p);
                    return;
                };
                if let Some(error) = self.desk.errors.get(&history.ticker)
                    .filter(|_| !history.sample && self.desk.imports.get(&history.ticker).is_none()).cloned() {
                    let latest = history.closes.last().map(|c| date(c.time));
                    let refreshing = self.desk.inflight.contains(&history.ticker);
                    let caption = latest.map_or_else(
                        || "Daily history refresh failed".into(),
                        |day| format!("Refresh failed · using closes through {day}"),
                    );
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(caption).color(p.red).size(11.5)).on_hover_text(error);
                        if ui.add_enabled(!refreshing, egui::Button::new(if refreshing { "Retrying…" } else { "Retry history" })).clicked() {
                            self.load_outlook_history(ui.ctx(), true);
                        }
                    });
                }
                let report = match report {
                    Ok(report) => report,
                    Err(reason) => {
                        text(ui, *reason, 14., p.muted);
                        text(ui, format!("{} daily closes available", history.closes.len()), 12., p.muted);
                        ui.add_space(100.);
                        return;
                    }
                };
                let horizon = report.current.fan.len() - 1;
                let range = report.current.end();
                if self.prefs.outlook_depth == Depth::MarketGod {
                    self.replay_controls(ui, p, history, report, horizon);
                }
                let playback = self.desk.playback.as_ref();
                let scenario = playback.map_or(&report.current, |r| r.scenario.as_ref());
                let range = if playback.is_some() { scenario.end() } else { range };
                let label = if playback.is_some() { "After that date".into() } else { format!("From the {} close", date(scenario.anchor.time)) };
                text(ui, format!("{label}, {} of {} similar periods ended higher. The middle range was {} to {} after {horizon} sessions.",
                    scenario.up_count(), scenario.matches.len(), pct(range.low), pct(range.high)), 13.5, p.muted);
                ui.add_space(6.);
                let realized = playback.filter(|r| r.revealed).map(|r| r.realized.as_slice());
                let stress = (self.prefs.outlook_depth == Depth::MarketGod).then_some(self.desk.stress / 100.);
                let height_fraction = if self.prefs.outlook_depth == Depth::Beginner { 0.38 } else { 0.30 };
                let desired = (ui.ctx().content_rect().height() * height_fraction).clamp(209., 366.);
                let max_fit = (ui.clip_rect().bottom() - ui.cursor().top() - 4.).max(185.);
                let height = desired.min(max_fit);
                draw_fan(ui, p, scenario, &history.currency, height, playback.is_some(), realized, stress);
                ui.add_space(9.);
                ui.horizontal_wrapped(|ui| {
                    legend(ui, p.text, "Historical closes", p);
                    legend(ui, alpha(p.green, 72), "Middle 80% of matches", p);
                    if realized.is_some() {
                        legend(ui, replay_color(p), "Realized path", p);
                    }
                });
                ui.add_space(7.);
                ui.separator();
                outcome_columns(ui, p, scenario, &history.currency, self.prefs.outlook_depth);
                if self.prefs.outlook_depth != Depth::Beginner {
                    show_evaluation(ui, p, playback.is_none().then_some(report.as_ref()), scenario);
                }
                if self.prefs.outlook_depth == Depth::MarketGod {
                    Self::stress_controls(ui, p, &mut self.desk.stress, scenario);
                }
            });
    }

    fn outlook_empty(&mut self, ui: &mut egui::Ui, p: Palette) {
        let ticker = self.prefs.selected.clone();
        ui.add_space(10.);
        if self.desk.inflight.contains(&ticker) {
            ui.horizontal(|ui| {
                ui.spinner();
                text(ui, "Loading completed daily closes…", 14., p.muted);
            });
        } else if let Some(error) = self.desk.errors.get(&ticker) {
            text(ui, "Daily history could not be loaded.", 16., p.text);
            text(ui, error, 12., p.muted);
            if button(ui, "Retry history", p).clicked() {
                self.load_outlook_history(ui.ctx(), true);
            }
        } else {
            text(
                ui,
                if self.prefs.offline {
                    "No sample history is available for this ticker."
                } else {
                    "Waiting for daily history."
                },
                14.,
                p.muted,
            );
        }
        text(
            ui,
            "The range appears once enough comparable trading periods are available.",
            12.,
            p.muted,
        );
        if link(ui, "Import daily prices…", p).clicked() {
            self.open_market_import();
        }
        ui.add_space(136.);
    }

    fn replay_controls(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        history: &History,
        report: &Report,
        horizon: usize,
    ) {
        let before = self.desk.playback.as_ref().map(|r| r.point.at);
        let mut selected = before;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("outlook_replay")
                .width(154.)
                .selected_text(before.map_or_else(|| "Historical replay".into(), date))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, None, "Latest close");
                    for row in &report.trials {
                        ui.selectable_value(&mut selected, Some(row.at), date(row.at));
                    }
                });
            if let Some(replay) = &mut self.desk.playback {
                if !replay.revealed {
                    if button(ui, "Reveal result", p).clicked() {
                        replay.revealed = true;
                    }
                } else {
                    text(
                        ui,
                        format!(
                            "{} on {} · {} the range",
                            pct(replay.point.result),
                            date(replay.point.through),
                            if replay.point.inside {
                                "inside"
                            } else {
                                "outside"
                            }
                        ),
                        12.5,
                        replay_color(p),
                    );
                }
            }
        });
        if before != selected {
            self.desk.playback = selected.and_then(|at| {
                let point = report.trials.iter().find(|r| r.at == at)?.clone();
                let (scenario, realized) = scenarios::replay(history, at, horizon)?;
                Some(Playback {
                    point,
                    scenario: Arc::new(scenario),
                    realized,
                    revealed: false,
                })
            });
        }
        ui.add_space(3.);
    }

    fn stress_controls(ui: &mut egui::Ui, p: Palette, stress: &mut f64, scenario: &Scenario) {
        ui.add_space(7.);
        ui.separator();
        ui.label(
            RichText::new("Stress a closing return")
                .size(16.)
                .strong()
                .color(p.text),
        );
        ui.add(
            egui::Slider::new(stress, -50.0..=50.0)
                .step_by(0.5)
                .suffix("%"),
        );
        let target = *stress / 100.;
        let count = scenario
            .matches
            .iter()
            .filter(|r| r.realized <= target)
            .count();
        let unconditional = scenario
            .baseline_returns
            .iter()
            .filter(|&&r| r <= target)
            .count();
        text(
            ui,
            format!(
                "{} or worse in {count}/{} similar periods and {unconditional}/{} unfiltered periods.",
                pct(target),
                scenario.matches.len(),
                scenario.baseline_returns.len()
            ),
            12.5,
            p.muted,
        );
    }

    fn outlook_news_signal(&self, ticker: &str, horizon: usize) -> crate::news_signal::NewsSignal {
        let articles = self.news.get(ticker).map(Vec::as_slice).unwrap_or(&[]);
        let earnings = self
            .quote(ticker)
            .filter(|q| !q.demo)
            .and_then(|q| q.details.earnings_start);
        crate::news_signal::analyze_headlines(articles, earnings, now(), horizon)
    }

    fn outlook_rail(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        history: Option<&History>,
        report: Option<&Result<Arc<Report>, &'static str>>,
    ) {
        let report = report.and_then(|r| r.as_ref().ok());
        let playback = self.desk.playback.as_ref().map(|r| r.scenario.clone());
        let scenario = playback.as_deref().or_else(|| report.map(|r| &r.current));
        let news_sig = history
            .zip(scenario)
            .filter(|_| playback.is_none())
            .map(|(h, s)| self.outlook_news_signal(&h.ticker, s.fan.len().saturating_sub(1)));
        let evidence = history.zip(report.zip(scenario)).map(|(h, (r, s))| {
            let trials = playback.is_none().then_some(r.trials.as_slice());
            let input = Input::from_scenario(&h.ticker, &h.currency, h.sample, s, trials);
            match &news_sig {
                Some(sig) => input.with_news_signal(sig),
                None => input,
            }
        });
        let active = self.prefs.briefing_engine;
        let mut engine = active;
        let key = history
            .zip(scenario)
            .zip(evidence.as_ref())
            .map(|((history, scenario), input)| briefing_key(history, scenario, input, engine));
        ui.label(display_text("The briefing", 23., p.text));
        let busy = self.desk.running.is_some();
        let installed = self.desk.engines.contains(&engine);
        text(
            ui,
            if busy {
                "Preparing briefing…"
            } else if installed {
                "●  Installed locally"
            } else {
                "○  CLI not found"
            },
            12.,
            if installed { p.green } else { p.muted },
        );
        egui::ComboBox::from_id_salt("briefing_engine")
            .width((ui.available_width() - 20.).clamp(110., 270.))
            .selected_text(engine.label())
            .show_ui(ui, |ui| {
                for option in Engine::ALL {
                    ui.selectable_value(&mut engine, option, option.label());
                }
            });
        if engine != active {
            self.prefs.briefing_engine = engine;
            self.save();
        }
        ui.add_space(3.);
        let label = if busy {
            "Cancel briefing"
        } else if history.is_some_and(|h| h.sample) {
            "Run sample briefing"
        } else {
            "Run briefing"
        };
        let enabled = !self.transient && (busy || installed && key.is_some());
        let action = ui.scope(|ui| {
            if !enabled {
                ui.disable();
            }
            ui.add_sized(
                [ui.available_width(), 35.],
                egui::Button::new(display_text(label, 13., action_ink(p)))
                    .fill(action_fill(p))
                    .corner_radius(6),
            )
        });
        if action.inner.clicked() {
            if let Some((_, cancel)) = &self.desk.running {
                cancel.store(true, Ordering::Relaxed);
            } else if let Some((key, evidence)) = key.clone().zip(evidence.clone()) {
                if key.engine == self.prefs.briefing_engine {
                    self.start_briefing(ui.ctx(), key, evidence);
                }
            }
        }
        if !installed && link(ui, "Check for installation", p).clicked() {
            self.desk.engines = Engine::ALL
                .into_iter()
                .filter(|e| e.executable().is_some())
                .collect();
        }
        text(
            ui,
            if installed {
                "Uses your local CLI sign-in and configuration."
            } else {
                "Install the selected assistant’s command-line app."
            },
            11.5,
            p.muted,
        );
        if let Some(key) = &key {
            if let Some((_, error)) = self.desk.briefing_errors.as_ref().filter(|(k, _)| k == key) {
                text(ui, error, 12., p.red);
            }
            if let Some((_, result)) = self.desk.briefings.iter().find(|(k, _)| k == key) {
                ui.add_space(7.);
                if let Some(input) = &evidence {
                    for (label, finding) in [
                        ("The read", &result.read),
                        ("Countercase", &result.countercase),
                        ("What would change it", &result.watch),
                    ] {
                        show_finding(ui, p, label, finding, input);
                    }
                    egui::CollapsingHeader::new("Briefing evidence").show(ui, |ui| {
                        for item in &input.evidence {
                            text(ui, format!("{} · {}", item.id, item.label), 12., p.text);
                            text(ui, &item.fact, 11.5, p.muted);
                        }
                    });
                }
            }
        }
        if let Some((history, scenario)) = history.zip(scenario) {
            ui.add_space(12.);
            ui.separator();
            ui.add_space(8.);
            ui.label(display_text("What to watch", 18., p.text));
            let end = scenario.end();
            for (label, ret, color) in [("Below", end.low, p.red), ("Above", end.high, p.green)] {
                ui.label(
                    RichText::new(format!(
                        "{label} {}",
                        quote_price(scenario.anchor.price * (1. + ret), &history.currency)
                    ))
                    .size(15.)
                    .color(color)
                    .strong(),
                );
            }
            text(
                ui,
                format!(
                    "A close beyond either boundary after {} sessions would fall outside the calibrated range.",
                    scenario.fan.len() - 1
                ),
                11.5,
                p.muted,
            );
            if let Some(sig) = news_sig.as_ref().filter(|s| s.has_signal()) {
                ui.add_space(10.);
                ui.separator();
                ui.add_space(6.);
                ui.label(display_text("News & events", 17., p.text));
                let tone_color = if sig.sentiment_score >= 0.18 {
                    p.green
                } else if sig.sentiment_score <= -0.18 {
                    p.red
                } else {
                    p.blue
                };
                if sig.article_count > 0 {
                    ui.horizontal_wrapped(|ui| {
                        pill(
                            ui,
                            &format!("{} ({:+.2})", sig.sentiment_label, sig.sentiment_score),
                            tone_color,
                        );
                        text(
                            ui,
                            format!(
                                "{} headlines (+{}/−{})",
                                sig.article_count, sig.positive_count, sig.negative_count
                            ),
                            11.,
                            p.muted,
                        );
                    });
                }
                if !sig.event_tags.is_empty() {
                    text(
                        ui,
                        format!("Topics · {}", sig.event_tags.join(" · ")),
                        11.,
                        p.muted,
                    );
                }
                if sig.earnings_in_horizon {
                    let msg = sig.days_to_earnings.map_or_else(
                        || "Expected earnings inside forecast window · elevated gap risk.".into(),
                        |d| {
                            format!(
                                "Expected earnings in ~{d}d (inside window) · elevated gap risk."
                            )
                        },
                    );
                    text(ui, msg, 11.5, p.red);
                }
            }
            ui.add_space(12.);
            ui.separator();
            ui.add_space(7.);
            ui.label(display_text("Similar periods", 18., p.text));
            let visible_rows = if self.prefs.outlook_depth == Depth::Beginner {
                2
            } else {
                4
            };
            for analog in scenario.matches.iter().take(visible_rows) {
                let (row, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 22.), Sense::hover());
                let painter = ui.painter_at(row);
                painter.text(
                    row.left_center(),
                    Align2::LEFT_CENTER,
                    format!("{} to {}", date(analog.start), date(analog.end)),
                    FontId::proportional(11.5),
                    p.muted,
                );
                painter.text(
                    row.right_center(),
                    Align2::RIGHT_CENTER,
                    pct(analog.realized),
                    FontId::proportional(12.),
                    if analog.realized < 0. { p.red } else { p.green },
                );
                ui.separator();
            }
            text(
                ui,
                if history.sample {
                    "Simulated history · sample data".into()
                } else {
                    format!(
                        "Yahoo Finance\nCloses through {} · retrieved {}",
                        date(scenario.anchor.time),
                        stamp(history.fetched)
                    )
                },
                10.5,
                p.muted,
            );
        }
    }

    fn start_briefing(&mut self, ctx: &egui::Context, key: BriefKey, evidence: Input) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.desk.running = Some((key.clone(), cancel.clone()));
        self.desk.briefing_errors = None;
        let sender = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = briefing::run(key.engine, &evidence, &cancel);
            let _ = sender.send(Event::Briefing(key, result));
            ctx.request_repaint();
        });
    }
}

fn segment(ui: &mut egui::Ui, label: &str, selected: bool, p: Palette) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(11.5).color(if selected {
            action_ink(p)
        } else {
            p.muted
        }))
        .fill(if selected { action_fill(p) } else { p.panel })
        .stroke(Stroke::new(
            1.,
            if selected { action_fill(p) } else { p.line },
        ))
        .corner_radius(5)
        .min_size(vec2(0., 28.)),
    )
}

fn action_fill(p: Palette) -> Color32 {
    if p.bg.r() > 128 {
        hex(0x194b39)
    } else {
        p.green
    }
}
fn action_ink(p: Palette) -> Color32 {
    if p.bg.r() > 128 { Color32::WHITE } else { p.bg }
}
fn replay_color(p: Palette) -> Color32 {
    if p.bg.r() > 128 {
        hex(0x6a4d86)
    } else {
        hex(0xc6a6e5)
    }
}

fn date(t: i64) -> String {
    stamp(t)
        .split_ascii_whitespace()
        .next()
        .unwrap_or("—")
        .into()
}

fn pct(v: f64) -> String {
    format!("{:+.1}%", v * 100.)
}

fn quote_price(value: f64, currency: &str) -> String {
    let amount = if value > 0. && value < 0.0001 {
        format!("{value:.3e}")
    } else {
        price(value)
    };
    match currency {
        "USD" => format!("${amount}"),
        "EUR" => format!("€{amount}"),
        "GBP" => format!("£{amount}"),
        "" | "—" => amount,
        _ => format!("{amount} {currency}"),
    }
}

fn legend(ui: &mut egui::Ui, c: Color32, label: &str, p: Palette) {
    let (rect, _) = ui.allocate_exact_size(vec2(20., 8.), Sense::hover());
    ui.painter().rect_filled(rect.shrink2(vec2(0., 2.)), 2., c);
    text(ui, label, 11.5, p.muted);
    ui.add_space(7.);
}

fn outcome_columns(
    ui: &mut egui::Ui,
    p: Palette,
    scenario: &Scenario,
    currency: &str,
    depth: Depth,
) {
    let band = scenario.end();
    ui.columns(3, |cols| {
        for (col, (label, ret, detail, color)) in cols.iter_mut().zip([
            ("Downside", band.low, "10th percentile", p.red),
            ("Typical", band.mid, "Median", p.text),
            ("Upside", band.high, "90th percentile", p.green),
        ]) {
            col.add_space(3.);
            col.label(RichText::new(label).size(14.).color(p.muted).strong());
            col.add(
                egui::Label::new(
                    RichText::new(quote_price(scenario.anchor.price * (1. + ret), currency))
                        .size(22.)
                        .color(p.text)
                        .strong(),
                )
                .truncate(),
            );
            col.label(RichText::new(pct(ret)).size(13.).color(color).strong());
            if depth != Depth::Beginner {
                text(col, detail, 11., p.muted);
            }
        }
    });
}

fn show_evaluation(ui: &mut egui::Ui, p: Palette, report: Option<&Report>, scenario: &Scenario) {
    ui.add_space(8.);
    ui.separator();
    let cal = scenario.calibration;
    if let Some(report) = report {
        ui.label(
            RichText::new("Historical track record")
                .strong()
                .size(17.)
                .color(p.text),
        );
        if report.trials.is_empty() {
            text(
                ui,
                "Too few earlier periods for a forward test.",
                12.,
                p.muted,
            );
        } else {
            let n = report.trials.len();
            let covered = report.trials.iter().filter(|r| r.inside).count();
            let baseline = report.trials.iter().filter(|r| r.baseline_inside).count();
            let ens_pb =
                report.trials.iter().map(|r| r.pinball_loss).sum::<f64>() * 100. / n as f64;
            let base_pb = report
                .trials
                .iter()
                .map(|r| r.baseline_pinball_loss)
                .sum::<f64>()
                * 100.
                / n as f64;
            let error =
                report.trials.iter().map(|r| r.absolute_error).sum::<f64>() * 100. / n as f64;
            let baseline_error =
                report.trials.iter().map(|r| r.baseline_error).sum::<f64>() * 100. / n as f64;
            ui.label(
                RichText::new(format!(
                    "Range coverage  {covered}/{n} calibrated ensemble  ·  {baseline}/{n} unfiltered"
                ))
                .color(p.text)
                .size(13.),
            )
            .on_hover_text(
                "How often the realized close landed inside each 80% interval on strictly causal walk-forward tests.",
            );
            text(
                ui,
                format!(
                    "Pinball loss  {ens_pb:.2} vs {base_pb:.2} pp ({:+.1}% skill)  ·  Median MAE {error:.1} vs {baseline_error:.1} pp",
                    cal.pinball_skill() * 100.
                ),
                12.,
                p.muted,
            );
            text(
                ui,
                format!(
                    "Self-calibrating weights · Analogs {:.0}% ({:.1} eff.) · Online QR {:.0}% · EWMA {:.0}% · Baseline {:.0}% · Conformal {:+.1} pp",
                    cal.weights.analogs * 100.,
                    cal.effective_matches,
                    cal.weights.online_qr * 100.,
                    cal.weights.ewma * 100.,
                    cal.weights.baseline * 100.,
                    cal.conformal_delta * 100.,
                ),
                11.5,
                p.muted,
            );
            text(
                ui,
                "Walk-forward self-calibration updates Hedge model weights and split-conformal intervals strictly from prior completed outcomes.",
                11.,
                p.muted,
            );
        }
    }
    let setup = scenario.setup;
    let vol_text = setup
        .volume_ratio
        .map(|v| format!(" · vol 5d/20d {v:.2}x"))
        .unwrap_or_default();
    text(
        ui,
        format!(
            "Setup · 5d/20d/60d {}, {}, {} · daily vol {:.2}% ({:.2}x) · 60d drawdown {}{vol_text}",
            pct(setup.momentum_5),
            pct(setup.momentum),
            pct(setup.momentum_60),
            setup.daily_volatility * 100.,
            setup.vol_ratio,
            pct(setup.drawdown)
        ),
        11.5,
        p.muted,
    );
}

fn show_finding(ui: &mut egui::Ui, p: Palette, label: &str, finding: &Finding, input: &Input) {
    ui.label(RichText::new(label).size(13.5).strong().color(p.text));
    text(ui, &finding.text, 12.5, p.text);
    ui.horizontal_wrapped(|ui| {
        for id in &finding.evidence {
            if let Some(item) = input.evidence.iter().find(|item| item.id == id) {
                ui.label(RichText::new(id).size(10.5).color(p.blue))
                    .on_hover_text(format!("{}\n{}", item.label, item.fact));
            }
        }
    });
}

fn draw_fan(
    ui: &mut egui::Ui,
    p: Palette,
    s: &Scenario,
    currency: &str,
    height: f32,
    replay: bool,
    revealed: Option<&[f64]>,
    stress: Option<f64>,
) {
    let (area, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(area);
    let compact = area.width() < 405.;
    let plot = Rect::from_min_max(
        area.left_top() + vec2(if compact { 46. } else { 60. }, 14.),
        area.right_bottom() - vec2(if compact { 43. } else { 72. }, 30.),
    );
    if plot.width() < 60. {
        return;
    }
    let anchor = s.anchor.price;
    let project = |ret| anchor * (1. + ret);
    let mut extrema = s
        .past
        .iter()
        .map(|c| c.price)
        .chain(s.fan.iter().flat_map(|b| [project(b.low), project(b.high)]))
        .chain(revealed.into_iter().flatten().map(|r| project(*r)));
    let first = extrema.next().unwrap_or(anchor);
    let (min, max) = extrema.fold((first, first), |(lo, hi), x| (lo.min(x), hi.max(x)));
    let padding = ((max - min) * 0.08).max(anchor * 0.005);
    let min = (min - padding).max(0.);
    let max = max + padding;
    let y = |value: f64| plot.bottom() - (((value - min) / (max - min)) as f32 * plot.height());
    let pivot = plot.left() + plot.width() * 0.56;
    let w = s.fan.len().saturating_sub(1) as f32;
    let forward = |i: usize| pivot + i as f32 / w * (plot.right() - pivot);
    let past =
        |i: usize| plot.left() + i as f32 / (s.past.len() - 1) as f32 * (pivot - plot.left());
    let step = nice_interval((max - min) / 5.);
    let start = (min / step).ceil() * step;

    for value in (0..12)
        .map(|i| start + f64::from(i) * step)
        .take_while(|v| *v <= max)
    {
        let at = y(value);
        painter.hline(plot.x_range(), at, Stroke::new(1., p.line));
        painter.text(
            pos2(plot.left() - 7., at),
            Align2::RIGHT_CENTER,
            axis_price(value, step),
            FontId::monospace(10.5),
            p.muted,
        );
    }
    let guide = Stroke::new(1., alpha(p.muted, 125));
    for i in (0..plot.height() as usize).step_by(6) {
        painter.line_segment(
            [
                pos2(pivot, plot.top() + i as f32),
                pos2(pivot, (plot.top() + i as f32 + 3.).min(plot.bottom())),
            ],
            guide,
        );
    }
    if let Some(stress) = stress.map(project).filter(|v| *v > min && *v < max) {
        painter.hline(
            pivot..=plot.right(),
            y(stress),
            Stroke::new(1., alpha(replay_color(p), 150)),
        );
    }

    let mut mesh = egui::Mesh::default();
    let tint = alpha(p.green, if p.bg.r() > 128 { 43 } else { 57 });
    for (i, band) in s.fan.iter().enumerate() {
        let x = forward(i);
        mesh.colored_vertex(pos2(x, y(project(band.high))), tint);
        mesh.colored_vertex(pos2(x, y(project(band.low))), tint);
        if i > 0 {
            let j = 2 * i as u32;
            mesh.add_triangle(j - 2, j - 1, j);
            mesh.add_triangle(j - 1, j + 1, j);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
    let points = s
        .past
        .iter()
        .enumerate()
        .map(|(i, bar)| pos2(past(i), y(bar.price)))
        .collect::<Vec<_>>();
    painter.add(egui::Shape::line(points, Stroke::new(2.3, p.text)));
    painter.circle_filled(pos2(pivot, y(anchor)), 4.5, p.text);
    let band = s.end();
    let mut labels = [(band.high, p.green), (band.mid, p.text), (band.low, p.red)]
        .map(|(ret, color)| (y(project(ret)), color, ret));
    let y_gap = 16.;
    labels[0].0 = labels[0].0.min(plot.bottom() - 2. * y_gap);
    labels[1].0 = labels[1]
        .0
        .max(labels[0].0 + y_gap)
        .min(plot.bottom() - y_gap);
    labels[2].0 = labels[2].0.max(labels[1].0 + y_gap);
    for (index, (label_y, color, ret)) in labels.into_iter().enumerate() {
        let values = s
            .fan
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let change = match index {
                    0 => b.high,
                    1 => b.mid,
                    _ => b.low,
                };
                pos2(forward(i), y(project(change)))
            })
            .collect::<Vec<_>>();
        if let Some(end) = values.last().copied() {
            painter.circle_filled(end, 3.7, color);
        }
        painter.add(egui::Shape::line(
            values,
            Stroke::new(if index == 1 { 2.3 } else { 1.35 }, color),
        ));
        let label = if compact {
            pct(ret)
        } else {
            quote_price(project(ret), currency)
        };
        let at = pos2(plot.right() + 5., label_y);
        let galley = painter.layout_no_wrap(label, FontId::proportional(11.), color);
        let label_width = galley.size().x.min(area.right() - at.x);
        let box_rect =
            Rect::from_min_size(at - vec2(2., 8.), vec2(label_width + 3., 16.)).intersect(area);
        painter.rect_filled(box_rect, 3., p.panel);
        painter
            .with_clip_rect(box_rect)
            .galley(at - vec2(0., galley.size().y / 2.), galley, color);
    }
    if let Some(values) = revealed {
        painter.add(egui::Shape::line(
            values
                .iter()
                .enumerate()
                .map(|(i, ret)| pos2(forward(i), y(project(*ret))))
                .collect(),
            Stroke::new(2.5, replay_color(p)),
        ));
    }

    let bottom = plot.bottom() + 11.;
    for (x, label, align) in [
        (plot.left(), date(s.past[0].time), Align2::LEFT_TOP),
        (
            pivot,
            if replay {
                "Replay date"
            } else {
                "Latest close"
            }
            .into(),
            Align2::CENTER_TOP,
        ),
        (
            plot.right(),
            format!("+{} sessions", w as usize),
            Align2::RIGHT_TOP,
        ),
    ] {
        painter.text(
            pos2(x, bottom),
            align,
            label,
            FontId::proportional(10.5),
            p.muted,
        );
    }
    if let Some(hover) = response.hover_pos().filter(|pos| plot.contains(*pos)) {
        let (x, detail) = if hover.x >= pivot {
            let index = ((hover.x - pivot) / (plot.right() - pivot) * w).round() as usize;
            let b = s.fan[index];
            (
                forward(index),
                format!(
                    "+{index} sessions\n{} to {}\nMedian {}",
                    quote_price(project(b.low), currency),
                    quote_price(project(b.high), currency),
                    quote_price(project(b.mid), currency)
                ),
            )
        } else {
            let index = (((hover.x - plot.left()) / (pivot - plot.left()))
                * (s.past.len() - 1) as f32)
                .round() as usize;
            let bar = s.past[index];
            (
                past(index),
                format!(
                    "{}\nClose {}",
                    date(bar.time),
                    quote_price(bar.price, currency)
                ),
            )
        };
        painter.vline(x, plot.y_range(), Stroke::new(1., alpha(p.muted, 100)));
        response.on_hover_text_at_pointer(detail);
    }
}

fn nice_interval(raw: f64) -> f64 {
    let base = 10_f64.powf(raw.log10().floor());
    let factor = raw / base;
    base * if factor <= 1. {
        1.
    } else if factor <= 2. {
        2.
    } else if factor <= 5. {
        5.
    } else {
        10.
    }
}

fn axis_price(price: f64, step: f64) -> String {
    if step < 0.001 {
        format!("{price:.2e}")
    } else if step < 0.01 {
        format!("{price:.3}")
    } else if step < 0.1 {
        format!("{price:.2}")
    } else if step < 1. {
        format!("{price:.1}")
    } else if price >= 10_000. {
        format!("{:.1}k", price / 1_000.)
    } else {
        format!("{price:.0}")
    }
}
