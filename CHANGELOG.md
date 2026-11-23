# Changelog

All notable changes to **Tenerus** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Educational Use Notice:** Every release of Tenerus is provided solely for educational and research purposes, not for real trading or investment decisions.

---

## [0.8.0] — 2026-11-23

Completes **100% of Tiers 1–3** of the quantitative modeling, benchmark-alpha evaluation, persisted news ablation, and local foundation-model forecasting roadmap across all platforms.

### Added
- **Cross-Asset S&P 500 (`^GSPC`) Market-Regime Features (`src/scenarios.rs`, `src/ui/outlook.rs`):** Outlook automatically loads completed S&P 500 daily closes alongside the selected ticker (or `History::sample("^GSPC")` offline) and extracts strictly causal 20-day relative strength (`rel_strength_20`) and broad market volatility (`market_vol_20`) into analogue matching and Online Quantile Regression.
- **Benchmark Alpha vs. `^GSPC` in Prediction Logbook (`src/journal.rs`, `src/network.rs`, `src/ui.rs`):** Automated 28/180/365-day outcome reviews now fetch best-effort S&P 500 closes over the exact same window to compute realized excess return (`alpha = ticker return − ^GSPC return`), surfaced per entry, aggregated as `mean_buy_alpha` / `mean_sell_alpha` / `mean_wait_alpha`, and included in `LearningContext` calibration feedback.
- **Mean-Reverting GARCH(1,1) + Fat-Tailed Skew-Adjusted Volatility (`src/scenarios.rs`):** Replaced non-mean-reverting RiskMetrics EWMA with a strictly causal stationary **GARCH(1,1)** recursion ($\alpha = 0.08$, $\beta = 0.88$, $\omega = 0.04\,\bar{\sigma}^2$) using exact analytical multi-step cumulative variance $V_h$ and Cornish-Fisher skewness-adjusted fat-tailed quantiles.
- **Persisted Daily News Sentiment Store & Walk-Forward Ablation (`src/news_signal.rs`, `src/scenarios.rs`, `src/ui.rs`):** Automatically persists daily headline sentiment snapshots to `sentiment-history.json` (bounded and strictly causal via 14-day exponentially decayed lookup), feeds historical sentiment as the 10th feature in Online Quantile Regression, and runs an out-of-sample walk-forward ablation (`sentiment_pinball_delta`) measuring whether stored headlines actually reduce pinball loss.
- **Chronos-2 + Batched Walk-Forward Conformal Foundation Model Bridge (`src/forecast.rs`, `src/ui.rs`, `src/ui/outlook.rs`):** Expanded the opt-in local Python bridge to support both **TimesFM 2.5/3.0** and **Amazon Chronos-2 (`chronos-forecasting`)**, execute up to 12 non-overlapping walk-forward cutoffs plus the live window in a single model load, apply causal split-conformal calibration, display an apples-to-apples **Side-by-Side Walk-Forward Scoreboard** (Foundation Calibrated vs. Outlook 4-Model Ensemble vs. Baseline — Coverage, Pinball Loss, Median MAE), and overlay calibrated foundation bounds directly on the Outlook fan chart.

### Changed
- **Briefings, Exports & Test Suite:** Updated CLI briefing evidence (`E2`/`E3`), `.xlsx`/`.pdf`/`.docx` report exports, and `OutlookSnapshot` with GARCH(1,1), S&P 500 regime context, and news sentiment ablation results. Expanded offline test coverage to **137 tests** (including zero-leakage regime/sentiment proofs, GARCH mean-reversion tests, benchmark-alpha tests, and a hermetic Python subprocess IPC roundtrip test).

---

## [0.7.0] — 2026-11-23

Major quantitative modeling, news/event integration, self-calibration, and forecasting upgrade across **Outlook**, **AI Research**, **Local Briefings**, **Prediction Logbook**, and **Forecast Model Guide**.

