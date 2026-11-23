#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod briefing;
mod disclaimer;
mod forecast;
mod journal;
mod market_data;
mod mechanics;
mod model;
mod network;
mod news_signal;
mod report_export;
mod scenarios;
mod ui;
use std::path::PathBuf;

#[derive(Default)]
pub struct Launch {
    pub screenshot: Option<PathBuf>,
    pub view: String,
    pub width: f32,
    pub height: f32,
    pub demo: bool,
    pub auto_size: bool,
    pub intent: Option<mechanics::NavigationIntent>,
    pub captured: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
fn main() -> eframe::Result {
    let mut launch = Launch {
        width: mechanics::DEFAULT_WINDOW_SIZE[0],
        height: mechanics::DEFAULT_WINDOW_SIZE[1],
        auto_size: true,
        ..Default::default()
    };
    let mut args = std::env::args().skip(1);
    let mut explicit_size = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--test-briefing" => {
                let name = args.next().unwrap_or_default();
                let Some(engine) = briefing::Engine::ALL
                    .into_iter()
                    .find(|e| e.command() == name)
                else {
                    eprintln!("--test-briefing requires codex, claude or agy");
                    std::process::exit(2);
                };
                let history = scenarios::History::sample("NVDA").unwrap();
                let report = scenarios::report(&history, 20).unwrap();
                let input = briefing::Input::from_scenario(
                    &history.ticker,
                    &history.currency,
                    true,
                    &report.current,
                    Some(&report.trials),
                );
                match briefing::run(engine, &input, &Default::default()) {
                    Ok(_) => println!("{} returned a validated sample briefing.", engine.label()),
                    Err(error) => {
                        eprintln!("{error}");
                        std::process::exit(1);
                    }
                }
                return Ok(());
            }
            "--check-local-model" => {
                let report = forecast::check();
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
                println!("{}", report.hardware_status());
                println!("{}", report.runtime_status());
                return Ok(());
            }
            "--test-meta" => {
                // Read the credential from a pipe, never an argument, file or log.
                // This uses the same request/usage path as the connection dialog.
                let mut key = String::new();
                if std::io::stdin().read_line(&mut key).is_err() {
                    eprintln!("Could not read the API key from standard input.");
                    std::process::exit(2);
                }
                let secret = model::Secret {
                    key: key.trim().into(),
                    ..Default::default()
                };
                match network::test_connection(&model::Provider::meta(), &secret) {
                    Ok(usage) => {
                        println!(
                            "Meta / {}: connection verified.\n{}\n{}",
                            model::META_MODEL,
                            usage.cost_label(),
                            usage.detail()
                        );
                        return Ok(());
                    }
                    Err(err) => {
                        eprintln!("Meta connection test failed: {err}");
                        std::process::exit(1);
                    }
                }
            }
            "--screenshot" => {
                let Some(path) = args.next() else {
                    eprintln!("--screenshot requires a PNG file name");
                    std::process::exit(2);
                };
                launch.screenshot = Some(path.into());
            }
            "--view" => launch.view = args.next().unwrap_or_default(),
            "--demo" => launch.demo = true,
            "--symbol" => {
                let ticker = args.next().unwrap_or_default().to_uppercase();
                if !model::valid_ticker(&ticker) {
                    eprintln!("--symbol requires a valid ticker");
                    std::process::exit(2);
                }
                launch.intent = Some(mechanics::NavigationIntent::Symbol(ticker));
            }
            "--watchlist" => {
                let id = args
                    .next()
                    .and_then(|id| id.parse::<u64>().ok())
                    .unwrap_or_else(|| {
                        eprintln!("--watchlist requires a numeric local watchlist ID");
                        std::process::exit(2);
                    });
                launch.intent = Some(mechanics::NavigationIntent::Watchlist(id));
            }
            "--business-news" => launch.intent = Some(mechanics::NavigationIntent::BusinessNews),
            "--size" => {
                if let Some(size) = args.next() {
                    if let Some((w, h)) = size.split_once('x') {
                        explicit_size = true;
                        launch.width = w
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .unwrap_or(1180.)
                            .clamp(860., 3000.);
                        launch.height = h
                            .parse::<f32>()
                            .ok()
                            .filter(|v| v.is_finite())
                            .unwrap_or(800.)
                            .clamp(670., 2000.);
                    }
                }
            }
            "--help" => {
                println!(
                    "Tenerus\n  --screenshot <file.png>  Draw 120 frames, save a PNG, and exit without taking focus\n  --demo                   Use labeled sample data\n  --view prices|expert|market-god|connections|news|saved|logbook|models|welcome|about|help|disclaimer|import-data  Open a review state\n  --symbol AAPL            Open a ticker without following it\n  --watchlist 1            Open a local watchlist at the top\n  --business-news          Open market-wide news\n  --test-briefing <cli>    Run a sample briefing through codex, claude or agy\n  --test-meta              Test Meta with an API key read from standard input\n  --check-local-model      Read local model hardware and package readiness\n  --size 1180x800          Set window size"
                );
                return Ok(());
            }
            _ => {
                eprintln!("Unknown option: {arg}");
                std::process::exit(2);
            }
        }
    }
    let shot = launch.screenshot.is_some();
    launch.auto_size = !shot && !explicit_size;
    if shot && !explicit_size {
        // Fixed screenshot viewport size, independent of saved window bounds.
        launch.width = 1440.;
        launch.height = 940.;
    }
    if !shot && !explicit_size {
        let saved = model::data_dir().join("settings.json");
        let legacy = model::legacy_data_dir().join("settings.json");
        if let Some([width, height]) = model::Preferences::load(&saved, &legacy)
            .ok()
            .and_then(|prefs| prefs.window_size)
        {
            launch.width = width;
            launch.height = height;
            launch.auto_size = false;
        }
    }
    if shot {
        let captured = launch.captured.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(model::SHOT_TIMEOUT));
            if !captured.load(std::sync::atomic::Ordering::Acquire) {
                eprintln!(
                    "Screenshot could not be drawn within 30 seconds in this desktop session."
                );
                std::process::exit(2);
            }
        });
    }
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([launch.width, launch.height])
        .with_min_inner_size(mechanics::MINIMUM_WINDOW_SIZE)
        .with_title("Tenerus")
        .with_active(!shot);
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }
    #[cfg(target_os = "windows")]
    let viewport = if shot {
        viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom)
    } else {
        viewport
    };
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    #[cfg(target_os = "macos")]
    if shot {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            b.with_activate_ignoring_other_apps(false);
        }));
    }
    eframe::run_native(
        "Tenerus",
        options,
        Box::new(move |cc| Ok(Box::new(ui::MarketApp::new(cc, launch)))),
    )
}

fn load_app_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../assets/tenerus-icon-256.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}
