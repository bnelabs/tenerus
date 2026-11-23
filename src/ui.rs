use crate::journal::{Journal, REVIEW_LABELS, ReviewData};
use crate::mechanics::{self, ChartRange, NavigationIntent, PriceChangeDisplay};
use crate::{Launch, disclaimer, market_data, model::*, network};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Id, Layout, Rect, RichText, Sense,
    Stroke, Vec2, pos2, vec2,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

mod outlook;
mod support;

#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    side: Color32,
    panel: Color32,
    raised: Color32,
    text: Color32,
    muted: Color32,
    line: Color32,
    blue: Color32,
    green: Color32,
    red: Color32,
}
impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                bg: hex(0x151e1b),
                side: hex(0x192420),
                panel: hex(0x202b26),
                raised: hex(0x293831),
                text: hex(0xeaf1ec),
                muted: hex(0xa5b8ac),
                line: hex(0x35453d),
                blue: hex(0x92cdb4),
                green: hex(0x85cba7),
                red: hex(0xf19589),
            }
        } else {
            Self {
                bg: hex(0xf6f8f3),
                side: hex(0xeef3ec),
                panel: Color32::WHITE,
                raised: hex(0xe7eee7),
                text: hex(0x182d26),
                muted: hex(0x54675e),
                line: hex(0xdce5dd),
                blue: hex(0x27674f),
                green: hex(0x267654),
                red: hex(0xab493b),
            }
        }
    }
}
fn hex(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}
fn alpha(c: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
}
fn text(ui: &mut egui::Ui, s: impl Into<String>, size: f32, c: Color32) {
    ui.label(RichText::new(s).size(size).color(c));
}
fn display_text(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s)
        .font(FontId::new(size, egui::FontFamily::Name("display".into())))
        .color(color)
}
fn pill(ui: &mut egui::Ui, s: &str, c: Color32) {
    Frame::new()
        .fill(alpha(c, 22))
        .corner_radius(5)
        .inner_margin(vec2(7., 3.))
        .show(ui, |ui| {
            text(ui, s, 11., c);
        });
}
fn button(ui: &mut egui::Ui, s: &str, p: Palette) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(s).size(12.).color(p.text))
            .fill(p.raised)
            .corner_radius(7)
            .min_size(vec2(0., 28.)),
    )
}
fn link(ui: &mut egui::Ui, s: &str, p: Palette) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(s).size(12.).color(p.blue)).frame(false))
}

fn remember_quote(quotes: &mut HashMap<String, Quote>, incoming: Quote) {
    let entry = quotes.entry(incoming.symbol.clone());
    match entry {
        std::collections::hash_map::Entry::Occupied(mut entry) => entry.get_mut().update(incoming),
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(incoming);
        }
    }
}

const MARKET_WORKERS: usize = 6;
const MARKET_INDEXES: [&str; 3] = ["^GSPC", "^DJI", "^IXIC"];

fn research_key(ticker: &str, sample: bool) -> String {
    if sample {
        format!("sample:{ticker}")
    } else {
        ticker.into()
    }
}
fn small_title(ui: &mut egui::Ui, s: &str, p: Palette) {
    text(ui, s, 10.5, p.muted);
}
fn dialog_window<'a>(ctx: &egui::Context, title: &'a str, size: Vec2) -> egui::Window<'a> {
    let bounds = ctx.content_rect().shrink(12.);
    egui::Window::new(title)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .constrain_to(bounds)
        .default_size(size.min(bounds.size()))
        .min_width(320.)
}
fn memory_label(days: u16) -> &'static str {
    match days {
        0 => "Off",
        90 => "3 months (90 days)",
        365 => "1 year (365 days)",
        _ => "6 months (180 days)",
    }
}

// An in-memory identity for async results; never persisted or used for authentication.
fn connection_token(p: &Provider, s: &Secret) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    p.fingerprint().hash(&mut hash);
    s.key.hash(&mut hash);
    s.headers.hash(&mut hash);
    hash.finish()
}

enum Event {
    Market(String, usize, Result<Market, String>),
    News(String, Result<Vec<Article>, String>),
    Search(u64, Result<Vec<Symbol>, String>),
    Models(u64, usize, u64, Result<Vec<String>, String>),
    Probe(u64, usize, u64, Result<InferenceUsage, String>),
    Analysis(String, bool, usize, u64, Result<Analysis, String>),
    Review(String, Vec<u64>, Result<ReviewData, String>),
    ReviewsDone,
    LocalModelCheck(crate::forecast::MacCheck),
    LocalForecast(Result<crate::forecast::LocalForecast, String>),
    ScenarioHistory(String, Result<crate::scenarios::History, String>),
    Briefing(outlook::BriefKey, Result<crate::briefing::Briefing, String>),
    HistoryImport(u64, Result<market_data::ImportedHistory, String>),
    ExportedReport(Result<PathBuf, String>),
}
struct Editor {
    session: u64,
    selected: usize,
    providers: Vec<Provider>,
    secrets: Vec<Secret>,
    headers: Vec<String>,
    revision: Vec<u64>,
    models: Vec<Vec<String>>,
    tested: Vec<bool>,
    usage: Vec<Option<InferenceUsage>>,
    message: Vec<Option<(bool, String)>>,
    busy: HashSet<(usize, u8)>,
    reveal: bool,
}
impl Editor {
    fn new(
        session: u64,
        p: &Preferences,
        secrets: &[Secret],
        verified: &[bool],
        usage: &[Option<InferenceUsage>],
    ) -> Self {
        Self {
            session,
            selected: p.active_provider,
            providers: p.providers.clone(),
            secrets: secrets.to_vec(),
            headers: secrets
                .iter()
                .map(|s| serde_json::to_string_pretty(&s.headers).unwrap_or("{}".into()))
                .collect(),
            revision: vec![0; p.providers.len()],
            models: vec![vec![]; p.providers.len()],
            tested: verified.to_vec(),
            usage: usage.to_vec(),
            message: vec![None; p.providers.len()],
            busy: HashSet::new(),
            reveal: false,
        }
    }

    fn endpoint_changed(&mut self, i: usize, previous: &str) {
        let origin = |endpoint: &str| url::Url::parse(endpoint.trim()).ok().map(|u| u.origin());
        if origin(previous) != origin(&self.providers[i].endpoint) {
            self.secrets[i] = Secret::default();
            self.headers[i] = "{}".into();
            self.reveal = false;
        }
        self.models[i].clear();
    }
}

