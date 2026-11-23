use crate::journal::DAY;
use crate::model::{PUBLIC_TIMEOUT, atomic_json, now, read_saved_json, valid_ticker};
use crate::network::{self, Request};
use crate::scenarios::{Close, History};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use url::Url;

const MAX_INPUT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_SESSIONS: usize = 4_000;
const MAX_IMPORTS: usize = 100;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Source {
    #[default]
    TwelveData,
    AlphaVantage,
    Custom,
    File,
}

impl Source {
    pub const ALL: [Self; 4] = [
        Self::TwelveData,
        Self::AlphaVantage,
        Self::Custom,
        Self::File,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TwelveData => "Twelve Data",
            Self::AlphaVantage => "Alpha Vantage",
            Self::Custom => "Custom endpoint",
            Self::File => "CSV or JSON file",
        }
    }

    pub fn price_basis(self) -> &'static str {
        match self {
            Self::TwelveData => "Split-adjusted daily prices",
            Self::AlphaVantage => "Split and dividend-adjusted daily prices",
            Self::Custom | Self::File => "Daily prices as supplied",
        }
    }
}

#[derive(Clone, Default)]
pub struct ImportRequest {
    pub source: Source,
    pub ticker: String,
    pub currency: String,
    pub exchange: String,
    pub endpoint: String,
    pub key: String,
    pub file: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportedHistory {
    pub source: Source,
    pub history: Arc<History>,
}

impl ImportRequest {
    pub fn fetch(&self) -> Result<ImportedHistory, String> {
        let ticker = self.ticker.trim().to_ascii_uppercase();
        if !valid_ticker(&ticker) {
            return Err("Enter a valid ticker as it appears at the selected source.".into());
        }
        let key = self.key.trim();
        if key.len() > 2_048 || key.chars().any(char::is_control) {
            return Err("Enter a valid API key or token.".into());
        }
        let fetched = now();
        let history = match self.source {
            Source::TwelveData => {
                if key.is_empty() {
                    return Err("Enter your Twelve Data API key.".into());
                }
                let mut url = Url::parse("https://api.twelvedata.com/time_series").unwrap();
                url.query_pairs_mut()
                    .append_pair("symbol", &ticker)
                    .append_pair("interval", "1day")
                    .append_pair("outputsize", &MAX_SESSIONS.to_string());
                let exchange = self.exchange.trim();
                if !exchange.is_empty() {
                    url.query_pairs_mut().append_pair("exchange", exchange);
                }
                let body = network::send(&request(url, Some(format!("apikey {key}"))))?;
                parse_twelve(&parse_json(&body)?, &ticker, fetched)?
            }
            Source::AlphaVantage => {
                if key.is_empty() {
                    return Err("Enter your Alpha Vantage API key.".into());
                }
                let currency = normalize_currency(&self.currency)?;
                let mut url = Url::parse("https://www.alphavantage.co/query").unwrap();
                url.query_pairs_mut()
                    .append_pair("function", "TIME_SERIES_DAILY_ADJUSTED")
                    .append_pair("symbol", &ticker)
                    .append_pair("outputsize", "full");
                let body = network::send_with_api_query_key(&request(url, None), key)?;
                parse_alpha(&parse_json(&body)?, &ticker, &currency, fetched)?
            }
            Source::Custom => {
                let url = network::validate_endpoint(&self.endpoint)?;
                let auth = (!key.is_empty()).then(|| format!("Bearer {key}"));
                let body = network::send(&request(url, auth))?;
                parse_custom(&body, &ticker, &self.currency, fetched)?
            }
            Source::File => {
                if self.file.as_os_str().is_empty() {
                    return Err("Choose a CSV or JSON price file.".into());
                }
                let body = read_file(&self.file)?;
                parse_custom(&body, &ticker, &self.currency, fetched)?
            }
        };
        Ok(ImportedHistory {
            source: self.source,
            history: Arc::new(history),
        })
    }
}

fn request(url: Url, authorization: Option<String>) -> Request {
    let mut headers = vec![("Accept".into(), "application/json, text/csv".into())];
    if let Some(value) = authorization {
        headers.push(("Authorization".into(), value));
    }
    Request {
        url: url.to_string(),
        headers,
        body: None,
        timeout: PUBLIC_TIMEOUT,
    }
}

fn read_file(path: &Path) -> Result<String, String> {
    let mut body = Vec::new();
    File::open(path)
        .and_then(|file| file.take(MAX_INPUT_BYTES + 1).read_to_end(&mut body))
        .map_err(|error| format!("Could not read the price file. {error}"))?;
    if body.len() as u64 > MAX_INPUT_BYTES {
        return Err("Choose a price file up to 4 MB.".into());
    }
    String::from_utf8(body).map_err(|_| "Use a UTF-8 CSV or JSON price file.".into())
}

fn parse_json(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|_| "The data source returned invalid JSON.".into())
}

