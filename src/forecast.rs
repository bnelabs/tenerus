//! Read-only local readiness checks and opt-in local foundation model bridge (TimesFM & Chronos-2).
//!
//! Runs strictly on explicit user request, sending numeric closing-price windows over stdin to a
//! user-installed local Python runtime, then applying strictly causal split-conformal calibration
//! and apples-to-apples walk-forward scoring against Outlook's 4-model ensemble.
use crate::news_signal::SentimentSeries;
use crate::scenarios::{self, Band, History};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const MODEL: &str = "google/timesfm-3.0-pytorch";
pub const CHRONOS_MODEL: &str = "amazon/chronos-2";
pub const RUNTIME: &str = "3.0.2";
pub const PILOT: &str = "One ticker at a time · 512 daily closes · walk-forward conformal pilot";
const BACKTEST_TRIALS: usize = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FoundationBackend {
    #[default]
    TimesFm,
    Chronos,
}
impl FoundationBackend {
    pub const ALL: [Self; 2] = [Self::TimesFm, Self::Chronos];

    pub fn label(self) -> &'static str {
        match self {
            Self::TimesFm => "TimesFM 2.5 / 3.0",
            Self::Chronos => "Chronos-2",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::TimesFm => "TimesFM",
            Self::Chronos => "Chronos-2",
        }
    }

    pub fn model_id(self) -> &'static str {
        match self {
            Self::TimesFm => MODEL,
            Self::Chronos => CHRONOS_MODEL,
        }
    }

    fn probe_script(self) -> &'static str {
        match self {
            Self::TimesFm => "import importlib.metadata as m; print(m.version('timesfm'))",
            Self::Chronos => concat!(
                "import importlib.metadata as m\n",
                "for pkg in ('chronos-forecasting', 'chronos'):\n",
                "    try:\n",
                "        print(m.version(pkg)); break\n",
                "    except m.PackageNotFoundError:\n",
                "        pass\n",
                "else:\n",
                "    raise SystemExit(1)"
            ),
        }
    }

    fn worker_script(self) -> &'static str {
        match self {
            Self::TimesFm => TIMESFM_WORKER_SCRIPT,
            Self::Chronos => CHRONOS_WORKER_SCRIPT,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct LocalBand {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
}
impl LocalBand {
    fn from_band(b: Band) -> Self {
        Self {
            low: b.low,
            mid: b.mid,
            high: b.high,
        }
    }
    fn to_band(self) -> Band {
        Band::ordered(self.low, self.mid, self.high)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct FoundationScoreboard {
    pub trials: usize,
    pub conformal_delta: f64,
    pub fm_raw_covered: usize,
    pub fm_cal_covered: usize,
    pub fm_pinball: f64,
    pub fm_mae: f64,
    pub outlook_matched_trials: usize,
    pub outlook_covered: usize,
    pub outlook_pinball: f64,
    pub outlook_mae: f64,
    pub baseline_covered: usize,
    pub baseline_pinball: f64,
    pub baseline_mae: f64,
}
impl FoundationScoreboard {
    /// Positive means calibrated foundation model beat Outlook's 4-model ensemble pinball loss
    /// on the exact same matched walk-forward dates.
    pub fn skill_vs_outlook(&self) -> Option<f64> {
        (self.outlook_matched_trials > 0 && self.outlook_pinball > 1e-9)
            .then(|| 1.0 - self.fm_pinball / self.outlook_pinball)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalForecast {
    pub ticker: String,
    pub horizon: usize,
    pub backend: FoundationBackend,
    pub model: String,
    pub runtime: String,
    pub anchor_price: f64,
    pub raw_band: LocalBand,
    /// Split-conformal calibrated band (equals `raw_band` if fewer than 5 walk-forward trials).
    pub band: LocalBand,
    pub scoreboard: FoundationScoreboard,
    pub elapsed_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MacCheck {
    pub architecture: String,
    pub chip: Option<String>,
    pub memory_gib: Option<u64>,
    pub macos_major: Option<u32>,
    pub python: Option<String>,
    pub python_architecture: Option<String>,
    pub timesfm: Option<String>,
    pub chronos: Option<String>,
    pub mlx: Option<String>,
}
impl MacCheck {
    pub fn hardware_status(&self) -> &'static str {
        if self.architecture != "arm64" {
            return "Apple Silicon backend requires a native Apple Silicon Mac";
        }
        match self.macos_major {
            Some(v) if v < 14 => return "Apple Silicon backend requires macOS 14 or newer",
            None => return "macOS version unavailable · Compatibility not confirmed",
            _ => {}
        }
        match self.memory_gib {
            Some(v) if v >= 16 => "Hardware suitable for a small local pilot",
            Some(_) => "Less than the recommended 16 GB · Local pilot needs a memory test",
            None => "Memory unavailable · Compatibility not confirmed",
        }
    }
    pub fn hardware_detail(&self) -> String {
        format!(
            "{} · {} · {}",
            self.chip.as_deref().unwrap_or(&self.architecture),
            self.memory_gib
                .map_or("Memory unknown".into(), |v| format!("{v} GB memory")),
            self.macos_major
                .map_or("macOS unknown".into(), |v| format!("macOS {v}"))
        )
    }
    pub fn runtime_status(&self) -> String {
        let mut packages = format!(
            "{} · {} · {}",
            self.python
                .as_ref()
                .map_or("Python not found".into(), |v| format!("Python {v}")),
            self.timesfm
                .as_ref()
                .map_or("TimesFM not installed".into(), |v| format!(
                    "TimesFM runtime {v}"
                )),
            self.mlx
                .as_ref()
                .map_or("MLX not installed".into(), |v| format!("MLX {v}"))
        );
        if let Some(ch) = &self.chronos {
            packages.push_str(&format!(" · Chronos {ch}"));
        }
        let state = if self.compatible_runtime() {
            "Packages found; weight loading and inference still untested."
        } else if self.timesfm.as_deref() == Some(RUNTIME) && self.mlx.is_some() {
            "Packages found. A native ARM64 Python 3.10 or newer is required."
        } else if self.chronos.is_some() {
            "Chronos package found; weight loading and inference still untested."
        } else {
            "Local setup required; inference has not been tested."
        };
        format!("{packages}. {state}")
    }

    pub fn compatible_runtime(&self) -> bool {
        self.timesfm.as_deref() == Some(RUNTIME)
            && self.mlx.is_some()
            && self.python_architecture.as_deref() == Some("arm64")
            && self.python.as_deref().is_some_and(|version| {
                let mut parts = version.split('.');
                let major = parts.next().and_then(|s| s.parse::<u32>().ok());
                let minor = parts.next().and_then(|s| s.parse::<u32>().ok());
                major == Some(3) && minor.is_some_and(|n| n >= 10)
            })
    }
}

fn python_candidates() -> Vec<String> {
    let mut out = Vec::new();
    #[cfg(target_os = "windows")]
    {
        use std::path::PathBuf;
        let local_py = crate::model::data_dir().join("forecast-runtime/Scripts/python.exe");
        out.push(local_py.to_string_lossy().into_owned());
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            for ver in ["Python313", "Python312", "Python311", "Python310"] {
                out.push(
                    local
                        .join("Programs/Python")
                        .join(ver)
                        .join("python.exe")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let local_py = crate::model::data_dir().join("forecast-runtime/bin/python3");
        out.push(local_py.to_string_lossy().into_owned());
        out.extend(
            [
                "/opt/homebrew/bin/python3.14",
                "/opt/homebrew/bin/python3.13",
                "/opt/homebrew/bin/python3.12",
                "/opt/homebrew/bin/python3",
                "/usr/local/bin/python3",
                "/usr/bin/python3",
            ]
            .into_iter()
            .map(String::from),
        );
    }
    out
}

const TIMESFM_WORKER_SCRIPT: &str = r#"import json, sys, numpy as np, timesfm
req = json.loads(sys.stdin.read())
windows = [np.asarray(w, dtype=np.float32) for w in req["windows"]]
h = int(req["horizon"])
if hasattr(timesfm, "TimesFM_2p5_200M_torch"):
    model = timesfm.TimesFM_2p5_200M_torch.from_pretrained("google/timesfm-2.5-200m-pytorch")
    model.compile(timesfm.ForecastConfig(max_context=512, max_horizon=max(h, 32), normalize_inputs=True, use_continuous_quantile_head=True))
    point, quant = model.forecast(horizon=h, inputs=windows)
elif hasattr(timesfm, "TimesFm"):
    hparams = timesfm.TimesFmHparams(horizon_len=h, context_len=512)
    ckpt = timesfm.TimesFmCheckpoint(huggingface_repo_id=req["model"])
    model = timesfm.TimesFm(hparams=hparams, checkpoint=ckpt)
    point, quant = model.forecast(windows, freq=[0] * len(windows))
else:
    raise RuntimeError("Unsupported installed timesfm Python API")
bands = []
for idx, w in enumerate(windows):
    anchor = float(w[-1])
    low_p, mid_p, high_p = float(quant[idx, h - 1, 1]), float(point[idx, h - 1]), float(quant[idx, h - 1, 9])
    bands.append({"low": (low_p - anchor) / anchor, "mid": (mid_p - anchor) / anchor, "high": (high_p - anchor) / anchor})
print(json.dumps({"bands": bands}))"#;

const CHRONOS_WORKER_SCRIPT: &str = r#"import json, sys, numpy as np, torch, chronos
req = json.loads(sys.stdin.read())
windows = [np.asarray(w, dtype=np.float32) for w in req["windows"]]
h = int(req["horizon"])
pipe_cls = getattr(chronos, "BaseChronosPipeline", None) or getattr(chronos, "Chronos2Pipeline", None)
if pipe_cls is None:
    raise RuntimeError("Installed chronos package does not expose BaseChronosPipeline")
pipe = pipe_cls.from_pretrained(req["model"], device_map="cpu", dtype=torch.float32)
q_levels = [0.1, 0.5, 0.9]
ctx = [torch.tensor(w, dtype=torch.float32) for w in windows]
bands = []
if hasattr(pipe, "predict_quantiles"):
    quants, _ = pipe.predict_quantiles(ctx, prediction_length=h, quantile_levels=q_levels)
    for w, q in zip(windows, quants):
        arr = q.detach().cpu().numpy() if hasattr(q, "detach") else np.asarray(q)
        if arr.ndim == 3:
            arr = arr[0]
        anchor = float(w[-1])
        lo, mi, hi = float(arr[h - 1, 0]), float(arr[h - 1, 1]), float(arr[h - 1, 2])
        bands.append({"low": (lo - anchor) / anchor, "mid": (mi - anchor) / anchor, "high": (hi - anchor) / anchor})
else:
    samples = pipe.predict(ctx, prediction_length=h)
    for w, s in zip(windows, samples):
        arr = s.detach().cpu().numpy() if hasattr(s, "detach") else np.asarray(s)
        end_vals = arr[:, h - 1]
        anchor = float(w[-1])
        lo, mi, hi = np.quantile(end_vals, q_levels)
        bands.append({"low": (float(lo) - anchor) / anchor, "mid": (float(mi) - anchor) / anchor, "high": (float(hi) - anchor) / anchor})
print(json.dumps({"bands": bands}))"#;

#[derive(Deserialize)]
struct WorkerBatchOutput {
    bands: Vec<LocalBand>,
}

/// Build strictly chronological non-overlapping past cutoff indices ending at `last - horizon`,
/// plus the live index `last` at the end.
fn build_walk_forward_indices(len: usize, horizon: usize) -> Vec<usize> {
    let last = len - 1;
    let mut past = Vec::new();
    if let Some(last_start) = last.checked_sub(horizon) {
        let mut idx = last_start;
        while idx >= 60 && past.len() < BACKTEST_TRIALS {
            past.push(idx);
            let Some(next) = idx.checked_sub(horizon) else {
                break;
            };
            idx = next;
        }
        past.reverse();
    }
    past
}

/// Strictly causal split-conformal calibration and apples-to-apples walk-forward scoring
/// for a batch of foundation-model outputs aligned to `past_indices` + `[live]`.
pub(crate) fn evaluate_foundation_batch(
    history: &History,
    horizon: usize,
    past_indices: &[usize],
    raw_bands: &[LocalBand],
    regime: Option<&History>,
    sentiment: Option<&SentimentSeries>,
) -> Result<(LocalBand, LocalBand, FoundationScoreboard), String> {
    if raw_bands.len() != past_indices.len() + 1 {
        return Err(
            "Local foundation worker returned a mismatched number of forecast windows.".into(),
        );
    }
    for b in raw_bands {
        if !b.low.is_finite()
            || !b.mid.is_finite()
            || !b.high.is_finite()
            || b.low > b.mid
            || b.mid > b.high
        {
            return Err("Local foundation worker returned invalid or unordered quantiles.".into());
        }
    }

    // Load Outlook's walk-forward replay on the exact same history/horizon for apples-to-apples comparison.
    let outlook_trials: HashMap<i64, scenarios::Replay> = if scenarios::HORIZONS.contains(&horizon)
    {
        scenarios::report_with_context(history, horizon, regime, sentiment)
            .map(|r| r.trials.into_iter().map(|t| (t.at, t)).collect())
            .unwrap_or_default()
    } else {
        HashMap::new()
    };

    let closes = &history.closes;
    let mut scores: Vec<f64> = Vec::with_capacity(past_indices.len());
    let mut fm_raw_covered = 0usize;
    let mut fm_cal_covered = 0usize;
    let mut fm_pinball_sum = 0.0;
    let mut fm_mae_sum = 0.0;
    let mut outlook_matched = 0usize;
    let mut outlook_covered = 0usize;
    let mut outlook_pinball_sum = 0.0;
    let mut outlook_mae_sum = 0.0;
    let mut baseline_covered = 0usize;
    let mut baseline_pinball_sum = 0.0;
    let mut baseline_mae_sum = 0.0;

    // Evaluate chronologically oldest-first so conformal calibration has zero look-ahead.
    for (&idx, raw) in past_indices.iter().zip(raw_bands.iter()) {
        let actual = scenarios::change(closes[idx].price, closes[idx + horizon].price);
        let raw_b = raw.to_band();
        let delta = scenarios::conformal_adjustment(&scores);
        let cal_b = raw_b.calibrate(delta);

        fm_raw_covered += usize::from(raw_b.contains(actual));
        fm_cal_covered += usize::from(cal_b.contains(actual));
        fm_pinball_sum += cal_b.pinball_loss(actual);
        fm_mae_sum += (actual - cal_b.mid).abs();

        if let Some(trial) = outlook_trials.get(&closes[idx].time) {
            outlook_matched += 1;
            outlook_covered += usize::from(trial.inside);
            outlook_pinball_sum += trial.pinball_loss;
            outlook_mae_sum += trial.absolute_error;
            baseline_covered += usize::from(trial.baseline_inside);
            baseline_pinball_sum += trial.baseline_pinball_loss;
            baseline_mae_sum += trial.baseline_error;
        }

        // Update conformal residual strictly AFTER observing `actual`.
        scores.push((raw_b.low - actual).max(actual - raw_b.high));
    }

    let final_delta = scenarios::conformal_adjustment(&scores);
    let live_raw = *raw_bands.last().unwrap();
    let live_cal = LocalBand::from_band(live_raw.to_band().calibrate(final_delta));

    let trials = past_indices.len();
    let scoreboard = FoundationScoreboard {
        trials,
        conformal_delta: final_delta,
        fm_raw_covered,
        fm_cal_covered,
        fm_pinball: if trials > 0 {
            fm_pinball_sum / trials as f64
        } else {
            0.0
        },
        fm_mae: if trials > 0 {
            fm_mae_sum / trials as f64
        } else {
            0.0
        },
        outlook_matched_trials: outlook_matched,
        outlook_covered,
        outlook_pinball: if outlook_matched > 0 {
            outlook_pinball_sum / outlook_matched as f64
        } else {
            0.0
        },
        outlook_mae: if outlook_matched > 0 {
            outlook_mae_sum / outlook_matched as f64
        } else {
            0.0
        },
        baseline_covered,
        baseline_pinball: if outlook_matched > 0 {
            baseline_pinball_sum / outlook_matched as f64
        } else {
            0.0
        },
        baseline_mae: if outlook_matched > 0 {
            baseline_mae_sum / outlook_matched as f64
        } else {
            0.0
        },
    };
    Ok((live_raw, live_cal, scoreboard))
}

fn run_foundation_with_runner(
    history: &History,
    horizon: usize,
    backend: FoundationBackend,
    python: &str,
    runtime: String,
    script: &str,
    regime: Option<&History>,
    sentiment: Option<&SentimentSeries>,
) -> Result<LocalForecast, String> {
    if !history.valid() || history.closes.len() < 60 {
        return Err(format!(
            "At least 60 completed daily closes are required for a local {} pilot.",
            backend.short_label()
        ));
    }
    if !(1..=128).contains(&horizon) {
        return Err("Local foundation pilot horizon must be between 1 and 128 sessions.".into());
    }
    let anchor = history
        .closes
        .last()
        .ok_or("No closing prices available.")?
        .price;

    let past_indices = build_walk_forward_indices(history.closes.len(), horizon);
    let mut windows: Vec<Vec<f64>> = Vec::with_capacity(past_indices.len() + 1);
    for &idx in &past_indices {
        let start = (idx + 1).saturating_sub(512);
        windows.push(
            history.closes[start..=idx]
                .iter()
                .map(|c| c.price)
                .collect(),
        );
    }
    let live_start = history.closes.len().saturating_sub(512);
    windows.push(
        history.closes[live_start..]
            .iter()
            .map(|c| c.price)
            .collect(),
    );

    let payload = serde_json::json!({
        "windows": windows,
        "horizon": horizon,
        "model": backend.model_id(),
    })
    .to_string();

    let started = Instant::now();
    let mut cmd = Command::new(python);
    cmd.args(["-I", "-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not launch local Python runtime: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }
    loop {
        if child
            .try_wait()
            .map_err(|e| {
                format!(
                    "Could not monitor local {} worker: {e}",
                    backend.short_label()
                )
            })?
            .is_some()
        {
            break;
        }
        if started.elapsed() > Duration::from_secs(150) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "Local {} pilot timed out after 150 seconds.",
                backend.short_label()
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("Local {} worker interrupted: {e}", backend.short_label()))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let brief: String = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Check local model weights and Python runtime setup.")
            .chars()
            .take(240)
            .collect();
        return Err(format!(
            "Local {} pilot failed: {brief}",
            backend.short_label()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim().starts_with('{'))
        .ok_or_else(|| {
            format!(
                "Local {} worker did not return quantile JSON.",
                backend.short_label()
            )
        })?;
    let parsed: WorkerBatchOutput = serde_json::from_str(line.trim()).map_err(|_| {
        format!(
            "Local {} worker returned malformed quantiles.",
            backend.short_label()
        )
    })?;

    let (raw_band, band, scoreboard) = evaluate_foundation_batch(
        history,
        horizon,
        &past_indices,
        &parsed.bands,
        regime,
        sentiment,
    )?;

    Ok(LocalForecast {
        ticker: history.ticker.clone(),
        horizon,
        backend,
        model: backend.model_id().into(),
        runtime,
        anchor_price: anchor,
        raw_band,
        band,
        scoreboard,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// Opt-in local foundation model bridge (TimesFM 2.5/3.0 or Chronos-2).
/// Runs strictly on explicit user action: executes batched walk-forward replay + live forecast
/// in one model load, applies causal split-conformal calibration, and scores apples-to-apples
/// against Outlook's 4-model ensemble.
pub fn run_foundation_pilot(
    history: &History,
    horizon: usize,
    backend: FoundationBackend,
    regime: Option<&History>,
    sentiment: Option<&SentimentSeries>,
) -> Result<LocalForecast, String> {
    let probe = backend.probe_script();
    let (python, runtime) = python_candidates()
        .into_iter()
        .find_map(|py| output(&py, &["-I", "-c", probe]).map(|v| (py, v)))
        .ok_or_else(|| {
            format!(
                "Local {} Python runtime was not found. Install {} in a local Python 3.10+ environment (e.g., forecast-runtime) and re-run Check this device.",
                backend.label(),
                match backend {
                    FoundationBackend::TimesFm => "timesfm",
                    FoundationBackend::Chronos => "chronos-forecasting",
                }
            )
        })?;
    run_foundation_with_runner(
        history,
        horizon,
        backend,
        &python,
        runtime,
        backend.worker_script(),
        regime,
        sentiment,
    )
}

#[allow(dead_code)]
pub fn run_local_forecast(history: &History, horizon: usize) -> Result<LocalForecast, String> {
    run_foundation_pilot(history, horizon, FoundationBackend::TimesFm, None, None)
}

// Bounded child processes keep a broken system utility from hanging the UI worker.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let start = Instant::now();
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let result = child.wait_with_output().ok()?;
    result
        .status
        .success()
        .then(|| String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

pub fn check() -> MacCheck {
    let mut report = MacCheck {
        architecture: output("/usr/bin/uname", &["-m"])
            .or_else(|| output("/bin/uname", &["-m"]))
            .unwrap_or_else(|| std::env::consts::ARCH.into()),
        chip: output("/usr/sbin/sysctl", &["-n", "machdep.cpu.brand_string"])
            .or_else(|| output("/sbin/sysctl", &["-n", "hw.model"])),
        memory_gib: output("/usr/sbin/sysctl", &["-n", "hw.memsize"])
            .or_else(|| output("/sbin/sysctl", &["-n", "hw.physmem"]))
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v / (1 << 30)),
        macos_major: output("/usr/bin/sw_vers", &["-productVersion"])
            .and_then(|v| v.split('.').next()?.parse().ok()),
        ..Default::default()
    };
    if (report.chip.is_none() || report.memory_gib.is_none())
        && let Some(raw) = output(
            "/usr/sbin/system_profiler",
            &["SPHardwareDataType", "-json", "-detailLevel", "mini"],
        )
        && let Ok(profile) = serde_json::from_str::<serde_json::Value>(&raw)
        && let Some(hardware) = profile["SPHardwareDataType"]
            .as_array()
            .and_then(|v| v.first())
    {
        // Keep only chip/memory. Never retain serial numbers or machine identifiers.
        report.chip = report
            .chip
            .or_else(|| hardware["chip_type"].as_str().map(str::to_owned));
        report.memory_gib = report.memory_gib.or_else(|| {
            hardware["physical_memory"]
                .as_str()
                .and_then(|v| v.strip_suffix(" GB")?.trim().parse().ok())
        });
    }
    let probe = r#"import importlib.metadata as m, json, platform
def version(name):
    try: return m.version(name)
    except m.PackageNotFoundError: return None
print(json.dumps(dict(python=platform.python_version(),python_architecture=platform.machine(),timesfm=version('timesfm'),chronos=version('chronos-forecasting') or version('chronos'),mlx=version('mlx'))))"#;
    let candidates = python_candidates();
    for python in candidates {
        if let Some(raw) = output(&python, &["-I", "-c", probe])
            && let Ok(packages) = serde_json::from_str::<MacCheck>(&raw)
        {
            // Prefer a configured runtime, otherwise report the first Python found.
            if report.python.is_none() || packages.timesfm.is_some() || packages.chronos.is_some() {
                report.python = packages.python;
                report.python_architecture = packages.python_architecture;
                report.timesfm = packages.timesfm;
                report.chronos = packages.chronos;
                report.mlx = packages.mlx;
            }
            if report.compatible_runtime() {
                break;
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_or_unsuitable_hardware_never_claims_compatibility() {
        let mut mac = MacCheck {
            architecture: "arm64".into(),
            macos_major: Some(27),
            memory_gib: Some(16),
            ..Default::default()
        };
        assert_eq!(
            mac.hardware_status(),
            "Hardware suitable for a small local pilot"
        );
        mac.memory_gib = None;
        assert!(mac.hardware_status().contains("not confirmed"));
        mac.memory_gib = Some(8);
        assert!(mac.hardware_status().contains("needs a memory test"));
        mac.macos_major = Some(13);
        assert!(mac.hardware_status().contains("macOS 14"));
        mac.architecture = "x86_64".into();
        assert!(mac.hardware_status().contains("native Apple Silicon"));
    }
    #[test]
    fn packages_are_never_reported_as_working_inference() {
        let mut mac = MacCheck::default();
        assert!(mac.runtime_status().contains("Local setup required"));
        mac.timesfm = Some(RUNTIME.into());
        mac.mlx = Some("0.32.4".into());
        mac.python = Some("3.9.6".into());
        mac.python_architecture = Some("x86_64".into());
        assert!(mac.runtime_status().contains("native ARM64 Python 3.10"));
        mac.python = Some("3.10.12".into());
        mac.python_architecture = Some("arm64".into());
        assert!(mac.runtime_status().contains("inference still untested"));
        mac.timesfm = Some("2.0.2".into());
        assert!(mac.runtime_status().contains("Local setup required"));
        mac.chronos = Some("2.0.0".into());
        assert!(mac.runtime_status().contains("Chronos 2.0.0"));
    }
    #[test]
    fn foundation_walk_forward_conformal_calibration_and_scoreboard_work() {
        let history = History::sample("NVDA").unwrap();
        let sp500 = History::sample("^GSPC").unwrap();
        let horizon = 20;
        let past = build_walk_forward_indices(history.closes.len(), horizon);
        assert_eq!(past.len(), BACKTEST_TRIALS);
        // Intentionally narrow raw bands (+/- 0.5%) so causal split-conformal must widen them.
        let narrow = vec![
            LocalBand {
                low: -0.005,
                mid: 0.0,
                high: 0.005,
            };
            past.len() + 1
        ];
        let (raw, cal, board) =
            evaluate_foundation_batch(&history, horizon, &past, &narrow, Some(&sp500), None)
                .unwrap();
        assert_eq!(board.trials, BACKTEST_TRIALS);
        assert_eq!(board.outlook_matched_trials, BACKTEST_TRIALS);
        assert!(board.conformal_delta > 0.02);
        assert!(cal.low < raw.low && cal.high > raw.high);
        assert!(board.fm_cal_covered >= board.fm_raw_covered);
        assert!(board.outlook_pinball > 0.0 && board.baseline_pinball > 0.0);
        assert!(board.skill_vs_outlook().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn foundation_worker_ipc_roundtrip_via_python_subprocess() {
        let Some(python) = python_candidates()
            .into_iter()
            .find(|p| output(p, &["-I", "-c", "print('ok')"]).as_deref() == Some("ok"))
        else {
            return;
        };
        let history = History::sample("NVDA").unwrap();
        let sp500 = History::sample("^GSPC").unwrap();
        let mock_worker = r#"import json, sys
req = json.loads(sys.stdin.read())
bands = []
for w in req["windows"]:
    r = (w[-1] - w[-21]) / w[-21] if len(w) > 21 else 0.0
    mid = max(-0.2, min(0.2, r * 0.25))
    bands.append({"low": mid - 0.04, "mid": mid, "high": mid + 0.04})
print(json.dumps({"bands": bands}))"#;
        let res = run_foundation_with_runner(
            &history,
            20,
            FoundationBackend::Chronos,
            &python,
            "mock-2.0".into(),
            mock_worker,
            Some(&sp500),
            None,
        )
        .unwrap();
        assert_eq!(res.backend, FoundationBackend::Chronos);
        assert_eq!(res.model, CHRONOS_MODEL);
        assert_eq!(res.scoreboard.trials, BACKTEST_TRIALS);
        assert_eq!(res.scoreboard.outlook_matched_trials, BACKTEST_TRIALS);
        assert!(res.band.low <= res.band.mid && res.band.mid <= res.band.high);
    }
}
