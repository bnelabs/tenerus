use super::*;
use crate::{
    market_data::{ImportRequest, ImportedHistory, Source},
    report_export::{self, Format, Report},
};

#[derive(Clone, Copy)]
pub(super) enum InfoPage {
    About,
    Help,
    Disclaimer,
}

impl InfoPage {
    fn title(self) -> &'static str {
        match self {
            Self::About => "About Tenerus",
            Self::Help => "Using Tenerus",
            Self::Disclaimer => disclaimer::TITLE,
        }
    }
}

pub(super) struct ImportForm {
    sequence: u64,
    request: ImportRequest,
    importing: bool,
    error: Option<String>,
}

fn info_section(ui: &mut egui::Ui, p: Palette, heading: &str, body: &str) {
    ui.label(RichText::new(heading).size(14.).strong().color(p.text));
    ui.label(RichText::new(body).size(13.).color(p.muted));
    ui.add_space(8.);
}

fn terms(ui: &mut egui::Ui, p: Palette) {
    for (heading, body) in disclaimer::TERMS {
        info_section(ui, p, heading, body);
    }
    info_section(ui, p, "Agreement", disclaimer::AGREEMENT);
}

pub(super) fn export_menu(ui: &mut egui::Ui) -> Option<Format> {
    ui.menu_button("Export analysis", |ui| {
        for &format in Format::ALL {
            if ui.button(format.label()).clicked() {
                ui.close();
                return Some(format);
            }
        }
        None
    })
    .inner
    .flatten()
}