### Added
- **Self-Calibrating 4-Model Outlook Ensemble (`src/scenarios.rs`):** Replaced unweighted 3-feature k-NN with a strictly causal, walk-forward ensemble combining four models blended via out-of-sample **pinball-loss Hedge weighting** and **split-conformal prediction interval calibration**:
  1. **Kernel- & Recency-Weighted Multivariable Analogues** (5d/20d/60d momentum, volatility, 5d/20d volatility ratio, 60d drawdown, plus optional 5d/20d volume ratio and OHLC intraday range ratio).
  2. **Online L2-Regularized Quantile Regression** trained sequentially forward-in-time on non-overlapping completed historical periods via pinball-loss AdaGrad updates.
  3. **RiskMetrics EWMA Fat-Tailed Parametric Volatility** fan ($\lambda = 0.94$).
  4. **Unconditional Non-Overlapping Historical Baseline** with automatic shrinkage when complex models underperform out-of-sample.
- **OHLCV History Enrichment (`src/network.rs`, `src/market_data.rs`):** Parsed daily `high`, `low`, and `volume` alongside closing prices from Yahoo Finance, Twelve Data, Alpha Vantage, JSON, and CSV imports (100% backwards-compatible with close-only files).
- **Local News Sentiment & Event-Risk Analyzer (`src/news_signal.rs`):** Deterministic, zero-dependency headline polarity scorer (with negation handling), topic/event tagger (`Earnings`, `Guidance`, `Analyst`, `M&A / Capital`, `Legal / Regulatory`, `Macro / Policy`), and **earnings-in-horizon gap-risk detector**. Surfaced in the Outlook rail, Local Assistant Briefings (`E7` evidence), AI Research prompts, and exported reports.
- **Confidence Calibration & Return Quality Engine (`src/journal.rs`, `src/ui.rs`):** Added **Brier Score**, **Expected Calibration Error (ECE)**, stated-vs-actual confidence calibration buckets (`0–49%`, `50–69%`, `70–84%`, `85–100%`), mean realized returns per decision (`BUY` / `SELL` / `WAIT`), and same-ticker-prioritized `LearningContext` calibration feedback.
- **Full-Window Quant Context & Outlook Bridge for AI Research (`src/network.rs`):** Eliminated the 40-bar truncation blind spot by supplying full-window multi-horizon returns (5/20/60/120/full bars), realized volatility, max drawdown, 50/200-bar SMA distance, volume ratio, downsampled full-range checkpoints, calibrated Outlook ensemble bands (`outlook_quant_model`), and structured `news_and_event_signal`.
- **Opt-In Local TimesFM Inference Bridge (`src/forecast.rs`, `src/ui.rs`):** Wired an explicit, user-triggered local Python worker (`Run local TimesFM pilot`) that pipes strictly numeric closing prices over `stdin` to a user-installed `timesfm` runtime and returns 10th/50th/90th quantile projections without bundling or auto-downloading weights.

### Changed
- **Walk-Forward Evaluation & Exports:** Expanded walk-forward evaluation up to 36 non-overlapping dates, added proper probabilistic scoring (**Pinball Loss** & **Pinball Skill %** vs. baseline), and included ensemble weights, conformal adjustment, match weights, component pinball losses, and news/event signals in `.xlsx`, `.pdf`, and `.docx` reports.

---

## [0.6.0] — 2026-11-23

Full multi-platform desktop release expanding Tenerus beyond macOS to **Windows (x86_64 & ARM64)**, **Linux (Ubuntu, Fedora, and Arch Linux — x86_64 & ARM64)**, and **FreeBSD (amd64)**.

