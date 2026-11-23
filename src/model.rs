use crate::mechanics::{
    ChartRange, PriceChangeDisplay, deserialize_chart_range_index, price_change_fraction,
    quote_value_order,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const RANGES: [&str; 11] = crate::mechanics::CHART_RANGE_LABELS;
pub const HORIZONS: [&str; 3] = ["Short term", "Medium term", "Long term"];
pub const PERIODS: [&str; 3] = ["1–4 weeks", "3–6 months", "1–3 years"];
pub const REFRESH_SECONDS: u64 = 60;
pub const PUBLIC_TIMEOUT: u64 = 18;
pub const MAX_CHART_POINTS: usize = 20_000;
pub const SHOT_FRAMES: usize = 120;
pub const SHOT_TIMEOUT: u64 = 30;
pub const META_ENDPOINT: &str = "https://api.meta.ai/v1";
pub const META_MODEL: &str = "muse-spark-1.3-contributor";
pub const META_PRICING_URL: &str = "https://dev.meta.ai/docs/pricing-rate-limits";

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Symbol {
    pub ticker: String,
    pub name: String,
    pub exchange: String,
}
impl Symbol {
    pub fn new(ticker: &str, name: &str, exchange: &str) -> Self {
        Self {
            ticker: ticker.into(),
            name: name.into(),
            exchange: exchange.into(),
        }
    }
}
pub fn catalog() -> Vec<Symbol> {
    [
        ("NVDA", "NVIDIA Corporation", "NASDAQ"),
        ("AAPL", "Apple Inc.", "NASDAQ"),
        ("MSFT", "Microsoft Corporation", "NASDAQ"),
        ("GOOGL", "Alphabet Inc.", "NASDAQ"),
        ("AMZN", "Amazon.com, Inc.", "NASDAQ"),
        ("TSLA", "Tesla, Inc.", "NASDAQ"),
        ("META", "Meta Platforms, Inc.", "NASDAQ"),
        ("AMD", "Advanced Micro Devices, Inc.", "NASDAQ"),
        ("^GSPC", "S&P 500", "INDEX"),
        ("^DJI", "Dow Jones Industrial Average", "INDEX"),
        ("^IXIC", "NASDAQ Composite", "INDEX"),
        ("BTC-USD", "Bitcoin USD", "CRYPTO"),
        ("SPY", "S&P 500 ETF", "NYSE"),
        ("QQQ", "NASDAQ 100 ETF", "NASDAQ"),
        ("GC=F", "Gold futures", "COMEX"),
        ("EURUSD=X", "EUR / USD", "CURRENCY"),
    ]
    .into_iter()
    .map(|(t, n, e)| Symbol::new(t, n, e))
    .collect()
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Watchlist {
    pub id: u64,
    pub name: String,
    pub symbols: Vec<String>,
    #[serde(default)]
    pub selected: String,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct ListView {
    pub sort: usize,
    pub display: usize,
    pub descending: bool,
}
impl Default for ListView {
    fn default() -> Self {
        Self {
            sort: 0,
            display: PriceChangeDisplay::default().saved_index(),
            descending: false,
        }
    }
}
fn default_true() -> bool {
    true
}
fn default_sidebar_width() -> f32 {
    240.
}
fn default_ai_width() -> f32 {
    328.
}
fn default_memory_days() -> u16 {
    180
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Protocol {
    Chat,
    Responses,
    Anthropic,
    Gemini,
}
impl Protocol {
    pub const ALL: [Self; 4] = [Self::Chat, Self::Responses, Self::Anthropic, Self::Gemini];
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "OpenAI compatible",
            Self::Responses => "OpenAI Responses",
            Self::Anthropic => "Anthropic Messages",
            Self::Gemini => "Google Gemini",
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum Auth {
    Bearer,
    ApiKey,
    GoogleKey,
    AzureKey,
    None,
}
impl Auth {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bearer => "Bearer token",
            Self::ApiKey => "x-api-key",
            Self::GoogleKey => "x-goog-api-key",
            Self::AzureKey => "api-key",
            Self::None => "No authentication",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Provider {
    pub name: String,
    pub endpoint: String,
    pub model: String,
    pub protocol: Protocol,
    pub auth: Auth,
    pub remember: bool,
    #[serde(default = "default_version")]
    pub api_version: String,
}
fn default_version() -> String {
    "2023-06-01".into()
}
impl Provider {
    pub fn presets() -> Vec<Self> {
        [
            (
                "OpenAI",
                "https://api.openai.com/v1",
                Protocol::Responses,
                Auth::Bearer,
            ),
            (
                "Anthropic",
                "https://api.anthropic.com/v1",
                Protocol::Anthropic,
                Auth::ApiKey,
            ),
            (
                "Google",
                "https://generativelanguage.googleapis.com/v1beta",
                Protocol::Gemini,
                Auth::GoogleKey,
            ),
            (
                "Grok · xAI",
                "https://api.x.ai/v1",
                Protocol::Chat,
                Auth::Bearer,
            ),
            (
                "DeepSeek",
                "https://api.deepseek.com",
                Protocol::Chat,
                Auth::Bearer,
            ),
            (
                "NVIDIA",
                "https://integrate.api.nvidia.com/v1",
                Protocol::Chat,
                Auth::Bearer,
            ),
            ("Meta", META_ENDPOINT, Protocol::Chat, Auth::Bearer),
            (
                "Custom",
                "http://localhost:8080/v1",
                Protocol::Chat,
                Auth::None,
            ),
        ]
        .into_iter()
        .map(|(name, endpoint, protocol, auth)| Self {
            name: name.into(),
            endpoint: endpoint.into(),
            model: if name == "Meta" {
                META_MODEL.into()
            } else {
                String::new()
            },
            protocol,
            auth,
            remember: false,
            api_version: default_version(),
        })
        .collect()
    }
    pub fn meta() -> Self {
        Self::presets()
            .into_iter()
            .find(|p| p.name == "Meta")
            .unwrap()
    }
    pub fn is_meta_endpoint(&self) -> bool {
        url::Url::parse(self.endpoint.trim()).is_ok_and(|u| {
            u.scheme() == "https"
                && u.host_str() == Some("api.meta.ai")
                && u.port_or_known_default() == Some(443)
        })
    }
    pub fn model_label(&self) -> &str {
        if self.model == META_MODEL {
            "Muse Spark 1.3 Contributor"
        } else {
            &self.model
        }
    }
    pub fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{:?}|{:?}|{}",
            self.name, self.endpoint, self.model, self.protocol, self.auth, self.api_version
        )
    }
    pub fn key_account(&self) -> String {
        format!("{}:{}", self.name, self.endpoint.trim_end_matches('/'))
    }
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret {
    pub key: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Preferences {
    pub watchlists: Vec<Watchlist>,
    pub active_list: u64,
    pub selected: String,
    pub symbols: Vec<Symbol>,
    #[serde(default, deserialize_with = "deserialize_chart_range_index")]
    pub range: usize,
    pub display: usize,
    pub sort: usize,
    pub show_volume: bool,
    pub auto_refresh: bool,
    pub offline: bool,
    pub providers: Vec<Provider>,
    pub active_provider: usize,
    #[serde(default)]
    pub dark: bool,
    #[serde(default)]
    pub scroll_offsets: HashMap<u64, f32>,
    #[serde(default)]
    pub list_views: HashMap<u64, ListView>,
    #[serde(default)]
    pub sort_descending: bool,
    #[serde(default)]
    pub saved_articles: Vec<Article>,
    #[serde(default = "default_true")]
    pub show_extended: bool,
    #[serde(default = "default_true")]
    pub show_sidebar: bool,
    #[serde(default = "default_true")]
    pub show_ai: bool,
    #[serde(default)]
    pub window_size: Option<[f32; 2]>,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default = "default_ai_width")]
    pub ai_width: f32,
    #[serde(default = "default_memory_days")]
    pub memory_days: u16,
    #[serde(default)]
    pub outlook_depth: crate::scenarios::Depth,
    #[serde(default)]
    pub briefing_engine: crate::briefing::Engine,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            watchlists: vec![
                Watchlist {
                    id: 0,
                    name: "My Symbols".into(),
                    symbols: vec![
                        "NVDA", "AAPL", "MSFT", "GOOGL", "AMZN", "TSLA", "META", "AMD", "^GSPC",
                        "^DJI", "^IXIC",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                    selected: "NVDA".into(),
                },
                Watchlist {
                    id: 1,
                    name: "Technology".into(),
                    symbols: vec!["NVDA", "AAPL", "MSFT", "GOOGL", "AMD"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                    selected: "NVDA".into(),
                },
                Watchlist {
                    id: 2,
                    name: "Market indices".into(),
                    symbols: vec!["^GSPC", "^DJI", "^IXIC"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                    selected: "^GSPC".into(),
                },
            ],
            active_list: 0,
            selected: "NVDA".into(),
            symbols: catalog(),
            range: ChartRange::default().index(),
            display: PriceChangeDisplay::default().saved_index(),
            sort: 0,
            show_volume: true,
            auto_refresh: true,
            offline: false,
            providers: Provider::presets(),
            active_provider: Provider::presets()
                .iter()
                .position(|p| p.name == "Meta")
                .unwrap(),
            dark: false,
            scroll_offsets: HashMap::new(),
            list_views: HashMap::new(),
            sort_descending: false,
            saved_articles: vec![],
            show_extended: true,
            show_sidebar: true,
            show_ai: true,
            window_size: None,
            sidebar_width: default_sidebar_width(),
            ai_width: default_ai_width(),
            memory_days: default_memory_days(),
            outlook_depth: crate::scenarios::Depth::default(),
            briefing_engine: crate::briefing::Engine::default(),
        }
    }
}
impl Preferences {
    pub fn load(path: &Path, legacy: &Path) -> Result<Self, String> {
        let Some(mut preferences) = read_with_legacy::<Self>(path, legacy)? else {
            return Ok(Self::default());
        };
        preferences.normalize();
        Ok(preferences)
    }
    pub fn normalize(&mut self) {
        if ![0, 90, 180, 365].contains(&self.memory_days) {
            self.memory_days = default_memory_days();
        }
        if !self.watchlists.iter().any(|w| w.id == 0) {
            self.watchlists.insert(
                0,
                Watchlist {
                    id: 0,
                    name: "My Symbols".into(),
                    symbols: vec![],
                    selected: String::new(),
                },
            );
        }
        let zero = self.watchlists.iter().position(|w| w.id == 0).unwrap();
        self.watchlists.swap(0, zero);
        let mut ids = std::collections::HashSet::new();
        let mut reserved = self
            .watchlists
            .iter()
            .map(|w| w.id)
            .collect::<std::collections::HashSet<_>>();
        let mut next_id = self
            .watchlists
            .iter()
            .map(|w| w.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut union = vec![];
        for w in &mut self.watchlists {
            if !ids.insert(w.id) {
                while !reserved.insert(next_id) {
                    next_id = next_id.wrapping_add(1).max(1);
                }
                w.id = next_id;
                ids.insert(w.id);
            }
            let mut unique = vec![];
            for s in &w.symbols {
                if valid_ticker(s) && !unique.contains(s) {
                    unique.push(s.clone());
                }
                if valid_ticker(s) && !union.contains(s) {
                    union.push(s.clone());
                }
            }
            w.symbols = unique;
        }
        self.watchlists
            .iter_mut()
            .find(|w| w.id == 0)
            .unwrap()
            .symbols = union;
        self.watchlists.iter_mut().find(|w| w.id == 0).unwrap().name = "My Symbols".into();
        if !self.watchlists.iter().any(|w| w.id == self.active_list) {
            self.active_list = 0;
        }
        // Merge by identity so adding a provider never erases existing endpoints,
        // model choices or Keychain references (including the former Custom slot).
        let selected = self
            .providers
            .get(self.active_provider)
            .map(|p| p.name.clone());
        let added_meta = !self.providers.iter().any(|p| p.name == "Meta");
        let unconfigured = self
            .providers
            .get(self.active_provider)
            .is_none_or(|p| p.model.trim().is_empty());
        let previous = std::mem::take(&mut self.providers);
        self.providers = Provider::presets()
            .into_iter()
            .map(|preset| {
                previous
                    .iter()
                    .find(|p| p.name == preset.name)
                    .cloned()
                    .unwrap_or(preset)
            })
            .collect();
        self.active_provider = selected
            .and_then(|name| self.providers.iter().position(|p| p.name == name))
            .filter(|_| !added_meta || !unconfigured)
            .unwrap_or_else(|| {
                self.providers
                    .iter()
                    .position(|p| p.name == "Meta")
                    .unwrap()
            });
        self.range = ChartRange::from_index(self.range)
            .unwrap_or_default()
            .index();
        self.display = PriceChangeDisplay::from_saved_index(self.display)
            .unwrap_or_default()
            .saved_index();
        self.sort = self.sort.min(6);
        let legacy_view = ListView {
            sort: self.sort,
            display: self.display,
            descending: self.sort > 1,
        };
        if self.list_views.is_empty() {
            for w in &self.watchlists {
                self.list_views.insert(w.id, legacy_view);
            }
        }
        self.list_views.retain(|id, _| ids.contains(id));
        for view in self.list_views.values_mut() {
            view.sort = view.sort.min(6);
            view.display = PriceChangeDisplay::from_saved_index(view.display)
                .unwrap_or_default()
                .saved_index();
        }
        self.restore_list_view();
        self.scroll_offsets
            .retain(|id, offset| ids.contains(id) && offset.is_finite() && *offset >= 0.);
        self.saved_articles
            .retain(|a| !a.title.trim().is_empty() && a.safe_link().is_some());
        self.saved_articles.truncate(200);
        self.window_size = self
            .window_size
            .filter(|s| s.iter().all(|v| v.is_finite()) && s[0] >= 860. && s[1] >= 626.)
            .map(|s| [s[0].min(3000.), s[1].min(2000.)]);
        self.sidebar_width = if self.sidebar_width.is_finite() {
            self.sidebar_width.clamp(235., 360.)
        } else {
            default_sidebar_width()
        };
        self.ai_width = if self.ai_width.is_finite() {
            self.ai_width.clamp(285., 440.)
        } else {
            default_ai_width()
        };
        if !valid_ticker(&self.selected) {
            self.selected = "NVDA".into();
        }
    }
    pub fn list(&self) -> &Watchlist {
        self.watchlists
            .iter()
            .find(|w| w.id == self.active_list)
            .unwrap_or(&self.watchlists[0])
    }
    pub fn symbol(&self, t: &str) -> Symbol {
        self.symbols
            .iter()
            .find(|s| s.ticker == t)
            .cloned()
            .unwrap_or_else(|| Symbol::new(t, t, ""))
    }
    pub fn add(&mut self, s: Symbol) {
        if !self.symbols.iter().any(|x| x.ticker == s.ticker) {
            self.symbols.push(s.clone());
        }
        for w in &mut self.watchlists {
            if (w.id == 0 || w.id == self.active_list) && !w.symbols.contains(&s.ticker) {
                w.symbols.push(s.ticker.clone());
            }
        }
    }
    pub fn remember_list_view(&mut self) {
        self.list_views.insert(
            self.active_list,
            ListView {
                sort: self.sort,
                display: self.display,
                descending: self.sort_descending,
            },
        );
    }
    pub fn restore_list_view(&mut self) {
        let view = self
            .list_views
            .get(&self.active_list)
            .copied()
            .unwrap_or_default();
        self.sort = view.sort;
        self.display = view.display;
        self.sort_descending = view.descending;
    }
    pub fn name_available(&self, name: &str, except: Option<u64>) -> bool {
        !name.trim().is_empty()
            && name.chars().count() <= 60
            && !self
                .watchlists
                .iter()
                .any(|w| Some(w.id) != except && w.name.trim().eq_ignore_ascii_case(name.trim()))
    }
    pub fn next_list_id(&self) -> u64 {
        let used = |id| {
            self.watchlists.iter().any(|list| list.id == id) || self.list_views.contains_key(&id)
        };
        self.watchlists
            .iter()
            .map(|list| list.id)
            .chain(self.list_views.keys().copied())
            .max()
            .and_then(|id| id.checked_add(1))
            .unwrap_or_else(|| (1..).find(|&id| !used(id)).unwrap())
    }
    pub fn set_membership(&mut self, id: u64, ticker: &str, followed: bool) {
        if !valid_ticker(ticker) {
            return;
        }
        if !followed && id == 0 {
            for w in &mut self.watchlists {
                w.symbols.retain(|s| s != ticker);
            }
        } else if let Some(w) = self.watchlists.iter_mut().find(|w| w.id == id) {
            if followed && !w.symbols.iter().any(|s| s == ticker) {
                w.symbols.push(ticker.into());
            }
            if !followed {
                w.symbols.retain(|s| s != ticker);
            }
            if followed {
                let all = self.watchlists.iter_mut().find(|w| w.id == 0).unwrap();
                if !all.symbols.iter().any(|s| s == ticker) {
                    all.symbols.push(ticker.into());
                }
            }
        }
    }
    pub fn sorted_symbols(&self, quote: impl Fn(&str) -> Option<Quote>) -> Vec<String> {
        let mut symbols = self.list().symbols.clone();
        if self.sort == 0 {
            return symbols;
        }
        let values = symbols
            .iter()
            .map(|s| (s.clone(), quote(s)))
            .collect::<HashMap<_, _>>();
        symbols.sort_by(|a, b| {
            let key = |s: &String| {
                values
                    .get(s)
                    .and_then(Option::as_ref)
                    .and_then(|q| match self.sort {
                        2 => Some(q.price),
                        3 => q.watchlist_percent(),
                        // Treat zero market cap as missing for sorting while retaining raw provider data.
                        4 => q.market_cap.filter(|cap| *cap != 0.0),
                        6 => q.change(),
                        _ => None,
                    })
                    .filter(|v| v.is_finite())
            };
            let numeric = |a: Option<f64>, b: Option<f64>| {
                if matches!(self.sort, 3 | 4 | 6) {
                    return quote_value_order(a, b, self.sort_descending);
                }
                // The added price sort keeps its independent missing-last policy.
                match (a, b) {
                    (Some(a), Some(b)) => {
                        if self.sort_descending {
                            b.total_cmp(&a)
                        } else {
                            a.total_cmp(&b)
                        }
                    }
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            };
            let order = match self.sort {
                1 => a.cmp(b),
                5 => self
                    .symbol(a)
                    .name
                    .to_lowercase()
                    .cmp(&self.symbol(b).name.to_lowercase()),
                _ => return numeric(key(a), key(b)).then_with(|| a.cmp(b)),
            };
            (if self.sort_descending {
                order.reverse()
            } else {
                order
            })
            .then_with(|| a.cmp(b))
        });
        symbols
    }
    pub fn remove(&mut self, t: &str) {
        for w in &mut self.watchlists {
            if self.active_list == 0 || w.id == self.active_list {
                w.symbols.retain(|x| x != t);
            }
        }
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        atomic_json(path, self)
    }
}
pub fn valid_ticker(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 32
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || ".^-=:_/".contains(c))
}
pub fn atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let existed = parent.exists();
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    if !existed {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    let _ = existed;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(&bytes).map_err(|e| e.to_string())?;
    temp.persist(path).map(|_| ()).map_err(|e| e.to_string())
}

pub fn read_saved_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn read_with_legacy<T: serde::de::DeserializeOwned>(
    path: &Path,
    legacy: &Path,
) -> Result<Option<T>, String> {
    match read_saved_json(path)? {
        None => read_saved_json(legacy),
        present => Ok(present),
    }
}
pub fn data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("TENERUS_DATA_DIR") {
        return PathBuf::from(path);
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join("Library/Application Support/Tenerus");
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA")
            .filter(|v| !v.is_empty())
            .or_else(|| std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()))
        {
            return PathBuf::from(appdata).join("Tenerus");
        }
        if let Some(profile) = std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()) {
            return PathBuf::from(profile).join("AppData/Roaming/Tenerus");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            return PathBuf::from(xdg).join("tenerus");
        }
        if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
            return PathBuf::from(home).join(".local/share/tenerus");
        }
    }
    legacy_data_dir()
}
pub fn legacy_data_dir() -> PathBuf {
    PathBuf::from("data")
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Point {
    pub time: i64,
    pub close: f64,
    pub volume: Option<f64>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub previous: Option<f64>,
    #[serde(default)]
    pub reported_change: Option<f64>,
    #[serde(default)]
    pub reported_percent: Option<f64>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub volume: Option<f64>,
    pub market_cap: Option<f64>,
    pub pe: Option<f64>,
    pub week_high: Option<f64>,
    pub week_low: Option<f64>,
    pub currency: String,
    pub exchange: String,
    pub market_time: i64,
    pub fetched: i64,
    pub demo: bool,
    #[serde(default)]
    pub chart_price: bool,
    #[serde(default)]
    pub extended: Option<ExtendedQuote>,
    #[serde(default)]
    pub market_state: String,
    #[serde(default)]
    pub details: QuoteDetails,
}
#[derive(Clone, Default, Serialize, Deserialize, Debug)]
pub struct QuoteDetails {
    pub average_volume: Option<f64>,
    pub dividend_yield: Option<f64>,
    pub beta: Option<f64>,
    pub earnings_per_share: Option<f64>,
    pub earnings_start: Option<i64>,
    pub earnings_end: Option<i64>,
}
pub fn earnings_calendar(symbol: &str, timestamp: i64) -> Result<String, String> {
    if !valid_ticker(symbol) || timestamp <= 0 || timestamp > 253_402_128_000 {
        return Err("No usable earnings date was returned.".into());
    }
    let day = stamp(timestamp)[..10].replace('-', "");
    let next = stamp(timestamp + 86400)[..10].replace('-', "");
    let source = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Tenerus//Earnings calendar//EN\r\nBEGIN:VEVENT\r\nUID:{symbol}-{timestamp}@tenerus.local\r\nDTSTAMP:{}\r\nDTSTART;VALUE=DATE:{day}\r\nDTEND;VALUE=DATE:{next}\r\nSUMMARY:{symbol} earnings expected\r\nDESCRIPTION:Expected date from the public market feed (UTC date). Verify with company investor relations. No report time is confirmed.\r\nSTATUS:TENTATIVE\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        stamp(now())
            .replace(['-', ':'], "")
            .replace(' ', "T")
            .replace("TUTC", "00Z")
    );
    let mut calendar = String::new();
    for line in source.split("\r\n").filter(|s| !s.is_empty()) {
        let mut bytes = 0;
        for c in line.chars() {
            if bytes + c.len_utf8() > 75 {
                calendar.push_str("\r\n ");
                bytes = 1;
            }
            calendar.push(c);
            bytes += c.len_utf8();
        }
        calendar.push_str("\r\n");
    }
    Ok(calendar)
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ExtendedQuote {
    pub session: String,
    pub price: f64,
    pub market_time: i64,
    #[serde(default)]
    pub reported_change: Option<f64>,
    #[serde(default)]
    pub reported_percent: Option<f64>,
}
impl Quote {
    pub fn update(&mut self, incoming: Self) {
        let same_trade = self.market_time > 0
            && self.market_time == incoming.market_time
            && self.price == incoming.price
            && self.currency == incoming.currency
            && self.chart_price == incoming.chart_price;
        let current = (self.market_time, !self.chart_price, self.fetched);
        let next = (
            incoming.market_time,
            !incoming.chart_price,
            incoming.fetched,
        );
        let other = if next >= current {
            std::mem::replace(self, incoming)
        } else {
            incoming
        };
        if same_trade {
            self.previous = self.previous.or(other.previous);
            self.reported_change = self.reported_change.or(other.reported_change);
            self.reported_percent = self.reported_percent.or(other.reported_percent);
            self.open = self.open.or(other.open);
            self.high = self.high.or(other.high);
            self.low = self.low.or(other.low);
            self.volume = self.volume.or(other.volume);
            self.market_cap = self.market_cap.or(other.market_cap);
            if other.extended.as_ref().is_some_and(|next| {
                self.extended
                    .as_ref()
                    .is_none_or(|current| next.market_time > current.market_time)
            }) {
                self.extended = other.extended;
            }
        }
    }

    pub fn observation_time(&self) -> String {
        if self.chart_price {
            format!("Chart price at {}", stamp(self.market_time))
        } else {
            format!("Last trade {}", stamp(self.market_time))
        }
    }

    pub fn change_label(&self) -> String {
        match (self.change(), self.percent()) {
            (Some(change), Some(percent)) => format!("{change:+.2} ({percent:+.2}%)"),
            (Some(change), None) => format!("{change:+.2}"),
            (None, Some(percent)) => format!("{percent:+.2}%"),
            (None, None) => "Daily change unavailable".into(),
        }
    }

    pub fn change(&self) -> Option<f64> {
        // Prefer feed-reported price change; fall back to price minus previous close.
        self.reported_change.filter(|x| x.is_finite()).or_else(|| {
            self.previous
                .filter(|x| x.is_finite() && *x > 0.0)
                .map(|p| self.price - p)
                .filter(|x| x.is_finite())
        })
    }
    pub fn percent(&self) -> Option<f64> {
        self.reported_percent
            .filter(|x| x.is_finite())
            .or_else(|| self.watchlist_percent())
    }

    pub fn watchlist_percent(&self) -> Option<f64> {
        // Compute watchlist percentage consistently from price and change.
        quote_percent(self.price, self.change())
    }
}
impl ExtendedQuote {
    pub fn change(&self, regular_price: f64) -> Option<f64> {
        self.reported_change.filter(|x| x.is_finite()).or_else(|| {
            (regular_price.is_finite() && regular_price > 0.)
                .then_some(self.price - regular_price)
                .filter(|x| x.is_finite())
        })
    }
    pub fn percent(&self, regular_price: f64) -> Option<f64> {
        self.reported_percent.filter(|x| x.is_finite()).or_else(|| {
            // Same independently mapped rule, using this session's own pair.
            quote_percent(self.price, self.change(regular_price))
        })
    }
}

fn quote_percent(price: f64, change: Option<f64>) -> Option<f64> {
    if !price.is_finite() {
        return None;
    }
    change
        .filter(|change| change.is_finite())
        .map(|change| 100.0 * price_change_fraction(change, price))
        .filter(|percent| percent.is_finite())
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Market {
    pub quote: Quote,
    pub points: Vec<Point>,
    pub range: usize,
    #[serde(default)]
    pub regular_session: Option<[i64; 2]>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Article {
    pub title: String,
    pub publisher: String,
    pub date: String,
    pub link: String,
    pub demo: bool,
}
impl Article {
    pub fn safe_link(&self) -> Option<url::Url> {
        let u = url::Url::parse(&self.link).ok()?;
        (["https", "http"].contains(&u.scheme())
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none())
        .then_some(u)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct MarketCache {
    pub markets: Vec<Market>,
    #[serde(default)]
    pub quotes: Vec<Quote>,
    pub news: HashMap<String, Vec<Article>>,
    #[serde(default)]
    pub news_fetched: HashMap<String, i64>,
}
impl MarketCache {
    pub fn load(path: &Path) -> Self {
        if std::fs::metadata(path)
            .map(|m| m.len() > 12_000_000)
            .unwrap_or(true)
        {
            return Self::default();
        }
        let mut cache: Self = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let valid_quote =
            |q: &Quote| !q.demo && valid_ticker(&q.symbol) && q.price.is_finite() && q.price > 0.;
        cache.markets.retain(|m| {
            valid_quote(&m.quote)
                && m.range < RANGES.len()
                && m.points.len() <= MAX_CHART_POINTS
                && m.points.iter().all(|p| {
                    p.time > 0
                        && p.close.is_finite()
                        && p.close > 0.
                        && p.volume.is_none_or(|v| v.is_finite() && v >= 0.)
                })
                && m.points.windows(2).all(|w| w[0].time < w[1].time)
        });
        cache.markets.truncate(100);
        cache.quotes.retain(valid_quote);
        cache.quotes.truncate(100);
        cache
            .news
            .retain(|key, _| key == "__market" || valid_ticker(key));
        let at = now();
        cache
            .news_fetched
            .retain(|key, timestamp| cache.news.contains_key(key) && (1..=at).contains(timestamp));
        for articles in cache.news.values_mut() {
            articles.retain(|a| !a.demo && a.safe_link().is_some());
            articles.truncate(20);
        }
        cache
    }
}

pub fn demo_market(t: &str, range: usize) -> Option<Market> {
    ChartRange::from_index(range)?;
    let (price, pct, cap) = match t {
        "NVDA" => (186.52, 2.34, 4.54e12),
        "AAPL" => (247.79, 0.78, 3.67e12),
        "MSFT" => (514.23, 1.12, 3.82e12),
        "GOOGL" => (201.42, -0.42, 2.44e12),
        "AMZN" => (231.84, 1.45, 2.48e12),
        "TSLA" => (352.60, -1.27, 1.13e12),
        "META" => (746.18, 1.86, 1.87e12),
        "AMD" => (168.24, 3.12, 272e9),
        "^GSPC" => (6584.29, 0.64, 0.),
        "^DJI" => (46218.36, 0.32, 0.),
        "^IXIC" => (22108.14, 1.08, 0.),
        "BTC-USD" => (113420.68, 1.92, 2.24e12),
        "SPY" => (658.24, 0.64, 0.),
        "QQQ" => (587.31, 1.08, 0.),
        "GC=F" => (3674.10, 0.58, 0.),
        "EURUSD=X" => (1.1732, -0.12, 0.),
        _ => return None,
    };
    let seed = t
        .bytes()
        .fold(1u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64));
    let prev = price / (1.0 + pct / 100.0);
    let n = 96;
    let scale = if range == 0 {
        0.007
    } else {
        0.025 + range as f64 * 0.013
    };
    let step = match range {
        0 => 240,
        1 => 4500,
        2 => 27000,
        3 => 81000,
        4 => 162000,
        5 => 230000,
        6 => 328500,
        7 => 657000,
        8 => 1642500,
        9 => 3285000,
        _ => 6570000,
    };
    let end = 1_767_220_200i64;
    let points = (0..n)
        .map(|i| {
            let x = i as f64 / (n - 1) as f64;
            let wobble = (x * 29.0 + seed as f64 % 9.0).sin() * 0.5
                + (x * 73.0).sin() * 0.18
                + (x * 157.0).cos() * 0.08;
            let end_wobble =
                (29.0 + seed as f64 % 9.0).sin() * 0.5 + 73f64.sin() * 0.18 + 157f64.cos() * 0.08;
            Point {
                time: end - (n - 1 - i) as i64 * step,
                close: prev + (price - prev) * x + price * scale * (wobble - end_wobble * x),
                volume: Some(3e5 + (i as f64 * 2.3).sin().abs() * 1.4e6),
            }
        })
        .collect::<Vec<_>>();
    Some(Market {
        quote: Quote {
            symbol: t.into(),
            price,
            previous: Some(prev),
            reported_change: None,
            reported_percent: None,
            open: Some(prev * 0.999),
            high: Some(price * 1.004),
            low: Some(prev * 0.996),
            volume: Some(123.4e6),
            market_cap: if cap > 0. { Some(cap) } else { None },
            pe: Some(42.3),
            week_high: Some(price * 1.18),
            week_low: Some(price * 0.54),
            currency: "USD".into(),
            exchange: if t.starts_with('^') {
                "INDEX".into()
            } else {
                "NASDAQ".into()
            },
            market_time: end,
            fetched: end,
            demo: true,
            chart_price: false,
            extended: None,
            market_state: String::new(),
            details: QuoteDetails::default(),
        },
        points,
        range,
        regular_session: None,
    })
}
pub fn demo_news(name: &str) -> Vec<Article> {
    [(format!("What to watch when researching {name}"),"A useful research checklist: earnings, valuation, cash flow and the competitive landscape."),
     ("The story behind a stock chart".into(),"Price, volume and time tell different parts of the market story."),
     ("A longer view of market uncertainty".into(),"Compare several scenarios before making an investment decision.")]
    .into_iter().map(|(title,summary)|Article{title,publisher:"Tenerus guide".into(),date:summary.into(),link:String::new(),demo:true}).collect()
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Outlook {
    #[serde(default)]
    pub decision: Option<Decision>,
    #[serde(default)]
    pub confidence_percent: Option<f64>,
    #[serde(default)]
    pub bias: String,
    #[serde(default)]
    pub confidence: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub catalysts: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub suggestion: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Analysis {
    pub symbol: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub endpoint: String,
    pub generated: i64,
    pub context_time: i64,
    #[serde(default)]
    pub reference_price: Option<f64>,
    #[serde(default)]
    pub reference_currency: String,
    pub demo: bool,
    pub horizons: Vec<Outlook>,
    pub raw: String,
    pub context_notes: Vec<String>,
    #[serde(default)]
    pub usage: Option<InferenceUsage>,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Buy,
    Sell,
    DoNotBuy,
}
impl Decision {
    pub fn label(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
            Self::DoNotBuy => "DO NOT BUY",
        }
    }
    pub fn meaning(self) -> &'static str {
        match self {
            Self::Buy => "Consider buying at this horizon.",
            Self::Sell => "Consider selling if you already own it.",
            Self::DoNotBuy => "Wait; no new purchase for now.",
        }
    }
}
impl Outlook {
    pub fn confidence_value(&self) -> Option<f64> {
        self.confidence_percent
            .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
    }
    pub fn confidence_label(&self) -> String {
        self.confidence_value()
            .map(|v| format!("AI confidence {v:.0}%"))
            .unwrap_or_else(|| "Confidence % not supplied".into())
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct InferenceUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub estimated: bool,
    pub pricing_note: String,
}
impl InferenceUsage {
    pub fn cost_label(&self) -> String {
        match self.cost_usd.filter(|v| v.is_finite() && *v >= 0.) {
            Some(cost) => {
                let amount = if cost > 0. && cost < 0.000001 {
                    "<$0.000001".into()
                } else {
                    format!("${cost:.6}")
                };
                format!(
                    "{}cost {amount}",
                    if self.estimated {
                        "Estimated "
                    } else {
                        "Inference "
                    }
                )
            }
            None => "Inference cost unavailable".into(),
        }
    }
    pub fn detail(&self) -> String {
        let count = |n: Option<u64>| {
            n.map(|n| n.to_string())
                .unwrap_or_else(|| "not reported".into())
        };
        format!(
            "Input: {} tokens\nOutput: {} tokens (includes reasoning when reported)\nCached input: {} tokens\n{}",
            count(self.input_tokens),
            count(self.output_tokens),
            count(self.cached_input_tokens),
            self.pricing_note
        )
    }
}
#[derive(Serialize, Deserialize)]
pub struct ExportLists {
    pub version: u32,
    pub watchlists: Vec<Watchlist>,
    pub symbols: Vec<Symbol>,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn stamp(t: i64) -> String {
    if t <= 0 {
        return "Unavailable".into();
    }
    let days = t.div_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    y += if m <= 2 { 1 } else { 0 };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        t.rem_euclid(86400) / 3600,
        t.rem_euclid(3600) / 60
    )
}
pub fn price(v: f64) -> String {
    if v != 0. && v.abs() < 0.0001 {
        format!("{v:.3e}")
    } else if v.abs() < 10.0 {
        format!("{v:.4}")
    } else {
        format!("{v:.2}")
    }
}
pub fn compact(v: Option<f64>) -> String {
    match v {
        None => "—".into(),
        Some(v) if v.abs() >= 1e12 => format!("{:.2}T", v / 1e12),
        Some(v) if v.abs() >= 1e9 => format!("{:.2}B", v / 1e9),
        Some(v) if v.abs() >= 1e6 => format!("{:.2}M", v / 1e6),
        Some(v) if v.abs() >= 1e3 => format!("{:.1}K", v / 1e3),
        Some(v) => price(v),
    }
}
pub fn opt_price(v: Option<f64>) -> String {
    v.map(price).unwrap_or_else(|| "—".into())
}

pub fn save_secret(p: &Provider, s: &Secret) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let bytes = serde_json::to_vec(s).map_err(|e| e.to_string())?;
        security_framework::passwords::set_generic_password(
            "Tenerus inference",
            &p.key_account(),
            &bytes,
        )
        .map_err(|e| format!("Keychain could not save this connection: {e}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (p, s);
        Err("Key storage is session-only on this platform.".into())
    }
}
pub fn load_secret(p: &Provider) -> Secret {
    #[cfg(target_os = "macos")]
    {
        security_framework::passwords::get_generic_password("Tenerus inference", &p.key_account())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = p;
        Secret::default()
    }
}
pub fn forget_secret(p: &Provider) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        match security_framework::passwords::delete_generic_password(
            "Tenerus inference",
            &p.key_account(),
        ) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == -25300 => Ok(()),
            Err(e) => Err(format!("Keychain could not remove the saved key: {e}")),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = p;
        Ok(())
    }
}

pub type Markets = HashMap<(String, usize), Market>;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chart_range_settings_keep_legacy_choices_and_repair_only_the_invalid_field() {
        let mut prefs = Preferences::default();
        prefs.selected = "AAPL".into();
        prefs.providers[0].model = "keep-this-model".into();
        let saved = serde_json::to_value(&prefs).unwrap();
        for range in ChartRange::ALL {
            for encoded in [
                serde_json::json!(range.index()),
                serde_json::json!(range.raw_value()),
            ] {
                let mut value = saved.clone();
                value["range"] = encoded;
                let mut restored: Preferences = serde_json::from_value(value).unwrap();
                restored.normalize();
                assert_eq!(restored.range, range.index());
                assert_eq!(restored.selected, "AAPL");
                assert_eq!(restored.providers[0].model, "keep-this-model");
                assert_eq!(
                    serde_json::to_value(restored).unwrap()["range"],
                    range.index()
                );
            }
        }
        for invalid in [
            serde_json::json!(11),
            serde_json::json!(u64::MAX),
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("not-a-range"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!([]),
        ] {
            let mut value = saved.clone();
            value["range"] = invalid;
            let restored: Preferences = serde_json::from_value(value).unwrap();
            assert_eq!(restored.range, 0, "damaged range must not become ALL");
            assert_eq!(restored.selected, "AAPL");
            assert_eq!(restored.providers[0].model, "keep-this-model");
        }
        prefs.range = usize::MAX;
        prefs.normalize();
        assert_eq!(prefs.range, 0);
        let mut missing = saved;
        missing.as_object_mut().unwrap().remove("range");
        assert_eq!(
            serde_json::from_value::<Preferences>(missing)
                .unwrap()
                .range,
            0
        );
    }

    #[test]
    fn adding_meta_preserves_legacy_connections_and_the_selected_custom_provider() {
        let mut p = Preferences::default();
        p.providers.retain(|provider| provider.name != "Meta");
        let custom = p.providers.iter().position(|p| p.name == "Custom").unwrap();
        p.providers[custom].endpoint = "http://localhost:9999/v1".into();
        p.providers[custom].model = "local-model".into();
        p.providers[custom].remember = true;
        p.providers[0].model = "previous-openai-model".into();
        let account = p.providers[custom].key_account();
        p.active_provider = custom;
        p.normalize();
        assert_eq!(p.providers.len(), 8);
        assert_eq!(p.providers[p.active_provider].name, "Custom");
        assert_eq!(p.providers[p.active_provider].model, "local-model");
        assert_eq!(p.providers[p.active_provider].key_account(), account);
        assert!(p.providers[p.active_provider].remember);
        assert_eq!(p.providers[0].model, "previous-openai-model");
        assert_eq!(
            p.providers.iter().find(|p| p.name == "Meta").unwrap().model,
            META_MODEL
        );
        p.normalize();
        assert_eq!(p.providers.len(), 8);
        assert_eq!(p.providers[p.active_provider].name, "Custom");
    }
    #[test]
    fn meta_is_selected_for_new_or_unconfigured_legacy_settings() {
        let mut p = Preferences::default();
        assert_eq!(p.providers[p.active_provider].model, META_MODEL);
        p.providers.retain(|provider| provider.name != "Meta");
        p.active_provider = 0;
        p.normalize();
        assert_eq!(p.providers[p.active_provider].name, "Meta");
        // An explicit later selection survives even before its model is entered.
        p.active_provider = 0;
        p.normalize();
        assert_eq!(p.active_provider, 0);
    }
    #[test]
    fn older_saved_analysis_loads_with_unknown_cost() {
        let analysis: Analysis = serde_json::from_value(serde_json::json!({
            "symbol":"META", "provider":"OpenAI", "model":"old", "generated":1,
            "context_time":1, "demo":false, "horizons":[], "raw":"previous answer", "context_notes":[]
        })).unwrap();
        assert!(analysis.usage.is_none());
        assert!(analysis.reference_price.is_none());
        assert!(analysis.reference_currency.is_empty());
        assert_eq!(analysis.raw, "previous answer");
        assert_eq!(
            InferenceUsage::default().cost_label(),
            "Inference cost unavailable"
        );
    }
    #[test]
    fn learning_preferences_default_for_legacy_settings_and_preserve_supported_choices() {
        let mut old = serde_json::to_value(Preferences::default()).unwrap();
        old.as_object_mut().unwrap().remove("memory_days");
        let mut restored: Preferences = serde_json::from_value(old).unwrap();
        assert_eq!(restored.memory_days, 180);
        let symbols = restored.list().symbols.clone();
        for days in [0, 90, 180, 365, 999] {
            restored.memory_days = days;
            restored.normalize();
            assert_eq!(restored.memory_days, if days == 999 { 180 } else { days });
            assert_eq!(restored.list().symbols, symbols);
        }
    }
    #[test]
    fn union_and_removal_do_not_corrupt_other_lists() {
        let mut p = Preferences::default();
        p.active_list = 1;
        p.add(Symbol::new("SPY", "ETF", "NYSE"));
        assert!(p.watchlists[0].symbols.contains(&"SPY".into()));
        assert!(p.watchlists[1].symbols.contains(&"SPY".into()));
        p.remove("SPY");
        assert!(p.watchlists[0].symbols.contains(&"SPY".into()));
        p.active_list = 0;
        p.remove("NVDA");
        assert!(
            p.watchlists
                .iter()
                .all(|w| !w.symbols.contains(&"NVDA".into()))
        );
    }
    #[test]
    fn missing_quote_denominator_is_missing() {
        let mut q = demo_market("NVDA", 0).unwrap().quote;
        q.previous = Some(0.);
        assert_eq!(q.percent(), None);
        q.previous = None;
        assert_eq!(q.change(), None);
    }
    #[test]
    fn percent_change_uses_matching_price_and_change_without_previous_close() {
        let mut q = demo_market("NVDA", 0).unwrap().quote;
        q.price = 110.;
        q.previous = None;
        q.reported_change = Some(10.);
        q.reported_percent = None;
        assert_eq!(q.percent(), Some(10.));
        // An unrelated previous close must not change the source pair's basis.
        q.previous = Some(50.);
        assert_eq!(q.percent(), Some(10.));
        q.reported_change = Some(-10.);
        assert!((q.percent().unwrap() - (-100. / 12.)).abs() < 1e-12);
        q.reported_percent = Some(7.25);
        assert_eq!(q.percent(), Some(7.25));
        q.reported_percent = None;
        q.reported_change = Some(110.);
        assert_eq!(q.percent(), Some(0.));
        q.reported_change = Some(120.);
        assert_eq!(q.percent(), Some(0.));
        q.previous = None;
        q.reported_change = None;
        assert_eq!(q.percent(), None);
        q.reported_change = Some(f64::INFINITY);
        assert_eq!(q.percent(), None);
        q.reported_change = Some(10.);
        q.price = f64::NAN;
        assert_eq!(q.percent(), None);
    }
    #[test]
    fn extended_percent_uses_its_own_session_pair() {
        let mut extended = ExtendedQuote {
            session: "After hours".into(),
            price: 120.,
            market_time: 1,
            reported_change: Some(20.),
            reported_percent: None,
        };
        assert_eq!(extended.percent(110.), Some(20.));
        assert_eq!(extended.percent(f64::NAN), Some(20.));
        extended.reported_percent = Some(19.5);
        assert_eq!(extended.percent(110.), Some(19.5));
        extended.reported_percent = None;
        extended.reported_change = None;
        assert_eq!(extended.percent(100.), Some(20.));
        assert_eq!(extended.percent(f64::NAN), None);
    }
    #[test]
    fn older_cached_quotes_load_without_reported_change_fields() {
        let q = demo_market("NVDA", 0).unwrap().quote;
        let mut legacy = serde_json::to_value(&q).unwrap();
        legacy.as_object_mut().unwrap().remove("reported_change");
        legacy.as_object_mut().unwrap().remove("reported_percent");
        let restored: Quote = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.change(), q.change());
        assert_eq!(restored.percent(), q.percent());
    }
    #[test]
    fn unknown_tickers_never_get_invented_quotes() {
        assert!(demo_market("NOTREAL", 0).is_none());
    }
    #[test]
    fn list_views_survive_serialization_and_switching() {
        let mut p = Preferences::default();
        p.sort = 5;
        p.display = 2;
        p.sort_descending = true;
        p.remember_list_view();
        p.active_list = 1;
        p.restore_list_view();
        assert_eq!((p.sort, p.display, p.sort_descending), (0, 1, false));
        p.sort = 6;
        p.display = 1;
        p.remember_list_view();
        let mut restored: Preferences =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        restored.normalize();
        assert_eq!((restored.sort, restored.display), (6, 1));
        restored.active_list = 0;
        restored.restore_list_view();
        assert_eq!(
            (restored.sort, restored.display, restored.sort_descending),
            (5, 2, true)
        );
    }
    #[test]
    fn watchlist_display_default_preserves_existing_settings_and_repairs_invalid_values() {
        let fresh = Preferences::default();
        assert_eq!((fresh.sort, fresh.display), (0, 1));
        for saved_index in 0..3 {
            // Old settings without per-list views must retain their meaning.
            let mut old = fresh.clone();
            old.display = saved_index;
            let mut restored: Preferences =
                serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
            restored.normalize();
            assert_eq!(restored.display, saved_index);
            restored.active_list = 1;
            restored.restore_list_view();
            assert_eq!(restored.display, saved_index);
            // A newly created list starts with the default currency change display.
            restored.watchlists.push(Watchlist {
                id: 7,
                name: "New".into(),
                symbols: vec![],
                selected: String::new(),
            });
            restored.active_list = 7;
            restored.restore_list_view();
            assert_eq!((restored.sort, restored.display), (0, 1));
        }
        let mut damaged = fresh;
        damaged.display = usize::MAX;
        damaged.normalize();
        assert_eq!(damaged.display, 1);
        damaged.list_views.get_mut(&0).unwrap().display = 3;
        damaged.normalize();
        assert_eq!(damaged.display, 1);
    }
    #[test]
    fn both_numeric_sort_directions_keep_missing_values_last() {
        let mut p = Preferences::default();
        p.watchlists[0].symbols = vec!["UNKNOWN".into(), "NVDA".into(), "AAPL".into()];
        p.sort = 2;
        let quotes = |s: &str| demo_market(s, 0).map(|m| m.quote);
        assert_eq!(p.sorted_symbols(quotes), ["NVDA", "AAPL", "UNKNOWN"]);
        p.sort_descending = true;
        assert_eq!(p.sorted_symbols(quotes), ["AAPL", "NVDA", "UNKNOWN"]);
        p.sort = 5;
        p.sort_descending = false;
        assert_eq!(p.sorted_symbols(quotes), ["AAPL", "NVDA", "UNKNOWN"]);
    }
    #[test]
    fn numeric_sorts_handle_missing_values_and_zero_market_cap() {
        let mut p = Preferences::default();
        p.watchlists[0].symbols = ["MISSING", "HIGH", "LOW", "ZERO"]
            .map(String::from)
            .to_vec();
        let quotes = |symbol: &str| {
            if symbol == "MISSING" {
                return None;
            }
            let mut q = demo_market("NVDA", 0).unwrap().quote;
            q.price = 110.;
            q.previous = None;
            q.reported_percent = None;
            q.reported_change = Some(match symbol {
                "HIGH" => 10.,
                "LOW" => -10.,
                _ => 0.,
            });
            q.market_cap = Some(match symbol {
                "HIGH" => 200.,
                "LOW" => 100.,
                _ => 0.,
            });
            Some(q)
        };
        for sort in [3, 6] {
            p.sort = sort;
            p.sort_descending = false;
            assert_eq!(p.sorted_symbols(quotes), ["MISSING", "LOW", "ZERO", "HIGH"]);
            p.sort_descending = true;
            assert_eq!(p.sorted_symbols(quotes), ["HIGH", "ZERO", "LOW", "MISSING"]);
        }
        p.sort = 4;
        p.sort_descending = false;
        assert_eq!(p.sorted_symbols(quotes), ["MISSING", "ZERO", "LOW", "HIGH"]);
        p.sort_descending = true;
        assert_eq!(p.sorted_symbols(quotes), ["HIGH", "LOW", "MISSING", "ZERO"]);
        // Zero capitalization stays intact as provider data; only its sort key is absent.
        assert_eq!(quotes("ZERO").unwrap().market_cap, Some(0.));
    }
    #[test]
    fn watchlist_percentage_order_uses_price_and_change_even_with_reported_percentages() {
        let mut p = Preferences::default();
        p.watchlists[0].symbols = ["GAIN", "LOSS", "NOCHANGE"].map(String::from).to_vec();
        p.sort = 3;
        let quotes = |symbol: &str| {
            let mut quote = demo_market("NVDA", 0).unwrap().quote;
            quote.price = 100.;
            quote.previous = None;
            (quote.reported_change, quote.reported_percent) = match symbol {
                "GAIN" => (Some(10.), Some(-30.)),
                "LOSS" => (Some(-10.), Some(30.)),
                _ => (None, Some(5.)),
            };
            Some(quote)
        };
        assert_eq!(p.sorted_symbols(quotes), ["NOCHANGE", "LOSS", "GAIN"]);
        p.sort_descending = true;
        assert_eq!(p.sorted_symbols(quotes), ["GAIN", "LOSS", "NOCHANGE"]);
        assert_eq!(quotes("GAIN").unwrap().percent(), Some(-30.));
    }
    #[test]
    fn legacy_settings_and_damaged_list_ids_are_repaired_without_losing_symbols() {
        let mut p = Preferences::default();
        p.watchlists[0].symbols = vec!["NVDA".into(), "NVDA".into(), "INVALID TICKER".into()];
        p.watchlists[2].id = 1;
        p.sort = 4;
        p.display = 2;
        p.normalize();
        let ids = p
            .watchlists
            .iter()
            .map(|w| w.id)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(ids.len(), p.watchlists.len());
        assert_eq!(
            p.watchlists[0]
                .symbols
                .iter()
                .filter(|s| *s == "NVDA")
                .count(),
            1
        );
        assert!(!p.watchlists[0].symbols.contains(&"INVALID TICKER".into()));
        assert_eq!((p.sort, p.display, p.sort_descending), (4, 2, true));
        assert!(!p.name_available("  TECHNOLOGY ", None));
        assert!(p.name_available("Technology", Some(1)));
    }
    #[test]
    fn membership_changes_preserve_the_aggregate_and_other_lists() {
        let mut p = Preferences::default();
        p.set_membership(2, "SPY", true);
        assert!(p.watchlists[0].symbols.contains(&"SPY".into()));
        assert!(p.watchlists[2].symbols.contains(&"SPY".into()));
        p.set_membership(1, "SPY", true);
        p.set_membership(2, "SPY", false);
        assert!(p.watchlists[1].symbols.contains(&"SPY".into()));
        p.set_membership(0, "SPY", false);
        assert!(
            p.watchlists
                .iter()
                .all(|w| !w.symbols.contains(&"SPY".into()))
        );
    }
    #[test]
    fn retained_data_survives_a_restart_without_accepting_demo_values() {
        let path = std::env::temp_dir().join(format!(
            "tenerus-cache-test-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut real = demo_market("NVDA", 0).unwrap();
        real.quote.demo = false;
        let sample = demo_market("AAPL", 0).unwrap();
        atomic_json(
            &path,
            &MarketCache {
                markets: vec![real, sample],
                quotes: vec![],
                news: HashMap::new(),
                news_fetched: HashMap::new(),
            },
        )
        .unwrap();
        let loaded = MarketCache::load(&path);
        std::fs::remove_file(path).unwrap();
        assert_eq!(loaded.markets.len(), 1);
        assert_eq!(loaded.markets[0].quote.symbol, "NVDA");
        assert!(!loaded.markets[0].quote.demo);
        assert_eq!(loaded.markets[0].points.len(), 96);
    }
    #[test]
    fn earnings_export_is_tentative_and_crosses_leap_day_correctly() {
        let calendar = earnings_calendar("AAPL", 1709164800).unwrap();
        assert!(calendar.contains("DTSTART;VALUE=DATE:20240229\r\n"));
        assert!(calendar.contains("DTEND;VALUE=DATE:20240301\r\n"));
        assert!(calendar.contains("STATUS:TENTATIVE\r\n"));
        assert!(calendar.split("\r\n").all(|s| s.len() <= 75));
        let stamp = calendar
            .lines()
            .find(|s| s.starts_with("DTSTAMP:"))
            .unwrap();
        assert_eq!(stamp.len(), 24);
        assert!(stamp.ends_with('Z'));
        assert!(earnings_calendar("BAD\nSUMMARY:invalid", 1709164800).is_err());
        assert!(earnings_calendar("AAPL", 0).is_err());
    }
    #[test]
    fn utc_dates_and_prices() {
        assert_eq!(stamp(86400), "1970-01-02 00:00 UTC");
        assert_eq!(stamp(1709164800), "2024-02-29 00:00 UTC");
        assert_eq!(compact(None), "—");
    }
    #[test]
    fn chart_range_identifiers_and_defaults_stay_consistent() {
        let cases = [
            (0, "oneDay"),
            (1, "oneWeek"),
            (2, "oneMonth"),
            (3, "threeMonths"),
            (4, "sixMonths"),
            (5, "yearToDate"),
            (6, "oneYear"),
            (7, "twoYears"),
            (8, "fiveYears"),
            (9, "tenYears"),
            (10, "all"),
        ];
        assert_eq!(ChartRange::ALL.len(), cases.len());
        for (tag, raw) in cases {
            let actual = ChartRange::from_index(tag).unwrap();
            assert_eq!(actual.raw_value(), raw);
            assert_eq!(ChartRange::from_raw(raw), Some(actual));
        }
        assert!(ChartRange::from_index(cases.len()).is_none());
        assert_eq!(REFRESH_SECONDS, 60);
        assert_eq!(PUBLIC_TIMEOUT, 18);
        assert_eq!(SHOT_FRAMES, 120);
        assert_eq!(RANGES.join(","), "1D,1W,1M,3M,6M,YTD,1Y,2Y,5Y,10Y,ALL");
    }
}