impl MarketApp {
    pub(super) fn help_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Help", |ui| {
            for page in [InfoPage::Help, InfoPage::About, InfoPage::Disclaimer] {
                if ui.button(page.title()).clicked() {
                    self.information = Some(page);
                    ui.close();
                }
            }
        });
    }

    pub(super) fn information_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(page) = self.information else {
            return;
        };
        let mut open = true;
        let size = match page {
            InfoPage::About => vec2(600., 540.),
            _ => vec2(535., 360.),
        };
        dialog_window(ctx, page.title(), size)
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .vscroll(true)
            .show(ctx, |ui| match page {
                InfoPage::About => {
                    ui.label(display_text("Tenerus", 28., p.text));
                    let platform = match std::env::consts::OS {
                        "macos" => "macOS",
                        "windows" => "Windows",
                        "linux" => "Linux",
                        "freebsd" => "FreeBSD",
                        other => other,
                    };
                    text(
                        ui,
                        format!(
                            "Version {} · {} ({}) · MIT License",
                            env!("CARGO_PKG_VERSION"),
                            platform,
                            std::env::consts::ARCH
                        ),
                        12.,
                        p.blue,
                    );
                    ui.add_space(8.);
                    text(
                        ui,
                        "An independent, local-first native Rust desktop workspace for macOS, Windows, Linux, and FreeBSD — built to study historical price distributions, test calibrated statistical ranges out-of-sample, and audit AI commentary against transparent baselines.",
                        13.,
                        p.text,
                    );
                    ui.add_space(10.);
                    info_section(
                        ui,
                        p,
                        "Educational use only",
                        "Provided solely for education and research. Nothing in Tenerus is financial advice or a forecast guarantee. It has no brokerage access and should not be used for real trading or investing.",
                    );
                    info_section(
                        ui,
                        p,
                        "Quantitative modeling & opt-in pilots",
                        "Outlook blends four strictly causal local models (kernel analogs with S&P 500 regime, online quantile regression with daily news ablation, mean-reverting GARCH(1,1), and an unconditional baseline) via walk-forward pinball-loss weights and split-conformal 80% calibration. Opt-in local TimesFM and Chronos-2 pilots run on user-installed Python runtimes and score side-by-side against Outlook.",
                    );
                    info_section(
                        ui,
                        p,
                        "Market data & AI boundaries",
                        "Quotes, OHLCV closes, and headlines load directly from Yahoo Finance or user-supplied Twelve Data, Alpha Vantage, CSV/JSON, and HTTPS sources. Optional AI research and CLI briefings send context only to the provider you choose, and log decisions locally for 28/180/365-day split-adjusted reviews, S&P 500 alpha, and Brier/ECE calibration (prompt context only, never model retraining).",
                    );
                    info_section(
                        ui,
                        p,
                        "Local-first storage",
                        "Watchlists, imports, daily sentiment history, prediction logs, and exported reports (.xlsx, .pdf, .docx) stay on this device with zero Tenerus servers or telemetry.",
                    );
                    ui.horizontal_wrapped(|ui| {
                        ui.hyperlink_to("GitHub", "https://github.com/bnelabs/tenerus");
                        ui.hyperlink_to("Yahoo Finance", "https://finance.yahoo.com/");
                        ui.hyperlink_to("Twelve Data", "https://twelvedata.com/");
                        ui.hyperlink_to("Alpha Vantage", "https://www.alphavantage.co/");
                    });
                }
                InfoPage::Help => {
                    info_section(ui, p, "Find a ticker", "Search for a company or symbol and select it. Follow it to add it to a watchlist. Price & news shows quotes, charts, and publisher links.");
                    info_section(ui, p, "Explore Outlook", "Choose 5, 20, or 60 trading sessions to see what happened after similar price patterns. Expert shows historical checks, and Market god lets you replay a past date.");
                    info_section(ui, p, "Import daily prices", "Choose User → Import market data. Select a provider, a CSV or JSON file, or a custom URL. The imported series opens in Outlook for that ticker.");
                    info_section(ui, p, "Run and save research", "Set up a model in User → API connections, then generate research in Price & news. Review past calls in Prediction logbook. Choose Export analysis in Outlook or below a completed AI result to save an Excel, PDF, or Word file.");
                    if link(ui, "Open market data import", p).clicked() {
                        self.open_market_import();
                        ui.close();
                    }
                }
                InfoPage::Disclaimer => terms(ui, p),
            });
        if !open {
            self.information = None;
        }
    }

    pub(super) fn first_launch(&mut self, root: &mut egui::Ui, p: Palette) {
        let ctx = root.ctx().clone();
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg))
            .show(root, |_| {});
        let bounds = ctx.content_rect();
        egui::Modal::new(Id::unique("educational_use_agreement"))
            .frame(
                Frame::new()
                    .fill(p.panel)
                    .stroke(Stroke::new(1., p.line))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(24)),
            )
            .show(&ctx, |ui| {
                ui.set_width((bounds.width() - 76.).clamp(228., 555.));
                text(ui, "Tenerus", 14., p.blue);
                ui.label(display_text(disclaimer::TITLE, 26., p.text));
                ui.add_space(8.);
                egui::ScrollArea::vertical()
                    .max_height((bounds.height() - 210.).max(72.))
                    .auto_shrink([false, true])
                    .show(ui, |ui| terms(ui, p));
                if let Some(error) = &self.terms_error {
                    text(ui, error, 12., p.red);
                }
                ui.add_space(10.);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let ink = if self.prefs.dark {
                        p.bg
                    } else {
                        Color32::WHITE
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Agree & Continue").strong().color(ink),
                            )
                            .fill(p.blue),
                        )
                        .clicked()
                    {
                        self.accept_terms(&ctx);
                    }
                    if button(ui, "Decline and quit", p).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
    }

    fn accept_terms(&mut self, ctx: &egui::Context) {
        if !self.transient
            && let Err(error) = disclaimer::accept(&self.terms_path)
        {
            self.terms_error = Some(format!("Could not save your agreement. {error}"));
            return;
        }
        self.terms_accepted = true;
        self.terms_error = None;
        self.start_workspace(ctx);
        ctx.request_repaint();
    }

    pub(super) fn open_market_import(&mut self) {
        self.import_sequence += 1;
        self.market_import = Some(ImportForm {
            sequence: self.import_sequence,
            request: ImportRequest {
                source: if self.prefs.offline {
                    Source::File
                } else {
                    Source::TwelveData
                },
                ticker: self.prefs.selected.clone(),
                ..ImportRequest::default()
            },
            importing: false,
            error: None,
        });
    }

    pub(super) fn finish_market_import(
        &mut self,
        sequence: u64,
        result: Result<ImportedHistory, String>,
    ) {
        let Some(form) = self
            .market_import
            .as_mut()
            .filter(|form| form.sequence == sequence)
        else {
            return;
        };
        form.importing = false;
        let message = result.and_then(|entry| {
            let history = &entry.history;
            let count = history.closes.len();
            let ticker = history.ticker.clone();
            let source = entry.source.label();
            self.desk.imports.insert(entry)?;
            Ok((ticker, count, source))
        });
        match message {
            Ok((ticker, count, source)) => {
                self.market_import = None;
                self.desk.active = true;
                self.desk.clear_replay();
                self.select_symbol(&ticker);
                self.notify(format!(
                    "Imported {count} daily closes for {ticker} from {source}."
                ));
            }
            Err(error) => form.error = Some(error),
        }
    }

    pub(super) fn market_import_dialog(&mut self, ctx: &egui::Context, p: Palette) {
        let Some(mut form) = self.market_import.take() else {
            return;
        };
        let mut open = true;
        dialog_window(ctx, "Import market data", vec2(565., 427.))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .vscroll(true)
            .show(ctx, |ui| {
                text(
                    ui,
                    "Use a different daily price series for a ticker in Outlook.",
                    14.,
                    p.text,
                );
                ui.add_space(5.);
                ui.add_enabled_ui(!form.importing, |ui| import_fields(ui, p, &mut form));
                if let Some(error) = form.error.as_ref() {
                    text(ui, error, 12., p.red);
                }
                if let Some(error) = self.desk.imports.load_error.as_ref() {
                    text(
                        ui,
                        format!("Saved imports could not be read. {error}"),
                        12.,
                        p.red,
                    );
                }
                let ticker = form.request.ticker.trim().to_ascii_uppercase();
                let existing = self.desk.imports.get(&ticker).map(|entry| entry.source);
                if let Some(origin) = existing {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        text(
                            ui,
                            format!("{ticker} uses {}", origin.label()),
                            12.,
                            p.muted,
                        );
                        if ui
                            .add_enabled(!form.importing, egui::Button::new("Use default history"))
                            .clicked()
                        {
                            match self.desk.imports.remove(&ticker) {
                                Ok(()) => {
                                    self.desk.clear_replay();
                                    self.load_outlook_history(ctx, true);
                                    self.notify(format!("Removed imported history for {ticker}."));
                                }
                                Err(error) => form.error = Some(error),
                            }
                        }
                    });
                }
                ui.add_space(6.);
                let online_required = self.prefs.offline && form.request.source != Source::File;
                if online_required {
                    text(
                        ui,
                        "Turn off Offline demonstration to fetch from a data service.",
                        12.,
                        p.muted,
                    );
                }
                ui.horizontal(|ui| {
                    let label = if existing.is_some() {
                        "Replace imported history"
                    } else {
                        "Import daily prices"
                    };
                    let ready = !form.importing
                        && !online_required
                        && self.desk.imports.load_error.is_none();
                    if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                        form.importing = true;
                        form.error = None;
                        let request = form.request.clone();
                        let tx = self.tx.clone();
                        let ctx = ctx.clone();
                        let sequence = form.sequence;
                        std::thread::spawn(move || {
                            let _ = tx.send(Event::HistoryImport(sequence, request.fetch()));
                            ctx.request_repaint();
                        });
                    }
                    if form.importing {
                        ui.spinner();
                        text(ui, "Importing daily closes…", 12., p.muted);
                    }
                });
            });
        if open {
            self.market_import = Some(form);
        }
    }

    pub(super) fn save_report(&mut self, ctx: &egui::Context, report: Report, format: Format) {
        if self.transient {
            return;
        }
        #[cfg(target_os = "macos")]
        let path = rfd::FileDialog::new()
            .set_title("Export Tenerus analysis")
            .set_file_name(report.filename(format))
            .add_filter(format.label(), &[format.extension()])
            .save_file();
        #[cfg(not(target_os = "macos"))]
        let path = {
            let folder = self.state_path.with_file_name("exports");
            if let Err(error) = std::fs::create_dir_all(&folder) {
                self.notify(format!("Could not create the exports folder. {error}"));
                return;
            }
            let base = report.filename(format);
            let mut candidate = folder.join(&base);
            if candidate.exists() {
                let stem = candidate
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Tenerus-report")
                    .to_owned();
                let ext = format.extension();
                for n in 2..10_000u32 {
                    let next = folder.join(format!("{stem}-{n}.{ext}"));
                    if !next.exists() {
                        candidate = next;
                        break;
                    }
                }
            }
            Some(candidate)
        };
        let Some(path) = path else {
            return;
        };
        let (path, appended) = report_export::with_extension(path, format);
        let replace = cfg!(target_os = "macos") && !appended;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = report.write(format, &path, replace).map(|()| path);
            let _ = tx.send(Event::ExportedReport(result));
            ctx.request_repaint();
        });
    }
}