pub struct MarketApp {
    prefs: Preferences,
    secrets: Vec<Secret>,
    verified: Vec<bool>,
    last_usage: Vec<Option<InferenceUsage>>,
    markets: Markets,
    quotes: HashMap<String, Quote>,
    news: HashMap<String, Vec<Article>>,
    news_fetched: HashMap<String, i64>,
    failures: HashMap<String, String>,
    inflight: HashSet<(String, usize)>,
    pending_markets: VecDeque<(String, usize)>,
    news_inflight: HashSet<String>,
    analyses: HashMap<String, Analysis>,
    analyses_load_error: Option<String>,
    analyzing: HashSet<String>,
    ai_errors: HashMap<String, String>,
    journal: Journal,
    journal_open: bool,
    model_guide_open: bool,
    local_model_check: Option<crate::forecast::MacCheck>,
    local_model_check_busy: bool,
    local_forecast_backend: crate::forecast::FoundationBackend,
    local_forecast: Option<Result<crate::forecast::LocalForecast, String>>,
    local_forecast_busy: bool,
    desk: outlook::Desk,
    journal_selected_only: bool,
    journal_limit: usize,
    journal_load_error: Option<String>,
    journal_error: Option<String>,
    review_errors: HashMap<String, String>,
    reviewing: bool,
    last_review: Option<Instant>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    search: String,
    search_results: Vec<Symbol>,
    search_generation: u64,
    search_at: Instant,
    search_sent: u64,
    search_error: Option<String>,
    focus_search: bool,
    editor: Option<Editor>,
    editor_sequence: u64,
    manage: bool,
    new_list: String,
    edit_rows: bool,
    dragging: Option<String>,
    range_drag: Option<i64>,
    hover_info: String,
    business_news: bool,
    saved_only: bool,
    manage_symbol: Option<String>,
    news_only: bool,
    allow_demo_ai: bool,
    analyze_all: bool,
    settings: bool,
    information: Option<support::InfoPage>,
    terms_path: PathBuf,
    terms_accepted: bool,
    terms_error: Option<String>,
    launch_intent: Option<NavigationIntent>,
    open_connections_at_start: bool,
    market_import: Option<support::ImportForm>,
    import_sequence: u64,
    import: bool,
    import_text: String,
    article: Option<Article>,
    raw_answer: Option<String>,
    toast: Option<(String, Instant)>,
    last_refresh: Instant,
    shot: Option<PathBuf>,
    frames: usize,
    shot_sent: bool,
    state_path: PathBuf,
    settings_load_error: Option<String>,
    transient: bool,
    deleted_list: Option<(usize, Watchlist)>,
    captured: std::sync::Arc<std::sync::atomic::AtomicBool>,
    initial_geometry: bool,
    geometry_screen: Option<[f32; 2]>,
}
impl MarketApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: Launch) -> Self {
        Self::from_context(&cc.egui_ctx, launch)
    }
    fn from_context(ctx: &egui::Context, launch: Launch) -> Self {
        Self::load_workspace(ctx, launch, data_dir(), legacy_data_dir())
    }
    fn load_workspace(ctx: &egui::Context, launch: Launch, data: PathBuf, legacy: PathBuf) -> Self {
        let transient = launch.screenshot.is_some();
        let state_path = data.join("settings.json");
        let terms_path = data.join("educational-use.json");
        let terms_accepted = if transient {
            launch.view != "welcome"
        } else {
            disclaimer::accepted(&terms_path)
        };
        let legacy_state_path = legacy.join("settings.json");
        let (mut prefs, settings_load_error) = if transient {
            (Preferences::default(), None)
        } else {
            match Preferences::load(&state_path, &legacy_state_path) {
                Ok(prefs) => (prefs, None),
                Err(error) => (Preferences::default(), Some(error)),
            }
        };
        if launch.demo || transient {
            prefs.offline = true;
        }
        let mut fonts = egui::FontDefinitions::default();
        let body_candidates: &[&str] = if cfg!(target_os = "macos") {
            &["/System/Library/Fonts/SFNS.ttf"]
        } else if cfg!(target_os = "windows") {
            &[
                "C:\\Windows\\Fonts\\arial.ttf",
                "C:\\Windows\\Fonts\\segoeui.ttf",
            ]
        } else {
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/local/share/fonts/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/liberation-sans/LiberationSans-Regular.ttf",
            ]
        };
        if let Some(data) = body_candidates.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts
                .font_data
                .insert("system".into(), egui::FontData::from_owned(data).into());
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "system".into());
        }
        let mut display_fonts = fonts.families[&egui::FontFamily::Proportional].clone();
        let display_candidates: &[&str] = if cfg!(target_os = "macos") {
            &["/System/Library/Fonts/Supplemental/Arial Bold.ttf"]
        } else if cfg!(target_os = "windows") {
            &[
                "C:\\Windows\\Fonts\\arialbd.ttf",
                "C:\\Windows\\Fonts\\segoeuib.ttf",
            ]
        } else {
            &[
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf",
                "/usr/local/share/fonts/dejavu/DejaVuSans-Bold.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                "/usr/share/fonts/liberation-sans/LiberationSans-Bold.ttf",
            ]
        };
        if let Some(data) = display_candidates
            .iter()
            .find_map(|p| std::fs::read(p).ok())
        {
            fonts
                .font_data
                .insert("display".into(), egui::FontData::from_owned(data).into());
            display_fonts.insert(0, "display".into());
        }
        fonts
            .families
            .insert(egui::FontFamily::Name("display".into()), display_fonts);
        ctx.set_fonts(fonts);
        let (tx, rx) = mpsc::channel();
        let (analyses, analyses_load_error) = if transient {
            (HashMap::new(), None)
        } else {
            match read_with_legacy::<HashMap<String, Analysis>>(
                &state_path.with_file_name("analyses.json"),
                &legacy_state_path.with_file_name("analyses.json"),
            ) {
                Ok(saved) => {
                    let mut results = HashMap::<String, Analysis>::new();
                    for analysis in saved.unwrap_or_default().into_values() {
                        let key = research_key(&analysis.symbol, analysis.demo);
                        if results
                            .get(&key)
                            .is_none_or(|a| a.generated < analysis.generated)
                        {
                            results.insert(key, analysis);
                        }
                    }
                    (results, None)
                }
                Err(error) => (HashMap::new(), Some(error)),
            }
        };
        let cache = if transient {
            MarketCache::default()
        } else {
            MarketCache::load(&state_path.with_file_name("market-cache.json"))
        };
        let mut markets = HashMap::new();
        let mut quotes = HashMap::new();
        for quote in cache.quotes {
            remember_quote(&mut quotes, quote);
        }
        for market in cache.markets {
            remember_quote(&mut quotes, market.quote.clone());
            markets.insert((market.quote.symbol.clone(), market.range), market);
        }
        let provider_count = prefs.providers.len();
        let (journal, journal_load_error) = if transient {
            (Journal::default(), None)
        } else {
            match Journal::load(&data.join("predictions.json")) {
                Ok(journal) => (journal, None),
                Err(error) => (Journal::default(), Some(error)),
            }
        };
        let mut app = Self {
            prefs,
            secrets: vec![Secret::default(); provider_count],
            verified: vec![false; provider_count],
            last_usage: vec![None; provider_count],
            markets,
            quotes,
            news: cache.news,
            news_fetched: cache.news_fetched,
            failures: HashMap::new(),
            inflight: HashSet::new(),
            pending_markets: VecDeque::new(),
            news_inflight: HashSet::new(),
            analyses,
            analyses_load_error,
            analyzing: HashSet::new(),
            ai_errors: HashMap::new(),
            journal,
            journal_open: launch.view == "logbook",
            model_guide_open: launch.view == "models",
            local_model_check: None,
            local_model_check_busy: false,
            local_forecast_backend: crate::forecast::FoundationBackend::TimesFm,
            local_forecast: None,
            local_forecast_busy: false,
            desk: outlook::Desk::new(
                launch.view != "prices",
                (!transient).then(|| data.join("scenario-history.json")),
            ),
            journal_selected_only: false,
            journal_limit: 100,
            journal_load_error,
            journal_error: None,
            review_errors: HashMap::new(),
            reviewing: false,
            last_review: None,
            tx,
            rx,
            search: String::new(),
            search_results: vec![],
            search_generation: 0,
            search_at: Instant::now(),
            search_sent: 0,
            search_error: None,
            focus_search: false,
            editor: None,
            editor_sequence: 0,
            manage: false,
            new_list: String::new(),
            edit_rows: false,
            dragging: None,
            range_drag: None,
            hover_info: String::new(),
            business_news: launch.view == "news",
            saved_only: launch.view == "saved",
            manage_symbol: None,
            news_only: false,
            allow_demo_ai: false,
            analyze_all: false,
            settings: false,
            information: match launch.view.as_str() {
                "about" => Some(support::InfoPage::About),
                "help" => Some(support::InfoPage::Help),
                "disclaimer" => Some(support::InfoPage::Disclaimer),
                _ => None,
            },
            terms_path,
            terms_accepted,
            terms_error: None,
            launch_intent: launch.intent,
            open_connections_at_start: launch.view == "connections",
            market_import: None,
            import_sequence: 0,
            import: false,
            import_text: String::new(),
            article: None,
            raw_answer: None,
            toast: None,
            last_refresh: Instant::now(),
            shot: launch.screenshot,
            frames: 0,
            shot_sent: false,
            state_path,
            settings_load_error,
            transient,
            deleted_list: None,
            captured: launch.captured,
            initial_geometry: launch.auto_size,
            geometry_screen: None,
        };
        app.style(ctx);
        if transient {
            match launch.view.as_str() {
                "expert" => app.prefs.outlook_depth = crate::scenarios::Depth::Expert,
                "market-god" => app.prefs.outlook_depth = crate::scenarios::Depth::MarketGod,
                _ => {}
            }
        }
        if app.terms_accepted {
            app.start_workspace(ctx);
        }
        if launch.view == "import-data" {
            app.open_market_import();
        }
        app
    }
    fn start_workspace(&mut self, ctx: &egui::Context) {
        if !self.transient {
            for (provider, secret) in self.prefs.providers.iter().zip(&mut self.secrets) {
                if provider.remember {
                    *secret = load_secret(provider);
                }
            }
        }
        if self.open_connections_at_start {
            self.open_editor();
        }
        if let Some(intent) = self.launch_intent.take() {
            self.dispatch_intent(intent, ctx);
        }
        if !self.prefs.offline {
            self.refresh(ctx);
            self.load_outlook_history(ctx, false);
        }
    }
    fn style(&self, ctx: &egui::Context) {
        ctx.set_theme(if self.prefs.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let p = Palette::new(self.prefs.dark);
        let mut v = if self.prefs.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.panel_fill = p.bg;
        v.window_fill = p.panel;
        v.extreme_bg_color = p.bg;
        v.faint_bg_color = p.raised;
        v.override_text_color = Some(p.text);
        v.selection.bg_fill = alpha(p.blue, 60);
        v.selection.stroke = Stroke::new(1., p.blue);
        v.widgets.noninteractive.bg_stroke = Stroke::new(1., p.line);
        v.widgets.inactive.bg_fill = p.raised;
        v.widgets.inactive.weak_bg_fill = p.raised;
        v.widgets.hovered.bg_fill = alpha(p.blue, 55);
        v.widgets.active.bg_fill = alpha(p.blue, 80);
        v.window_corner_radius = CornerRadius::same(14);
        ctx.set_visuals(v);
        ctx.global_style_mut(|s| {
            s.spacing.item_spacing = vec2(8., 7.);
            s.spacing.button_padding = vec2(10., 6.);
            s.text_styles
                .insert(egui::TextStyle::Body, FontId::proportional(14.));
            s.text_styles
                .insert(egui::TextStyle::Button, FontId::proportional(13.));
        });
    }
    fn notify(&mut self, s: impl Into<String>) {
        self.toast = Some((s.into(), Instant::now()));
    }
    fn save(&mut self) {
        self.prefs.remember_list_view();
        if !self.transient
            && self.settings_load_error.is_none()
            && let Err(e) = self.prefs.save(&self.state_path)
        {
            self.notify(format!("Could not save changes: {e}"));
        }
    }
    fn open_editor(&mut self) {
        self.editor_sequence += 1;
        self.editor = Some(Editor::new(
            self.editor_sequence,
            &self.prefs,
            &self.secrets,
            &self.verified,
            &self.last_usage,
        ));
    }
    fn quote(&self, t: &str) -> Option<Quote> {
        if self.prefs.offline {
            return demo_market(t, 0).map(|m| m.quote);
        }
        self.quotes.get(t).cloned()
    }
    fn market_loading(&self, t: &str, range: usize) -> bool {
        let key = (t.to_owned(), range);
        !self.prefs.offline && (self.inflight.contains(&key) || self.pending_markets.contains(&key))
    }
    fn refresh_view(&mut self, ctx: &egui::Context) {
        self.refresh(ctx);
        self.load_outlook_history(ctx, true);
    }

    fn data_mode_changed(&mut self, ctx: &egui::Context) {
        self.pending_markets.clear();
        self.allow_demo_ai = false;
        self.range_drag = None;
        self.hover_info.clear();
        self.reset_search_results();
        self.desk.clear_replay();
        if !self.prefs.offline {
            self.refresh_view(ctx);
        }
    }
    fn market_failure(&self) -> Option<&str> {
        if self.prefs.offline {
            return None;
        }
        self.failures
            .get(&format!(
                "market:{}:{}",
                self.prefs.selected, self.prefs.range
            ))
            .or_else(|| {
                self.quote(&self.prefs.selected)
                    .is_none()
                    .then(|| {
                        self.failures
                            .get(&format!("market:{}:0", self.prefs.selected))
                    })
                    .flatten()
            })
            .map(String::as_str)
    }
    fn market(&self) -> Option<Market> {
        let mut market = if self.prefs.offline {
            demo_market(&self.prefs.selected, self.prefs.range)
        } else {
            self.markets
                .get(&(self.prefs.selected.clone(), self.prefs.range))
                .cloned()
        }?;
        if market.range == 0 && !self.prefs.show_extended {
            if let Some([start, end]) = market.regular_session {
                market.points.retain(|p| p.time >= start && p.time < end);
            }
        }
        Some(market)
    }
    fn choose(&mut self, t: &str, ctx: &egui::Context) {
        self.select_symbol(t);
        if !self.prefs.offline {
            self.load_market(t.to_owned(), self.prefs.range, ctx);
            self.load_market(t.to_owned(), 0, ctx);
            self.load_news(t.to_owned(), ctx);
            self.load_outlook_history(ctx, false);
        }
    }
    fn select_symbol(&mut self, t: &str) {
        if self.prefs.selected != t {
            self.desk.clear_replay();
        }
        self.prefs.selected = t.into();
        let id = self.prefs.active_list;
        if let Some(w) = self.prefs.watchlists.iter_mut().find(|w| w.id == id) {
            w.selected = t.into();
        }
        self.business_news = false;
        self.saved_only = false;
        self.hover_info.clear();
        self.range_drag = None;
        self.save();
    }
    fn choose_list(&mut self, id: u64, ctx: &egui::Context) {
        self.prefs.remember_list_view();
        self.prefs.active_list = id;
        self.prefs.restore_list_view();
        if let Some(w) = self.prefs.watchlists.iter().find(|w| w.id == id) {
            let s = if w.symbols.contains(&w.selected) {
                w.selected.clone()
            } else {
                w.symbols
                    .first()
                    .cloned()
                    .unwrap_or(self.prefs.selected.clone())
            };
            self.choose(&s, ctx);
        }
        self.clear_search();
        self.dragging = None;
        self.save();
        self.refresh(ctx);
    }
    fn dispatch_intent(&mut self, intent: NavigationIntent, ctx: &egui::Context) {
        // Navigation intents do not alter saved watchlist membership.
        match intent {
            NavigationIntent::Symbol(ticker) if valid_ticker(&ticker) => {
                self.choose(&ticker, ctx);
            }
            NavigationIntent::Watchlist(id) => {
                if self.prefs.watchlists.iter().any(|w| w.id == id) {
                    self.prefs.scroll_offsets.insert(id, 0.);
                    self.choose_list(id, ctx);
                } else {
                    self.notify("That watchlist is no longer available.");
                }
            }
            NavigationIntent::BusinessNews => {
                self.business_news = true;
                self.saved_only = false;
                self.clear_search();
                self.article = None;
                if !self.prefs.offline {
                    self.load_news("__market".into(), ctx);
                }
            }
            NavigationIntent::Symbol(_) => self.notify("That ticker is not valid."),
        }
    }
    fn adjacent_list(&mut self, forward: bool, ctx: &egui::Context) {
        let len = self.prefs.watchlists.len();
        let i = self
            .prefs
            .watchlists
            .iter()
            .position(|w| w.id == self.prefs.active_list)
            .unwrap_or(0);
        let next = if forward {
            (i + 1) % len
        } else {
            (i + len - 1) % len
        };
        self.choose_list(self.prefs.watchlists[next].id, ctx);
    }
    fn save_cache(&mut self) {
        if self.transient || self.prefs.offline {
            return;
        }
        let mut markets = self
            .markets
            .values()
            .filter(|m| !m.quote.demo)
            .cloned()
            .collect::<Vec<_>>();
        markets.sort_by_key(|m| std::cmp::Reverse(m.quote.fetched));
        markets.truncate(100);
        let mut quotes = self.quotes.values().cloned().collect::<Vec<_>>();
        quotes.sort_by_key(|q| std::cmp::Reverse(q.fetched));
        quotes.truncate(100);
        if let Err(e) = atomic_json(
            &self.state_path.with_file_name("market-cache.json"),
            &MarketCache {
                markets,
                quotes,
                news: self.news.clone(),
                news_fetched: self.news_fetched.clone(),
            },
        ) {
            self.notify(format!("Could not save the last market data: {e}"));
        }
    }
    fn apply_market(&mut self, key: (String, usize), market: Market) {
        remember_quote(&mut self.quotes, market.quote.clone());
        let current = self.markets.get(&key).filter(|m| m.points.len() >= 2);
        let issue = if market.points.len() < 2 {
            Some(if current.is_some() {
                "The feed returned no usable chart update. The last available chart is shown."
            } else {
                "The feed returned a price without usable chart history."
            })
        } else if current
            .is_some_and(|m| m.points.last().map(|p| p.time) > market.points.last().map(|p| p.time))
        {
            Some("The feed returned an older chart. The most recent available chart is shown.")
        } else {
            None
        };
        let status = format!("market:{}:{}", key.0, key.1);
        if let Some(issue) = issue {
            self.failures.insert(status, issue.into());
        } else {
            self.failures.remove(&status);
        }
        if issue.is_none() || current.is_none() {
            self.markets.insert(key, market);
        }
    }

    fn reset_search_results(&mut self) {
        self.search_generation += 1;
        self.search_at = Instant::now();
        self.search_results.clear();
        self.search_error = None;
    }

    fn clear_search(&mut self) {
        self.search.clear();
        self.reset_search_results();
    }
    fn load_market(&mut self, t: String, r: usize, ctx: &egui::Context) {
        let key = (t, r);
        if self.inflight.contains(&key) {
            return;
        }
        let selected_chart = key.0 == self.prefs.selected && r == self.prefs.range;
        if let Some(index) = self.pending_markets.iter().position(|job| *job == key) {
            if selected_chart {
                self.pending_markets.remove(index);
                self.pending_markets.push_front(key);
            }
            return;
        }
        if self.inflight.len() >= MARKET_WORKERS {
            if selected_chart {
                self.pending_markets.push_front(key);
            } else {
                self.pending_markets.push_back(key);
            }
            return;
        }
        self.inflight.insert(key.clone());
        let (t, r) = key;
        let tx = self.tx.clone();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let result = network::fetch_market(&t, r);
            let _ = tx.send(Event::Market(t, r, result));
            c.request_repaint();
        });
    }

    fn prune_market_queue(&mut self) {
        let selected = &self.prefs.selected;
        let range = self.prefs.range;
        let symbols = &self.prefs.list().symbols;
        self.pending_markets.retain(|(ticker, period)| {
            (ticker == selected && (*period == range || *period == 0))
                || (*period == 0
                    && (symbols.contains(ticker) || MARKET_INDEXES.contains(&ticker.as_str())))
        });
    }
    fn load_news(&mut self, t: String, ctx: &egui::Context) {
        if !self.news_inflight.insert(t.clone()) {
            return;
        }
        let tx = self.tx.clone();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let result = network::fetch_news(&t);
            let _ = tx.send(Event::News(t, result));
            c.request_repaint();
        });
    }
    fn refresh(&mut self, ctx: &egui::Context) {
        self.last_refresh = Instant::now();
        if self.prefs.offline {
            return;
        }
        let s = self.prefs.selected.clone();
        self.load_market(s.clone(), self.prefs.range, ctx);
        self.load_market(s.clone(), 0, ctx);
        self.load_news(s, ctx);
        self.prune_market_queue();
        let mut symbols = self.prefs.list().symbols.clone();
        for t in MARKET_INDEXES {
            if !symbols.contains(&t.to_owned()) {
                symbols.push(t.into());
            }
        }
        for t in symbols {
            self.load_market(t, 0, ctx);
        }
        if self.business_news {
            self.load_news("__market".into(), ctx);
        }
        if !self.transient && self.last_review.is_none() {
            self.launch_reviews(ctx);
        }
    }
    fn save_journal(&mut self) {
        if self.transient || self.journal_load_error.is_some() {
            return;
        }
        self.journal_error = self
            .journal
            .save(&self.state_path.with_file_name("predictions.json"))
            .err();
        if self.journal_error.is_some() {
            self.notify("Could not save the prediction logbook. Open Logbook for details.");
        }
    }
    fn launch_reviews(&mut self, ctx: &egui::Context) {
        if self.transient || self.prefs.offline || self.reviewing {
            return;
        }
        self.last_review = Some(Instant::now());
        let mut jobs: HashMap<String, (i64, Vec<u64>)> = HashMap::new();
        for entry in self.journal.entries.iter().filter(|e| e.can_review(now())) {
            let job = jobs
                .entry(entry.symbol.clone())
                .or_insert((entry.quote_time, vec![]));
            job.0 = job.0.min(entry.quote_time);
            job.1.push(entry.id);
        }
        if jobs.is_empty() {
            return;
        }
        self.reviewing = true;
        self.review_errors.clear();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        // One queue limits review traffic, including old tickers no longer followed.
        std::thread::spawn(move || {
            for (symbol, (quote_time, ids)) in jobs {
                let result = network::fetch_review_market(&symbol, quote_time);
                if tx.send(Event::Review(symbol, ids, result)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
            let _ = tx.send(Event::ReviewsDone);
            ctx.request_repaint();
        });
    }
    fn poll(&mut self) {
        let mut cache_changed = false;
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Market(t, r, result) => {
                    self.inflight.remove(&(t.clone(), r));
                    if !self.prefs.offline {
                        match result {
                            Ok(m) => {
                                self.apply_market((t, r), m);
                                cache_changed = true;
                            }
                            Err(e) => {
                                self.failures.insert(format!("market:{t}:{r}"), e);
                            }
                        }
                    }
                }
                Event::News(t, result) => {
                    self.news_inflight.remove(&t);
                    if !self.prefs.offline {
                        match result {
                            Ok(n) => {
                                self.failures.remove(&format!("news:{t}"));
                                let observed = now();
                                if t != "__market" {
                                    let sig = crate::news_signal::analyze_headlines(
                                        &n, None, observed, 20,
                                    );
                                    self.desk.sentiment.record(&t, observed, &sig);
                                }
                                self.news_fetched.insert(t.clone(), observed);
                                self.news.insert(t, n);
                                cache_changed = true;
                            }
                            Err(e) => {
                                self.failures.insert(format!("news:{t}"), e);
                            }
                        }
                    }
                }
                Event::Search(g, result) => {
                    if g == self.search_generation && !self.prefs.offline {
                        match result {
                            Ok(results) => {
                                self.search_results = results;
                                self.search_error = None;
                            }
                            Err(_) => self.search_error = Some("Online search unavailable. Local matches are shown and you can enter an exact ticker.".into()),
                        }
                    }
                }
                Event::Models(session, i, rev, result) => {
                    if let Some(e) = self.editor.as_mut().filter(|e| e.session == session) {
                        e.busy.remove(&(i, 0));
                        if e.revision[i] == rev {
                            match result {
                                Ok(m) => {
                                    e.models[i] = m;
                                    e.message[i] =
                                        Some((true, "Choose a model from the list.".into()));
                                }
                                Err(s) => e.message[i] = Some((false, s)),
                            }
                        }
                    }
                }
                Event::Probe(session, i, rev, result) => {
                    if let Some(e) = self.editor.as_mut().filter(|e| e.session == session) {
                        e.busy.remove(&(i, 1));
                        if e.revision[i] == rev {
                            e.tested[i] = result.is_ok();
                            match result {
                                Ok(usage) => {
                                    e.message[i] = Some((
                                        true,
                                        format!(
                                            "Connection verified with an inference request. {}.",
                                            usage.cost_label()
                                        ),
                                    ));
                                    e.usage[i] = Some(usage);
                                }
                                Err(err) => e.message[i] = Some((false, err)),
                            }
                        }
                    }
                }
                Event::Analysis(t, sample, connection_index, token, result) => {
                    self.analyzing.remove(&t);
                    let key = research_key(&t, sample);
                    match result {
                        Ok(a) => {
                            if token
                                == connection_token(
                                    &self.prefs.providers[connection_index],
                                    &self.secrets[connection_index],
                                )
                            {
                                self.verified[connection_index] = true;
                                self.last_usage[connection_index] = a.usage.clone();
                            }
                            self.ai_errors.remove(&key);
                            if self.journal.record(&a) > 0 {
                                self.save_journal();
                            }
                            self.analyses.insert(key, a);
                            if !self.transient && self.analyses_load_error.is_none() {
                                if let Err(e) = atomic_json(
                                    &self.state_path.with_file_name("analyses.json"),
                                    &self.analyses,
                                ) {
                                    self.notify(format!("Analysis is available this session, but could not be saved: {e}"));
                                }
                            }
                        }
                        Err(e) => {
                            self.ai_errors.insert(key, e);
                        }
                    }
                }
                Event::Review(symbol, ids, result) => {
                    if !self.prefs.offline {
                        match result {
                            Ok(data) => {
                                self.review_errors.remove(&symbol);
                                if self.journal.resolve(&ids, &data, now()) > 0 {
                                    self.save_journal();
                                }
                            }
                            Err(error) => {
                                self.review_errors.insert(symbol, error);
                            }
                        }
                    }
                }
                Event::ReviewsDone => {
                    self.reviewing = false;
                }
                Event::LocalModelCheck(report) => {
                    self.local_model_check = Some(report);
                    self.local_model_check_busy = false;
                }
                Event::LocalForecast(result) => {
                    self.local_forecast = Some(result);
                    self.local_forecast_busy = false;
                }
                Event::ScenarioHistory(ticker, result) => {
                    self.desk.finish_history(ticker, result, self.prefs.offline);
                }
                Event::Briefing(key, result) => self.desk.finish_briefing(key, result),
                Event::HistoryImport(sequence, result) => {
                    self.finish_market_import(sequence, result)
                }
                Event::ExportedReport(result) => match result {
                    Ok(path) => self.notify(format!("Saved analysis to {}", path.display())),
                    Err(error) => self.notify(format!("Could not export analysis. {error}")),
                },
            }
        }
        if cache_changed {
            self.save_cache();
        }
    }
    fn analysis_tickers(&self) -> Vec<String> {
        if self.analyze_all {
            self.prefs.list().symbols.clone()
        } else {
            vec![self.prefs.selected.clone()]
        }
    }

    fn launch_analysis(&mut self, ctx: &egui::Context) {
        if !self.analyzing.is_empty() {
            return;
        }
        let i = self.prefs.active_provider;
        let p = self.prefs.providers[i].clone();
        let s = self.secrets[i].clone();
        let token = connection_token(&p, &s);
        let sample = self.prefs.offline;
        let mut groups: [Vec<_>; 3] = std::array::from_fn(|_| Vec::new());
        for t in self.analysis_tickers() {
            let key = research_key(&t, sample);
            let m = if sample {
                self.allow_demo_ai.then(|| demo_market(&t, 0)).flatten()
            } else {
                self.quote(&t).map(|quote| {
                    let mut market = [ChartRange::OneYear.index(), self.prefs.range, 0]
                        .into_iter()
                        .filter_map(|range| self.markets.get(&(t.clone(), range)))
                        .find(|market| {
                            market.quote.currency == quote.currency && !market.points.is_empty()
                        })
                        .cloned()
                        .unwrap_or_else(|| Market {
                            quote: quote.clone(),
                            points: vec![],
                            range: self.prefs.range,
                            regular_session: None,
                        });
                    market.quote = quote;
                    market
                })
            };
            let Some(m) = m else {
                self.ai_errors
                    .insert(key, if sample {
                        "A sample quote and permission to use it are required for demonstration analysis.".into()
                    } else {
                        "Load a market quote before requesting analysis.".into()
                    });
                continue;
            };
            let index = self.analyzing.len() % groups.len();
            self.analyzing.insert(t.clone());
            self.ai_errors.remove(&key);
            let news = self.news.get(&t).cloned().unwrap_or_default();
            let regime = self.desk.market_regime(sample);
            let sent = self.desk.sentiment.series(&t);
            let outlook_snap = self
                .desk
                .history(&t, sample)
                .and_then(|h| {
                    crate::scenarios::report_with_context(&h, 20, regime.as_deref(), sent.as_ref())
                        .ok()
                })
                .map(|r| network::OutlookSnapshot::from_scenario(&r.current));
            let mem = self.journal.learning_context_for(
                &p,
                Some(&t),
                if sample { 0 } else { self.prefs.memory_days },
                now(),
            );
            groups[index].push((t, m, news, mem, outlook_snap));
        }
        for jobs in groups.into_iter().filter(|group| !group.is_empty()) {
            let tx = self.tx.clone();
            let c = ctx.clone();
            let (p, s) = (p.clone(), s.clone());
            std::thread::spawn(move || {
                for (ticker, market, news, memory, outlook_snap) in jobs {
                    let result = network::analyze_with_outlook(
                        &p,
                        &s,
                        &ticker,
                        market,
                        news,
                        &memory,
                        outlook_snap.as_ref(),
                    );
                    if tx
                        .send(Event::Analysis(ticker, sample, i, token, result))
                        .is_err()
                    {
                        break;
                    }
                    c.request_repaint();
                }
            });
        }
    }
    fn top(&mut self, root: &mut egui::Ui, p: Palette) {
        let ctx = root.ctx().clone();
        egui::Panel::top("toolbar")
            .exact_size(55.)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .stroke(Stroke::new(1., p.line))
                    .inner_margin(egui::Margin::symmetric(17, 8)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    let (mark, _) = ui.allocate_exact_size(vec2(24., 24.), Sense::hover());
                    let painter = ui.painter();
                    let badge_bg = if self.prefs.dark {
                        hex(0x184333)
                    } else {
                        hex(0x123427)
                    };
                    let ivory = hex(0xf5f5ef);
                    let gold = hex(0xe7cf93);
                    let glass = alpha(hex(0x78b897), 165);
                    let glass_edge = alpha(gold, 150);
                    painter.rect_filled(mark, 5.5, badge_bg);
                    painter.rect_stroke(
                        mark.shrink(0.5),
                        5.5,
                        Stroke::new(0.8, alpha(gold, 70)),
                        egui::StrokeKind::Inside,
                    );
                    let pt = |u: f32, v: f32| {
                        pos2(
                            mark.left() + mark.width() * u,
                            mark.top() + mark.height() * v,
                        )
                    };
                    // Ascending ivory crossbar
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            pt(0.22, 0.31),
                            pt(0.78, 0.18),
                            pt(0.78, 0.30),
                            pt(0.22, 0.43),
                        ],
                        ivory,
                        Stroke::NONE,
                    ));
                    // Champagne-gold vertical pillar
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            pt(0.44, 0.40),
                            pt(0.56, 0.37),
                            pt(0.56, 0.83),
                            pt(0.44, 0.83),
                        ],
                        gold,
                        Stroke::NONE,
                    ));
                    // Frosted emerald-gold triangular prism facet
                    painter.add(egui::Shape::convex_polygon(
                        vec![pt(0.60, 0.36), pt(0.77, 0.32), pt(0.60, 0.56)],
                        glass,
                        Stroke::new(0.7, glass_edge),
                    ));
                    ui.label(display_text("Tenerus", 20., p.text));
                    ui.add_space(12.);
                    let width = (ui.available_width() - 390.).clamp(105., 370.);
                    let search = Frame::new()
                        .fill(p.panel)
                        .stroke(Stroke::new(1., p.line))
                        .corner_radius(5)
                        .inner_margin(egui::Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.search)
                                    .id(Id::unique("symbol_search"))
                                    .hint_text("Search symbol or company")
                                    .desired_width(width)
                                    .frame(Frame::NONE)
                                    .margin(Vec2::ZERO),
                            )
                        })
                        .inner;
                    if self.focus_search {
                        search.request_focus();
                        self.focus_search = false;
                    }
                    if search.changed() {
                        self.prefs.show_sidebar = true;
                        self.reset_search_results();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.style_mut().visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                        let sample = self.prefs.offline;
                        if link(ui, if sample { "Use live data" } else { "Refresh" }, p)
                            .on_hover_text(if sample {
                                "Leave the demonstration and connect to public data feeds"
                            } else {
                                "Refresh public market data and daily history"
                            })
                            .clicked()
                        {
                            if sample {
                                self.prefs.offline = false;
                                self.data_mode_changed(&ctx);
                                self.save();
                            } else {
                                self.refresh_view(&ctx);
                            }
                        }
                        ui.separator();
                        self.help_menu(ui);
                        ui.menu_button("User", |ui| {
                            if ui.button("API connections…").clicked() {
                                self.open_editor();
                                ui.close();
                            }
                            if ui.button("Import market data…").clicked() {
                                self.open_market_import();
                                ui.close();
                            }
                            if ui.button("Preferences…").clicked() {
                                self.settings = true;
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Export watchlists").clicked() {
                                self.export_lists();
                                ui.close();
                            }
                            if ui.button("Import watchlists…").clicked() {
                                self.import = true;
                                ui.close();
                            }
                            if ui.button("Copy selected ticker").clicked() {
                                ctx.copy_text(self.prefs.selected.clone());
                                ui.close();
                            }
                        });
                        ui.menu_button("View", |ui| {
                            if ui.button("Outlook").clicked() {
                                self.desk.active = true;
                                self.business_news = false;
                                self.saved_only = false;
                                self.load_outlook_history(&ctx, false);
                                ui.close();
                            }
                            if ui.button("Price & news").clicked() {
                                self.desk.active = false;
                                self.business_news = false;
                                self.saved_only = false;
                                ui.close();
                            }
                            if ui.button("Market overview").clicked() {
                                self.dispatch_intent(NavigationIntent::BusinessNews, &ctx);
                                ui.close();
                            }
                            if ui.button("Saved stories").clicked() {
                                self.saved_only = true;
                                self.business_news = false;
                                ui.close();
                            }
                            ui.separator();
                            let sidebar = ui
                                .checkbox(&mut self.prefs.show_sidebar, "Show watchlists")
                                .changed();
                            let api = ui
                                .checkbox(&mut self.prefs.show_ai, "API research pane")
                                .changed();
                            if api && self.prefs.show_ai {
                                self.desk.active = false;
                                self.business_news = false;
                                self.saved_only = false;
                            }
                            let extended = ui
                                .checkbox(
                                    &mut self.prefs.show_extended,
                                    "Show extended-hours trading",
                                )
                                .changed();
                            if sidebar || api || extended {
                                self.save();
                            }
                            ui.separator();
                            if ui.button("Prediction logbook").clicked() {
                                self.journal_open = true;
                                ui.close();
                            }
                            if ui.button("Forecast model guide").clicked() {
                                self.model_guide_open = true;
                                ui.close();
                            }
                        });
                        ui.menu_button("Watchlists", |ui| {
                            let mut selected = None;
                            for w in &self.prefs.watchlists {
                                if ui
                                    .selectable_label(w.id == self.prefs.active_list, &w.name)
                                    .clicked()
                                {
                                    selected = Some(w.id);
                                    ui.close();
                                }
                            }
                            if let Some(id) = selected {
                                self.prefs.show_sidebar = true;
                                self.choose_list(id, &ctx);
                            }
                            ui.separator();
                            if ui.button("Manage watchlists…").clicked() {
                                self.manage = true;
                                ui.close();
                            }
                            if ui.button("Previous watchlist   ⌘⌥←").clicked() {
                                self.adjacent_list(false, &ctx);
                                ui.close();
                            }
                            if ui.button("Next watchlist   ⌘⌥→").clicked() {
                                self.adjacent_list(true, &ctx);
                                ui.close();
                            }
                        });
                    });
                });
            });
    }
    fn status(&mut self, root: &mut egui::Ui, p: Palette) {
        egui::Panel::bottom("status")
            .exact_size(29.)
            .frame(
                Frame::new()
                    .fill(p.side)
                    .inner_margin(egui::Margin::symmetric(18, 5)),
            )
            .show(root, |ui| {
                let quote = self.quote(&self.prefs.selected);
                let demo = self.prefs.offline;
                let imported =
                    self.using_outlook() && self.desk.imports.get(&self.prefs.selected).is_some();
                let failed = !imported && self.market_failure().is_some();
                let message = if self.settings_load_error.is_some() {
                    "Preferences could not be read · changes apply for this session"
                } else if imported {
                    "Outlook uses imported daily closes"
                } else if demo {
                    "SAMPLE DATA  ·  Illustrative prices and historical periods"
                } else if self.market_loading(&self.prefs.selected, self.prefs.range) {
                    "Loading public market data…"
                } else if failed && quote.is_none() {
                    "Market feed unavailable"
                } else if failed {
                    "Market update incomplete · data shown with timestamps"
                } else if quote.is_none() {
                    "No quote loaded"
                } else {
                    "Public quotes may be delayed · times shown in UTC"
                };
                let ink = if self.settings_load_error.is_some() || (failed && !demo) {
                    p.red
                } else {
                    p.muted
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    text(
                        ui,
                        concat!("Tenerus ", env!("CARGO_PKG_VERSION")),
                        11.,
                        p.muted,
                    );
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(message).size(11.).color(ink))
                                .truncate(),
                        )
                        .on_hover_text(message);
                    });
                });
            });
    }
    fn export_lists(&mut self) {
        let file = data_dir()
            .join("exports")
            .join(format!("watchlists-{}.json", now()));
        let data = ExportLists {
            version: 1,
            watchlists: self.prefs.watchlists.clone(),
            symbols: self.prefs.symbols.clone(),
        };
        match atomic_json(&file, &data) {
            Ok(()) => {
                self.notify(format!("Exported watchlists to {}", file.display()));
            }
            Err(e) => self.notify(format!("Export failed: {e}")),
        }
    }
}