fn check_ticker(actual: &str, expected: &str) -> Result<(), String> {
    if actual.trim().eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err("The returned symbol does not match the requested ticker.".into())
    }
}

fn normalize_currency(currency: &str) -> Result<String, String> {
    let currency = currency.trim();
    if !(3..=4).contains(&currency.len()) || !currency.bytes().all(|c| c.is_ascii_alphabetic()) {
        return Err("Supply the price currency, such as USD, EUR or GBP.".into());
    }
    // GBp (pence) and GBP (pounds) are distinct price units.
    Ok(if currency == "GBp" {
        currency.into()
    } else {
        currency.to_ascii_uppercase()
    })
}

fn parse_twelve(data: &Value, ticker: &str, fetched: i64) -> Result<History, String> {
    if data["status"].as_str() == Some("error") {
        return Err(match data["code"].as_i64() {
            Some(429) => "Twelve Data rate limit reached. Try again later.",
            Some(401 | 403) => "Twelve Data refused access. Check your key and plan.",
            _ => "Twelve Data did not supply daily prices. Check the ticker, exchange and plan.",
        }
        .into());
    }
    let meta = &data["meta"];
    check_ticker(meta["symbol"].as_str().unwrap_or_default(), ticker)?;
    if meta["interval"].as_str() != Some("1day") {
        return Err("Twelve Data did not return daily data.".into());
    }
    let currency = normalize_currency(meta["currency"].as_str().unwrap_or_default())?;
    let rows = data["values"]
        .as_array()
        .ok_or("Twelve Data did not return daily closes.")?;
    let closes = rows
        .iter()
        .map(|row| read_close(row["datetime"].as_str().unwrap_or_default(), &row["close"]))
        .collect::<Result<_, _>>()?;
    completed_history(ticker, currency, fetched, closes, true)
}

fn parse_alpha(
    data: &Value,
    ticker: &str,
    currency: &str,
    fetched: i64,
) -> Result<History, String> {
    let series = data["Time Series (Daily)"].as_object().ok_or_else(|| {
        if data.get("Note").is_some() {
            "Alpha Vantage rate limit reached. Try again later."
        } else {
            "Alpha Vantage did not return adjusted prices. Check the ticker, API key and full daily adjusted history access."
        }
    })?;
    check_ticker(
        data["Meta Data"]["2. Symbol"].as_str().unwrap_or_default(),
        ticker,
    )?;
    let closes = series
        .iter()
        .map(|(date, row)| read_close(date, &row["5. adjusted close"]))
        .collect::<Result<_, _>>()?;
    completed_history(ticker, currency.into(), fetched, closes, true)
}

fn parse_custom(body: &str, ticker: &str, currency: &str, fetched: i64) -> Result<History, String> {
    let body = body.trim_start_matches('\u{feff}').trim();
    if body.starts_with('{') {
        let data = parse_json(body)?;
        check_ticker(data["symbol"].as_str().unwrap_or_default(), ticker)?;
        let currency = normalize_currency(data["currency"].as_str().unwrap_or_default())?;
        let rows = data["prices"]
            .as_array()
            .ok_or("JSON needs a prices array.")?;
        let closes = rows
            .iter()
            .map(|row| read_close(row["date"].as_str().unwrap_or_default(), &row["close"]))
            .collect::<Result<_, _>>()?;
        return completed_history(ticker, currency, fetched, closes, false);
    }
    let mut lines = body.lines().filter(|line| !line.trim().is_empty());
    let headers = csv_row(lines.next().ok_or("The source contains no prices.")?)?;
    let column = |names: &[&str]| {
        headers
            .iter()
            .position(|h| names.iter().any(|name| h.trim().eq_ignore_ascii_case(name)))
    };
    let date = column(&["date", "timestamp", "datetime"]).ok_or("CSV needs a date column.")?;
    let close = column(&["close"]).ok_or("CSV needs a close column.")?;
    let symbol = column(&["symbol", "ticker"]);
    let currency_col = column(&["currency"]);
    let mut currency = if currency.trim().is_empty() {
        None
    } else {
        Some(normalize_currency(currency)?)
    };
    let mut closes = Vec::new();
    for (i, line) in lines.enumerate() {
        let row = csv_row(line)?;
        if row.len() != headers.len() {
            return Err(format!(
                "CSV row {} has the wrong number of columns.",
                i + 2
            ));
        }
        if let Some(symbol) = symbol {
            check_ticker(&row[symbol], ticker)?;
        }
        if let Some(index) = currency_col {
            let reported = normalize_currency(&row[index])?;
            if currency.as_ref().is_some_and(|unit| *unit != reported) {
                return Err("CSV contains mismatched currencies.".into());
            }
            currency = Some(reported);
        }
        closes.push(
            close_at(&row[date], row[close].trim().parse().unwrap_or(f64::NAN))
                .map_err(|error| format!("CSV row {}. {error}", i + 2))?,
        );
    }
    let currency = currency.ok_or("Enter a currency for CSV without a currency column.")?;
    completed_history(ticker, currency, fetched, closes, false)
}