### Added
- **Windows Support (`x86_64` & `ARM64`):** Added native Windows GUI subsystem support, multi-resolution `tenerus.ico` icon embedding via `build.rs`, `%APPDATA%\Tenerus` storage resolution, silent background `curl.exe` / CLI execution (`CREATE_NO_WINDOW`), Windows CLI discovery (`.exe`/`.cmd`/`.bat`), Segoe UI system font loading, and `scripts/package-windows.ps1`.
- **Linux Support (`x86_64` & `ARM64`):** Added FreeDesktop `.desktop` launcher & hicolor icon assets, `$XDG_DATA_HOME` storage support, system font detection (`DejaVu Sans` / `Liberation Sans`), and native packaging for **Ubuntu/Debian (`.deb`)**, **Fedora (`.rpm`)**, **Arch Linux (`.pkg.tar.zst` + `PKGBUILD`)**, and portable `.tar.gz` archives.
- **FreeBSD Support (`amd64`):** Added FreeBSD `pkg` & `.tar.xz` packaging (`scripts/package-freebsd.sh`), `sysctl` hardware fallback checks, and FreeBSD 14 CI test & build verification.
- **Cross-Platform PDF Export:** Added a zero-dependency pure-Rust PDF 1.4 report writer (`src/report_export/pdf_portable.rs`) so Excel (`.xlsx`), PDF (`.pdf`), and Word (`.docx`) analysis exports work on Windows, Linux, and FreeBSD as well as macOS.
- **Automated Multi-Platform Release CI:** Added `.github/workflows/release.yml` using Node.js 24 actions pinned to immutable commit SHAs, deny-by-default token permissions, `persist-credentials: false`, OS-signed toolchains, and unified `SHA256SUMS.txt` checksums across macOS, Windows (x86_64 & ARM64), Linux (Ubuntu, Fedora, Arch — x86_64 & ARM64), and FreeBSD.
- **Security Hardening, Path Remapping & Policy (`SECURITY.md`):** Resolved OS-trusted absolute `curl` paths (preventing Windows CWD binary hijacking), enforced explicit curl `proto` scheme locking, set `0700` permissions on newly created Unix data directories, guarded Unix process-group cancellation PIDs, tightened exported CSV/ICS ticker filename sanitization, stripped compile-time host paths (`--remap-path-prefix` & removed `CARGO_MANIFEST_DIR` fallback), accepted any valid local CLI assistant authentication, and added `SECURITY.md`.

### Changed
- **Cross-Platform About Dialog & UI Copy:** Updated **About Tenerus** to reflect macOS, Windows, Linux, and FreeBSD support, display the active OS and CPU architecture, and highlight the educational-use-only notice. Updated platform-neutral device/local wording across Outlook, Saved Stories, Import, Briefings, and AI Connections (showing the macOS Keychain checkbox only on macOS).
- **Public Documentation:** Updated `README.md` and `CREDITS.md` with cross-platform installation tables, architecture diagrams, per-OS build prerequisites, and refreshed screenshots.

---

## [0.5.0] — 2026-11-23

First public-ready **Tenerus** release with native macOS branding, custom app icon, educational-use safeguards, analysis report exports, and custom market-data imports.

### Added
- **Tenerus Identity & Native Icon:** Renamed the application, binary (`tenerus`), bundle identifier (`com.tenerus.desktop`), environment variable (`TENERUS_DATA_DIR`), data directory, and macOS Keychain service (`Tenerus inference`). Added a minimalist macOS HIG squircle icon (`Tenerus.icns` + embedded window PNG) and matching vector toolbar emblem.
- **First-Launch Educational Disclaimer, About & Help:** Added a mandatory one-time **Educational Use and Disclaimer** agreement gate on first launch (persisted locally and reopenable under **Help**), plus native **About Tenerus** and **Using Tenerus** guides.
- **Analysis Report Exports:** Export Outlook scenarios and completed AI research reports to **Excel (`.xlsx`)**, **PDF (`.pdf`)**, and **Word (`.docx`)**, including data provenance, timestamps, and an educational-use notice.
- **Custom Daily Market Data Imports:** Import up to 4,000 daily closes per ticker into Outlook from **Twelve Data**, **Alpha Vantage**, local **CSV/JSON** files, or a custom HTTPS/localhost endpoint with optional bearer authentication.
- **Self-Contained macOS Packaging:** Updated `scripts/package.sh` and `scripts/verify_package.py` to assemble, locally sign, zip, and verify `Tenerus.app` and `Tenerus-macOS.zip` cleanly from tracked templates.
- **Public Documentation & Visuals:** Rewrote `README.md` with live screenshots, Mermaid architecture and methodology diagrams, educational-use warnings, build instructions, and added `LICENSE` (MIT) and `CHANGELOG.md`.

