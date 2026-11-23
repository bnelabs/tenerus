//! Read-only local readiness checks and opt-in local TimesFM inference bridge.
use crate::scenarios::History;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const MODEL: &str = "google/timesfm-3.0-pytorch";
pub const RUNTIME: &str = "3.0.2";
pub const PILOT: &str = "One ticker at a time · 512 daily observations · 28-session forecast";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalForecast {
    pub ticker: String,
    pub horizon: usize,
    pub model: String,
    pub runtime: String,
    pub anchor_price: f64,
    pub band: LocalBand,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct LocalBand {
    pub low: f64,
    pub mid: f64,
    pub high: f64,
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
        let packages = format!(
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
        let state = if self.compatible_runtime() {
            "Packages found; weight loading and inference still untested."
        } else if self.timesfm.as_deref() == Some(RUNTIME) && self.mlx.is_some() {
            "Packages found. A native ARM64 Python 3.10 or newer is required."
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

/// Opt-in local TimesFM inference bridge. Runs only when explicitly requested by the user,
/// passing strictly numeric closing prices over stdin to a locally installed Python runtime.
pub fn run_local_forecast(history: &History, horizon: usize) -> Result<LocalForecast, String> {
    if !history.valid() || history.closes.len() < 60 {
        return Err(
            "At least 60 completed daily closes are required for a local TimesFM pilot.".into(),
        );
    }
    if !(1..=128).contains(&horizon) {
        return Err("Local TimesFM pilot horizon must be between 1 and 128 sessions.".into());
    }
    let anchor = history
        .closes
        .last()
        .ok_or("No closing prices available.")?
        .price;
    let slice_start = history.closes.len().saturating_sub(512);
    let prices: Vec<f64> = history.closes[slice_start..]
        .iter()
        .map(|c| c.price)
        .collect();

    let probe = "import importlib.metadata as m; print(m.version('timesfm'))";
    let (python, runtime) = python_candidates()
        .into_iter()
        .find_map(|py| output(&py, &["-I", "-c", probe]).map(|v| (py, v)))
        .ok_or_else(|| {
            "Local TimesFM Python runtime was not found. Install timesfm in a local Python 3.10+ environment (e.g., forecast-runtime) and re-run Check this device.".to_string()
        })?;

    let script = r#"import json, sys, numpy as np, timesfm
req = json.loads(sys.stdin.read())
prices = np.asarray(req["prices"], dtype=np.float32)
h = int(req["horizon"])
anchor = float(prices[-1])
model = None
if hasattr(timesfm, "TimesFM_2p5_200M_torch"):
    model = timesfm.TimesFM_2p5_200M_torch.from_pretrained("google/timesfm-2.5-200m-pytorch")
    model.compile(timesfm.ForecastConfig(max_context=512, max_horizon=max(h, 32), normalize_inputs=True, use_continuous_quantile_head=True))
    point, quant = model.forecast(horizon=h, inputs=[prices])
    low_p, mid_p, high_p = float(quant[0, h - 1, 1]), float(point[0, h - 1]), float(quant[0, h - 1, 9])
elif hasattr(timesfm, "TimesFm"):
    hparams = timesfm.TimesFmHparams(horizon_len=h, context_len=512)
    ckpt = timesfm.TimesFmCheckpoint(huggingface_repo_id=req["model"])
    model = timesfm.TimesFm(hparams=hparams, checkpoint=ckpt)
    point, quant = model.forecast([prices], freq=[0])
    low_p, mid_p, high_p = float(quant[0, h - 1, 1]), float(point[0, h - 1]), float(quant[0, h - 1, 9])
else:
    raise RuntimeError("Unsupported installed timesfm Python API")
print(json.dumps({
    "low": (low_p - anchor) / anchor,
    "mid": (mid_p - anchor) / anchor,
    "high": (high_p - anchor) / anchor,
}))"#;

    let payload = serde_json::json!({
        "prices": prices,
        "horizon": horizon,
        "model": MODEL,
    })
    .to_string();

    let started = Instant::now();
    let mut cmd = Command::new(&python);
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
            .map_err(|e| format!("Could not monitor local TimesFM worker: {e}"))?
            .is_some()
        {
            break;
        }
        if started.elapsed() > Duration::from_secs(90) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Local TimesFM pilot timed out after 90 seconds.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("Local TimesFM worker interrupted: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let brief: String = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("Check local TimesFM weights and backend setup.")
            .chars()
            .take(240)
            .collect();
        return Err(format!("Local TimesFM pilot failed: {brief}"));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .rev()
        .find(|l| l.trim().starts_with('{'))
        .ok_or("Local TimesFM worker did not return quantile JSON.")?;
    let parsed: LocalBand = serde_json::from_str(line.trim())
        .map_err(|_| "Local TimesFM worker returned malformed quantiles.")?;
    if !parsed.low.is_finite()
        || !parsed.mid.is_finite()
        || !parsed.high.is_finite()
        || parsed.low > parsed.mid
        || parsed.mid > parsed.high
    {
        return Err("Local TimesFM worker returned invalid or unordered quantiles.".into());
    }
    Ok(LocalForecast {
        ticker: history.ticker.clone(),
        horizon,
        model: MODEL.into(),
        runtime,
        anchor_price: anchor,
        band: parsed,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
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
print(json.dumps(dict(python=platform.python_version(),python_architecture=platform.machine(),timesfm=version('timesfm'),mlx=version('mlx'))))"#;
    let candidates = python_candidates();
    for python in candidates {
        if let Some(raw) = output(&python, &["-I", "-c", probe])
            && let Ok(packages) = serde_json::from_str::<MacCheck>(&raw)
        {
            // Prefer a configured runtime, otherwise report the first Python found.
            if report.python.is_none() || packages.timesfm.is_some() {
                report.python = packages.python;
                report.python_architecture = packages.python_architecture;
                report.timesfm = packages.timesfm;
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
    }
}