fn csv_row(line: &str) -> Result<Vec<String>, String> {
    let mut cells = Vec::new();
    let mut rest = line.trim();
    loop {
        if let Some(tail) = rest.strip_prefix('"') {
            let mut cell = String::new();
            let mut quoted = tail;
            loop {
                let (part, suffix) = quoted
                    .split_once('"')
                    .ok_or("CSV contains an unclosed quote.")?;
                cell.push_str(part);
                if let Some(tail) = suffix.strip_prefix('"') {
                    cell.push('"');
                    quoted = tail;
                } else {
                    rest = suffix.trim_start();
                    break;
                }
            }
            cells.push(cell);
        } else {
            let separator = rest.find(',').unwrap_or(rest.len());
            cells.push(rest[..separator].trim().into());
            rest = &rest[separator..];
        }
        if rest.is_empty() {
            return Ok(cells);
        }
        rest = rest
            .strip_prefix(',')
            .ok_or("CSV contains unexpected text after a quoted field.")?
            .trim_start();
    }
}

fn read_close(date: &str, close: &Value) -> Result<Close, String> {
    let close = close
        .as_f64()
        .or_else(|| close.as_str()?.trim().parse().ok())
        .unwrap_or(f64::NAN);
    close_at(date, close)
}

fn close_at(date: &str, price: f64) -> Result<Close, String> {
    let time = date_timestamp(date.trim()).ok_or("Use valid daily dates in YYYY-MM-DD format.")?;
    if !price.is_finite() || price <= 0. {
        return Err("Closing prices must be positive numbers.".into());
    }
    Ok(Close { time, price })
}

fn date_timestamp(date: &str) -> Option<i64> {
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(i, byte)| i == 4 || i == 7 || byte.is_ascii_digit())
    {
        return None;
    }
    let year: i64 = date[..4].parse().ok()?;
    let month: i64 = date[5..7].parse().ok()?;
    let day: i64 = date[8..].parse().ok()?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year < 1970 || !(1..=12).contains(&month) || !(1..=days[(month - 1) as usize]).contains(&day)
    {
        return None;
    }
    // Inverse of model::stamp's Gregorian calendar conversion.
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let relative_year = year - era * 400;
    let month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month + 2) / 5 + day - 1;
    let day_of_era = relative_year * 365 + relative_year / 4 - relative_year / 100 + day_of_year;
    let unix_days = era * 146_097 + day_of_era - 719_468;
    (unix_days > 0).then_some(unix_days * DAY)
}

fn completed_history(
    ticker: &str,
    currency: String,
    fetched: i64,
    mut closes: Vec<Close>,
    provider: bool,
) -> Result<History, String> {
    if provider {
        // A date-only response cannot establish whether its current bar has
        // closed. Keep dates that have elapsed in all market time zones.
        let first_open_day = (fetched - DAY / 2).div_euclid(DAY) * DAY;
        closes.retain(|bar| bar.time < first_open_day);
    }
    if closes.iter().any(|bar| bar.time > fetched) {
        return Err("The source contains future trading dates.".into());
    }
    closes.sort_unstable_by_key(|bar| bar.time);
    if closes.windows(2).any(|pair| pair[0].time == pair[1].time) {
        return Err("Each trading date must occur only once.".into());
    }
    if closes.len() > MAX_SESSIONS {
        closes.drain(..closes.len() - MAX_SESSIONS);
    }
    let history = History {
        ticker: ticker.into(),
        currency,
        fetched,
        sample: false,
        closes,
    };
    if history.closes.len() < 2 || !history.valid() {
        return Err("The source must supply at least two completed daily closing prices.".into());
    }
    Ok(history)
}

pub struct Imports {
    path: Option<PathBuf>,
    histories: HashMap<String, Arc<ImportedHistory>>,
    pub load_error: Option<String>,
}

impl Imports {
    pub fn load(path: Option<PathBuf>) -> Self {
        let loaded = path.as_ref().map_or(Ok(None), |path| {
            if path.metadata().is_ok_and(|meta| meta.len() > 40_000_000) {
                return Err("Saved imports exceed the supported size.".into());
            }
            read_saved_json::<Vec<ImportedHistory>>(path)
        });
        let (histories, load_error) = match loaded {
            Ok(data) => (
                data.unwrap_or_default()
                    .into_iter()
                    .filter(|entry| {
                        !entry.history.sample
                            && entry.history.closes.len() >= 2
                            && entry.history.valid()
                    })
                    .take(MAX_IMPORTS)
                    .map(|entry| (entry.history.ticker.clone(), Arc::new(entry)))
                    .collect(),
                None,
            ),
            Err(error) => (HashMap::new(), Some(error)),
        };
        Self {
            path,
            histories,
            load_error,
        }
    }

