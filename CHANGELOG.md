# Changelog

All notable changes to **Tenerus** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> **Educational Use Notice:** Every release of Tenerus is provided solely for educational and research purposes, not for real trading or investment decisions.

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

[0.6.0]: https://github.com/bnelabs/tenerus/releases/tag/v0.6.0