impl MarketApp {
    fn sidebar(&mut self, root: &mut egui::Ui, p: Palette) {
        let ctx = root.ctx().clone();
        let panel = egui::Panel::left("watchlist")
            .default_size(self.prefs.sidebar_width)
            .size_range(235.0..=360.0)
            .resizable(true)
            .frame(
                Frame::new()
                    .fill(p.side)
                    .inner_margin(egui::Margin::symmetric(13, 18)),
            )
            .show(root, |ui| {
                let mut new_id = None;
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("watchlists")
                        .selected_text(RichText::new(&self.prefs.list().name).size(15.).strong())
                        .width(ui.available_width() - 52.)
                        .show_ui(ui, |ui| {
                            for w in &self.prefs.watchlists {
                                if ui
                                    .selectable_label(w.id == self.prefs.active_list, &w.name)
                                    .clicked()
                                {
                                    new_id = Some(w.id);
                                }
                            }
                            ui.separator();
                            if ui.button("Manage watchlists…").clicked() {
                                self.manage = true;
                            }
                        });
                    ui.menu_button("•••", |ui| {
                        if ui.button("New watchlist…").clicked() {
                            self.manage = true;
                            ui.close();
                        }
                        if ui
                            .button(if self.edit_rows {
                                "Done editing"
                            } else {
                                "Edit symbols"
                            })
                            .clicked()
                        {
                            self.edit_rows = !self.edit_rows;
                            ui.close();
                        }
                        ui.separator();
                        text(ui, "Sort symbols", 12., p.muted);
                        for (i, s) in [
                            "Manual order",
                            "Symbol",
                            "Price",
                            "Percentage change",
                            "Market cap",
                            "Company name",
                            "Price change",
                        ]
                        .iter()
                        .enumerate()
                        {
                            if ui.selectable_value(&mut self.prefs.sort, i, *s).changed() {
                                self.save();
                            }
                        }
                        if self.prefs.sort != 0 {
                            ui.horizontal(|ui| {
                                if ui
                                    .selectable_value(
                                        &mut self.prefs.sort_descending,
                                        false,
                                        "Ascending",
                                    )
                                    .changed()
                                {
                                    self.save();
                                }
                                if ui
                                    .selectable_value(
                                        &mut self.prefs.sort_descending,
                                        true,
                                        "Descending",
                                    )
                                    .changed()
                                {
                                    self.save();
                                }
                            });
                        }
                        ui.separator();
                        text(ui, "Row value", 12., p.muted);
                        for mode in PriceChangeDisplay::ALL {
                            if ui
                                .selectable_value(
                                    &mut self.prefs.display,
                                    mode.saved_index(),
                                    mode.label(),
                                )
                                .changed()
                            {
                                self.save();
                            }
                        }
                    });
                });
                if let Some(id) = new_id {
                    self.choose_list(id, &ctx);
                }
                if !self.search.is_empty() {
                    small_title(ui, "SEARCH RESULTS", p);
                }
                ui.add_space(9.);
                let footer = 91.;
                let scroll_h = (ui.available_height() - footer).max(0.);
                let list_id = self.prefs.active_list;
                let searching = !self.search.trim().is_empty();
                let mut scroll_area = egui::ScrollArea::vertical()
                    .id_salt(("watchlist_scroll", list_id, searching))
                    .max_height(scroll_h)
                    .auto_shrink([false, false]);
                if !searching {
                    scroll_area = scroll_area.vertical_scroll_offset(
                        self.prefs
                            .scroll_offsets
                            .get(&list_id)
                            .copied()
                            .unwrap_or(0.),
                    );
                }
                let scroll = scroll_area.show(ui, |ui| {
                    if !self.search.trim().is_empty() {
                        let query = self.search.trim().to_lowercase();
                        let mut matches = self.search_results.clone();
                        for s in &self.prefs.symbols {
                            if (s.ticker.to_lowercase().contains(&query)
                                || s.name.to_lowercase().contains(&query))
                                && !matches.iter().any(|x| x.ticker == s.ticker)
                            {
                                matches.push(s.clone());
                            }
                        }
                        if matches.is_empty() {
                            text(ui, "No local matches yet.", 13., p.muted);
                        }
                        for s in matches.into_iter().take(20) {
                            Frame::new().inner_margin(8).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        if link(ui, &s.ticker, p).clicked() {
                                            if !self
                                                .prefs
                                                .symbols
                                                .iter()
                                                .any(|x| x.ticker == s.ticker)
                                            {
                                                self.prefs.symbols.push(s.clone());
                                            }
                                            self.choose(&s.ticker, &ctx);
                                            self.clear_search();
                                        }
                                        text(ui, elide(&s.name, 25), 12., p.muted);
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let followed =
                                            self.prefs.list().symbols.contains(&s.ticker);
                                        if button(ui, if followed { "✓" } else { "+" }, p)
                                            .on_hover_text("Add to this watchlist")
                                            .clicked()
                                        {
                                            self.prefs.add(s.clone());
                                            self.choose(&s.ticker, &ctx);
                                            self.clear_search();
                                        }
                                    });
                                });
                            });
                            ui.separator();
                        }
                        if let Some(e) = &self.search_error {
                            text(ui, e, 11., p.muted);
                        }
                        let exact = self.search.trim().to_uppercase();
                        if valid_ticker(&exact)
                            && button(ui, &format!("Open exact ticker: {exact}"), p).clicked()
                        {
                            self.choose(&exact, &ctx);
                            self.clear_search();
                        }
                    } else {
                        let symbols = self.prefs.sorted_symbols(|t| self.quote(t));
                        if symbols.is_empty() {
                            ui.add_space(35.);
                            text(ui, "Your watchlist is ready.", 17., p.text);
                            text(
                                ui,
                                "Search for a company or ticker above, then use + to follow it.",
                                13.,
                                p.muted,
                            );
                        }
                        for t in symbols {
                            let symbol = self.prefs.symbol(&t);
                            let q = self.quote(&t);
                            let selected = t == self.prefs.selected && !self.business_news;
                            let (response, cycle) =
                                self.watch_row(ui, &symbol, q.as_ref(), selected, p);
                            if cycle {
                                self.prefs.display =
                                    PriceChangeDisplay::from_saved_index(self.prefs.display)
                                        .unwrap_or_default()
                                        .next()
                                        .saved_index();
                                self.save();
                            }
                            if response.clicked() && !cycle {
                                self.choose(&t, &ctx);
                            }
                            if response.drag_started() {
                                self.dragging = Some(t.clone());
                            }
                            if response.contains_pointer()
                                && ctx.input(|i| i.pointer.any_released())
                            {
                                if let Some(from) = self.dragging.take() {
                                    if from != t {
                                        let id = self.prefs.active_list;
                                        let list = self
                                            .prefs
                                            .watchlists
                                            .iter_mut()
                                            .find(|w| w.id == id)
                                            .unwrap();
                                        if let (Some(a), Some(b)) = (
                                            list.symbols.iter().position(|x| x == &from),
                                            list.symbols.iter().position(|x| x == &t),
                                        ) {
                                            let s = list.symbols.remove(a);
                                            list.symbols.insert(b, s);
                                            self.prefs.sort = 0;
                                            self.save();
                                        }
                                    }
                                }
                            }
                            response.context_menu(|ui| {
                                if ui.button("Copy ticker").clicked() {
                                    ctx.copy_text(t.clone());
                                    ui.close();
                                }
                                if ui.button("Add to watchlists…").clicked() {
                                    self.manage_symbol = Some(t.clone());
                                    ui.close();
                                }
                                if ui.button("Remove from this watchlist").clicked() {
                                    self.prefs.remove(&t);
                                    self.save();
                                    ui.close();
                                }
                            });
                            if self.edit_rows {
                                ui.horizontal(|ui| {
                                    if link(ui, "Remove", p).clicked() {
                                        self.prefs.remove(&t);
                                        self.save();
                                    }
                                    if link(ui, "Move up", p).clicked() {
                                        let id = self.prefs.active_list;
                                        let list = self
                                            .prefs
                                            .watchlists
                                            .iter_mut()
                                            .find(|w| w.id == id)
                                            .unwrap();
                                        if let Some(i) = list.symbols.iter().position(|x| x == &t) {
                                            if i > 0 {
                                                list.symbols.swap(i, i - 1);
                                                self.prefs.sort = 0;
                                                self.save();
                                            }
                                        }
                                    }
                                });
                            }
                            ui.add_space(3.);
                        }
                    }
                });
                if ctx.input(|i| i.pointer.any_released()) {
                    self.dragging = None;
                }
                if !searching {
                    self.prefs
                        .scroll_offsets
                        .insert(list_id, scroll.state.clamped_offset().y);
                }
                ui.add_space(10.);
                ui.separator();
                ui.add_space(7.);
                if ui
                    .add_sized(
                        [ui.available_width(), 30.],
                        egui::Button::new(RichText::new("+  Add a symbol").color(p.blue))
                            .fill(Color32::TRANSPARENT),
                    )
                    .clicked()
                {
                    self.focus_search = true;
                }
                ui.horizontal(|ui| {
                    if link(ui, "Manage lists", p).clicked() {
                        self.manage = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if link(ui, "Business news  →", p).clicked() {
                            self.dispatch_intent(NavigationIntent::BusinessNews, &ctx);
                        }
                    });
                });
            });
        self.prefs.sidebar_width = panel.response.rect.width();
    }
    fn watch_row(
        &self,
        ui: &mut egui::Ui,
        s: &Symbol,
        q: Option<&Quote>,
        selected: bool,
        p: Palette,
    ) -> (egui::Response, bool) {
        let (r, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 58.), Sense::click_and_drag());
        let paint = ui.painter();
        if selected || response.hovered() {
            paint.rect_filled(
                r,
                9,
                if selected {
                    alpha(p.blue, 24)
                } else {
                    p.raised
                },
            );
        }
        if selected {
            paint.rect_filled(
                Rect::from_min_size(pos2(r.left(), r.top() + 5.), vec2(3., r.height() - 10.)),
                2,
                p.blue,
            );
        }
        let x = r.left() + 12.;
        paint.text(
            pos2(x, r.top() + 11.),
            Align2::LEFT_TOP,
            &s.ticker,
            FontId::proportional(15.),
            p.text,
        );
        let name_rect = Rect::from_min_max(
            pos2(x, r.top() + 35.),
            pos2(r.right() - 83., r.bottom() - 4.),
        );
        let name_paint = paint.with_clip_rect(name_rect);
        name_paint.text(
            name_rect.left_top(),
            Align2::LEFT_TOP,
            elide(&s.name, 20),
            FontId::proportional(11.5),
            p.muted,
        );
        let color = match q.and_then(Quote::change) {
            Some(c) if c < 0. => p.red,
            Some(_) => p.green,
            None => p.muted,
        };
        paint.text(
            pos2(r.right() - 10., r.top() + 33.),
            Align2::RIGHT_TOP,
            q.map(|q| price(q.price)).unwrap_or("—".into()),
            FontId::proportional(12.),
            p.muted,
        );
        paint.hline(r.x_range(), r.bottom(), Stroke::new(1., p.line));
        let label = match PriceChangeDisplay::from_saved_index(self.prefs.display)
            .unwrap_or_default()
        {
            PriceChangeDisplay::Currency => q.and_then(Quote::change).map(|d| format!("{d:+.2}")),
            PriceChangeDisplay::MarketCap => Some(compact(q.and_then(|q| q.market_cap))),
            PriceChangeDisplay::Percentage => q
                .and_then(Quote::watchlist_percent)
                .map(|d| format!("{d:+.2}%")),
        }
        .unwrap_or("—".into());
        let badge = Rect::from_min_size(pos2(r.right() - 82., r.top() + 8.), vec2(72., 20.));
        paint.text(
            badge.right_center(),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(11.5),
            color,
        );
        let cycle = ui
            .interact(
                badge,
                Id::unique(("row_value", self.prefs.active_list, &s.ticker)),
                Sense::click(),
            )
            .on_hover_text("Click to change the row value for this watchlist")
            .clicked();
        (
            response.on_hover_text(format!(
                "{}\n{}\nDrag to reorder · Right-click for options",
                s.ticker, s.name
            )),
            cycle,
        )
    }
}
fn elide(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!(
            "{}…",
            s.chars().take(max.saturating_sub(1)).collect::<String>()
        )
    } else {
        s.into()
    }
}
impl MarketApp {
    fn center(&mut self, root: &mut egui::Ui, p: Palette) {
        let ctx = root.ctx().clone();
        egui::CentralPanel::default().frame(Frame::new().fill(p.bg).inner_margin(egui::Margin::symmetric(26,23))).show(root,|ui|{egui::ScrollArea::vertical().id_salt("main_scroll").auto_shrink([false,false]).show(ui,|ui|{
        if self.saved_only {self.saved_stories(ui,p);return;}
        if self.business_news{self.overview(ui,p,&ctx);return;}
        self.workspace_tabs(ui, p, false);
        ui.add_space(9.);
        let sym=self.prefs.symbol(&self.prefs.selected);let market=self.market();let quote=self.quote(&sym.ticker);
        ui.horizontal(|ui|{small_title(ui,&format!("{}  /  {}",self.prefs.list().name.to_uppercase(),sym.ticker),p);ui.with_layout(Layout::right_to_left(Align::Center),|ui|{ui.menu_button("•••",|ui|{if ui.button("Add to watchlists…").clicked(){self.manage_symbol=Some(sym.ticker.clone());ui.close();}if ui.button("Copy ticker link").clicked(){ctx.copy_text(format!("https://finance.yahoo.com/quote/{}",sym.ticker));self.notify("Ticker link copied");ui.close();}if ui.button("Export chart data").clicked(){if let Some(m)=&market{self.export_chart(m);}ui.close();}});let followed=self.prefs.list().symbols.contains(&sym.ticker);if link(ui,if followed{"✓ Following"}else{"+ Follow"},p).clicked(){if followed{self.prefs.remove(&sym.ticker);}else{self.prefs.add(sym.clone());}self.save();}});});
        ui.add_space(13.);
        ui.horizontal(|ui|{let(r,_)=ui.allocate_exact_size(vec2(44.,44.),Sense::hover());ui.painter().rect_filled(r,12,alpha(p.green,24));ui.painter().text(r.center(),Align2::CENTER_CENTER,sym.ticker.trim_start_matches('^').chars().next().unwrap_or('M'),FontId::proportional(24.),p.green);
            ui.add_space(3.);ui.vertical(|ui|{ui.label(RichText::new(&sym.name).size(25.).strong());text(ui,format!("{}  ·  {}",sym.ticker,if sym.exchange.is_empty(){"Market data"}else{&sym.exchange}),12.,p.muted);});});
        ui.add_space(16.);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(quote.as_ref().map(|q|price(q.price)).unwrap_or("—".into())).size(43.).strong());
            if let Some(q) = &quote {
                let ink = match q.change().or_else(|| q.percent()) {
                    Some(v) if v < 0. => p.red,
                    Some(v) if v > 0. => p.green,
                    _ => p.muted,
                };
                text(ui, q.change_label(), 16., ink);
            }
        });
        if let Some(q) = &quote {
            ui.horizontal_wrapped(|ui| {
                text(ui, format!("{} · {}", q.currency, if q.demo { "Illustrative quote" } else { "Public market feed" }), 12., p.muted);
                if q.demo {
                    pill(ui, "NOT LIVE", p.muted);
                } else {
                    text(ui, q.observation_time(), 11., p.muted);
                }
            });
        }
        if let Some(q)=quote.as_ref().filter(|q|!q.demo) {
            text(ui,format!("Retrieved {} · retained until a refresh succeeds",stamp(q.fetched)),10.5,p.muted);
            if self.prefs.show_extended {
                if let Some(e)=&q.extended {
                    let delta=e.change(q.price);
                    let change=delta.map(|d|format!("{d:+.2}")).unwrap_or("—".into());
                    let percent=e.percent(q.price).map(|d|format!("{d:+.2}%")).unwrap_or("—".into());
                    text(ui,format!("{}  {} {}  {change} ({percent}) · {}",e.session,price(e.price),q.currency,stamp(e.market_time)),12.,if delta.is_some_and(|d|d<0.){p.red}else{p.green});
                }
            }
        }
        if let Some(error) = self.market_failure().map(str::to_owned) {
            ui.add_space(8.);
            Frame::new().fill(alpha(p.red, 12)).corner_radius(7).inner_margin(10).show(ui, |ui| {
                text(ui, if quote.is_some() {
                    "Market update incomplete. Check the quote and chart dates."
                } else {
                    "Market data could not be loaded."
                }, 12., p.text);
                text(ui, error, 12., p.muted);
                let loading = self.market_loading(&sym.ticker, self.prefs.range);
                if ui.add_enabled(!loading, egui::Button::new(if loading { "Retrying…" } else { "Retry" })).clicked() {
                    self.load_market(sym.ticker.clone(), self.prefs.range, &ctx);
                    self.load_market(sym.ticker.clone(), 0, &ctx);
                }
            });
        }
        ui.add_space(20.);
        ui.horizontal(|ui|{for(label,v)in[("Overview",false),("News",true)]{if ui.add(egui::Button::new(RichText::new(label).size(13.).color(if self.news_only==v{p.text}else{p.muted})).fill(if self.news_only==v{p.raised}else{Color32::TRANSPARENT}).corner_radius(7)).clicked(){self.news_only=v;}}});
        ui.add_space(13.);
        if !self.news_only {
            Frame::new().fill(p.panel).stroke(Stroke::new(1.,p.line)).corner_radius(13).inner_margin(15).show(ui,|ui|{
                ui.horizontal_wrapped(|ui|{for(i,r)in RANGES.iter().enumerate(){let active=i==self.prefs.range;if ui.add(egui::Button::new(RichText::new(*r).size(11.).color(if active{p.blue}else{p.muted})).fill(if active{alpha(p.blue,30)}else{Color32::TRANSPARENT}).corner_radius(5).min_size(vec2(26.,25.))).clicked(){self.prefs.range=i;self.range_drag=None;self.hover_info.clear();self.save();if !self.prefs.offline{self.load_market(sym.ticker.clone(),i,&ctx);}}}});
                ui.add_space(5.);
                let info=if self.hover_info.is_empty(){"Move over the chart to inspect · Drag to compare".to_string()}else{self.hover_info.clone()};text(ui,info,11.,p.muted);
                if let Some(m) = &market {
                    self.plot(ui, m, p);
                    if !m.quote.demo && let Some(last) = m.points.last() {
                        text(ui, format!("History through {}", stamp(last.time)), 10.5, p.muted);
                    }
                } else {
                    ui.add_space(100.);
                    if self.market_loading(&sym.ticker, self.prefs.range) {
                        ui.horizontal(|ui| { ui.spinner(); text(ui, "Loading chart data…", 14., p.muted); });
                    } else {
                        text(ui, if self.prefs.offline {
                            "No sample chart is available for this ticker."
                        } else if self.market_failure().is_some() {
                            "Chart unavailable. Review the connection message above and retry."
                        } else {
                            "No chart data loaded. Use Refresh to try again."
                        }, 14., p.muted);
                    }
                    ui.add_space(100.);
                }
                ui.horizontal_wrapped(|ui|{if let Some(m)=&market{small_title(ui,if m.quote.demo{"ILLUSTRATIVE CHART"}else{"PUBLIC CHART · UTC · MAY BE DELAYED"},p);}if self.prefs.range==0 && ui.checkbox(&mut self.prefs.show_extended,RichText::new("Extended hours").size(11.)).changed(){self.range_drag=None;self.hover_info.clear();self.save();}if ui.checkbox(&mut self.prefs.show_volume,RichText::new("Volume").size(11.)).changed(){self.save();}});
            });
            ui.add_space(18.);self.details(ui,quote.as_ref(),p);ui.add_space(22.);
        }
        self.headlines(ui, p, &sym.ticker, &format!("News for {}", sym.ticker), &sym.name, 8);
        ui.add_space(8.);
    });});
    }
    fn details(&mut self, ui: &mut egui::Ui, q: Option<&Quote>, p: Palette) {
        let metrics = [
            ("Open", opt_price(q.and_then(|q| q.open))),
            ("Day high", opt_price(q.and_then(|q| q.high))),
            ("Day low", opt_price(q.and_then(|q| q.low))),
            ("Volume", compact(q.and_then(|q| q.volume))),
            ("Market cap", compact(q.and_then(|q| q.market_cap))),
            (
                "P/E ratio",
                q.and_then(|q| q.pe)
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or("—".into()),
            ),
            ("52-week high", opt_price(q.and_then(|q| q.week_high))),
            ("52-week low", opt_price(q.and_then(|q| q.week_low))),
            (
                "Average volume",
                compact(q.and_then(|q| q.details.average_volume)),
            ),
            (
                "Dividend yield",
                q.and_then(|q| q.details.dividend_yield)
                    .map(|v| format!("{:.2}%", 100. * v))
                    .unwrap_or("—".into()),
            ),
            (
                "Earnings per share",
                opt_price(q.and_then(|q| q.details.earnings_per_share)),
            ),
            (
                "Beta",
                q.and_then(|q| q.details.beta)
                    .map(|v| format!("{v:.2}"))
                    .unwrap_or("—".into()),
            ),
        ];
        let columns = if ui.available_width() > 530. { 4 } else { 2 };
        for row in metrics.chunks(columns) {
            ui.columns(columns, |cols| {
                for (col, (label, value)) in cols.iter_mut().zip(row) {
                    Frame::new()
                        .inner_margin(egui::Margin::symmetric(0, 5))
                        .show(col, |ui| {
                            text(ui, *label, 11., p.muted);
                            text(ui, value, 15., p.text);
                        });
                }
            });
        }
        if let Some(quote) = q.filter(|q| !q.demo) {
            if let Some(start) = quote.details.earnings_start {
                let date = stamp(start);
                let range = quote
                    .details
                    .earnings_end
                    .filter(|end| *end > start)
                    .map(|end| format!(" – {}", stamp(end)))
                    .unwrap_or_default();
                ui.horizontal_wrapped(|ui| {
                    text(
                        ui,
                        format!("Expected earnings: {date}{range}"),
                        12.,
                        p.muted,
                    );
                    if link(ui, "Export calendar", p).clicked() {
                        match earnings_calendar(&quote.symbol, start) {
                            Ok(calendar) => {
                                let safe_sym: String = quote
                                    .symbol
                                    .chars()
                                    .map(|c| {
                                        if c.is_ascii_alphanumeric() || c == '-' {
                                            c
                                        } else {
                                            '_'
                                        }
                                    })
                                    .collect();
                                let file = data_dir()
                                    .join("exports")
                                    .join(format!("{safe_sym}-earnings.ics"));
                                match std::fs::create_dir_all(file.parent().unwrap())
                                    .and_then(|_| std::fs::write(&file, calendar))
                                {
                                    Ok(()) => self
                                        .notify(format!("Calendar exported to {}", file.display())),
                                    Err(e) => {
                                        self.notify(format!("Could not export the calendar: {e}"))
                                    }
                                }
                            }
                            Err(e) => self.notify(e),
                        }
                    }
                });
                text(
                    ui,
                    "An estimated date from the feed; confirm it with the company.",
                    10.5,
                    p.muted,
                );
            }
        }
        if q.is_some_and(|q| q.demo) {
            text(
                ui,
                "All figures above are illustrative sample values.",
                10.5,
                p.muted,
            );
        }
    }
    fn plot(&mut self, ui: &mut egui::Ui, m: &Market, p: Palette) {
        let h = if self.prefs.show_volume { 274. } else { 244. };
        let (outer, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::drag());
        if m.points.len() < 2 {
            self.range_drag = None;
            self.hover_info.clear();
            ui.painter().text(
                outer.center(),
                Align2::CENTER_CENTER,
                "No history is available for this range",
                FontId::proportional(13.),
                p.muted,
            );
            return;
        }
        let r = Rect::from_min_max(
            outer.min + vec2(1., 15.),
            outer.max - vec2(56., if self.prefs.show_volume { 68. } else { 28. }),
        );
        if !r.is_positive() {
            return;
        }
        let low = m
            .points
            .iter()
            .map(|v| v.close)
            .fold(f64::INFINITY, f64::min);
        let high = m
            .points
            .iter()
            .map(|v| v.close)
            .fold(f64::NEG_INFINITY, f64::max);
        let low = low / high;
        let pad = ((1. - low) * 0.13).max(0.0002);
        let min = (low - pad).max(0.);
        let max = 1. + pad;
        let span = max - min;
        let y = |v: f64| r.bottom() - r.height() * ((v / high - min) / span) as f32;
        let x = |i: usize| r.left() + r.width() * i as f32 / (m.points.len() - 1) as f32;
        let index_at = |x: f32| {
            (((x - r.left()) / r.width()) * (m.points.len() - 1) as f32)
                .round()
                .clamp(0., (m.points.len() - 1) as f32) as usize
        };
        let paint = ui.painter();
        let c = if m.points.last().unwrap().close >= m.points[0].close {
            p.green
        } else {
            p.red
        };
        for i in 0..5 {
            let yy = r.top() + r.height() * i as f32 / 4.;
            paint.line_segment(
                [pos2(r.left(), yy), pos2(r.right() + 8., yy)],
                Stroke::new(0.6, p.line),
            );
            paint.text(
                pos2(outer.right() - 1., yy),
                Align2::RIGHT_CENTER,
                price(((max - span * i as f64 / 4.) * high).min(f64::MAX)),
                FontId::proportional(10.),
                p.muted,
            );
        }
        if m.range == 0
            && let Some(prev) = m.quote.previous
        {
            let yy = y(prev);
            if r.contains(pos2(r.left(), yy)) {
                let mut xx = r.left();
                while xx < r.right() {
                    paint.line_segment(
                        [pos2(xx, yy), pos2((xx + 4.).min(r.right()), yy)],
                        Stroke::new(0.7, alpha(p.muted, 100)),
                    );
                    xx += 9.;
                }
            }
        }
        let line = m
            .points
            .iter()
            .enumerate()
            .map(|(i, v)| pos2(x(i), y(v.close)))
            .collect::<Vec<_>>();
        let mut mesh = egui::Mesh::default();
        for pt in &line {
            mesh.colored_vertex(*pt, alpha(c, 40));
            mesh.colored_vertex(pos2(pt.x, r.bottom()), alpha(c, 0));
        }
        for i in 0..line.len() - 1 {
            let a = 2 * i as u32;
            mesh.add_triangle(a, a + 1, a + 2);
            mesh.add_triangle(a + 1, a + 3, a + 2);
        }
        paint.add(egui::Shape::mesh(mesh));
        paint.add(egui::Shape::line(line.clone(), Stroke::new(1.8, c)));
        let last = *line.last().unwrap();
        paint.circle_filled(last, 3., c);
        if self.prefs.show_volume {
            let vtop = r.bottom() + 12.;
            let vh = 29.;
            let vmax = m
                .points
                .iter()
                .filter_map(|v| v.volume)
                .fold(0., f64::max)
                .max(1.);
            let bw = (r.width() / m.points.len() as f32 * 0.65).clamp(0.5, 7.);
            for (i, v) in m.points.iter().enumerate() {
                let Some(volume) = v.volume else { continue };
                let bh = (volume / vmax) as f32 * vh;
                paint.rect_filled(
                    Rect::from_min_max(
                        pos2(x(i) - bw / 2., vtop + vh - bh),
                        pos2(x(i) + bw / 2., vtop + vh),
                    ),
                    1,
                    alpha(c, 55),
                );
            }
        }
        for i in 0..5 {
            let index = (m.points.len() - 1) * i / 4;
            let stamp = stamp(m.points[index].time);
            let label = if m.range == 0 {
                stamp.get(11..16).unwrap_or("").to_owned()
            } else {
                stamp.get(0..10).unwrap_or("").to_owned()
            };
            paint.text(
                pos2(x(index), outer.bottom() - 11.),
                if i == 0 {
                    Align2::LEFT_CENTER
                } else if i == 4 {
                    Align2::RIGHT_CENTER
                } else {
                    Align2::CENTER_CENTER
                },
                label,
                FontId::proportional(10.),
                p.muted,
            );
        }
        if let Some(pos) = response.hover_pos().or_else(|| {
            if response.dragged() {
                ui.ctx().input(|i| i.pointer.interact_pos())
            } else {
                None
            }
        }) {
            let idx = index_at(pos.x);
            let point = &m.points[idx];
            let px = x(idx);
            let py = y(point.close);
            paint.line_segment(
                [pos2(px, r.top()), pos2(px, r.bottom())],
                Stroke::new(0.8, p.muted),
            );
            paint.circle_filled(pos2(px, py), 4., p.text);
            paint.circle_stroke(pos2(px, py), 6., Stroke::new(1., c));
            if response.drag_started() {
                let origin = ui.ctx().input(|i| i.pointer.press_origin()).unwrap_or(pos);
                self.range_drag = Some(m.points[index_at(origin.x)].time);
            }
            let anchor = self.range_drag.and_then(|time| {
                m.points
                    .binary_search_by_key(&time, |point| point.time)
                    .ok()
            });
            if response.dragged()
                && let Some(a) = anchor
            {
                let start = &m.points[a];
                let change = point.close - start.close;
                let pct = 100. * (change / start.close);
                let rr =
                    Rect::from_min_max(pos2(x(a).min(px), r.top()), pos2(x(a).max(px), r.bottom()));
                paint.rect_filled(rr, 0, alpha(p.blue, 20));
                let percent = if pct.is_finite() {
                    format!("{pct:+.2}%")
                } else {
                    "percentage unavailable".into()
                };
                self.hover_info = format!(
                    "{} → {}   {change:+.2} ({percent})",
                    stamp(start.time),
                    stamp(point.time)
                );
            } else {
                self.range_drag = None;
                self.hover_info = format!(
                    "{}   ·   {} {}",
                    stamp(point.time),
                    price(point.close),
                    m.quote.currency
                );
            }
        } else {
            self.hover_info.clear();
        }
        if response.drag_stopped() {
            self.range_drag = None;
        }
    }
    fn overview(&mut self, ui: &mut egui::Ui, p: Palette, ctx: &egui::Context) {
        ui.label(RichText::new("Business news").size(32.).strong());
        ui.add_space(22.);
        ui.columns(3, |cols| {
            for (col, t) in cols.iter_mut().zip(MARKET_INDEXES) {
                let q = self.quote(t);
                let response = Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1., p.line))
                    .corner_radius(10)
                    .inner_margin(12)
                    .show(col, |ui| {
                        text(
                            ui,
                            match t {
                                "^GSPC" => "S&P 500",
                                "^DJI" => "Dow Jones",
                                _ => "NASDAQ",
                            },
                            12.,
                            p.muted,
                        );
                        text(
                            ui,
                            q.as_ref().map(|q| price(q.price)).unwrap_or("—".into()),
                            20.,
                            p.text,
                        );
                        text(
                            ui,
                            q.as_ref()
                                .and_then(|q| q.percent())
                                .map(|v| format!("{v:+.2}%"))
                                .unwrap_or("—".into()),
                            12.,
                            match q.as_ref().and_then(Quote::percent) {
                                Some(change) if change < 0. => p.red,
                                Some(change) if change > 0. => p.green,
                                _ => p.muted,
                            },
                        );
                    })
                    .response
                    .interact(Sense::click());
                if response.clicked() {
                    self.choose(t, ctx);
                }
            }
        });
        ui.add_space(12.);
        text(
            ui,
            if self.prefs.offline {
                "Sample index values"
            } else {
                "Public index quotes may be delayed"
            },
            11.,
            p.muted,
        );
        ui.add_space(18.);
        self.headlines(ui, p, "__market", "Top stories", "the market", 12);
    }

    fn headlines(
        &mut self,
        ui: &mut egui::Ui,
        p: Palette,
        ticker: &str,
        title: &str,
        sample_name: &str,
        limit: usize,
    ) {
        let sample = self.prefs.offline;
        let loading = !sample && self.news_inflight.contains(ticker);
        ui.horizontal(|ui| {
            ui.label(display_text(title, 20., p.text));
            if !sample {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if loading {
                        ui.spinner().on_hover_text("Refreshing headlines");
                    } else if link(ui, "Refresh news", p).clicked() {
                        self.load_news(ticker.into(), ui.ctx());
                    }
                });
            }
        });
        ui.add_space(5.);
        let articles = if sample {
            text(ui, "Sample reading cards", 11., p.muted);
            demo_news(sample_name)
        } else {
            let articles = self.news.get(ticker);
            if let Some(err) = self.failures.get(&format!("news:{ticker}")) {
                let label = if articles.is_some_and(|items| !items.is_empty()) {
                    "News refresh failed. Showing saved headlines."
                } else {
                    "Headlines could not be loaded."
                };
                text(ui, label, 12., p.red);
                text(ui, err, 11., p.muted);
            } else if loading {
                text(ui, "Refreshing headlines…", 12., p.muted);
            } else if articles.is_none_or(Vec::is_empty) {
                text(
                    ui,
                    if articles.is_some() {
                        "The feed returned no stories."
                    } else {
                        "Use Refresh news to load headlines."
                    },
                    12.,
                    p.muted,
                );
            }
            if let Some(fetched) = self.news_fetched.get(ticker) {
                text(
                    ui,
                    format!("Last retrieved {}", stamp(*fetched)),
                    11.,
                    p.muted,
                );
            }
            articles.cloned().unwrap_or_default()
        };
        for (i, article) in articles.into_iter().enumerate().take(limit) {
            self.news_card(ui, article, i, p);
        }
    }
    fn news_card(&mut self, ui: &mut egui::Ui, a: Article, index: usize, p: Palette) {
        ui.add_space(9.);
        let mut open_story = false;
        Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1., p.line))
            .corner_radius(11)
            .inner_margin(15)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 97.).max(140.);
                    ui.allocate_ui_with_layout(vec2(w, 85.), Layout::top_down(Align::LEFT), |ui| {
                        text(
                            ui,
                            if a.demo {
                                "TENERUS GUIDE · SAMPLE".into()
                            } else {
                                a.publisher.to_uppercase()
                            },
                            10.5,
                            if a.demo { p.muted } else { p.blue },
                        );
                        open_story |= ui
                            .add(
                                egui::Label::new(RichText::new(&a.title).size(17.).strong())
                                    .sense(Sense::click()),
                            )
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked();
                        text(
                            ui,
                            if a.demo {
                                "A practical reading guide".into()
                            } else {
                                a.date.clone()
                            },
                            10.5,
                            p.muted,
                        );
                    });
                    let (r, image_response) =
                        ui.allocate_exact_size(vec2(80., 75.), Sense::click());
                    open_story |= image_response.clicked();
                    let c = [p.blue, p.green, hex(0xb59cff)][index % 3];
                    ui.painter().rect_filled(r, 8, alpha(c, 21));
                    for i in 0..6 {
                        let h = 12. + ((i * 7 + index * 11) % 38) as f32;
                        let x = r.left() + 12. + i as f32 * 10.;
                        ui.painter().rect_filled(
                            Rect::from_min_size(pos2(x, r.bottom() - 12. - h), vec2(5., h)),
                            2,
                            alpha(c, 90 + i as u8 * 18),
                        );
                    }
                });
                if !a.demo && a.safe_link().is_some() {
                    let saved = self.prefs.saved_articles.iter().any(|s| s.link == a.link);
                    ui.horizontal(|ui| {
                        if link(ui, if saved { "Unsave story" } else { "Save story" }, p).clicked()
                        {
                            self.toggle_saved(&a);
                        }
                        if link(ui, "Copy link", p).clicked() {
                            ui.ctx().copy_text(a.link.clone());
                            self.notify("Story link copied");
                        }
                    });
                }
            });
        if open_story {
            if a.demo {
                self.article = Some(a);
            } else if let Some(url) = a.safe_link() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url.to_string()));
            } else {
                self.notify("This story did not include a web link.");
            }
        }
    }
    fn toggle_saved(&mut self, article: &Article) {
        if article.demo || article.safe_link().is_none() {
            return;
        }
        if let Some(i) = self
            .prefs
            .saved_articles
            .iter()
            .position(|a| a.link == article.link)
        {
            self.prefs.saved_articles.remove(i);
        } else if self.prefs.saved_articles.len() < 200 {
            self.prefs.saved_articles.insert(0, article.clone());
        } else {
            self.notify("You have 200 saved stories. Unsave one to add another.");
        }
        self.save();
    }
    fn saved_stories(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.label(RichText::new("Saved stories").size(32.).strong());
        text(
            ui,
            "Your saved headlines and publisher links are kept locally on this device.",
            13.,
            p.muted,
        );
        text(
            ui,
            "Articles open at their publisher and may require a subscription.",
            11.,
            p.muted,
        );
        ui.add_space(18.);
        if self.prefs.saved_articles.is_empty() {
            text(
                ui,
                "Choose Save story on a news card to keep it here.",
                15.,
                p.muted,
            );
        }
        for (i, a) in self.prefs.saved_articles.clone().into_iter().enumerate() {
            self.news_card(ui, a, i, p);
        }
    }
    fn export_chart(&mut self, m: &Market) {
        let safe_sym: String = m
            .quote
            .symbol
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let file = data_dir().join("exports").join(format!(
            "{}-{}-{}.csv",
            safe_sym,
            RANGES[m.range],
            now()
        ));
        let mut csv = format!(
            "# {} data; source={}, demo={}\ntime_utc,close,volume\n",
            m.quote.symbol,
            if m.quote.demo {
                "illustrative"
            } else {
                "Yahoo public chart"
            },
            m.quote.demo
        );
        for p in &m.points {
            let volume = p.volume.map(|v| v.to_string()).unwrap_or_default();
            csv.push_str(&format!("{},{},{}\n", stamp(p.time), p.close, volume));
        }
        let result = std::fs::create_dir_all(file.parent().unwrap())
            .and_then(|_| std::fs::write(&file, csv));
        match result {
            Ok(()) => self.notify(format!("Chart exported to {}", file.display())),
            Err(e) => self.notify(format!("Could not export chart: {e}")),
        }
    }
}

