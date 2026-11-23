//! Core chart range, quote comparison, display, and window sizing rules.

pub const DEFAULT_WINDOW_SIZE: [f32; 2] = [1180., 800.];
pub const MINIMUM_WINDOW_SIZE: [f32; 2] = [860., 670.];

// Returns relative price change as a fraction (not percent units).
// Non-positive or unordered bases resolve to 0.0.
pub fn price_change_fraction(change: f64, current_price: f64) -> f64 {
    let previous_price = current_price - change;
    if previous_price > 0.0 {
        change / previous_price
    } else {
        0.0
    }
}

// Numeric quote comparator: missing values sort first ascending and last descending.
// Equal keys remain equal here (callers add deterministic ticker tie-breaking).
pub fn quote_value_order(
    left: Option<f64>,
    right: Option<f64>,
    descending: bool,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let order = match (left, right) {
        (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    };
    if descending { order.reverse() } else { order }
}

// Supported chart time-range identifiers and short UI labels.
pub const CHART_RANGE_RAW_VALUES: [&str; 11] = [
    "oneDay",
    "oneWeek",
    "oneMonth",
    "threeMonths",
    "sixMonths",
    "yearToDate",
    "oneYear",
    "twoYears",
    "fiveYears",
    "tenYears",
    "all",
];
pub const CHART_RANGE_LABELS: [&str; 11] = [
    "1D", "1W", "1M", "3M", "6M", "YTD", "1Y", "2Y", "5Y", "10Y", "ALL",
];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ChartRange {
    #[default]
    OneDay = 0,
    OneWeek = 1,
    OneMonth = 2,
    ThreeMonths = 3,
    SixMonths = 4,
    YearToDate = 5,
    OneYear = 6,
    TwoYears = 7,
    FiveYears = 8,
    TenYears = 9,
    All = 10,
}
impl ChartRange {
    pub const ALL: [Self; 11] = [
        Self::OneDay,
        Self::OneWeek,
        Self::OneMonth,
        Self::ThreeMonths,
        Self::SixMonths,
        Self::YearToDate,
        Self::OneYear,
        Self::TwoYears,
        Self::FiveYears,
        Self::TenYears,
        Self::All,
    ];
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    pub fn from_raw(value: &str) -> Option<Self> {
        CHART_RANGE_RAW_VALUES
            .iter()
            .position(|raw| *raw == value)
            .and_then(Self::from_index)
    }
    pub const fn index(self) -> usize {
        self as usize
    }
    pub const fn raw_value(self) -> &'static str {
        CHART_RANGE_RAW_VALUES[self.index()]
    }
    pub const fn label(self) -> &'static str {
        CHART_RANGE_LABELS[self.index()]
    }
}

// Accept numeric indices or string identifiers when deserializing saved range settings,
// repairing invalid values to OneDay.
pub fn deserialize_chart_range_index<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let value = serde_json::Value::deserialize(deserializer)?;
    let range = value
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .and_then(ChartRange::from_index)
        .or_else(|| value.as_str().and_then(ChartRange::from_raw))
        .unwrap_or_default();
    Ok(range.index())
}

// Watchlist row value display modes (default: currency price change).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum PriceChangeDisplay {
    #[default]
    Currency = 0,
    Percentage = 1,
    MarketCap = 2,
}
impl PriceChangeDisplay {
    pub const ALL: [Self; 3] = [Self::Currency, Self::Percentage, Self::MarketCap];

    pub fn from_raw(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Currency),
            1 => Some(Self::Percentage),
            2 => Some(Self::MarketCap),
            _ => None,
        }
    }
    // Preserve saved/menu indices (percentage 0, currency 1, market cap 2).
    pub fn from_saved_index(index: usize) -> Option<Self> {
        Self::from_raw(match index {
            0 => 1,
            1 => 0,
            _ => index as u64,
        })
    }
    pub fn saved_index(self) -> usize {
        match self {
            Self::Currency => 1,
            Self::Percentage => 0,
            Self::MarketCap => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Currency => "Price change",
            Self::Percentage => "Percentage change",
            Self::MarketCap => "Market cap",
        }
    }

    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }
}

// Screen-aware default window dimensions.
pub fn default_window_size([width, height]: [f32; 2]) -> [f32; 2] {
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return DEFAULT_WINDOW_SIZE;
    }
    if width <= 1280. && height <= 800. {
        [width.min(1080.), height.min(626.)]
    } else if width <= 1366. && height <= 768. {
        [1080., height.min(626.)]
    } else if width <= 1440. && height <= 900. {
        [1080., 750.]
    } else if width <= 1680. && height <= 1050. {
        [1180., 800.]
    } else if width <= 1920. && height <= 1080. {
        [1180., 800.]
    } else {
        [1180., 1024.]
    }
}

// Minimum supported desktop window dimensions.
pub fn minimum_window_size(screen_height: f32) -> [f32; 2] {
    let height = if screen_height.is_finite() && screen_height > 0. {
        screen_height.min(670.)
    } else {
        670.
    };
    [860., height]
}

// Local CLI navigation intents.
#[derive(Clone, Debug)]
pub enum NavigationIntent {
    Symbol(String),
    Watchlist(u64),
    BusinessNews,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_rules_cover_display_breakpoints_and_edges() {
        for (screen, expected) in [
            ([1024., 600.], [1024., 600.]),
            ([1280., 800.], [1080., 626.]),
            ([1280.1, 800.], [1080., 750.]),
            ([1366., 768.], [1080., 626.]),
            ([1366.1, 768.], [1080., 750.]),
            ([1440., 900.], [1080., 750.]),
            ([1440.1, 900.], [1180., 800.]),
            ([1680., 1050.], [1180., 800.]),
            ([1920., 1080.], [1180., 800.]),
            ([1920.1, 1080.], [1180., 1024.]),
            ([1920., 1080.1], [1180., 1024.]),
        ] {
            assert_eq!(default_window_size(screen), expected, "screen {screen:?}");
        }
        assert_eq!(minimum_window_size(1080.), [860., 670.]);
        assert_eq!(minimum_window_size(600.), [860., 600.]);
        assert_eq!(default_window_size([f32::NAN, 800.]), DEFAULT_WINDOW_SIZE);
    }
}