### Changed
- Decoupled unit tests from external analysis files so clean clones compile and pass all **124 offline tests** out of the box (`cargo test --offline`).

---

## [0.4.0] — 2026-11-22

### Added
- **Outlook (Historical Scenario Engine):** Added local 5-, 20-, and 60-session historical analogue matching (momentum, daily volatility, and 60-session drawdown) plotting 10th/50th/90th percentile outcome bands across **Beginner**, **Expert** (out-of-sample baseline comparison), and **Market god** (historical date replay and tail-return stress slider) views.
- **Installed-Assistant Briefings:** Added on-demand, cancellable structured briefings via locally installed **Codex**, **Claude Code**, and **Antigravity (`agy`)** CLIs using existing sign-ins without passing API keys.
- **Reliability & Feed Resilience:** Added partial-response handling (preserving usable quotes when chart history is missing or older), strict completion checks for local inference replies, direct proxy-bypassing for `localhost` endpoints, and separate persistence for live vs. sample research.

---

## [0.3.0] — 2026-11-12

### Added
- **Structured AI Outlooks:** Required explicit `BUY`, `SELL`, or `DO NOT BUY` actions with numeric `0–100%` subjective confidence across short (1–4 weeks), medium (3–6 months), and long (1–3 years) horizons.
- **Prediction Logbook & Split-Adjusted Reviews:** Added atomic local logging (`predictions.json`) with fixed 28-, 180-, and 365-day historical price reviews, split alignment, ticker filtering, and JSON export.
- **Rolling Learning Memory & Model Comparisons:** Added configurable prompt context memory (Off / 90 / 180 / 365 days, up to 24 reviewed calls per provider/model/endpoint) and fixed-decision cohort benchmarks (**Always BUY / Always SELL / Always WAIT**).
- **Forecast Model Guide & Local Hardware Check:** Added an in-app forecasting guide and read-only `--check-local-model` hardware/runtime readiness probe.
- **Display Scaling & Responsive Dialogs:** Preserved Retina display scaling and UI zoom with scrollable dialogs down to `860×670`.

---

## [0.2.0] — 2026-11-11

### Added
- **Multi-Provider AI Connections:** Added 8 configurable presets (OpenAI, Anthropic, Google, Grok · xAI, DeepSeek, NVIDIA, Meta, and Custom/localhost) supporting 4 API protocols, optional **macOS Keychain** credential storage, connection testing, and token-cost display.
- **Watchlist Power Controls:** Added per-watchlist sort order, direction, and row metric cycling (price change, percentage change, market cap), drag-to-reorder, multi-list membership, and watchlist JSON export/import.
- **Extended Hours, Saved Stories & Earnings Calendar:** Added separate extended-hours quotes/filtering, local saved headline links, CSV chart export, and tentative `.ics` earnings calendar export.

---

## [0.1.0] — 2026-11-11

### Added
- Initial native desktop market workspace in **Rust + `egui` / `eframe`** featuring watchlists, interactive price charts (`1D`–`ALL`), public Yahoo Finance quotes and news feeds, offline demonstration mode, and deterministic screenshot capture.

[0.7.0]: https://github.com/bnelabs/tenerus/releases/tag/v0.7.0
[0.6.0]: https://github.com/bnelabs/tenerus/releases/tag/v0.6.0