impl MarketApp {
    fn model_comparisons(&self, ui: &mut egui::Ui, p: Palette, selected_only: bool) {
        let provider = &self.prefs.providers[self.prefs.active_provider];
        let scope = if selected_only {
            self.prefs.selected.as_str()
        } else {
            "All tickers"
        };
        text(
            ui,
            format!(
                "{scope} · {} / {} · All reviewed history",
                provider.name,
                if provider.model.is_empty() {
                    "No model selected"
                } else {
                    provider.model_label()
                }
            ),
            12.,
            p.text,
        );
        let scores = self.journal.benchmarks(
            provider,
            selected_only.then_some(self.prefs.selected.as_str()),
            now(),
        );
        let reviewed = scores.iter().map(|s| s.reviewed).sum::<usize>();
        if reviewed == 0 {
            text(
                ui,
                "No comparable outcomes yet. Rates appear after real predictions reach their review dates and a closing price is checked.",
                12.,
                p.blue,
            );
        } else {
            egui::ScrollArea::horizontal()
                .id_salt(("model_scores", selected_only))
                .show(ui, |ui| {
                    egui::Grid::new(("model_scores_grid", selected_only))
                        .striped(true)
                        .spacing(vec2(18., 8.))
                        .show(ui, |ui| {
                            for heading in [
                                "Review",
                                "Calls",
                                "AI",
                                "Always BUY",
                                "Always SELL",
                                "Always WAIT",
                            ] {
                                text(ui, heading, 11., p.muted);
                            }
                            ui.end_row();
                            for (i, score) in scores.iter().enumerate() {
                                text(ui, REVIEW_LABELS[i], 12., p.text);
                                text(ui, score.reviewed.to_string(), 12., p.text);
                                for correct in [
                                    score.correct,
                                    score.always_buy,
                                    score.always_sell,
                                    score.always_wait,
                                ] {
                                    text(ui, score.rate(correct), 12., p.text);
                                }
                                ui.end_row();
                            }
                        });
                });
        }
        let repeated = scores.iter().map(|s| s.repeated).sum::<usize>();
        let cal = self.journal.calibration(
            provider,
            selected_only.then_some(self.prefs.selected.as_str()),
            now(),
        );
        if cal.reviewed > 0 {
            ui.add_space(6.);
            let mut stats = Vec::new();
            if let Some(brier) = cal.brier_score {
                stats.push(format!("Brier {brier:.3}"));
            }
            if let Some(ece) = cal.expected_calibration_error {
                stats.push(format!("Calibration error (ECE) {:.1}%", ece * 100.));
            }
            if let Some(r) = cal.mean_buy_return {
                let alpha = cal
                    .mean_buy_alpha
                    .map(|a| format!(" (α {a:+.1}% vs ^GSPC)"))
                    .unwrap_or_default();
                stats.push(format!("Avg BUY {r:+.1}%{alpha}"));
            }
            if let Some(r) = cal.mean_sell_return {
                let alpha = cal
                    .mean_sell_alpha
                    .map(|a| format!(" (α {a:+.1}% vs ^GSPC)"))
                    .unwrap_or_default();
                stats.push(format!("Avg SELL {r:+.1}%{alpha}"));
            }
            if let Some(r) = cal.mean_wait_return {
                let alpha = cal
                    .mean_wait_alpha
                    .map(|a| format!(" (α {a:+.1}% vs ^GSPC)"))
                    .unwrap_or_default();
                stats.push(format!("Avg WAIT {r:+.1}%{alpha}"));
            }
            if !stats.is_empty() {
                text(ui, stats.join("  ·  "), 11.5, p.text);
            }
            if !cal.buckets.is_empty() {
                let buckets_line = cal
                    .buckets
                    .iter()
                    .map(|b| {
                        format!(
                            "{}: {:.0}% stated → {:.0}% actual (n={})",
                            b.label, b.avg_confidence_percent, b.actual_accuracy_percent, b.count
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("  |  ");
                text(
                    ui,
                    format!("Confidence calibration · {buckets_line}"),
                    11.,
                    p.muted,
                );
            }
        }
        text(
            ui,
            format!(
                "Same calls, dates and ±2% price-direction rules for every column. WAIT means DO NOT BUY. First reviewed call per ticker, horizon and UTC day; {repeated} repeated calls excluded. Other providers, models and endpoints are kept separate."
            ),
            11.,
            p.muted,
        );
        text(
            ui,
            "Review periods can overlap. These descriptive rates exclude dividends and costs; they are neither trading returns nor proof of predictive skill. They do not change the learning-memory window.",
            11.,
            p.muted,
        );
    }
    fn model_guide(&mut self, ctx: &egui::Context, p: Palette) {
        if !self.model_guide_open {
            return;
        }
        let mut open = true;
        let mut show_logbook = false;
        let mut show_connections = false;
        dialog_window(ctx, "Forecast model guide", vec2(760., 580.))
            .id(Id::unique("forecast_model_guide"))
            .resizable(true).collapsible(false).open(&mut open)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().id_salt("model_guide_scroll").auto_shrink([false, false]).show(ui, |ui| {
                    text(ui, "One shared forecaster. Evaluate every ticker.", 23., p.text);
                    text(ui, "Training a model for each ticker is feasible. Our recommendation is to test a pretrained forecasting model first, then specialize only where fresh, unseen results justify the extra work.", 13., p.text);
                    ui.add_space(8.);
                    ui.horizontal_wrapped(|ui| {
                        show_logbook = button(ui, "Open prediction logbook", p).clicked();
                        show_connections = button(ui, "AI connections", p).clicked();
                        if ui.add_enabled(!self.local_model_check_busy, egui::Button::new("Check this device")).clicked() {
                            self.local_model_check_busy = true;
                            let tx = self.tx.clone();
                            let ctx = ctx.clone();
                            std::thread::spawn(move || {
                                let _ = tx.send(Event::LocalModelCheck(crate::forecast::check()));
                                ctx.request_repaint();
                            });
                        }
                        let selected_ticker = self.prefs.selected.clone();
                        let pilot_horizon = self.desk.horizon;
                        let pilot_history = self.desk.history(&selected_ticker, self.prefs.offline);
                        egui::ComboBox::from_id_salt("foundation_backend_select")
                            .width(142.)
                            .selected_text(self.local_forecast_backend.label())
                            .show_ui(ui, |ui| {
                                for backend in crate::forecast::FoundationBackend::ALL {
                                    ui.selectable_value(
                                        &mut self.local_forecast_backend,
                                        backend,
                                        backend.label(),
                                    );
                                }
                            });
                        let backend = self.local_forecast_backend;
                        let can_pilot = !self.local_forecast_busy && !self.transient && pilot_history.is_some();
                        if ui.add_enabled(can_pilot, egui::Button::new(format!("Run {} pilot ({selected_ticker}, {pilot_horizon}d)", backend.short_label()))).clicked()
                            && let Some(history) = pilot_history
                        {
                            let regime = self.desk.market_regime(self.prefs.offline);
                            let sent = self.desk.sentiment.series(&selected_ticker);
                            self.local_forecast_busy = true;
                            let tx = self.tx.clone();
                            let ctx = ctx.clone();
                            std::thread::spawn(move || {
                                let _ = tx.send(Event::LocalForecast(crate::forecast::run_foundation_pilot(
                                    &history,
                                    pilot_horizon,
                                    backend,
                                    regime.as_deref(),
                                    sent.as_ref(),
                                )));
                                ctx.request_repaint();
                            });
                        }
                    });
                    ui.add_space(12.);
                    if self.local_forecast_busy {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            text(
                                ui,
                                format!(
                                    "Running opt-in local {} walk-forward conformal pilot ({} sessions)…",
                                    self.local_forecast_backend.short_label(),
                                    self.desk.horizon
                                ),
                                12.,
                                p.muted,
                            );
                        });
                        ui.add_space(6.);
                    } else if let Some(result) = &self.local_forecast {
                        Frame::new().fill(p.panel).stroke(Stroke::new(1., p.line)).corner_radius(9).inner_margin(10).show(ui, |ui| {
                            match result {
                                Ok(f) => {
                                    let sb = f.scoreboard;
                                    text(
                                        ui,
                                        format!(
                                            "Local {} pilot · {} (+{} sessions · runtime {} · {} ms · overlaid on Outlook chart)",
                                            f.backend.label(),
                                            f.ticker,
                                            f.horizon,
                                            f.runtime,
                                            f.elapsed_ms
                                        ),
                                        13.,
                                        p.green,
                                    );
                                    text(
                                        ui,
                                        format!(
                                            "Calibrated 10th/50th/90th: {:+.1}% / {:+.1}% / {:+.1}%  (raw {:+.1}%..{:+.1}%, conformal {:+.1} pp, anchor {})",
                                            f.band.low * 100.,
                                            f.band.mid * 100.,
                                            f.band.high * 100.,
                                            f.raw_band.low * 100.,
                                            f.raw_band.high * 100.,
                                            sb.conformal_delta * 100.,
                                            price(f.anchor_price),
                                        ),
                                        12.,
                                        p.text,
                                    );
                                    if sb.trials > 0 {
                                        ui.add_space(6.);
                                        egui::Grid::new("foundation_scoreboard_grid")
                                            .striped(true)
                                            .spacing(vec2(16., 6.))
                                            .show(ui, |ui| {
                                                for h in ["Model (same dates)", "80% Coverage", "Pinball Loss", "Median MAE"] {
                                                    text(ui, h, 11., p.muted);
                                                }
                                                ui.end_row();
                                                text(ui, format!("{} (calibrated)", f.backend.short_label()), 11.5, p.text);
                                                text(ui, format!("{}/{} (raw {})", sb.fm_cal_covered, sb.trials, sb.fm_raw_covered), 11.5, p.text);
                                                text(ui, format!("{:.2} pp", sb.fm_pinball * 100.), 11.5, p.text);
                                                text(ui, format!("{:.1} pp", sb.fm_mae * 100.), 11.5, p.text);
                                                ui.end_row();
                                                if sb.outlook_matched_trials > 0 {
                                                    text(ui, "Outlook 4-Model Ensemble", 11.5, p.text);
                                                    text(ui, format!("{}/{}", sb.outlook_covered, sb.outlook_matched_trials), 11.5, p.text);
                                                    text(ui, format!("{:.2} pp", sb.outlook_pinball * 100.), 11.5, p.text);
                                                    text(ui, format!("{:.1} pp", sb.outlook_mae * 100.), 11.5, p.text);
                                                    ui.end_row();
                                                    text(ui, "Unconditional Baseline", 11.5, p.muted);
                                                    text(ui, format!("{}/{}", sb.baseline_covered, sb.outlook_matched_trials), 11.5, p.muted);
                                                    text(ui, format!("{:.2} pp", sb.baseline_pinball * 100.), 11.5, p.muted);
                                                    text(ui, format!("{:.1} pp", sb.baseline_mae * 100.), 11.5, p.muted);
                                                    ui.end_row();
                                                }
                                            });
                                        if let Some(skill) = sb.skill_vs_outlook() {
                                            text(
                                                ui,
                                                format!(
                                                    "{} vs Outlook 4-Model Ensemble: {:+.1}% pinball skill across {} matched walk-forward dates.",
                                                    f.backend.short_label(),
                                                    skill * 100.,
                                                    sb.outlook_matched_trials
                                                ),
                                                11.,
                                                if skill > 0. { p.green } else { p.muted },
                                            );
                                        }
                                    }
                                }
                                Err(err) => {
                                    text(ui, err, 12., p.red);
                                }
                            }
                        });
                        ui.add_space(8.);
                    }
                    Frame::new().fill(alpha(p.blue, 12)).corner_radius(9).inner_margin(12).show(ui, |ui| {
                        small_title(ui, "RECOMMENDED START", p);
                        text(ui, "TimesFM 3.0 & Chronos-2 · Opt-in local foundation pilots", 17., p.text);
                        text(ui, format!("Models: {} (runtime {}) · {} (chronos-forecasting)", crate::forecast::MODEL, crate::forecast::RUNTIME, crate::forecast::CHRONOS_MODEL), 11., p.muted);
                        text(ui, "Choose either TimesFM (Google's MLX/PyTorch backend) or Chronos-2 (Amazon's Apache-2.0 quantile forecaster via chronos-forecasting) above. Each pilot runs batched non-overlapping walk-forward cutoffs + split-conformal calibration in one model load and compares directly against Outlook's 4-model ensemble.", 12., p.muted);
                        text(ui, crate::forecast::PILOT, 12., p.text);
                        text(ui, "Start with short forecasts and a small batch. The weights download is about 1.32 GB; allow at least 4 GB free for setup. A 16 GB memory budget is our conservative pilot recommendation, not a measured requirement or speed guarantee. Six-month and one-year forecasts need separate validation.", 12., p.muted);
                        text(ui, "Downloaded 3.0 weights permit non-commercial, non-production research only, including limits on using results for commercial decisions. For production, Google offers an authorized Cloud route. TimesFM 2.5 has Apache-2.0 weights but is an older alternative, not the latest model.", 12., p.muted);
                    });
                    ui.add_space(8.);
                    if self.local_model_check_busy {
                        ui.horizontal(|ui| { ui.spinner(); text(ui, "Checking local hardware and installed packages…", 12., p.muted); });
                    } else if let Some(report) = &self.local_model_check {
                        text(ui, report.hardware_status(), 14., p.blue);
                        text(ui, report.hardware_detail(), 12., p.text);
                        text(ui, report.runtime_status(), 12., p.muted);
                    } else {
                        text(ui, "Local compatibility · Check required", 12., p.blue);
                    }
                    text(ui, "This check reads local system information and package versions only. It does not install packages, download weights or run a forecast.", 11., p.muted);
                    ui.add_space(12.);
                    small_title(ui, "WHERE EACH APPROACH FITS", p);
                    text(ui, "Per-ticker training · Possible, but start small", 15., p.text);
                    text(ui, "One ticker supplies limited independent history, especially for six-month and one-year targets. Large separate models can memorize that history and need repeated maintenance. Compare a simple model trained across tickers before attempting per-ticker fine-tuning or calibration.", 12., p.muted);
                    ui.add_space(7.);
                    text(ui, "Gemini · Research and explanations", 15., p.text);
                    text(ui, "The existing Google connection sends quotes, a limited chart excerpt and headlines to Gemini for an explanation and decision. It is a separate workflow from TimesFM. Keep explanations and numeric forecast measurements separate.", 12., p.muted);
                    ui.add_space(7.);
                    text(ui, "Test on the future, in date order", 15., p.text);
                    text(ui, "Reserve later dates for testing. Train or tune only on earlier data, leave a gap for overlapping target windows, and use only news and fundamentals known at each forecast date. Compare against an unchanged-price forecast and a simple shared model, then evaluate simulated returns after fees and spreads. Report uncertainty by ticker and horizon.", 12., p.muted);
                    ui.add_space(12.); ui.separator(); ui.add_space(10.);
                    small_title(ui, "WHAT RUNS IN TENERUS TODAY", p);
                    text(ui, "Self-calibrating Outlook ensemble + Benchmark-Alpha Logbook + Opt-in TimesFM / Chronos-2 bridge", 14., p.text);
                    text(ui, "Outlook · 4-model GARCH(1,1) + S&P 500 regime conformal ensemble    /    Local TimesFM & Chronos-2 · Opt-in conformal pilot", 12., p.blue);
                    text(ui, "Outlook runs four strictly causal local models (kernel-weighted multivariable analogs with S&P 500 regime, 10-feature online L2 quantile regression with causal news history ablation, mean-reverting GARCH(1,1) volatility, and baseline) blended by out-of-sample pinball loss with split-conformal calibration. The Prediction Logbook tracks Brier/ECE calibration and excess return vs. ^GSPC (prompt context only, never LLM weight retraining). Opt-in local TimesFM and Chronos-2 pilots run batched walk-forward conformal backtests scored apples-to-apples against Outlook.", 12., p.muted);
                    ui.add_space(10.);
                    self.model_comparisons(ui, p, true);
                    ui.add_space(12.);
                    egui::CollapsingHeader::new("Sources · checked 12 Nov 2026").show(ui, |ui| {
                        ui.hyperlink_to("Google Research · TimesFM-3", "https://research.google/blog/timesfm-3-a-zero-shot-foundation-model-for-multivariate-forecasting/");
                        ui.hyperlink_to("Google's TimesFM repository · Models and licenses", "https://github.com/google-research/timesfm");
                        ui.hyperlink_to("Amazon Science · Chronos-2 forecasting repository", "https://github.com/amazon-science/chronos-forecasting");
                        ui.hyperlink_to("Official Apple Silicon backend", "https://github.com/google-research/timesfm#apple-silicon-mlx-backend");
                        ui.hyperlink_to("Runtime release · TimesFM 3.0.2", "https://pypi.org/project/timesfm/3.0.2/");
                        ui.hyperlink_to("Apple MLX · System requirements", "https://ml-explore.github.io/mlx/build/html/install.html");
                        ui.hyperlink_to("Financial-return forecasting study · Five equities, June 2026 preprint", "https://arxiv.org/abs/2606.27100");
                        text(ui, "The small financial study found limited gains over a random walk. It evaluates older models, including TimesFM 2.5, and does not validate TimesFM 3.0 or this app.", 11., p.muted);
                    });
                });
            });
        self.model_guide_open = open && !show_logbook && !show_connections;
        if show_logbook {
            self.journal_open = true;
        }
        if show_connections {
            self.open_editor();
        }
    }
    fn logbook(&mut self, ctx: &egui::Context, p: Palette) {
        if !self.journal_open {
            return;
        }
        let mut open = true;
        dialog_window(ctx, "Prediction logbook", vec2(780., 580.))
            .id(Id::unique("prediction_logbook"))
            .resizable(true)
            .collapsible(false)
            .open(&mut open)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().id_salt("logbook_scroll").auto_shrink([false, false]).show(ui, |ui| {
                    text(ui, "Every call. What happened next.", 22., p.text);
                    text(ui, "New forecasts are saved automatically. Earlier calls stay here when you generate again.", 12., p.muted);
                    ui.add_space(10.);
                    let before = self.prefs.memory_days;
                    ui.horizontal_wrapped(|ui| {
                        text(ui, "AI learning memory", 13., p.text);
                        egui::ComboBox::from_id_salt("learning_window")
                            .selected_text(memory_label(self.prefs.memory_days))
                            .show_ui(ui, |ui| {
                                for days in [180, 90, 365, 0] {
                                    ui.selectable_value(&mut self.prefs.memory_days, days, memory_label(days));
                                }
                            });
                    });
                    if before != self.prefs.memory_days { self.save(); }
                    let provider = &self.prefs.providers[self.prefs.active_provider];
                    let memory = self.journal.learning_context(provider, self.prefs.memory_days, now());
                    text(ui, format!("For {} / {}", provider.name, provider.model_label()), 11., p.muted);
                    text(ui, "Future analyses receive up to 24 recent reviewed calls from this connection, including mistakes and original reasons. The model is not retrained. Older entries stay in this logbook.", 12., p.muted);
                    text(ui, "The memory window follows outcome dates. Changing it affects future requests.", 11., p.muted);
                    ui.add_space(7.);
                    if self.prefs.memory_days == 0 {
                        text(ui, "Memory is off. Predictions and price checks still save.", 12., p.blue);
                    } else if memory.reviewed == 0 {
                        text(ui, "No reviewed results in this window yet. Learning starts after a real outcome is checked.", 12., p.blue);
                    } else {
                        text(ui, format!("Measured results · {} correct / {} reviewed ({:.0}%)", memory.correct, memory.reviewed, 100. * memory.correct as f64 / memory.reviewed as f64), 15., p.text);
                        text(ui, "This simple price-direction score is separate from AI confidence. Repeated calls can be correlated.", 11., p.muted);
                    }
                    ui.add_space(8.);
                    ui.horizontal_wrapped(|ui| {
                        if ui.add_enabled(!self.reviewing && !self.prefs.offline && !self.transient, egui::Button::new("Check outcomes")).clicked() { self.launch_reviews(ctx); }
                        if self.reviewing { ui.spinner(); text(ui, "Checking historical closes…", 11., p.muted); }
                        if ui.add_enabled(!self.transient, egui::Button::new("Export logbook")).clicked() {
                            let path = data_dir().join("exports").join(format!("predictions-{}.json", now()));
                            match self.journal.save(&path) {
                                Ok(()) => self.notify(format!("Logbook exported to {}", path.display())),
                                Err(e) => self.notify(format!("Could not export logbook: {e}")),
                            }
                        }
                        ui.checkbox(&mut self.journal_selected_only, format!("Only {}", self.prefs.selected));
                    });
                    text(ui, if self.prefs.offline { "Offline demonstration: outcome checks are paused." } else { "Checks run while the app is open and online, and catch up on your next launch." }, 11., p.muted);
                    if let Some(error) = &self.journal_load_error { text(ui, error, 12., p.red); text(ui, "New calls are session-only until the saved logbook can be read; its file will not be overwritten.", 11., p.red); }
                    if let Some(error) = &self.journal_error.clone() {
                        text(ui, format!("The logbook has unsaved changes: {error}"), 12., p.red);
                        if button(ui, "Retry saving logbook", p).clicked() { self.save_journal(); }
                    }
                    for (symbol, error) in &self.review_errors { text(ui, format!("{symbol}: {error} · Previous results kept."), 11., p.red); }
                    egui::CollapsingHeader::new("How results are checked").show(ui, |ui| {
                        text(ui, "Review checkpoints: 4 weeks, 6 months and 1 year from generation. The long-term score is a first-year checkpoint, not a full three-year verdict.", 11., p.muted);
                        text(ui, "Use the first completed daily session starting on or after the checkpoint, within 7 days. Future dates, missing data and sample inputs remain unscored. Reference quotes older than 7 days are excluded.", 11., p.muted);
                        text(ui, "BUY is correct above +2%; SELL below −2%; DO NOT BUY at or below +2%. Prices are adjusted for reported splits. This is a price-only comparison, excluding dividends, fees and actual trading returns. Feed revisions can affect results.", 11., p.muted);
                        text(ui, "Original decisions, confidence, dates and reference prices stay unchanged; a completed review is saved once. Legacy analysis without an explicit action cannot be reconstructed as a past prediction.", 11., p.muted);
                    });
                    egui::CollapsingHeader::new("Model comparisons").show(ui, |ui| {
                        self.model_comparisons(ui, p, self.journal_selected_only);
                        if link(ui, "Forecast model guide", p).clicked() { self.model_guide_open = true; }
                    });
                    ui.add_space(8.); ui.separator(); ui.add_space(8.);
                    let matching = self.journal.entries.iter().filter(|e| !self.journal_selected_only || e.symbol == self.prefs.selected).count();
                    let entries = self.journal.entries.iter().rev().filter(|e| !self.journal_selected_only || e.symbol == self.prefs.selected).take(self.journal_limit).cloned().collect::<Vec<_>>();
                    if entries.is_empty() {
                        text(ui, "No predictions saved yet", 17., p.text);
                        text(ui, "Generate an outlook with your connected AI. Each action, confidence %, reason, price and review date will appear here. Past outcomes will appear after their review dates.", 13., p.muted);
                    }
                    let checked_at = now();
                    for e in &entries {
                        Frame::new().fill(p.panel).stroke(Stroke::new(1., p.line)).corner_radius(9).inner_margin(12).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                text(ui, format!("{} · {}", e.symbol, HORIZONS[e.horizon]), 14., p.text);
                                pill(ui, e.decision.label(), match e.decision { Decision::Buy => p.green, Decision::Sell => p.red, _ => p.blue });
                                text(ui, e.confidence_percent.filter(|v| v.is_finite() && (0.0..=100.0).contains(v)).map(|v| format!("AI confidence {v:.0}%")).unwrap_or("Confidence % not supplied".into()), 11., p.muted);
                            });
                            text(ui, format!("Made {} · Review {}", stamp(e.generated), stamp(e.review_at)), 10.5, p.muted);
                            let verified = e.verified_outcome(checked_at);
                            text(ui, e.status(checked_at), 13., verified.map(|o| if o.correct { p.green } else { p.red }).unwrap_or(p.blue));
                            if let Some(outcome) = verified {
                                let bench_text = match (
                                    outcome.benchmark_change_percent,
                                    outcome.excess_return_percent,
                                ) {
                                    (Some(b), Some(a)) => {
                                        format!(" · ^GSPC {b:+.2}% · Alpha {a:+.2}%")
                                    }
                                    _ => String::new(),
                                };
                                text(ui, format!("Price change {:+.2}%{bench_text} · {} {} → {} {}", outcome.change_percent, price(outcome.comparison_reference_price), e.currency, price(outcome.price), e.currency), 12., p.text);
                                if e.reference_price.is_some_and(|v| (v - outcome.comparison_reference_price).abs() > 0.000001) { text(ui, "Comparison prices adjusted for reported stock splits.", 10.5, p.muted); }
                                text(ui, format!("Closing session {} · Checked {}", stamp(outcome.price_time), stamp(outcome.checked_at)), 10.5, p.muted);
                            }
                            egui::CollapsingHeader::new("Original reason & source").id_salt(("prediction_reason", e.id)).show(ui, |ui| {
                                text(ui, &e.reason, 12., p.text);
                                for risk in &e.risks { text(ui, format!("Risk · {risk}"), 11., p.muted); }
                                text(ui, format!("{} / {} · {}", e.provider, e.model, e.endpoint), 10.5, p.muted);
                                text(ui, format!("Reference {} {} · Quote {}", opt_price(e.reference_price), e.currency, stamp(e.quote_time)), 10.5, p.muted);
                            });
                        });
                        ui.add_space(7.);
                    }
                    if matching > entries.len() && button(ui, "Show 100 more calls", p).clicked() { self.journal_limit += 100; }
                });
            });
        self.journal_open = open;
    }
    fn ai_panel(&mut self, root: &mut egui::Ui, p: Palette) {
        let ctx = root.ctx().clone();
        let max_width = (root.available_width() - 245.).clamp(285., 440.);
        let panel=egui::Panel::right("ai_outlook").default_size(self.prefs.ai_width.min(max_width)).size_range(285.0..=max_width).resizable(true).frame(Frame::new().fill(p.side).inner_margin(egui::Margin::symmetric(19,23))).show(root,|ui|{egui::ScrollArea::vertical().id_salt("ai_scroll").auto_shrink([false,false]).show(ui,|ui|{
        ui.horizontal(|ui|{ui.label(RichText::new("AI outlook").size(23.).strong());ui.with_layout(Layout::right_to_left(Align::Center),|ui|{pill(ui,"RESEARCH",p.blue);});});
        text(ui,"Buy, sell or wait, with a clear confidence %.",12.,p.muted);ui.add_space(7.);
        ui.horizontal(|ui|{if link(ui,"Prediction logbook",p).clicked(){self.journal_open=true;}text(ui,format!("{} saved calls",self.journal.entries.len()),10.5,p.muted);});
        if !self.prefs.offline {
            let memory=self.journal.learning_context(&self.prefs.providers[self.prefs.active_provider],self.prefs.memory_days,now());
            text(ui,if self.prefs.memory_days==0{"Learning memory off".into()}else{format!("Learning · {} · {} reviewed",memory_label(self.prefs.memory_days),memory.reviewed)},11.,p.muted);
        }
        if link(ui,"Forecast model guide",p).clicked(){self.model_guide_open=true;}
        if self.journal_load_error.is_some()||self.journal_error.is_some(){text(ui,"Logbook needs attention. Open it for details.",11.,p.red);}
        if self.analyses_load_error.is_some() {
            text(ui, "Saved research could not be read. New research stays in this session.", 11., p.red);
        }
        ui.add_space(12.);
        let idx=self.prefs.active_provider;let provider=self.prefs.providers[idx].clone();
        Frame::new().fill(p.panel).stroke(Stroke::new(1.,p.line)).corner_radius(10).inner_margin(12).show(ui,|ui|{
            ui.horizontal(|ui|{small_title(ui,"CONNECTION",p);ui.with_layout(Layout::right_to_left(Align::Center),|ui|{if link(ui,"Configure",p).clicked(){self.open_editor();}});});
            egui::ComboBox::from_id_salt("active_provider").selected_text(&provider.name).width(ui.available_width()).show_ui(ui,|ui|{for(i,p)in self.prefs.providers.iter().enumerate(){ui.selectable_value(&mut self.prefs.active_provider,i,&p.name);}});
            if self.prefs.active_provider!=idx{self.save();}
            let selected=&self.prefs.providers[self.prefs.active_provider];text(ui,if selected.model.is_empty(){"Choose a model in connections".into()}else{selected.model_label().to_owned()},11.,p.muted);
            text(ui,if self.verified[self.prefs.active_provider]{"●  Inference verified this session"}else{"○  Connection not yet verified"},10.5,if self.verified[self.prefs.active_provider]{p.green}else{p.muted});
            if let Some(usage)=&self.last_usage[self.prefs.active_provider]{
                ui.label(RichText::new(format!("Last request · {}",usage.cost_label())).size(10.5).color(p.muted)).on_hover_text(usage.detail());
            }else{text(ui,"Inference cost updates after each request",10.5,p.muted);}
        });
        ui.add_space(16.);ui.horizontal(|ui|{small_title(ui,"RESEARCH SCOPE",p);ui.with_layout(Layout::right_to_left(Align::Center),|ui|{pill(ui,&self.prefs.selected,p.blue);});});
        egui::ComboBox::from_id_salt("analysis_scope").selected_text(if self.analyze_all{format!("Current watchlist ({} tickers)",self.prefs.list().symbols.len())}else{"Selected ticker".into()}).width(ui.available_width()).show_ui(ui,|ui|{ui.selectable_value(&mut self.analyze_all,false,"Selected ticker");ui.selectable_value(&mut self.analyze_all,true,"Current watchlist");});
        let i = self.prefs.active_provider;
        let provider = &self.prefs.providers[i];
        let configured = !provider.model.trim().is_empty()
            && (provider.auth == Auth::None || !self.secrets[i].key.trim().is_empty());
        let tickers = self.analysis_tickers();
        let available = tickers.iter().filter(|ticker| {
            if self.prefs.offline { demo_market(ticker, 0).is_some() } else { self.quotes.contains_key(*ticker) }
        }).count();
        let busy = !self.analyzing.is_empty();
        ui.add_space(7.);
        let title = if busy {
            format!("Researching {} ticker(s)…", self.analyzing.len())
        } else if !configured {
            "Connect a provider".into()
        } else if self.analyze_all {
            format!("Analyze {available} tickers")
        } else {
            "Generate outlook  →".into()
        };
        let enabled = !busy && (!configured || (available > 0 && (!self.prefs.offline || self.allow_demo_ai)));
        if ui.add_enabled(enabled,egui::Button::new(RichText::new(title).size(13.).strong().color(hex(0x102034))).fill(p.blue).corner_radius(8).min_size(vec2(ui.available_width(),37.))).clicked(){if configured{self.launch_analysis(&ctx);}else{self.open_editor();}}
        if configured && self.prefs.offline && available > 0 {
            ui.add_space(4.);
            ui.checkbox(&mut self.allow_demo_ai, RichText::new("Allow analysis of sample data").size(11.).color(p.muted));
        }
        if configured && available == 0 && !busy {
            text(ui, if tickers.is_empty() { "This watchlist is empty." }
                else if self.prefs.offline { "These tickers have no sample quotes." }
                else { "Refresh the quote before requesting research." }, 11., p.muted);
        } else if configured && available < tickers.len() && !busy {
            text(ui, format!("{available} of {} tickers have quotes ready for research.", tickers.len()), 11., p.muted);
        }
        if busy{ui.horizontal(|ui|{ui.spinner();text(ui,"Gathering price history and provider analysis…",11.,p.muted);});}
        let key = research_key(&self.prefs.selected, self.prefs.offline);
        if let Some(e)=self.ai_errors.get(&key){ui.add_space(7.);Frame::new().fill(alpha(p.red,12)).corner_radius(7).inner_margin(10).show(ui,|ui|{text(ui,e,12.,p.red);});}
        ui.add_space(19.);ui.separator();ui.add_space(13.);
        if let Some(a)=self.analyses.get(&key).cloned(){
            small_title(ui,if a.demo{"SAMPLE-DATA ANALYSIS"}else{"SAVED PROVIDER ANALYSIS"},p);text(ui,format!("{} · {}",a.provider,a.model),11.,p.muted);text(ui,format!("Generated {}",stamp(a.generated)),10.5,p.muted);text(ui,format!("Quote {}",stamp(a.context_time)),10.5,p.muted);
            if let Some(usage)=&a.usage{ui.label(RichText::new(usage.cost_label()).size(10.5).color(p.muted)).on_hover_text(usage.detail());}
            if a.provider!=self.prefs.providers[self.prefs.active_provider].name||a.model!=self.prefs.providers[self.prefs.active_provider].model||a.endpoint!=self.prefs.providers[self.prefs.active_provider].endpoint{text(ui,"This saved result came from a different connection.",11.,p.blue);}
            ui.add_space(10.);
            for(i,o)in a.horizons.iter().enumerate().take(3){self.outlook_card(ui,i,Some(o),p);}
            if a.horizons.is_empty(){Frame::new().fill(p.panel).corner_radius(9).inner_margin(12).show(ui,|ui|{text(ui,"No valid decision and confidence received. Generate again.",13.,p.blue);text(ui,&a.raw,12.,p.muted);});}
            if !a.context_notes.is_empty(){egui::CollapsingHeader::new(RichText::new("Context & limitations").size(12.)).show(ui,|ui|{for n in &a.context_notes{text(ui,format!("• {n}"),11.,p.muted);}});}
            ui.horizontal_wrapped(|ui|{if link(ui,"Full provider answer",p).clicked(){self.raw_answer=Some(a.raw.clone());}if link(ui,"Copy research",p).clicked(){ctx.copy_text(format!("{} · {} / {}\nGenerated {} · Quote {}\nSample data: {}\n\n{}",a.symbol,a.provider,a.model,stamp(a.generated),stamp(a.context_time),a.demo,a.raw));self.notify("Research copied with source and timestamps");}});
            if let Some(format) = support::export_menu(ui) {
                self.save_report(&ctx, crate::report_export::Report::research(&a), format);
            }
        }else{for i in 0..3{self.outlook_card(ui,i,None,p);}ui.add_space(7.);text(ui,"Generate an outlook to see BUY, SELL or DO NOT BUY for each horizon. Every new structured call is saved in the logbook.",12.,p.muted);}
        ui.add_space(15.);text(ui,"AI confidence is the model’s own estimate. See the logbook for reviewed results.",10.5,p.muted);ui.add_space(10.);
    });});
        self.prefs.ai_width = panel.response.rect.width();
    }
    fn outlook_card(&self, ui: &mut egui::Ui, i: usize, o: Option<&Outlook>, p: Palette) {
        let c = [p.blue, p.green, hex(0xb69cff)][i];
        Frame::new()
            .fill(p.panel)
            .stroke(Stroke::new(1., p.line))
            .corner_radius(10)
            .inner_margin(13)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(24., 24.), Sense::hover());
                    ui.painter().rect_filled(r, 6, alpha(c, 25));
                    ui.painter().text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        format!("0{}", i + 1),
                        FontId::proportional(11.),
                        c,
                    );
                    ui.label(RichText::new(HORIZONS[i]).size(15.).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        text(ui, PERIODS[i], 10.5, p.muted);
                    });
                });
                ui.add_space(9.);
                if let Some(o) = o {
                    ui.horizontal_wrapped(|ui| {
                        pill(
                            ui,
                            o.decision
                                .map(Decision::label)
                                .unwrap_or("NO DECISION SUPPLIED"),
                            match o.decision {
                                Some(Decision::Buy) => p.green,
                                Some(Decision::Sell) => p.red,
                                _ => p.muted,
                            },
                        );
                    });
                    text(ui, o.confidence_label(), 14., p.text);
                    text(
                        ui,
                        "AI estimate; not a measured success rate.",
                        10.,
                        p.muted,
                    );
                    text(
                        ui,
                        o.decision.map(Decision::meaning).unwrap_or(
                            "Older result: generate again for an action and percentage.",
                        ),
                        11.5,
                        c,
                    );
                    ui.add_space(4.);
                    text(ui, &o.summary, 13., p.text);
                    text(
                        ui,
                        format!("Logbook review at {}", REVIEW_LABELS[i]),
                        10.5,
                        p.muted,
                    );
                    egui::CollapsingHeader::new(
                        RichText::new("Drivers, risks & suggestion")
                            .size(11.)
                            .color(c),
                    )
                    .id_salt(("outlook", i))
                    .show(ui, |ui| {
                        for item in &o.catalysts {
                            text(ui, format!("Watch · {item}"), 11.5, p.text);
                        }
                        for item in &o.risks {
                            text(ui, format!("Risk · {item}"), 11.5, p.muted);
                        }
                        if !o.suggestion.is_empty() {
                            text(ui, &o.suggestion, 12., c);
                        }
                        text(
                            ui,
                            "* Model judgment, not a measured probability.",
                            10.,
                            p.muted,
                        );
                    });
                } else {
                    small_title(
                        ui,
                        [
                            "PRICE ACTION & CATALYSTS",
                            "EARNINGS & MARKET TRENDS",
                            "BUSINESS & VALUATION",
                        ][i],
                        p,
                    );
                    ui.add_space(5.);
                    text(ui, "No decision yet", 13., p.muted);
                    text(ui, "BUY / SELL / DO NOT BUY + confidence %", 11., p.muted);
                }
            });
        ui.add_space(9.);
    }
    fn submit_editor(&mut self, e: &mut Editor, kind: u8, ctx: &egui::Context) {
        let i = e.selected;
        match serde_json::from_str::<std::collections::BTreeMap<String, String>>(&e.headers[i]) {
            Ok(h) => e.secrets[i].headers = h,
            Err(_) => {
                e.message[i] = Some((
                    false,
                    "Additional headers must be a JSON object with text values.".into(),
                ));
                return;
            }
        }
        let p = e.providers[i].clone();
        let s = e.secrets[i].clone();
        let tx = self.tx.clone();
        let c = ctx.clone();
        let session = e.session;
        let rev = e.revision[i];
        e.busy.insert((i, kind));
        e.message[i] = None;
        std::thread::spawn(move || {
            let event = if kind == 0 {
                Event::Models(session, i, rev, network::load_models(&p, &s))
            } else {
                Event::Probe(session, i, rev, network::test_connection(&p, &s))
            };
            let _ = tx.send(event);
            c.request_repaint();
        });
    }
    fn connections(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(mut e) = self.editor.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        dialog_window(ctx,"AI connections",vec2(824.,650.)).id(Id::unique("connections_dialog")).min_height(300.).resizable(true).collapsible(false).open(&mut open).show(ctx,|ui|{
            text(ui,"Bring your own intelligence",23.,p.text);text(ui,"Choose a provider, add your key, and select a model. Requests go directly to your endpoint.",13.,p.muted);ui.add_space(16.);
            let compact=ui.available_width()<640.;
            if compact {
                let before=e.selected;
                egui::ComboBox::from_id_salt("connection_provider").selected_text(&e.providers[e.selected].name).width(ui.available_width()).show_ui(ui,|ui|{for(i,provider)in e.providers.iter().enumerate(){ui.selectable_value(&mut e.selected,i,&provider.name);}});
                if e.selected!=before{e.reveal=false;}
                ui.add_space(8.);
            }
            // Reserve the footer before allocating either scrolling column. A full-height
            // horizontal row/vertical separator would otherwise push Save below the window.
            let footer_height=if compact{76.}else{54.};
            let body_height=(ui.available_height()-footer_height).max(0.);
            ui.allocate_ui_with_layout(vec2(ui.available_width(),body_height),Layout::left_to_right(Align::Min),|ui|{
                ui.set_height(body_height);
                if !compact {
                    ui.allocate_ui_with_layout(vec2(155.,body_height),Layout::top_down(Align::LEFT),|ui|{egui::ScrollArea::vertical().id_salt("connection_providers_scroll").max_height(body_height).auto_shrink([false,false]).show(ui,|ui|{small_title(ui,"PROVIDERS",p);ui.add_space(8.);for i in 0..e.providers.len(){let selected=e.selected==i;let label=e.providers[i].name.clone();if ui.add_sized([150.,41.],egui::Button::new(RichText::new(label).size(14.).color(if selected{p.blue}else{p.text})).fill(if selected{alpha(p.blue,27)}else{Color32::TRANSPARENT}).corner_radius(8)).clicked(){e.selected=i;e.reveal=false;}}ui.add_space(15.);text(ui,if cfg!(target_os="macos"){"Keys stay in memory unless you choose Keychain."}else{"Keys stay in memory for this session only."},11.,p.muted);});});
                    ui.separator();
                }
                let right_width=ui.available_width();
                ui.allocate_ui_with_layout(vec2(right_width,body_height),Layout::top_down(Align::LEFT),|ui|{egui::ScrollArea::vertical().id_salt(("connection_form_scroll",e.selected)).max_height(body_height).auto_shrink([false,false]).show(ui,|ui|{
                    let i = e.selected;
                    let before = e.providers[i].fingerprint();
                    let before_endpoint = e.providers[i].endpoint.clone();
                    let before_protocol = e.providers[i].protocol;
                    let before_auth = e.providers[i].auth;
                    let before_key = e.secrets[i].key.clone();
                    let before_headers = e.headers[i].clone();
                    ui.horizontal(|ui|{ui.label(RichText::new(&e.providers[i].name).size(21.).strong());if e.tested[i]{pill(ui,"VERIFIED",p.green);}});
                    ui.add_space(10.);small_title(ui,"INFERENCE ENDPOINT",p);
                    if ui.add(egui::TextEdit::singleline(&mut e.providers[i].endpoint).desired_width(f32::INFINITY).margin(vec2(9.,8.))).changed() {
                        e.endpoint_changed(i, &before_endpoint);
                    }
                    text(ui,"Use a base URL or the complete inference URL. Changing servers clears the old key and headers.",11.,p.muted);ui.add_space(8.);
                    small_title(ui,"API KEY",p);ui.horizontal(|ui|{ui.add(egui::TextEdit::singleline(&mut e.secrets[i].key).password(!e.reveal).hint_text(if e.providers[i].auth==Auth::None{"Optional for local inference"}else{"Enter your provider API key"}).desired_width((ui.available_width()-57.).max(0.)).margin(vec2(9.,8.)));if button(ui,if e.reveal{"Hide"}else{"Show"},p).clicked(){e.reveal=!e.reveal;}});
                    ui.add_space(10.);small_title(ui,"MODEL",p);
                    let load_models = ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut e.providers[i].model).hint_text("Enter a model ID or load available models").desired_width((ui.available_width()-114.).max(0.)).margin(vec2(9.,8.)));
                        ui.add_enabled(!e.busy.contains(&(i,0)), egui::Button::new("Load models").min_size(vec2(100.,32.))).clicked()
                    }).inner;
                    if !e.models[i].is_empty(){let ids=e.models[i].clone();egui::ComboBox::from_id_salt(("model_list",i)).selected_text("Choose from available models…").width(ui.available_width()).show_ui(ui,|ui|{for id in ids{let selected=e.providers[i].model==id;if ui.selectable_label(selected,&id).clicked(){e.providers[i].model=id;}}});}
                    if e.providers[i].is_meta_endpoint(){
                        if e.providers[i].model.ends_with("-contributor"){text(ui,"Contributor mode lets Meta use your prompts and responses to train future models.",11.,p.muted);}
                        if e.providers[i].model==META_MODEL{text(ui,"Token estimate: $0.10 input · $0.002 cached · $0.20 output / 1M tokens",11.,p.muted);}
                        ui.hyperlink_to("Meta pricing & Contributor terms",META_PRICING_URL);
                    }
                    ui.add_space(9.);
                    #[cfg(target_os = "macos")]
                    ui.checkbox(&mut e.providers[i].remember,RichText::new("Remember secrets in macOS Keychain").size(12.));
                    #[cfg(not(target_os="macos"))]{e.providers[i].remember=false;text(ui,"Secrets remain in memory for this session and are never written to disk.",11.,p.muted);}
                    ui.add_space(8.);
                    egui::CollapsingHeader::new(RichText::new("Advanced connection details").size(12.)).default_open(e.providers[i].name=="Custom").show(ui,|ui|{
                        ui.horizontal(|ui|{text(ui,"API format",12.,p.muted);egui::ComboBox::from_id_salt(("protocol",i)).selected_text(e.providers[i].protocol.label()).show_ui(ui,|ui|{for v in Protocol::ALL{ui.selectable_value(&mut e.providers[i].protocol,v,v.label());}});});
                        ui.horizontal(|ui|{text(ui,"Authentication",12.,p.muted);egui::ComboBox::from_id_salt(("auth",i)).selected_text(e.providers[i].auth.label()).show_ui(ui,|ui|{for v in [Auth::Bearer,Auth::ApiKey,Auth::GoogleKey,Auth::AzureKey,Auth::None]{ui.selectable_value(&mut e.providers[i].auth,v,v.label());}});});
                        if e.providers[i].protocol==Protocol::Anthropic{ui.horizontal(|ui|{text(ui,"API version",12.,p.muted);ui.text_edit_singleline(&mut e.providers[i].api_version);});}
                        text(ui,"Additional headers (JSON text values)",11.,p.muted);ui.add(egui::TextEdit::multiline(&mut e.headers[i]).code_editor().desired_rows(3).desired_width(f32::INFINITY));text(ui,"Header values follow the same secret-storage choice as the API key.",10.5,p.muted);
                    });
                    let credentials_changed = e.secrets[i].key != before_key || e.headers[i] != before_headers;
                    if credentials_changed || e.providers[i].protocol != before_protocol || e.providers[i].auth != before_auth {
                        e.models[i].clear();
                    }
                    if credentials_changed || e.providers[i].fingerprint() != before {
                        e.revision[i] += 1;
                        e.tested[i] = false;
                        e.usage[i] = None;
                        e.message[i] = None;
                    }
                    if load_models { self.submit_editor(&mut e, 0, ctx); }
                    ui.add_space(9.);ui.horizontal(|ui|{if ui.add_enabled(!e.busy.contains(&(i,1)),egui::Button::new("Test connection").min_size(vec2(135.,33.))).clicked(){self.submit_editor(&mut e,1,ctx);}if e.busy.iter().any(|(index,_)|*index==i){ui.spinner();}});
                    text(ui,"Testing sends a small inference request and may incur a provider charge.",10.5,p.muted);
                    if let Some((ok,msg))=&e.message[i]{ui.add_space(6.);text(ui,msg,12.,if *ok{p.green}else{p.red});}
                });});
            });
            ui.separator();ui.add_space(6.);
            if compact{text(ui,"API keys are never written to the settings file.",11.,p.muted);}
            ui.horizontal(|ui|{if !compact{text(ui,"API keys are never written to the settings file.",11.,p.muted);}ui.with_layout(Layout::right_to_left(Align::Center),|ui|{if ui.add(egui::Button::new(RichText::new("Save connections").strong().color(hex(0x102034))).fill(p.blue).min_size(vec2(151.,34.))).clicked(){save=true;}});});
        });
        if save {
            match self.commit_editor(&mut e) {
                Ok(warning) => {
                    open = false;
                    self.notify(warning.unwrap_or_else(|| {
                        "Connections saved. Select a ticker and generate its outlook.".into()
                    }));
                }
                Err(err) => e.message[e.selected] = Some((false, err)),
            }
        }
        if open {
            self.editor = Some(e);
        }
    }
    fn commit_editor(&mut self, e: &mut Editor) -> Result<Option<String>, String> {
        for i in 0..e.providers.len() {
            let provider = &e.providers[i];
            network::validate_endpoint(&provider.endpoint)
                .map_err(|err| format!("{} · {err}", provider.name))?;
            e.secrets[i].headers = serde_json::from_str(&e.headers[i])
                .map_err(|_| format!("{} has invalid additional headers.", provider.name))?;
            let secret = &e.secrets[i];
            network::validate_headers(secret.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .map_err(|err| format!("{} · {err}", provider.name))?;
        }
        let mut updated = self.prefs.clone();
        updated.providers = e.providers.clone();
        updated.active_provider = e.selected;
        let mut warning = self.settings_load_error.as_ref().map(|_| {
            "Connections updated for this session. The saved preferences could not be read.".into()
        });
        if !self.transient && self.settings_load_error.is_none() {
            for i in 0..e.providers.len() {
                let old = &self.prefs.providers[i];
                let new = &e.providers[i];
                if new.remember
                    && (!old.remember
                        || old.key_account() != new.key_account()
                        || self.secrets[i] != e.secrets[i])
                {
                    save_secret(new, &e.secrets[i])?;
                }
            }
            updated.save(&self.state_path)?;
            for (old, new) in self.prefs.providers.iter().zip(&e.providers) {
                if old.remember
                    && (!new.remember || old.key_account() != new.key_account())
                    && let Err(err) = forget_secret(old)
                {
                    warning = Some(format!("Connections saved. {err}"));
                }
            }
        }
        self.prefs = updated;
        self.secrets = e.secrets.clone();
        self.verified = e.tested.clone();
        self.last_usage = e.usage.clone();
        Ok(warning)
    }
}