fn import_fields(ui: &mut egui::Ui, p: Palette, form: &mut ImportForm) {
    let request = &mut form.request;
    let before = request.source;
    egui::ComboBox::from_id_salt("import_price_source")
        .selected_text(request.source.label())
        .width(215.)
        .show_ui(ui, |ui| {
            for source in Source::ALL {
                ui.selectable_value(&mut request.source, source, source.label());
            }
        });
    if request.source != before {
        request.key.clear();
        form.error = None;
    }
    if input(ui, "Ticker", &mut request.ticker, "AAPL").changed() {
        form.error = None;
    }
    match request.source {
        Source::TwelveData => {
            input(ui, "Exchange (optional)", &mut request.exchange, "NASDAQ");
            ui.hyperlink_to("Twelve Data API access", "https://twelvedata.com/docs");
        }
        Source::AlphaVantage => {
            input(ui, "Price currency", &mut request.currency, "USD");
            text(
                ui,
                "Requires access to full daily adjusted history.",
                12.,
                p.muted,
            );
            ui.hyperlink_to(
                "Alpha Vantage API access",
                "https://www.alphavantage.co/documentation/#dailyadj",
            );
        }
        Source::Custom => {
            let previous = request.endpoint.clone();
            if input(
                ui,
                "Endpoint URL",
                &mut request.endpoint,
                "https://example.com/prices?symbol=AAPL",
            )
            .changed()
            {
                let origin = |endpoint: &str| {
                    network::validate_endpoint(endpoint)
                        .ok()
                        .map(|url| url.origin())
                };
                if origin(&previous) != origin(&request.endpoint) {
                    request.key.clear();
                }
                form.error = None;
            }
            text(
                ui,
                "Use the final HTTPS URL, or HTTP for a local service on this device.",
                12.,
                p.muted,
            );
        }
        Source::File => file_input(ui, p, &mut request.file),
    }
    if request.source == Source::Custom || request.source == Source::File {
        input(
            ui,
            "Currency for CSV without a currency column",
            &mut request.currency,
            "USD",
        );
        egui::CollapsingHeader::new("Accepted CSV and JSON formats").show(ui, |ui| {
            text(ui, "Supply completed daily closes with one row per trading date.", 12., p.muted);
            ui.monospace("date,close\n2026-01-05,190.25\n2026-01-06,192.10");
            text(ui, "CSV may also include symbol and currency columns. For JSON, use symbol, currency, and a prices array.", 12., p.muted);
            ui.add(egui::Label::new(RichText::new(r#"{"symbol":"AAPL","currency":"USD","prices":[{"date":"2026-01-05","close":190.25},{"date":"2026-01-06","close":192.10}]}"#).monospace().size(11.)).wrap());
        });
    }
    if request.source != Source::File {
        let custom = request.source == Source::Custom;
        ui.label(if custom {
            "Bearer token (optional)"
        } else {
            "API key"
        });
        ui.add(
            egui::TextEdit::singleline(&mut request.key)
                .password(true)
                .char_limit(2048)
                .desired_width(f32::INFINITY),
        );
        text(
            ui,
            "Credentials are used for this import only.",
            11.,
            p.muted,
        );
    }
}

fn input(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) -> egui::Response {
    ui.label(label);
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(f32::INFINITY),
    )
}

fn file_input(ui: &mut egui::Ui, p: Palette, path: &mut PathBuf) {
    #[cfg(target_os = "macos")]
    ui.horizontal_wrapped(|ui| {
        if button(ui, "Choose file…", p).clicked()
            && let Some(file) = rfd::FileDialog::new()
                .add_filter("Daily price data", &["csv", "json"])
                .pick_file()
        {
            *path = file;
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("No file selected");
        text(ui, name, 12., p.muted);
    });
    #[cfg(not(target_os = "macos"))]
    {
        let _ = p;
        let mut file = path.to_string_lossy().into_owned();
        if input(
            ui,
            "CSV or JSON file path",
            &mut file,
            "/path/to/prices.csv",
        )
        .changed()
        {
            *path = file.into();
        }
    }
}