    pub fn get(&self, ticker: &str) -> Option<&Arc<ImportedHistory>> {
        self.histories.get(ticker)
    }

    pub fn insert(&mut self, entry: ImportedHistory) -> Result<(), String> {
        let ticker = entry.history.ticker.clone();
        if self.histories.len() >= MAX_IMPORTS && !self.histories.contains_key(&ticker) {
            return Err("Remove an older import before adding another ticker.".into());
        }
        let mut updated = self.histories.clone();
        updated.insert(ticker, Arc::new(entry));
        self.save(updated)
    }

    pub fn remove(&mut self, ticker: &str) -> Result<(), String> {
        let mut updated = self.histories.clone();
        updated.remove(ticker);
        self.save(updated)
    }

    fn save(&mut self, updated: HashMap<String, Arc<ImportedHistory>>) -> Result<(), String> {
        if let Some(path) = &self.path {
            if self.load_error.is_some() {
                return Err(
                    "Saved imports could not be read. The existing file was preserved.".into(),
                );
            }
            let mut entries = updated.values().map(Arc::as_ref).collect::<Vec<_>>();
            entries.sort_unstable_by_key(|entry| &entry.history.ticker);
            atomic_json(path, &entries)?;
        }
        self.histories = updated;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn provider_imports_validate_identity_currency_and_price_basis() {
        let fetched = date_timestamp("2026-11-23").unwrap() + DAY / 2;
        let twelve = json!({
            "meta": {"symbol": "AAPL", "interval": "1day", "currency": "USD"},
            "values": [
                {"datetime": "2026-11-23", "close": "999"},
                {"datetime": "2026-11-20", "close": "110.5"},
                {"datetime": "2026-11-19", "close": "100"}
            ]
        });
        let history = parse_twelve(&twelve, "AAPL", fetched).unwrap();
        assert_eq!(history.closes.len(), 2);
        assert_eq!(history.closes[1].price, 110.5);
        assert!(parse_twelve(&twelve, "NVDA", fetched).is_err());
        let alpha = json!({
            "Meta Data": {"2. Symbol": "AAPL"},
            "Time Series (Daily)": {
                "2026-11-20": {"4. close": "110", "5. adjusted close": "55"},
                "2026-11-19": {"4. close": "100", "5. adjusted close": "50"}
            }
        });
        assert_eq!(
            parse_alpha(&alpha, "AAPL", "USD", fetched).unwrap().closes[1].price,
            55.
        );
        assert!(parse_alpha(&json!({"Note":"limit"}), "AAPL", "USD", fetched).is_err());
    }

    #[test]
    fn custom_inputs_sort_dates_preserve_price_units_and_reject_duplicates() {
        let fetched = date_timestamp("2026-11-23").unwrap();
        assert!(date_timestamp("2025-02-29").is_none());
        let csv = "\u{feff}date,close,symbol,currency,notes\r\n2024-03-01,12,VOD,GBp,\"x,y\"\r\n2024-02-29,\"11\",VOD,GBp,z";
        let history = parse_custom(csv, "VOD", "", fetched).unwrap();
        assert_eq!(history.currency, "GBp");
        assert_eq!(history.closes[0].price, 11.);
        assert!(parse_custom(csv, "VOD", "GBP", fetched).is_err());
        let custom = r#"{"symbol":"VOD","currency":"GBP","prices":[{"date":"2024-03-01","close":12},{"date":"2024-03-01","close":11}]}"#;
        assert!(
            parse_custom(custom, "VOD", "", fetched)
                .unwrap_err()
                .contains("only once")
        );
    }

    #[test]
    fn replacing_and_removing_imports_keeps_other_tickers_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("imports.json");
        let mut imports = Imports::load(Some(path.clone()));
        for ticker in ["NVDA", "AAPL"] {
            let mut history = History::sample(ticker).unwrap();
            history.sample = false;
            imports
                .insert(ImportedHistory {
                    source: Source::File,
                    history: Arc::new(history),
                })
                .unwrap();
        }
        let mut restored = Imports::load(Some(path.clone()));
        assert_eq!(restored.get("AAPL").unwrap().source, Source::File);
        restored.remove("AAPL").unwrap();
        let restored = Imports::load(Some(path));
        assert!(restored.get("AAPL").is_none());
        assert!(restored.get("NVDA").is_some());
    }
}