impl MarketApp {
    fn dialogs(&mut self, ctx: &egui::Context, p: Palette) {
        self.connections(ctx, p);
        if let Some(ticker) = self.manage_symbol.clone() {
            let mut open = true;
            dialog_window(ctx, &format!("Watchlists for {ticker}"), vec2(400.,360.))
                .vscroll(true).collapsible(false).open(&mut open)
                .show(ctx,|ui| {
                    text(ui,"Choose the lists that follow this symbol",17.,p.text);
                    text(ui,"My Symbols includes every followed symbol. Unchecking it removes the symbol from all lists.",12.,p.muted);
                    ui.add_space(12.);
                    for w in self.prefs.watchlists.clone() {
                        let mut followed = self.prefs.watchlists.iter().find(|l|l.id==w.id).is_some_and(|l|l.symbols.contains(&ticker));
                        if ui.checkbox(&mut followed,&w.name).changed() {
                            self.prefs.set_membership(w.id,&ticker,followed);
                            self.save();
                        }
                    }
                    ui.add_space(10.);
                    if button(ui,"Done",p).clicked() {self.manage_symbol=None;}
                });
            if !open {
                self.manage_symbol = None;
            }
        }
        if self.manage {
            let mut open = true;
            let mut changed = false;
            let mut remove = None;
            let mut swap = None;
            let names = self
                .prefs
                .watchlists
                .iter()
                .map(|w| (w.id, w.name.clone()))
                .collect::<Vec<_>>();
            let mut duplicate_name = false;
            dialog_window(ctx, "Manage watchlists", vec2(540., 480.))
                .vscroll(true)
                .collapsible(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    text(ui, "Organize the markets you follow", 22., p.text);
                    text(
                        ui,
                        "My Symbols includes every symbol across your watchlists.",
                        12.,
                        p.muted,
                    );
                    ui.add_space(15.);
                    for (i, w) in self.prefs.watchlists.iter_mut().enumerate() {
                        Frame::new()
                            .fill(p.raised)
                            .corner_radius(8)
                            .inner_margin(10)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    if w.id == 0 {
                                        ui.add_sized(
                                            [265., 27.],
                                            egui::Label::new(RichText::new("My Symbols").strong()),
                                        );
                                    } else {
                                        let before = w.name.clone();
                                        ui.add(
                                            egui::TextEdit::singleline(&mut w.name)
                                                .char_limit(60)
                                                .desired_width(260.),
                                        );
                                        if w.name.trim().is_empty()
                                            || names.iter().any(|(id, name)| {
                                                *id != w.id
                                                    && name
                                                        .trim()
                                                        .eq_ignore_ascii_case(w.name.trim())
                                            })
                                        {
                                            duplicate_name = true;
                                            w.name = before.clone();
                                        }
                                        changed |= w.name != before;
                                    }
                                    text(ui, format!("{}", w.symbols.len()), 12., p.muted);
                                    if w.id != 0 {
                                        if ui
                                            .small_button("↑")
                                            .on_hover_text("Move list up")
                                            .clicked()
                                            && i > 1
                                        {
                                            swap = Some((i, i - 1));
                                        }
                                        if ui
                                            .small_button("↓")
                                            .on_hover_text("Move list down")
                                            .clicked()
                                        {
                                            swap = Some((i, i + 1));
                                        }
                                        if link(ui, "Delete", p).clicked() {
                                            remove = Some(i);
                                        }
                                    }
                                });
                            });
                        ui.add_space(5.);
                    }
                    if duplicate_name {
                        text(
                            ui,
                            "Each watchlist needs a different, nonempty name.",
                            12.,
                            p.red,
                        );
                    }
                    if let Some((i, w)) = &self.deleted_list.clone() {
                        ui.horizontal(|ui| {
                            text(ui, format!("Deleted {}", w.name), 12., p.muted);
                            if link(ui, "Undo", p).clicked() {
                                self.prefs
                                    .watchlists
                                    .insert((*i).min(self.prefs.watchlists.len()), w.clone());
                                self.deleted_list = None;
                                changed = true;
                            }
                        });
                    }
                    ui.add_space(10.);
                    ui.separator();
                    ui.add_space(8.);
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.new_list)
                                .char_limit(60)
                                .hint_text("Name a new watchlist")
                                .desired_width(355.),
                        );
                        if ui
                            .add_enabled(
                                self.prefs.name_available(&self.new_list, None),
                                egui::Button::new("Create"),
                            )
                            .clicked()
                        {
                            self.prefs.remember_list_view();
                            let id = self.prefs.next_list_id();
                            self.prefs.watchlists.push(Watchlist {
                                id,
                                name: self.new_list.trim().into(),
                                symbols: vec![],
                                selected: String::new(),
                            });
                            self.prefs.active_list = id;
                            self.prefs.restore_list_view();
                            self.new_list.clear();
                            changed = true;
                        }
                    });
                    if !self.new_list.trim().is_empty()
                        && !self.prefs.name_available(&self.new_list, None)
                    {
                        text(
                            ui,
                            "That name is already in use. Choose a different name.",
                            12.,
                            p.red,
                        );
                    }
                    ui.add_space(8.);
                    if button(ui, "Done", p).clicked() {
                        self.manage = false;
                    }
                });
            if let Some((a, b)) = swap {
                if b < self.prefs.watchlists.len() {
                    self.prefs.watchlists.swap(a, b);
                    changed = true;
                }
            }
            if let Some(i) = remove {
                if i < self.prefs.watchlists.len() && self.prefs.watchlists[i].id != 0 {
                    self.prefs.remember_list_view();
                    let w = self.prefs.watchlists.remove(i);
                    if self.prefs.active_list == w.id {
                        self.prefs.active_list = 0;
                        self.prefs.restore_list_view();
                    }
                    self.deleted_list = Some((i, w));
                    changed = true;
                }
            }
            self.manage &= open;
            if changed {
                self.save();
            }
        }
        if self.settings {
            let mut open = true;
            let before = (
                self.prefs.dark,
                self.prefs.offline,
                self.prefs.auto_refresh,
                self.prefs.show_volume,
            );
            dialog_window(ctx,"Preferences",vec2(460.,420.)).vscroll(true).resizable(false).open(&mut open).show(ctx,|ui|{
                if let Some(error) = &self.settings_load_error {
                    text(ui, "Preferences could not be loaded. The original file is kept and changes apply for this session.", 12., p.red);
                    text(ui, error, 11., p.muted);
                }
                text(ui,"Make this workspace yours",22.,p.text);ui.add_space(16.);ui.checkbox(&mut self.prefs.dark,"Dark appearance");ui.checkbox(&mut self.prefs.auto_refresh,"Refresh public quotes every 60 seconds");ui.checkbox(&mut self.prefs.show_volume,"Show trading volume below charts");ui.add_space(9.);ui.separator();ui.add_space(9.);ui.checkbox(&mut self.prefs.offline,"Offline demonstration");text(ui,"Demonstration mode uses labeled sample values. It never presents them as current quotes.",12.,p.muted);ui.add_space(12.);text(ui,"Public feeds may be delayed or unavailable. Missing fields remain blank. No market data account is required for the public feed.",12.,p.muted);ui.add_space(12.);text(ui,"⌘F  Search     ⌘R  Refresh     ⌘,  Preferences",12.,p.muted);ui.add_space(12.);if button(ui,"AI connections…",p).clicked(){self.open_editor();}
            });
            self.settings = open;
            if before.0 != self.prefs.dark {
                self.style(ctx);
            }
            if before.1 != self.prefs.offline {
                self.data_mode_changed(ctx);
            }
            if before
                != (
                    self.prefs.dark,
                    self.prefs.offline,
                    self.prefs.auto_refresh,
                    self.prefs.show_volume,
                )
            {
                self.save();
            }
        }
        if self.import {
            let mut open = true;
            dialog_window(ctx,"Import watchlists",vec2(590.,480.)).vscroll(true).open(&mut open).show(ctx,|ui|{
            text(ui,"Bring your watchlists with you",22.,p.text);text(ui,"Drop a Tenerus export file onto this window, or paste its contents below. Import merges lists and keeps your existing symbols.",13.,p.muted);ui.add_space(12.);ui.add(egui::TextEdit::multiline(&mut self.import_text).desired_rows(12).desired_width(f32::INFINITY).hint_text("Drop an exported watchlist file here"));ui.add_space(10.);if button(ui,"Import watchlists",p).clicked(){let text=self.import_text.clone();match self.import_lists(&text){Ok(())=>{self.import=false;self.import_text.clear();
                self.notify(if self.settings_load_error.is_some() { "Watchlists imported for this session" } else { "Watchlists imported and merged" });
                self.refresh(ctx);
            },Err(e)=>self.notify(e)}}
        });
            self.import &= open;
        }
        self.information_dialog(ctx, p);
        self.market_import_dialog(ctx, p);
        if let Some(a) = self.article.clone() {
            let mut open = true;
            dialog_window(ctx,"Tenerus reading guide",vec2(570.,560.)).vscroll(true).open(&mut open).show(ctx,|ui|{
            small_title(ui,"ORIGINAL SAMPLE READING · NOT A NEWS REPORT",p);ui.add_space(12.);ui.label(RichText::new(&a.title).size(25.).strong());ui.add_space(10.);text(ui,&a.date,16.,p.text);ui.add_space(14.);for s in ["Start with the data timestamp. A last-traded price can be stale, especially outside the regular session. Compare price changes against the stated previous close and inspect several chart ranges.","Then check original company filings, earnings releases and material announcements. A chart alone does not establish valuation or future performance. Missing information should remain unknown rather than become a confident guess.","An AI outlook is one research input. Ask what evidence supports each scenario, which assumptions could fail, and what would change the conclusion. Sample values in this workspace illustrate the controls; they are not market evidence."]{text(ui,s,14.,p.muted);ui.add_space(12.);}
        });
            if !open {
                self.article = None;
            }
        }
        if let Some(raw) = self.raw_answer.clone() {
            let mut open = true;
            dialog_window(ctx, "Full provider answer", vec2(700., 580.))
                .open(&mut open)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add(egui::Label::new(raw).selectable(true).wrap());
                    });
                });
            if !open {
                self.raw_answer = None;
            }
        }
        if let Some((message, at)) = self.toast.clone() {
            if at.elapsed() < Duration::from_secs(9) {
                egui::Area::new(Id::unique("toast"))
                    .anchor(Align2::CENTER_BOTTOM, vec2(0., -43.))
                    .order(egui::Order::Foreground)
                    .show(ctx, |ui| {
                        Frame::new()
                            .fill(p.raised)
                            .stroke(Stroke::new(1., p.line))
                            .corner_radius(9)
                            .inner_margin(13)
                            .show(ui, |ui| {
                                ui.set_max_width(650.);
                                ui.horizontal_wrapped(|ui| {
                                    text(ui, message, 12., p.text);
                                    if link(ui, "Dismiss", p).clicked() {
                                        self.toast = None;
                                    }
                                });
                            });
                    });
            } else {
                self.toast = None;
            }
        }
    }
    fn import_lists(&mut self, input: &str) -> Result<(), String> {
        if input.len() > 2_000_000 {
            return Err("This file is too large for a watchlist export.".into());
        }
        let data: ExportLists = serde_json::from_str(input)
            .map_err(|_| "That is not a valid Tenerus watchlist export.".to_string())?;
        if data.version != 1 || data.watchlists.len() > 100 || data.symbols.len() > 500 {
            return Err("Unsupported export version or too many entries.".into());
        }
        for w in &data.watchlists {
            if w.name.trim().is_empty()
                || w.name.chars().count() > 60
                || w.symbols.len() > 500
                || !w.symbols.iter().all(|s| valid_ticker(s))
            {
                return Err("The export contains an invalid list or ticker.".into());
            }
        }
        let mut p = self.prefs.clone();
        for s in data.symbols {
            if valid_ticker(&s.ticker) && !p.symbols.iter().any(|x| x.ticker == s.ticker) {
                p.symbols.push(s);
            }
        }
        for w in data.watchlists {
            let found = if w.id == 0 {
                p.watchlists.iter().position(|x| x.id == 0)
            } else {
                p.watchlists
                    .iter()
                    .position(|x| x.name.trim().eq_ignore_ascii_case(w.name.trim()))
            };
            if let Some(i) = found {
                for s in w.symbols {
                    if !p.watchlists[i].symbols.contains(&s) {
                        p.watchlists[i].symbols.push(s);
                    }
                }
            } else {
                let id = p.next_list_id();
                p.watchlists.push(Watchlist { id, ..w });
            }
        }
        p.normalize();
        if !self.transient && self.settings_load_error.is_none() {
            p.save(&self.state_path)?;
        }
        self.prefs = p;
        Ok(())
    }
    fn screenshot(&mut self, ctx: &egui::Context) {
        if self.shot.is_none() {
            return;
        }
        self.frames += 1;
        if self.frames >= SHOT_FRAMES && !self.shot_sent {
            let path = self.shot.as_ref().unwrap().clone();
            let c = ctx.clone();
            let captured = self.captured.clone();
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                egui::ScreenshotCallback::new(move |image| {
                    let result = (|| -> Result<(), String> {
                        if let Some(parent) = path.parent() {
                            if !parent.as_os_str().is_empty() {
                                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                            }
                        }
                        let rgba = image
                            .pixels
                            .iter()
                            .flat_map(|p| p.to_array())
                            .collect::<Vec<_>>();
                        image::save_buffer(
                            &path,
                            &rgba,
                            image.size[0] as u32,
                            image.size[1] as u32,
                            image::ColorType::Rgba8,
                        )
                        .map_err(|e| e.to_string())
                    })();
                    if let Err(e) = result {
                        eprintln!("Screenshot failed: {e}");
                        std::process::exit(2);
                    }
                    captured.store(true, std::sync::atomic::Ordering::Release);
                    println!("Saved screenshot after 120 frames: {}", path.display());
                    c.send_viewport_cmd(egui::ViewportCommand::Close);
                    c.request_repaint();
                }),
            ));
            self.shot_sent = true;
        }
        if self.frames > 600 && self.shot.is_some() {
            eprintln!("Screenshot capture did not complete.");
            std::process::exit(2);
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        ctx.request_repaint();
    }
}
impl eframe::App for MarketApp {
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        if self.terms_accepted {
            self.save();
            self.save_cache();
            self.save_journal();
        }
    }
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if !self.terms_accepted {
            return;
        }
        self.poll();
        self.prune_market_queue();
        while !self.prefs.offline && self.inflight.len() < MARKET_WORKERS {
            let Some((ticker, range)) = self.pending_markets.pop_front() else {
                break;
            };
            self.load_market(ticker, range, ctx);
        }
        self.ensure_outlook_history(ctx);
        if !self.transient
            && !self.prefs.offline
            && self.prefs.auto_refresh
            && self.last_refresh.elapsed() >= Duration::from_secs(REFRESH_SECONDS)
        {
            self.refresh(ctx);
        }
        if !self.transient
            && !self.prefs.offline
            && self
                .last_review
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(3600))
        {
            self.launch_reviews(ctx);
        }
        if !self.prefs.offline
            && self.search.trim().len() >= 2
            && self.search_generation != self.search_sent
            && self.search_at.elapsed() > Duration::from_millis(400)
        {
            self.search_sent = self.search_generation;
            let query = self.search.trim().to_string();
            let g = self.search_generation;
            let tx = self.tx.clone();
            let c = ctx.clone();
            std::thread::spawn(move || {
                let result = network::search(&query);
                let _ = tx.send(Event::Search(g, result));
                c.request_repaint();
            });
        }
        if self.shot.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
    fn ui(&mut self, root: &mut egui::Ui, _: &mut eframe::Frame) {
        self.draw(root);
    }
}
impl MarketApp {
    fn draw(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        if !self.transient {
            if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
                let screen = [monitor.x, monitor.y];
                if self.geometry_screen != Some(screen) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(
                        mechanics::minimum_window_size(monitor.y).into(),
                    ));
                    self.geometry_screen = Some(screen);
                    if self.initial_geometry {
                        let size = mechanics::default_window_size(screen);
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
                        self.initial_geometry = false;
                    }
                }
            }
        }
        if let Some(rect) = ctx.input(|i| i.viewport().inner_rect) {
            let size = rect.size();
            if size.x >= 860. && size.y >= 626. {
                self.prefs.window_size = Some([size.x, size.y]);
            }
        }
        let p = Palette::new(self.prefs.dark);
        if !self.terms_accepted {
            self.first_launch(root, p);
            self.screenshot(&ctx);
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.focus_search = true;
            self.prefs.show_sidebar = true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R)) {
            self.refresh_view(&ctx);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.settings = true;
        }
        let list_keys = egui::Modifiers {
            command: true,
            alt: true,
            ..Default::default()
        };
        if ctx.input_mut(|i| i.consume_key(list_keys, egui::Key::ArrowRight)) {
            self.adjacent_list(true, &ctx);
        }
        if ctx.input_mut(|i| i.consume_key(list_keys, egui::Key::ArrowLeft)) {
            self.adjacent_list(false, &ctx);
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let path = f.path();
            match std::fs::metadata(&path) {
                Ok(m) if m.len() <= 2_000_000 => match std::fs::read_to_string(&path) {
                    Ok(s) => {
                        self.import_text = s;
                        self.import = true;
                    }
                    Err(_) => self.notify("Could not read that watchlist file."),
                },
                _ => self.notify("Drop a watchlist export smaller than 2 MB."),
            }
        }
        self.top(root, p);
        self.status(root, p);
        if self.prefs.show_sidebar {
            self.sidebar(root, p);
        }
        if self.using_outlook() {
            self.outlook(root, p);
        } else {
            if self.prefs.show_ai && !self.saved_only && !self.business_news {
                self.ai_panel(root, p);
            }
            self.center(root, p);
        }
        self.dialogs(&ctx, p);
        self.logbook(&ctx, p);
        self.model_guide(&ctx, p);
        self.screenshot(&ctx);
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use egui::Pos2;
    #[test]
    fn a_preset_model_without_credentials_opens_connection_setup() {
        let (ctx, mut app) = setup();
        let i = app.prefs.active_provider;
        assert!(!app.prefs.providers[i].model.is_empty());
        assert!(app.secrets[i].key.is_empty());
        let size = vec2(860., 670.);
        frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        let connect = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "Connect a provider")
            .unwrap()
            .1;
        click(&ctx, &mut app, connect, size);
        assert!(app.editor.is_some());
        assert!(app.analyzing.is_empty());
    }

    #[test]
    fn model_guide_opens_from_the_real_panel_and_links_to_the_logbook_at_small_sizes() {
        let (ctx, mut app) = setup();
        let size = vec2(860., 670.);
        let provider_before = app.prefs.providers[app.prefs.active_provider].fingerprint();
        for _ in 0..4 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let guide = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "Forecast model guide")
            .unwrap()
            .1;
        let _ = click(&ctx, &mut app, guide, size);
        assert!(app.model_guide_open);
        for _ in 0..8 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let all = labels(&out);
        for expected in [
            "One shared forecaster. Evaluate every ticker.",
            "Open prediction logbook",
            "AI connections",
            "Check this device",
            "TimesFM 3.0 & Chronos-2 · Opt-in local foundation pilots",
            "RECOMMENDED START",
        ] {
            let pos = all
                .iter()
                .find(|(s, _)| s == expected)
                .unwrap_or_else(|| panic!("Missing {expected}"))
                .1;
            assert!(
                pos.x >= 0. && pos.x <= size.x && pos.y >= 0. && pos.y <= size.y,
                "{expected} is off screen: {pos:?}"
            );
        }
        let target = all
            .iter()
            .find(|(s, _)| s == "Open prediction logbook")
            .unwrap()
            .1;
        let _ = click(&ctx, &mut app, target, size);
        assert!(app.journal_open && !app.model_guide_open);
        assert!(app.journal.entries.is_empty());
        assert_eq!(
            app.prefs.providers[app.prefs.active_provider].fingerprint(),
            provider_before
        );
    }
    #[test]
    fn foundation_pilot_renders_side_by_side_scoreboard_and_outlook_overlay() {
        let (ctx, mut app) = setup();
        app.model_guide_open = true;
        app.local_forecast_busy = true;
        app.tx
            .send(Event::LocalForecast(Ok(crate::forecast::LocalForecast {
                ticker: app.prefs.selected.clone(),
                horizon: 20,
                backend: crate::forecast::FoundationBackend::Chronos,
                model: crate::forecast::CHRONOS_MODEL.into(),
                runtime: "2.0.0".into(),
                anchor_price: 135.0,
                raw_band: crate::forecast::LocalBand {
                    low: -0.04,
                    mid: 0.02,
                    high: 0.08,
                },
                band: crate::forecast::LocalBand {
                    low: -0.06,
                    mid: 0.02,
                    high: 0.10,
                },
                scoreboard: crate::forecast::FoundationScoreboard {
                    trials: 12,
                    conformal_delta: 0.02,
                    fm_raw_covered: 8,
                    fm_cal_covered: 10,
                    fm_pinball: 0.018,
                    fm_mae: 0.041,
                    outlook_matched_trials: 12,
                    outlook_covered: 10,
                    outlook_pinball: 0.019,
                    outlook_mae: 0.043,
                    baseline_covered: 9,
                    baseline_pinball: 0.021,
                    baseline_mae: 0.048,
                },
                elapsed_ms: 420,
            })))
            .unwrap();
        app.poll();
        let size = vec2(1280., 860.);
        for _ in 0..8 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let all = labels(&out);
        assert!(
            all.iter()
                .any(|(s, _)| s.contains("Chronos-2 (calibrated)"))
        );
        assert!(
            all.iter()
                .any(|(s, _)| s.contains("Outlook 4-Model Ensemble"))
        );
        assert!(
            all.iter()
                .any(|(s, _)| s.contains("Unconditional Baseline"))
        );
        assert!(all.iter().any(|(s, _)| s.contains("pinball skill")));

        // Closing the guide reveals Outlook with the Chronos-2 calibrated overlay legend.
        app.model_guide_open = false;
        app.desk.active = true;
        app.prefs.offline = true;
        for _ in 0..4 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out2 = frame(&ctx, &mut app, vec![], size);
        let all2 = labels(&out2);
        assert!(all2.iter().any(|(s, _)| s == "Chronos-2 (calibrated)"));
    }
    #[test]
    fn local_check_result_keeps_setup_needed_separate_from_hardware_suitability() {
        let (ctx, mut app) = setup();
        app.model_guide_open = true;
        app.local_model_check_busy = true;
        let before = app.prefs.providers[app.prefs.active_provider].fingerprint();
        app.tx
            .send(Event::LocalModelCheck(crate::forecast::MacCheck {
                architecture: "arm64".into(),
                chip: Some("Apple M1".into()),
                memory_gib: Some(16),
                macos_major: Some(27),
                python: Some("3.14.10".into()),
                ..Default::default()
            }))
            .unwrap();
        app.poll();
        assert!(!app.local_model_check_busy);
        let report = app.local_model_check.as_ref().unwrap();
        assert_eq!(
            report.hardware_status(),
            "Hardware suitable for a small local pilot"
        );
        assert!(report.runtime_status().contains("Local setup required"));
        let size = vec2(1000., 900.);
        for _ in 0..8 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(
            labels(&out)
                .iter()
                .any(|(label, _)| label == "Check this device")
        );
        assert_eq!(
            app.prefs.providers[app.prefs.active_provider].fingerprint(),
            before
        );
        assert!(app.journal.entries.is_empty());
    }
    #[test]
    fn model_comparison_controls_display_real_cohort_rates_and_apply_the_ticker_filter() {
        let (ctx, mut app) = setup();
        let size = vec2(1440., 1100.);
        let mut a = crate::journal::tests::analysis();
        a.generated = now() - 400 * crate::journal::DAY;
        a.context_time = a.generated - 60;
        app.journal.record(&a);
        let due = app.journal.entries[0].review_at;
        app.journal.resolve(
            &[1],
            &crate::journal::tests::prices(due, 110.),
            due + crate::journal::DAY,
        );
        let mut other = app.journal.entries[0].clone();
        other.symbol = "AAPL".into();
        other.id = 50;
        other.decision = Decision::Sell;
        app.journal.entries.push(other);
        app.journal_open = true;
        for _ in 0..8 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let comparison = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "Model comparisons")
            .unwrap()
            .1;
        let _ = click(&ctx, &mut app, comparison, size);
        for _ in 0..12 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let all = labels(&out);
        for expected in ["Always BUY", "Always SELL", "Always WAIT", "50%"] {
            assert!(all.iter().any(|(s, _)| s == expected), "Missing {expected}");
        }
        assert!(all.iter().any(|(s, _)| s.starts_with("All tickers · Meta")));
        let filter = all.iter().find(|(s, _)| s == "Only NVDA").unwrap().1;
        let _ = click(&ctx, &mut app, filter, size);
        for _ in 0..4 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let filtered = labels(&out);
        assert!(app.journal_selected_only);
        assert!(filtered.iter().any(|(s, _)| s.starts_with("NVDA · Meta")));
        assert!(filtered.iter().any(|(s, _)| s == "100%"));
        assert!(!filtered.iter().any(|(s, _)| s == "50%"));
        assert_eq!(app.journal.entries.len(), 4);
        assert_eq!(app.prefs.memory_days, 180);
    }
    #[test]
    fn received_decisions_append_to_the_logbook_and_its_actual_controls_work() {
        let (ctx, mut app) = setup();
        let size = vec2(1440., 1100.);
        let i = app.prefs.active_provider;
        let token = connection_token(&app.prefs.providers[i], &app.secrets[i]);
        let mut a = crate::journal::tests::analysis();
        a.demo = true;
        for n in 0..2 {
            a.generated += n;
            app.tx
                .send(Event::Analysis(
                    "NVDA".into(),
                    a.demo,
                    i,
                    token,
                    Ok(a.clone()),
                ))
                .unwrap();
        }
        app.poll();
        assert_eq!(app.journal.entries.len(), 6);
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        let all_labels = labels(&out);
        assert!(all_labels.iter().any(|(s, _)| s == "BUY"));
        assert!(all_labels.iter().any(|(s, _)| s == "AI confidence 68%"));
        let logbook = all_labels
            .iter()
            .find(|(s, _)| s == "Prediction logbook")
            .unwrap()
            .1;
        let _ = click(&ctx, &mut app, logbook, size);
        assert!(app.journal_open);
        for _ in 0..5 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(
            labels(&out)
                .iter()
                .any(|(s, _)| s.contains("No reviewed results"))
        );
        let selector = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "6 months (180 days)")
            .unwrap()
            .1;
        let _ = click(&ctx, &mut app, selector, size);
        for _ in 0..3 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let off = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "Off")
            .unwrap_or_else(|| panic!("Off option is rendered: {:?}", labels(&out)))
            .1;
        let _ = click(&ctx, &mut app, off, size);
        assert_eq!(app.prefs.memory_days, 0);
        assert_eq!(app.journal.entries.len(), 6);
    }
    #[test]
    fn all_action_labels_and_missing_legacy_percentages_render_without_invention() {
        let (ctx, app) = setup();
        let sample = crate::journal::tests::analysis();
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(860., 900.))),
            ..Default::default()
        };
        input.time = Some(1.);
        let mut out = ctx.run_ui(input, |ui| {
            for (i, outlook) in sample.horizons.iter().enumerate() {
                app.outlook_card(ui, i, Some(outlook), Palette::new(false));
            }
            app.outlook_card(
                ui,
                0,
                Some(&Outlook {
                    bias: "bullish".into(),
                    confidence: "high".into(),
                    ..Default::default()
                }),
                Palette::new(false),
            );
        });
        out.textures_delta.clear();
        for expected in [
            "BUY",
            "SELL",
            "DO NOT BUY",
            "NO DECISION SUPPLIED",
            "Confidence % not supplied",
        ] {
            assert!(
                labels(&out).iter().any(|(s, _)| s == expected),
                "{expected} should render"
            );
        }
    }
    #[test]
    fn review_events_update_the_logbook_without_replacing_the_original_call() {
        let (_, mut app) = setup();
        let mut a = crate::journal::tests::analysis();
        a.generated = now() - 40 * crate::journal::DAY;
        a.context_time = a.generated - 60;
        app.journal.record(&a);
        let due = app.journal.entries[0].review_at;
        app.prefs.offline = false;
        app.tx
            .send(Event::Review(
                "NVDA".into(),
                vec![1],
                Ok(crate::journal::tests::prices(due, 90.)),
            ))
            .unwrap();
        app.tx.send(Event::ReviewsDone).unwrap();
        app.poll();
        let entry = &app.journal.entries[0];
        assert_eq!(entry.decision, Decision::Buy);
        assert_eq!(entry.confidence_percent, Some(68.));
        assert!(!entry.outcome.as_ref().unwrap().correct);
        assert_eq!(
            app.journal
                .learning_context(&Provider::meta(), 180, now())
                .reviewed,
            1
        );
        assert!(!app.reviewing);
    }
    #[test]
    fn empty_logbook_fits_the_smallest_supported_window() {
        let (ctx, mut app) = setup();
        app.journal_open = true;
        let size = vec2(860., 670.);
        for _ in 0..8 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        for expected in [
            "Every call. What happened next.",
            "AI learning memory",
            "Check outcomes",
            "No predictions saved yet",
        ] {
            let position = labels(&out)
                .into_iter()
                .find(|(s, _)| s == expected)
                .unwrap_or_else(|| panic!("Missing {expected}"))
                .1;
            assert!(
                position.x >= 0.
                    && position.x <= size.x
                    && position.y >= 0.
                    && position.y <= size.y,
                "{expected} is off screen: {position:?}"
            );
        }
    }
    fn setup() -> (egui::Context, MarketApp) {
        let ctx = egui::Context::default();
        let mut app = MarketApp::from_context(
            &ctx,
            Launch {
                screenshot: Some("unused-test.png".into()),
                view: "prices".into(),
                demo: true,
                ..Default::default()
            },
        );
        app.shot = None;
        (ctx, app)
    }

    #[test]
    fn agreement_gates_first_launch_and_is_remembered_after_restart() {
        let storage = tempfile::tempdir().unwrap();
        let path = storage.path();
        let ctx = egui::Context::default();
        let launch = Launch {
            demo: true,
            intent: Some(NavigationIntent::Symbol("AAPL".into())),
            ..Default::default()
        };
        let mut app = MarketApp::load_workspace(&ctx, launch, path.into(), path.join("legacy"));
        let size = vec2(860., 670.);
        for _ in 0..6 {
            frame(&ctx, &mut app, vec![], size);
        }
        let output = frame(&ctx, &mut app, vec![], size);
        let agree = assert_visible_label(&output, "Agree & Continue", size);
        assert_visible_label(&output, "Decline and quit", size);
        assert!(
            labels(&output)
                .iter()
                .all(|(label, _)| label != "Watchlists")
        );
        assert_eq!(app.prefs.selected, "NVDA");
        assert!(!app.terms_path.exists());

        click(&ctx, &mut app, agree.center(), size);
        assert!(app.terms_accepted && disclaimer::accepted(&app.terms_path));
        assert_eq!(app.prefs.selected, "AAPL");
        let ctx = egui::Context::default();
        let mut app = MarketApp::load_workspace(
            &ctx,
            Launch {
                demo: true,
                ..Default::default()
            },
            path.into(),
            path.join("legacy"),
        );
        assert!(app.terms_accepted);
        for _ in 0..4 {
            frame(&ctx, &mut app, vec![], size);
        }
        let output = frame(&ctx, &mut app, vec![], size);
        let help = assert_visible_label(&output, "Help", size);
        assert!(
            !labels(&output)
                .iter()
                .any(|(label, _)| label == "Agree & Continue")
        );
        click(&ctx, &mut app, help.center(), size);
        let output = frame(&ctx, &mut app, vec![], size);
        let terms = assert_visible_label(&output, disclaimer::TITLE, size);
        click(&ctx, &mut app, terms.center(), size);
        assert!(matches!(
            app.information,
            Some(support::InfoPage::Disclaimer)
        ));
    }

    #[test]
    fn file_import_opens_outlook_with_the_selected_source_and_ignores_closed_requests() {
        let (ctx, mut app) = setup();
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("aapl.csv");
        std::fs::write(
            &file,
            "date,close,currency\n2025-06-02,123,USD\n2025-06-03,125,USD",
        )
        .unwrap();
        let imported = market_data::ImportRequest {
            source: market_data::Source::File,
            ticker: "AAPL".into(),
            file,
            ..Default::default()
        }
        .fetch()
        .unwrap();
        app.open_market_import();
        let stale = app.import_sequence;
        app.open_market_import();
        app.finish_market_import(stale, Ok(imported.clone()));
        assert!(app.desk.imports.get("AAPL").is_none());
        app.finish_market_import(app.import_sequence, Ok(imported));
        assert_eq!(app.prefs.selected, "AAPL");
        assert!(app.using_outlook() && app.prefs.offline);
        let output = frame(&ctx, &mut app, vec![], vec2(1180., 800.));
        let all = labels(&output);
        assert!(
            all.iter()
                .any(|(label, _)| label.contains("CSV or JSON file · Daily prices as supplied"))
        );
        assert!(all.iter().any(|(label, _)| label.contains("125.00")));
    }
    #[test]
    fn intent_navigation_keeps_membership_and_resets_only_the_target_list_scroll() {
        let (ctx, mut app) = setup();
        app.prefs.scroll_offsets.insert(0, 95.);
        app.prefs.scroll_offsets.insert(1, 123.);
        app.dispatch_intent(NavigationIntent::Watchlist(1), &ctx);
        assert_eq!(app.prefs.active_list, 1);
        assert_eq!(app.prefs.scroll_offsets[&1], 0.);
        assert_eq!(app.prefs.scroll_offsets[&0], 95.);
        let symbols = app.prefs.list().symbols.clone();
        app.dispatch_intent(NavigationIntent::Symbol("SPY".into()), &ctx);
        assert_eq!(app.prefs.selected, "SPY");
        assert_eq!(app.prefs.list().symbols, symbols);
        app.search = "old search".into();
        app.saved_only = true;
        app.dispatch_intent(NavigationIntent::BusinessNews, &ctx);
        assert!(app.business_news && !app.saved_only && app.search.is_empty());
        let selected = app.prefs.selected.clone();
        app.dispatch_intent(NavigationIntent::Watchlist(u64::MAX), &ctx);
        assert_eq!(app.prefs.active_list, 1);
        assert_eq!(app.prefs.selected, selected);
        assert!(
            app.toast
                .as_ref()
                .unwrap()
                .0
                .contains("no longer available")
        );
    }
    fn frame(
        ctx: &egui::Context,
        app: &mut MarketApp,
        events: Vec<egui::Event>,
        size: Vec2,
    ) -> egui::FullOutput {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        };
        input.time = Some(ctx.input(|i| i.time) + 1. / 60.);
        let mut output = ctx.run_ui(input, |ui| app.draw(ui));
        output.textures_delta.clear();
        output
    }
    fn labels(output: &egui::FullOutput) -> Vec<(String, Pos2)> {
        fn collect(shape: &egui::Shape, out: &mut Vec<(String, Pos2)>) {
            match shape {
                egui::Shape::Text(t) => {
                    out.push((t.galley.job.text.clone(), t.pos + t.galley.size() * 0.5))
                }
                egui::Shape::Vec(v) => {
                    for s in v {
                        collect(s, out)
                    }
                }
                _ => {}
            }
        }
        let mut out = vec![];
        for s in &output.shapes {
            collect(&s.shape, &mut out);
        }
        out
    }
    fn click(ctx: &egui::Context, app: &mut MarketApp, pos: Pos2, size: Vec2) -> egui::FullOutput {
        let _ = frame(ctx, app, vec![egui::Event::PointerMoved(pos)], size);
        let _ = frame(
            ctx,
            app,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
            size,
        );
        frame(
            ctx,
            app,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            size,
        )
    }
    #[test]
    fn outlook_horizons_depth_and_replay_controls_work() {
        let (ctx, mut app) = setup();
        app.desk.active = true;
        let size = vec2(1440., 925.);
        let pos = |out: &egui::FullOutput, label: &str| {
            labels(out)
                .into_iter()
                .find(|(s, _)| s == label)
                .unwrap_or_else(|| panic!("Missing {label}"))
                .1
        };
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        pos(&out, "+20 sessions");
        let _ = click(&ctx, &mut app, pos(&out, "60 sessions"), size);
        let out = frame(&ctx, &mut app, vec![], size);
        pos(&out, "+60 sessions");
        let mode = pos(&out, "Market god");
        let _ = click(&ctx, &mut app, mode, size);
        let out = frame(&ctx, &mut app, vec![], size);
        assert_eq!(
            app.prefs.outlook_depth,
            crate::scenarios::Depth::MarketGod,
            "Mode control at {mode:?}"
        );
        pos(&out, "Historical track record");
        let replay =
            crate::scenarios::report(&crate::scenarios::History::sample("NVDA").unwrap(), 60)
                .unwrap()
                .trials[0]
                .at;
        let _ = click(&ctx, &mut app, pos(&out, "Historical replay"), size);
        for _ in 0..6 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let date = stamp(replay)
            .split_ascii_whitespace()
            .next()
            .unwrap()
            .to_owned();
        let _ = click(&ctx, &mut app, pos(&out, &date), size);
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(!labels(&out).iter().any(|(s, _)| s == "Realized path"));
        let _ = click(&ctx, &mut app, pos(&out, "Reveal result"), size);
        let out = frame(&ctx, &mut app, vec![], size);
        pos(&out, "Realized path");
    }

    #[test]
    fn outlook_compact_layout_and_missing_history_are_explicit() {
        let (ctx, mut app) = setup();
        app.desk.active = true;
        let size = vec2(860., 670.);
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        for label in [
            "What could happen next?",
            "Market god",
            "20 sessions",
            "+20 sessions",
        ] {
            assert_visible_label(&out, label, size);
        }
        app.prefs.offline = false;
        let out = frame(&ctx, &mut app, vec![], size);
        assert_visible_label(&out, "Waiting for daily history.", size);
        assert!(
            !labels(&out)
                .iter()
                .any(|(s, _)| s == "Middle 80% of matches")
        );
    }

    #[test]
    fn default_and_saved_percentage_reach_the_watchlist_badge() {
        let (ctx, mut app) = setup();
        let size = vec2(1440., 940.);
        let q = demo_market("NVDA", 0).unwrap().quote;
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        let badge = format!("{:+.2}", q.change().unwrap());
        assert!(
            labels(&out)
                .iter()
                .any(|(s, pos)| s == &badge && pos.x < 300.)
        );
        app.prefs.display = 0; // Existing Tenerus percentage preference.
        app.prefs.remember_list_view();
        app.prefs = serde_json::from_str(&serde_json::to_string(&app.prefs).unwrap()).unwrap();
        app.prefs.normalize();
        let out = frame(&ctx, &mut app, vec![], size);
        let badge = format!("{:+.2}%", q.percent().unwrap());
        assert!(
            labels(&out)
                .iter()
                .any(|(s, pos)| s == &badge && pos.x < 300.)
        );
    }
    #[test]
    fn clicking_watchlist_badge_cycles_display_modes_with_consistent_values() {
        let (ctx, mut app) = setup();
        let size = vec2(1440., 940.);
        app.prefs.offline = false;
        let mut quote = demo_market("NVDA", 0).unwrap().quote;
        quote.price = 110.;
        quote.previous = None;
        quote.reported_change = Some(10.);
        quote.reported_percent = Some(99.);
        quote.market_cap = Some(2_000_000_000.);
        app.quotes.insert("NVDA".into(), quote);
        let badge = |out: &egui::FullOutput, expected| {
            labels(out)
                .into_iter()
                .find(|(text, pos)| text == expected && pos.x < 300.)
                .unwrap_or_else(|| panic!("Missing watchlist badge {expected}"))
                .1
        };
        let _ = frame(&ctx, &mut app, vec![], size);
        let mut out = frame(&ctx, &mut app, vec![], size);
        for (current, next) in [
            ("+10.00", PriceChangeDisplay::Percentage),
            ("+10.00%", PriceChangeDisplay::MarketCap),
            ("2.00B", PriceChangeDisplay::Currency),
        ] {
            let _ = click(&ctx, &mut app, badge(&out, current), size);
            assert_eq!(app.prefs.display, next.saved_index());
            assert_eq!(app.prefs.list_views[&0].display, next.saved_index());
            out = frame(&ctx, &mut app, vec![], size);
        }
        badge(&out, "+10.00");
        assert_eq!(app.prefs.selected, "NVDA");
    }
    #[test]
    fn actual_ticker_and_range_controls_update_the_whole_workspace() {
        let (ctx, mut app) = setup();
        let size = vec2(1440., 940.);
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        let range = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "1M")
            .expect("range selector rendered")
            .1;
        let out = click(&ctx, &mut app, range, size);
        assert_eq!(app.prefs.range, 2);
        let ticker = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "AAPL")
            .expect("watchlist row rendered")
            .1;
        let _ = click(&ctx, &mut app, ticker, size);
        assert_eq!(app.prefs.selected, "AAPL");
        assert_eq!(app.market().unwrap().range, 2);
        assert_eq!(app.prefs.range, 2);
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(labels(&out).iter().any(|(s, _)| s == "Apple Inc."));
    }
    #[test]
    fn all_provider_forms_open_at_the_smallest_supported_window() {
        let (ctx, mut app) = setup();
        let size = vec2(860., 670.);
        app.open_editor();
        for _ in 0..12 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        for p in Provider::presets() {
            assert!(
                labels(&out).iter().any(|(s, _)| s == &p.name),
                "{} is visible",
                p.name
            );
        }
        let google = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "Google")
            .unwrap()
            .1;
        let _ = click(&ctx, &mut app, google, size);
        assert_eq!(app.editor.as_ref().unwrap().selected, 2);
        let mut e = app.editor.take().unwrap();
        e.providers[2].model = "gemini-fixture".into();
        e.secrets[2].key = "never-serialize-this-key".into();
        app.commit_editor(&mut e).unwrap();
        assert_eq!(app.prefs.active_provider, 2);
        assert_eq!(app.secrets[2].key, "never-serialize-this-key");
        assert!(
            !serde_json::to_string(&app.prefs)
                .unwrap()
                .contains("never-serialize")
        );
    }
    fn assert_visible_label(output: &egui::FullOutput, expected: &str, viewport: Vec2) -> Rect {
        fn find(shape: &egui::Shape, expected: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.job.text == expected => {
                    Some(Rect::from_min_size(t.pos, t.galley.size()))
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, expected)),
                _ => None,
            }
        }
        let (rect, clip) = output
            .shapes
            .iter()
            .find_map(|s| find(&s.shape, expected).map(|rect| (rect, s.clip_rect)))
            .unwrap_or_else(|| panic!("Missing {expected}"));
        assert!(
            Rect::from_min_size(Pos2::ZERO, viewport).contains_rect(rect)
                && clip.contains_rect(rect),
            "{expected} is clipped at {viewport:?}: text {rect:?}, clip {clip:?}"
        );
        rect
    }
    #[test]
    fn connection_save_stays_visible_when_resizing_and_using_larger_display_scaling() {
        let (ctx, mut app) = setup();
        app.open_editor();
        // Smaller logical viewports also cover 125% and 150% scaling on an 860x670 window.
        for size in [
            vec2(1911., 925.),
            vec2(1180., 800.),
            vec2(860., 670.),
            vec2(688., 536.),
            vec2(573., 447.),
            vec2(1911., 925.),
        ] {
            for _ in 0..8 {
                let _ = frame(&ctx, &mut app, vec![], size);
            }
            let output = frame(&ctx, &mut app, vec![], size);
            assert_visible_label(&output, "Save connections", size);
            assert_visible_label(&output, "Bring your own intelligence", size);
            let rect = ctx
                .memory(|m| m.area_rect(Id::unique("connections_dialog")))
                .unwrap();
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
                "Connection window overflows: {rect:?}"
            );
        }
    }
    #[test]
    fn advanced_connection_form_scrolls_and_save_remains_clickable() {
        let (ctx, mut app) = setup();
        let size = vec2(573., 447.);
        app.open_editor();
        let editor = app.editor.as_mut().unwrap();
        let custom = editor
            .providers
            .iter()
            .position(|p| p.name == "Custom")
            .unwrap();
        editor.selected = custom;
        editor.providers[custom].model = "layout-check".into();
        editor.message[custom] = Some((false, "Check the endpoint and try again. ".repeat(4)));
        for _ in 0..8 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let pointer = pos2(size.x * 0.6, size.y * 0.55);
        let _ = frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(pointer)],
            size,
        );
        let _ = frame(
            &ctx,
            &mut app,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0., -1800.),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::NONE,
            }],
            size,
        );
        for _ in 0..30 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let output = frame(&ctx, &mut app, vec![], size);
        assert_visible_label(&output, "Test connection", size);
        let save = assert_visible_label(&output, "Save connections", size);
        let _ = click(&ctx, &mut app, save.center(), size);
        assert!(
            app.editor.is_none(),
            "The visible Save button must also accept clicks"
        );
        assert_eq!(app.prefs.active_provider, custom);
        assert_eq!(app.prefs.providers[custom].model, "layout-check");
    }
    #[test]
    fn dialogs_stay_inside_a_scaled_viewport_with_long_content() {
        let size = vec2(573., 447.);
        for title in [
            "Preferences",
            "Manage watchlists",
            "Watchlists for AAPL",
            "Import watchlists",
            "About Tenerus",
            "Tenerus reading guide",
            "Full provider answer",
            "prediction_logbook",
            "forecast_model_guide",
        ] {
            let (ctx, mut app) = setup();
            for id in 10..30 {
                app.prefs.watchlists.push(Watchlist {
                    id,
                    name: format!("Watchlist {id}"),
                    symbols: vec![],
                    selected: String::new(),
                });
            }
            match title {
                "Preferences" => app.settings = true,
                "Manage watchlists" => app.manage = true,
                "Watchlists for AAPL" => app.manage_symbol = Some("AAPL".into()),
                "Import watchlists" => app.import = true,
                "About Tenerus" => app.information = Some(support::InfoPage::About),
                "Tenerus reading guide" => app.article = demo_news("Apple").into_iter().next(),
                "Full provider answer" => {
                    app.raw_answer = Some("A long provider answer.\n".repeat(200))
                }
                "prediction_logbook" => app.journal_open = true,
                "forecast_model_guide" => app.model_guide_open = true,
                _ => unreachable!(),
            }
            for _ in 0..8 {
                let _ = frame(&ctx, &mut app, vec![], size);
            }
            let id = if matches!(title, "prediction_logbook" | "forecast_model_guide") {
                Id::unique(title)
            } else {
                Id::unique(Some(title))
            };
            let rect = ctx
                .memory(|m| m.area_rect(id))
                .unwrap_or_else(|| panic!("Missing window: {title}"));
            assert!(
                Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect),
                "{title} overflows: {rect:?}"
            );
        }
    }
    #[test]
    fn initialization_preserves_retina_resolution_and_user_zoom() {
        let ctx = egui::Context::default();
        ctx.set_zoom_factor(1.25);
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(860., 670.))),
            ..Default::default()
        };
        input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .native_pixels_per_point = Some(2.);
        let mut output = ctx.run_ui(input.clone(), |_| {
            let _ = MarketApp::from_context(
                &ctx,
                Launch {
                    screenshot: Some("unused-test.png".into()),
                    demo: true,
                    ..Default::default()
                },
            );
        });
        output.textures_delta.clear();
        let mut output = ctx.run_ui(input, |_| {});
        output.textures_delta.clear();
        assert_eq!(ctx.zoom_factor(), 1.25);
        assert_eq!(output.pixels_per_point, 2.5);
    }
    #[test]
    fn meta_selection_probe_and_completed_analysis_update_visible_cost() {
        let (ctx, mut app) = setup();
        let size = vec2(1000., 700.);
        app.open_editor();
        for _ in 0..12 {
            let _ = frame(&ctx, &mut app, vec![], size);
        }
        let out = frame(&ctx, &mut app, vec![], size);
        let meta_button = labels(&out)
            .into_iter()
            .find(|(label, _)| label == "Meta")
            .unwrap()
            .1;
        let _ = click(&ctx, &mut app, meta_button, size);
        let e = app.editor.as_mut().unwrap();
        let i = e.selected;
        assert_eq!(e.providers[i].model, META_MODEL);
        e.secrets[i].key = "fixture-only-secret".into();
        let session = e.session;
        let revision = e.revision[i];
        let probe_usage = network::extract_usage(
            &serde_json::json!({"usage":{"prompt_tokens":100,"completion_tokens":50}}),
            &e.providers[i],
        );
        app.tx
            .send(Event::Probe(session, i, revision, Ok(probe_usage)))
            .unwrap();
        app.poll();
        let mut e = app.editor.take().unwrap();
        assert!(e.tested[i]);
        app.commit_editor(&mut e).unwrap();
        assert_eq!(app.prefs.active_provider, i);
        assert!(
            !serde_json::to_string(&app.prefs)
                .unwrap()
                .contains("fixture-only-secret")
        );
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(
            labels(&out)
                .iter()
                .any(|(s, _)| s.contains("Estimated cost $0.000020"))
        );

        let usage = network::extract_usage(
            &serde_json::json!({"usage":{"prompt_tokens":2000,"completion_tokens":500}}),
            &app.prefs.providers[i],
        );
        let token = connection_token(&app.prefs.providers[i], &app.secrets[i]);
        let analysis = Analysis {
            symbol: "NVDA".into(),
            provider: "Meta".into(),
            model: META_MODEL.into(),
            endpoint: META_ENDPOINT.into(),
            generated: 1,
            context_time: 1,
            reference_price: None,
            reference_currency: String::new(),
            demo: true,
            horizons: vec![],
            raw: "Fixture only".into(),
            context_notes: vec![],
            usage: Some(usage),
        };
        app.tx
            .send(Event::Analysis(
                "NVDA".into(),
                analysis.demo,
                i,
                token,
                Ok(analysis),
            ))
            .unwrap();
        app.poll();
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(
            labels(&out)
                .iter()
                .any(|(s, _)| s.contains("Estimated cost $0.000300"))
        );
        assert!(!labels(&out).iter().any(|(s, _)| s.contains("$0.000020")));
        let saved = serde_json::to_string(&app.analyses).unwrap();
        let restored: HashMap<String, Analysis> = serde_json::from_str(&saved).unwrap();
        assert_eq!(
            restored["sample:NVDA"].usage.as_ref().unwrap().cost_label(),
            "Estimated cost $0.000300"
        );
    }
    #[test]
    fn import_merges_lists_without_importing_provider_secrets() {
        let (_, mut app) = setup();
        let data = ExportLists {
            version: 1,
            watchlists: vec![Watchlist {
                id: 99,
                name: "Technology".into(),
                symbols: vec!["SPY".into()],
                selected: "SPY".into(),
            }],
            symbols: vec![Symbol::new("SPY", "S&P 500 ETF", "NYSE")],
        };
        app.import_lists(&serde_json::to_string(&data).unwrap())
            .unwrap();
        assert_eq!(app.prefs.watchlists.len(), 3);
        assert!(app.prefs.watchlists[0].symbols.contains(&"SPY".into()));
        assert!(app.prefs.watchlists[1].symbols.contains(&"SPY".into()));
        assert!(app.import_lists(r#"{"version":1,"watchlists":[{"id":1,"name":"bad","symbols":["INVALID TICKER"]}],"symbols":[]}"#).is_err());
    }
    #[test]
    fn a_late_search_result_cannot_replace_the_current_query() {
        let (_, mut app) = setup();
        app.prefs.offline = false;
        app.search_generation = 3;
        app.tx
            .send(Event::Search(
                2,
                Ok(vec![Symbol::new("OLD", "Old result", "")]),
            ))
            .unwrap();
        app.poll();
        assert!(app.search_results.is_empty());
    }

    #[test]
    fn opening_an_exact_ticker_leaves_watchlist_membership_alone() {
        let (ctx, mut app) = setup();
        let size = vec2(1140., 820.);
        app.search = "THYAO.IS".into();
        let drawn = frame(&ctx, &mut app, vec![], size);
        let button = labels(&drawn)
            .into_iter()
            .find(|(s, _)| s == "Open exact ticker: THYAO.IS")
            .unwrap()
            .1;
        click(&ctx, &mut app, button, size);
        assert_eq!(app.prefs.selected, "THYAO.IS");
        assert!(
            app.prefs
                .watchlists
                .iter()
                .all(|w| !w.symbols.iter().any(|s| s == "THYAO.IS"))
        );
        assert!(app.search.is_empty());
    }

    #[test]
    fn changing_inference_origin_clears_prior_credentials_and_model_choices() {
        let (_, mut app) = setup();
        app.open_editor();
        let edit = app.editor.as_mut().unwrap();
        let i = edit.selected;
        edit.secrets[i].key = "fixture-key".into();
        edit.headers[i] = r#"{"x-api-key":"fixture-header"}"#.into();
        edit.models[i].push("fixture-model".into());
        let before = edit.providers[i].endpoint.clone();
        edit.providers[i].endpoint = format!("{before}/chat/completions");
        edit.endpoint_changed(i, &before);
        assert_eq!(edit.secrets[i].key, "fixture-key");
        assert!(edit.models[i].is_empty());
        edit.providers[i].endpoint = "https://example.com/v1".into();
        edit.endpoint_changed(i, &before);
        assert!(edit.secrets[i].key.is_empty());
        assert_eq!(edit.headers[i], "{}");
    }

    #[test]
    fn unreadable_settings_and_research_are_preserved_during_session_changes() {
        let storage = tempfile::tempdir().unwrap();
        let path = storage.path();
        let settings = path.join("settings.json");
        let research = path.join("analyses.json");
        std::fs::write(&settings, b"damaged settings").unwrap();
        std::fs::write(&research, b"damaged research").unwrap();
        let ctx = egui::Context::default();
        let launch = Launch {
            demo: true,
            ..Default::default()
        };
        let mut app = MarketApp::load_workspace(&ctx, launch, path.into(), path.join("legacy"));
        assert!(app.settings_load_error.is_some() && app.analyses_load_error.is_some());
        app.prefs.add(Symbol::new("TST", "Fixture", ""));
        app.save();
        let i = app.prefs.active_provider;
        let token = connection_token(&app.prefs.providers[i], &app.secrets[i]);
        let analysis = crate::journal::tests::analysis();
        app.tx
            .send(Event::Analysis(
                analysis.symbol.clone(),
                analysis.demo,
                i,
                token,
                Ok(analysis),
            ))
            .unwrap();
        app.poll();
        assert!(app.analyses.contains_key("NVDA"));
        assert_eq!(std::fs::read(settings).unwrap(), b"damaged settings");
        assert_eq!(std::fs::read(research).unwrap(), b"damaged research");
    }
    #[test]
    fn a_late_analysis_does_not_verify_an_edited_connection() {
        let (_, mut app) = setup();
        let token = connection_token(&app.prefs.providers[0], &app.secrets[0]);
        app.prefs.providers[0].endpoint = "https://different.example/v1".into();
        let a = Analysis {
            symbol: "NVDA".into(),
            provider: "OpenAI".into(),
            model: "fixture".into(),
            endpoint: "https://api.openai.com/v1".into(),
            generated: 1,
            context_time: 1,
            reference_price: None,
            reference_currency: String::new(),
            demo: true,
            horizons: vec![],
            raw: "Fixture only".into(),
            context_notes: vec![],
            usage: None,
        };
        app.tx
            .send(Event::Analysis("NVDA".into(), a.demo, 0, token, Ok(a)))
            .unwrap();
        app.poll();
        assert!(!app.verified[0]);
        assert_eq!(
            app.analyses["sample:NVDA"].endpoint,
            "https://api.openai.com/v1"
        );
    }
    #[test]
    fn online_mode_never_falls_back_to_an_illustrative_quote_or_curve() {
        let (_, mut app) = setup();
        assert!(app.quote("NVDA").unwrap().demo);
        app.prefs.offline = false;
        assert!(app.quote("NVDA").is_none());
        assert!(app.market().is_none());
        let mut m = demo_market("NVDA", 0).unwrap();
        m.quote.demo = false;
        app.quotes.insert("NVDA".into(), m.quote.clone());
        app.markets.insert(("NVDA".into(), 0), m);
        assert!(!app.market().unwrap().quote.demo);
        app.prefs.range = 2;
        assert!(app.market().is_none());
        assert!(!app.quote("NVDA").unwrap().demo);
    }
    fn public_market_fixture(range: usize, price: f64, time: i64) -> Market {
        network::parse_market(
            &serde_json::json!({"chart":{"result":[{
                "meta":{"symbol":"NVDA","regularMarketPrice":price,"regularMarketTime":time,"currency":"USD"},
                "timestamp":[time-60,time],
                "indicators":{"quote":[{"close":[price-1.,price],"volume":[100,200]}]}
            }]}}),
            "NVDA",
            range,
        )
        .unwrap()
    }

    #[test]
    fn quote_only_refresh_preserves_the_chart_and_new_quote_across_restart() {
        let (ctx, mut app) = setup();
        let storage = tempfile::tempdir().unwrap();
        app.state_path = storage.path().join("settings.json");
        app.prefs.offline = false;
        app.transient = false;
        app.apply_market(("NVDA".into(), 0), public_market_fixture(0, 100., 1000));
        let mut next = public_market_fixture(0, 110., 2000);
        next.points.clear();
        next.quote.currency = "—".into();
        app.apply_market(("NVDA".into(), 0), next);
        assert!(
            app.market_failure()
                .unwrap()
                .contains("last available chart")
        );
        app.save_cache();
        let mut reopened = MarketApp::load_workspace(
            &ctx,
            Launch {
                demo: true,
                ..Default::default()
            },
            storage.path().into(),
            storage.path().join("legacy"),
        );
        reopened.prefs.offline = false;
        assert_eq!(reopened.quote("NVDA").unwrap().price, 110.);
        assert_eq!(
            reopened.market().unwrap().points.last().unwrap().close,
            100.
        );
    }

    #[test]
    fn chart_comparison_survives_a_refresh_that_removes_the_drag_anchor() {
        let (ctx, mut app) = setup();
        app.desk.active = false;
        app.prefs.show_sidebar = false;
        app.prefs.show_ai = false;
        app.prefs.offline = false;
        let mut market = demo_market("NVDA", 0).unwrap();
        market.quote.demo = false;
        app.apply_market(("NVDA".into(), 0), market.clone());
        let size = vec2(1150., 800.);
        let drawn = frame(&ctx, &mut app, vec![], size);
        let y = labels(&drawn)
            .iter()
            .find(|(s, _)| s.contains("Drag to compare"))
            .unwrap()
            .1
            .y
            + 60.;
        let start = pos2(800., y);
        let end = pos2(560., y);
        frame(
            &ctx,
            &mut app,
            vec![
                egui::Event::PointerMoved(start),
                egui::Event::PointerButton {
                    pos: start,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            size,
        );
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(end)], size);
        assert!(app.range_drag.is_some());
        assert!(app.hover_info.contains('→'));
        market.points.drain(..market.points.len() - 2);
        app.apply_market(("NVDA".into(), 0), market);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(end - vec2(10., 0.))],
            size,
        );
        assert!(app.range_drag.is_none());
        assert!(!app.hover_info.contains('→'));
    }
    #[test]
    fn parsed_market_results_reach_the_quote_chart_and_visible_screen() {
        let (ctx, mut app) = setup();
        app.prefs.offline = false;
        app.prefs.range = 2;
        app.prefs.show_ai = false;
        app.inflight.insert(("NVDA".into(), 2));
        app.tx
            .send(Event::Market(
                "NVDA".into(),
                2,
                Ok(public_market_fixture(2, 123.45, 1000)),
            ))
            .unwrap();
        app.poll();
        assert!(app.inflight.is_empty());
        assert_eq!(app.quote("NVDA").unwrap().price, 123.45);
        assert_eq!(app.market().unwrap().points.len(), 2);
        let size = vec2(1440., 940.);
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(labels(&out).iter().any(|(s, _)| s == "123.45"));
    }
    #[test]
    fn failed_refresh_keeps_data_and_displays_the_actual_failure_without_hovering() {
        let (ctx, mut app) = setup();
        app.prefs.offline = false;
        app.prefs.range = 0;
        app.prefs.show_ai = false;
        app.tx
            .send(Event::Market(
                "NVDA".into(),
                0,
                Ok(public_market_fixture(0, 123.45, 1000)),
            ))
            .unwrap();
        app.poll();
        let error = "Could not resolve query1.finance.yahoo.com. Check your internet connection or DNS settings, then retry.";
        app.tx
            .send(Event::Market("NVDA".into(), 0, Err(error.into())))
            .unwrap();
        app.poll();
        assert_eq!(app.quote("NVDA").unwrap().price, 123.45);
        assert_eq!(app.market().unwrap().points.len(), 2);
        let size = vec2(1440., 940.);
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        let visible = labels(&out);
        assert!(visible.iter().any(|(s, _)| s == error));
        assert!(visible.iter().any(|(s, _)| s == "Retry"));
    }
    #[test]
    fn a_quote_only_response_is_visible_when_chart_history_is_missing() {
        let (ctx, mut app) = setup();
        app.prefs.offline = false;
        app.prefs.range = 0;
        app.prefs.show_ai = false;
        let m = network::parse_market(
            &serde_json::json!({"chart":{"result":[{
                "meta":{"symbol":"NVDA","regularMarketPrice":123.45,"regularMarketTime":1000}
            }]}}),
            "NVDA",
            0,
        )
        .unwrap();
        app.tx.send(Event::Market("NVDA".into(), 0, Ok(m))).unwrap();
        app.poll();
        let size = vec2(1440., 940.);
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        let visible = labels(&out);
        assert!(visible.iter().any(|(s, _)| s == "123.45"));
        assert!(
            visible
                .iter()
                .any(|(s, _)| s == "No history is available for this range")
        );
    }
    #[test]
    fn one_chart_range_succeeding_does_not_clear_another_ranges_error() {
        let (_, mut app) = setup();
        app.prefs.offline = false;
        app.prefs.range = 2;
        app.tx
            .send(Event::Market(
                "NVDA".into(),
                2,
                Err("Range request failed".into()),
            ))
            .unwrap();
        app.tx
            .send(Event::Market(
                "NVDA".into(),
                0,
                Ok(public_market_fixture(0, 123.45, 1000)),
            ))
            .unwrap();
        app.poll();
        assert_eq!(app.market_failure(), Some("Range request failed"));
        assert!(app.market().is_none());
        assert!(app.quote("NVDA").is_some());
        app.tx
            .send(Event::Market(
                "NVDA".into(),
                2,
                Ok(public_market_fixture(2, 124., 2000)),
            ))
            .unwrap();
        app.poll();
        assert!(app.market_failure().is_none());
        assert!(app.market().is_some());
    }
    #[test]
    fn a_late_older_intraday_response_cannot_replace_a_newer_quote() {
        let (_, mut app) = setup();
        app.prefs.offline = false;
        for (range, price, time) in [(0, 10., 1000), (6, 11., 2000), (0, 9., 1000)] {
            app.tx
                .send(Event::Market(
                    "NVDA".into(),
                    range,
                    Ok(public_market_fixture(range, price, time)),
                ))
                .unwrap();
            app.poll();
        }
        assert_eq!(app.quote("NVDA").unwrap().price, 11.);
    }
    #[test]
    fn queued_chart_requests_show_loading_instead_of_a_no_data_message() {
        let (ctx, mut app) = setup();
        app.prefs.offline = false;
        app.prefs.range = 0;
        app.prefs.show_ai = false;
        app.pending_markets.push_back(("NVDA".into(), 0));
        let size = vec2(1440., 940.);
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        let visible = labels(&out);
        assert!(visible.iter().any(|(s, _)| s == "Loading chart data…"));
        assert!(
            visible
                .iter()
                .any(|(s, _)| s.contains("Loading public market data"))
        );
    }
    #[test]
    fn switching_watchlists_queues_every_member_without_waiting_for_auto_refresh() {
        let (ctx, mut app) = setup();
        app.prefs.offline = false;
        app.prefs.auto_refresh = false;
        app.prefs.watchlists.push(Watchlist {
            id: 99,
            name: "Data loading test".into(),
            symbols: vec!["SPY".into(), "AAPL".into(), "MSFT".into()],
            selected: "SPY".into(),
        });
        // Fill the worker slots and mark news active so this queue test never
        // starts a network request.
        app.inflight
            .extend((0..MARKET_WORKERS).map(|i| (format!("BUSY{i}"), 0)));
        app.pending_markets
            .extend([("NVDA".into(), 0), ("SPY".into(), 3), ("SPY".into(), 0)]);
        app.news_inflight.insert("SPY".into());
        app.choose_list(99, &ctx);
        assert_eq!(app.pending_markets.front(), Some(&("SPY".into(), 0)));
        assert!(!app.pending_markets.contains(&("NVDA".into(), 0)));
        assert!(!app.pending_markets.contains(&("SPY".into(), 3)));
        for ticker in ["SPY", "AAPL", "MSFT"] {
            assert!(
                app.pending_markets.contains(&(ticker.into(), 0)),
                "{ticker} was not queued"
            );
        }
    }

    #[test]
    fn live_and_sample_research_survive_a_restart_without_replacing_each_other() {
        let (ctx, _) = setup();
        let storage = tempfile::tempdir().unwrap();
        let launch = || Launch {
            demo: true,
            ..Default::default()
        };
        let load = || {
            MarketApp::load_workspace(
                &ctx,
                launch(),
                storage.path().into(),
                storage.path().join("legacy"),
            )
        };
        let mut public = crate::journal::tests::analysis();
        public.raw = "Live result".into();
        // Simulate a file written before results were separated by data mode.
        atomic_json(
            &storage.path().join("analyses.json"),
            &HashMap::from([("NVDA", &public)]),
        )
        .unwrap();
        let mut app = load();
        let mut sample = public.clone();
        sample.demo = true;
        sample.raw = "Sample result".into();
        let i = app.prefs.active_provider;
        let token = connection_token(&app.prefs.providers[i], &app.secrets[i]);
        app.tx
            .send(Event::Analysis("NVDA".into(), true, i, token, Ok(sample)))
            .unwrap();
        app.poll();
        let reopened = load();
        assert_eq!(
            reopened.analyses[&research_key("NVDA", false)].raw,
            "Live result"
        );
        assert_eq!(
            reopened.analyses[&research_key("NVDA", true)].raw,
            "Sample result"
        );
    }

    #[test]
    fn market_news_distinguishes_a_failed_refresh_from_a_valid_empty_feed() {
        let (ctx, mut app) = setup();
        app.prefs.offline = false;
        app.business_news = true;
        let size = vec2(860., 670.);
        app.tx
            .send(Event::News("__market".into(), Ok(vec![])))
            .unwrap();
        app.poll();
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(
            labels(&out)
                .iter()
                .any(|(s, _)| s == "The feed returned no stories.")
        );
        let article = Article {
            title: "Cached headline".into(),
            publisher: "Publisher".into(),
            link: "https://example.com/story".into(),
            date: "Today".into(),
            demo: false,
        };
        app.tx
            .send(Event::News("__market".into(), Ok(vec![article])))
            .unwrap();
        app.poll();
        let fetched = app.news_fetched["__market"];
        app.tx
            .send(Event::News(
                "__market".into(),
                Err("Connection timed out.".into()),
            ))
            .unwrap();
        app.poll();
        let out = frame(&ctx, &mut app, vec![], size);
        let visible = labels(&out);
        assert_eq!(app.news_fetched["__market"], fetched);
        for expected in [
            "Cached headline",
            "News refresh failed. Showing saved headlines.",
            "Refresh news",
        ] {
            assert!(
                visible.iter().any(|(s, _)| s == expected),
                "Missing {expected}"
            );
        }
        assert!(!visible.iter().any(|(s, _)| s == "AI outlook"));
    }
    #[test]
    fn regular_session_filter_preserves_cached_extended_points() {
        let (_, mut app) = setup();
        app.prefs.offline = false;
        let mut m = demo_market("NVDA", 0).unwrap();
        m.quote.demo = false;
        let start = m.points[10].time;
        let end = m.points[90].time;
        m.regular_session = Some([start, end]);
        app.markets.insert(("NVDA".into(), 0), m);
        app.prefs.show_extended = false;
        assert_eq!(app.market().unwrap().points.len(), 80);
        app.prefs.show_extended = true;
        assert_eq!(app.market().unwrap().points.len(), 96);
    }
    #[test]
    fn saving_a_story_is_persistent_and_does_not_open_the_publisher() {
        let (ctx, mut app) = setup();
        app.prefs.offline = false;
        app.news_only = true;
        app.prefs.show_ai = false;
        let a = Article {
            title: "Fixture headline".into(),
            publisher: "Fixture publisher".into(),
            date: "Fixture date".into(),
            link: "https://example.com/story".into(),
            demo: false,
        };
        app.news.insert("NVDA".into(), vec![a]);
        let size = vec2(1000., 700.);
        let _ = frame(&ctx, &mut app, vec![], size);
        let out = frame(&ctx, &mut app, vec![], size);
        let pos = labels(&out)
            .into_iter()
            .find(|(s, _)| s == "Save story")
            .expect("save control is visible")
            .1;
        let out = click(&ctx, &mut app, pos, size);
        assert_eq!(app.prefs.saved_articles.len(), 1);
        assert!(
            out.platform_output.commands.is_empty(),
            "saving must not open the browser"
        );
        let mut restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&app.prefs).unwrap()).unwrap();
        restored.normalize();
        assert_eq!(restored.saved_articles.len(), 1);
        app.saved_only = true;
        let out = frame(&ctx, &mut app, vec![], size);
        assert!(labels(&out).iter().any(|(s, _)| s == "Saved stories"));
        assert!(labels(&out).iter().any(|(s, _)| s == "Fixture headline"));
    }
}
