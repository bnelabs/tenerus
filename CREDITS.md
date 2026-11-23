# Credits and data boundaries

Tenerus is an independently authored native desktop market research workspace for macOS, Windows, Linux, and FreeBSD. It is not affiliated with Apple, Microsoft, or any market-data or AI inference provider. Market curves shown in offline demonstration mode and sample reading guides are original generated fixtures clearly labeled in the interface.

## Open-source dependencies

Rust dependencies retain their respective licenses:
- `egui` / `eframe` (MIT OR Apache-2.0)
- `image` (MIT OR Apache-2.0)
- `serde` / `serde_json` (MIT OR Apache-2.0)
- `roxmltree` (MIT OR Apache-2.0)
- `url` (MIT OR Apache-2.0)
- `winit` (Apache-2.0)
- `tempfile` (MIT OR Apache-2.0)
- `crc32fast` (MIT OR Apache-2.0)
- `libc` (MIT OR Apache-2.0, Unix targets)
- `security-framework` (MIT OR Apache-2.0, macOS)
- `rfd` (MIT, macOS)
- `objc2-core-foundation` / `objc2-core-graphics` / `objc2-core-text` (Zlib OR Apache-2.0 OR MIT, macOS)

System fonts are loaded from the installed operating system (falling back to `egui`'s bundled default fonts) and are not redistributed.

## Data and service boundaries

- Public quote, search, and RSS headline requests go directly from the user's device to unofficial, unaffiliated Yahoo Finance public web endpoints; linked stories belong to their publishers and open externally. Data may be delayed, unavailable, incomplete, rate-limited, or subject to third-party usage terms.
- User-requested daily price imports from Twelve Data, Alpha Vantage, or custom HTTPS endpoints are subject to the selected provider's terms.
- AI requests go only to the endpoint or locally installed CLI assistant the user explicitly invokes using their own local authentication. API charges and compliance with provider terms of service are the user's responsibility.
- Prediction logs, settings, and credentials remain local to the user's device and are excluded from packaged builds.
- Forecast model guidance links to public Google Research / TimesFM and Apple MLX documentation. No TimesFM code or model weights are bundled, downloaded, or executed by Tenerus.
- Report exports use macOS Core Text and Core Graphics for PDF on macOS, a pure-Rust PDF 1.4 writer on Windows/Linux/FreeBSD, and pure-Rust Office Open XML for Excel and Word across all platforms.
- No brokerage, trade execution, or financial account access is implemented.
