//! Read-only local readiness checks. This is not a forecasting connection.
use serde::{Deserialize, Serialize};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const MODEL: &str = "google/timesfm-3.0-pytorch";
pub const RUNTIME: &str = "3.0.2";
pub const PILOT: &str = "One ticker at a time · 512 daily observations · 28-session forecast";

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

    fn compatible_runtime(&self) -> bool {
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
    let local_python = crate::model::data_dir().join("forecast-runtime/bin/python3");
    let candidates = [
        local_python.to_string_lossy().into_owned(),
        "/opt/homebrew/bin/python3.14".into(),
        "/opt/homebrew/bin/python3.13".into(),
        "/opt/homebrew/bin/python3.12".into(),
        "/opt/homebrew/bin/python3".into(),
        "/usr/local/bin/python3".into(),
        "/usr/bin/python3".into(),
    ];
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
