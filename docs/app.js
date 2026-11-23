/* Tenerus Website — Vanilla JS (zero dependencies, zero network requests) */
(function () {
  "use strict";

  const RELEASE = "0.8.0";
  const RELEASE_BASE = `https://github.com/bnelabs/tenerus/releases/download/v${RELEASE}`;

  /* ---------- Theme Toggle + Adaptive SVG Sync ---------- */
  const root = document.documentElement;
  const themeBtn = document.getElementById("theme-toggle");

  function preferredTheme() {
    try {
      const saved = localStorage.getItem("tenerus-site-theme");
      if (saved === "light" || saved === "dark") return saved;
    } catch (_) {}
    return window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  }

  function applyTheme(theme) {
    root.classList.add("theme-switching");
    root.setAttribute("data-theme", theme);
    document.querySelectorAll("img[data-light][data-dark]").forEach((img) => {
      img.src = theme === "dark" ? img.dataset.dark : img.dataset.light;
    });
    if (themeBtn) {
      themeBtn.setAttribute(
        "aria-label",
        theme === "dark" ? "Switch to light mode" : "Switch to dark mode"
      );
      themeBtn.title = theme === "dark" ? "Switch to light mode" : "Switch to dark mode";
      themeBtn.innerHTML =
        theme === "dark"
          ? '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.41 1.41m11.32 11.32l1.41 1.41M2 12h2m16 0h2M4.93 19.07l1.41-1.41m11.32-11.32l1.41-1.41"/></svg>'
          : '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>';
    }
    void root.offsetHeight;
    requestAnimationFrame(() => root.classList.remove("theme-switching"));
  }

  window.__setTenerusTheme = applyTheme;
  applyTheme(preferredTheme());
  if (themeBtn) {
    themeBtn.addEventListener("click", () => {
      const next = root.getAttribute("data-theme") === "dark" ? "light" : "dark";
      try {
        localStorage.setItem("tenerus-site-theme", next);
      } catch (_) {}
      applyTheme(next);
    });
  }

  /* ---------- Interactive Hero Screenshot Switcher ---------- */
  const VIEWS = {
    outlook: {
      src: "screenshots/outlook.png",
      alt: "Tenerus Outlook view showing calibrated 4-model fan chart, walk-forward track record, and briefing rail",
      caption: "Outlook · Expert view · Self-calibrating 4-model fan & walk-forward skill",
      note: "Shows 10th, 50th, and 90th percentile calibrated ranges over 5, 20, or 60 trading sessions—alongside real out-of-sample coverage, Pinball Skill %, and automatic model weights.",
    },
    replay: {
      src: "screenshots/market-god.png",
      alt: "Tenerus Market god view showing historical date replay and tail return stress testing",
      caption: "Outlook · Market god · Blind historical date replay & stress slider",
      note: "Jump back to any past trading date, inspect the range computed strictly from history before that day, and click Reveal to see what actually happened.",
    },
    prices: {
      src: "screenshots/prices-ai.png",
      alt: "Tenerus Price & news workspace showing interactive chart and structured 3-horizon AI research",
      caption: "Price & news · Interactive 1D–ALL chart + structured BYO AI research",
      note: "Track watchlists, fundamentals, extended-hours quotes, and public publisher news—or send a full quantitative summary to your own AI model for structured 3-horizon analysis.",
    },
    logbook: {
      src: "screenshots/logbook.png",
      alt: "Tenerus Prediction Logbook showing immutable reviews, Always BUY/SELL/WAIT baselines, and calibration",
      caption: "Prediction logbook · Automated 28 / 180 / 365-day reality checks",
      note: "Every AI decision is saved once with its reference price and automatically scored against split-adjusted closes, Always BUY/SELL/WAIT baselines, S&P 500 alpha, Brier score, and ECE.",
    },
    models: {
      src: "screenshots/models.png",
      alt: "Tenerus Forecast Model Guide showing opt-in TimesFM 3.0 and Chronos-2 local pilot controls",
      caption: "Forecast model guide · Opt-in local TimesFM 2.5/3.0 & Chronos-2 pilots",
      note: "Test pretrained local time-series foundation models head-to-head against Outlook on identical past cutoffs with split-conformal calibration—strictly opt-in via your local Python environment.",
    },
  };

  const heroImg = document.getElementById("hero-screenshot");
  const heroCaption = document.getElementById("hero-caption");
  const heroNote = document.getElementById("hero-note");

  document.querySelectorAll("[data-view]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const key = btn.dataset.view;
      const v = VIEWS[key];
      if (!v || !heroImg) return;
      document.querySelectorAll("[data-view]").forEach((b) => {
        b.classList.toggle("active", b === btn);
        b.setAttribute("aria-selected", b === btn ? "true" : "false");
      });
      heroImg.src = v.src;
      heroImg.alt = v.alt;
      if (heroCaption) heroCaption.textContent = v.caption;
      if (heroNote) heroNote.textContent = v.note;
    });
  });

  /* ---------- Interactive 4-Step Pipeline Explainer ---------- */
  const STEPS = [
    {
      title: "1. Look only at what was actually known on that day",
      plain:
        "Before estimating a range, Tenerus gathers up to 10 years of completed daily prices and volume, checks how the ticker is moving relative to the S&P 500, and reads any headline tone saved on or before that date. Future closes are completely invisible—even during historical replays.",
      eng:
        "10 strictly causal features (5d/20d/60d momentum, 20d volatility, 5d/20d vol ratio, 60d drawdown, volume/range ratios, ^GSPC relative strength & market vol, and 14-day decayed sentiment from sentiment-history.json).",
    },
    {
      title: "2. Ask four very different models, not just one",
      plain:
        "No single math formula works in every market mood. Tenerus runs four distinct local models in parallel: one looks for similar past periods, one learns relationships across all 10 features, one models how volatility calms down after storms, and one is a plain historical baseline that keeps the others honest.",
      eng:
        "M1: Kernel & recency-weighted non-overlapping analogs · M2: 10-feature L2 AdaGrad online quantile regression (with clean news ablation) · M3: Stationary mean-reverting GARCH(1,1) + Cornish-Fisher tails · M4: Unconditional non-overlapping baseline.",
    },
    {
      title: "3. Measure past misses and self-calibrate automatically",
      plain:
        "Tenerus steps backward across up to 36 past dates and checks how well each model actually performed out-of-sample. Models that missed less earn higher weight; if fancy models lose to plain history, the baseline automatically takes over. Finally, Tenerus checks whether its 80% range really caught ~80% of outcomes and widens or tightens the band accordingly.",
      eng:
        "Pinball-loss exponential Hedge weighting across strictly prior outcomes, followed by finite-sample split-conformal quantile calibration (δ = Q₀.₈₀ of past out-of-sample residuals).",
    },
    {
      title: "4. Stress-test, flag earnings risk, or race against SOTA models",
      plain:
        "You get a clear 10th / 50th / 90th percentile fan for 5, 20, or 60 sessions—plus warnings when an earnings date falls inside the window. Want a second opinion? Replay a past date blind, drag a tail-loss stress slider, or run an opt-in local TimesFM or Chronos-2 pilot scored side-by-side on the exact same past dates.",
      eng:
        "Pure-Rust egui fan chart + earnings-in-horizon detector + optional stdin-piped Python worker executing 12 walk-forward cutoffs + live window in one model load with apples-to-apples Pinball/MAE scoreboard.",
    },
  ];

  const stepTitle = document.getElementById("step-detail-title");
  const stepPlain = document.getElementById("step-detail-plain");
  const stepEng = document.getElementById("step-detail-eng");

  document.querySelectorAll("[data-step]").forEach((card) => {
    card.addEventListener("click", () => {
      const idx = Number(card.dataset.step);
      const data = STEPS[idx];
      if (!data) return;
      document.querySelectorAll("[data-step]").forEach((c) => {
        c.classList.toggle("active", c === card);
        c.setAttribute("aria-pressed", c === card ? "true" : "false");
      });
      if (stepTitle) stepTitle.textContent = data.title;
      if (stepPlain) stepPlain.textContent = data.plain;
      if (stepEng) stepEng.textContent = data.eng;
    });
  });

  /* ---------- OS-Aware Download Panel ---------- */
  const PLATFORMS = {
    macos: {
      label: "macOS",
      title: "macOS (Apple Silicon · macOS 13+)",
      subtitle: "Native arm64 .app bundle with macOS Keychain support and Core Graphics PDF export.",
      buttons: [
        { label: `Download Tenerus ${RELEASE} (Apple Silicon .zip)`, file: `Tenerus-${RELEASE}-macos-arm64.zip`, primary: true },
      ],
      cmd: `# Verify checksum, unzip, and move Tenerus.app to /Applications\nshasum -a 256 --ignore-missing -c SHA256SUMS.txt\nunzip Tenerus-${RELEASE}-macos-arm64.zip`,
      note: "Locally ad-hoc signed community build. macOS Gatekeeper may ask you to confirm on first open (Right-click → Open).",
    },
    windows: {
      label: "Windows",
      title: "Windows 10 & 11 (x86_64 & ARM64)",
      subtitle: "Native portable desktop binary with embedded icons and built-in Windows curl.exe — zero extra runtimes.",
      buttons: [
        { label: "Download Windows x64 (.zip)", file: `Tenerus-${RELEASE}-windows-x86_64.zip`, primary: true },
        { label: "Download Windows ARM64 (.zip)", file: `Tenerus-${RELEASE}-windows-arm64.zip`, primary: false },
      ],
      cmd: `# Verify checksum in PowerShell, unzip, and launch tenerus.exe\nGet-FileHash .\\Tenerus-${RELEASE}-windows-x86_64.zip -Algorithm SHA256`,
      note: "Unsigned community build. Windows SmartScreen may show a first-run prompt; you can also build from source below.",
    },
    ubuntu: {
      label: "Ubuntu / Debian",
      title: "Ubuntu & Debian (.deb · x64 & ARM64)",
      subtitle: "Native Debian packages with desktop launcher and hicolor app icon.",
      buttons: [
        { label: "Download Ubuntu amd64 (.deb)", file: `tenerus_${RELEASE}_amd64.deb`, primary: true },
        { label: "Download Ubuntu arm64 (.deb)", file: `tenerus_${RELEASE}_arm64.deb`, primary: false },
      ],
      cmd: `shasum -a 256 --ignore-missing -c SHA256SUMS.txt\nsudo apt install ./tenerus_${RELEASE}_amd64.deb`,
      note: "Supports both X11 and Wayland sessions. Data stays in $XDG_DATA_HOME/tenerus (~/.local/share/tenerus).",
    },
    fedora: {
      label: "Fedora / RHEL",
      title: "Fedora & RHEL (.rpm · x86_64 & aarch64)",
      subtitle: "Signed Fedora 43 container builds for both x86_64 and ARM64.",
      buttons: [
        { label: "Download Fedora x86_64 (.rpm)", file: `tenerus-${RELEASE}-1.fc43.x86_64.rpm`, primary: true },
        { label: "Download Fedora aarch64 (.rpm)", file: `tenerus-${RELEASE}-1.fc43.aarch64.rpm`, primary: false },
      ],
      cmd: `shasum -a 256 --ignore-missing -c SHA256SUMS.txt\nsudo dnf install ./tenerus-${RELEASE}-1.fc43.x86_64.rpm`,
      note: "Installs /usr/bin/tenerus and desktop launcher entry automatically.",
    },
    arch: {
      label: "Arch Linux",
      title: "Arch Linux (.pkg.tar.zst · x86_64 & aarch64)",
      subtitle: "Prebuilt pacman packages plus tracked PKGBUILD in packaging/arch/PKGBUILD.",
      buttons: [
        { label: "Download Arch x86_64 (.pkg.tar.zst)", file: `tenerus-${RELEASE}-1-x86_64.pkg.tar.zst`, primary: true },
        { label: "Download Arch aarch64 (.pkg.tar.zst)", file: `tenerus-${RELEASE}-1-aarch64.pkg.tar.zst`, primary: false },
      ],
      cmd: `shasum -a 256 --ignore-missing -c SHA256SUMS.txt\nsudo pacman -U ./tenerus-${RELEASE}-1-x86_64.pkg.tar.zst`,
      note: "Prefer building locally? Run cd packaging/arch && makepkg -si from a clean clone.",
    },
    linux: {
      label: "Linux Portable",
      title: "Linux Portable Tarball (.tar.gz · x64 & ARM64)",
      subtitle: "Works on any modern Linux distro with curl and OpenGL/X11 or Wayland.",
      buttons: [
        { label: "Download Linux x86_64 (.tar.gz)", file: `tenerus-${RELEASE}-linux-x86_64.tar.gz`, primary: true },
        { label: "Download Linux arm64 (.tar.gz)", file: `tenerus-${RELEASE}-linux-arm64.tar.gz`, primary: false },
      ],
      cmd: `tar -xzf tenerus-${RELEASE}-linux-x86_64.tar.gz\n./tenerus-${RELEASE}-linux-x86_64/bin/tenerus`,
      note: "No root privileges required. Run with --demo to explore offline sample data immediately.",
    },
    freebsd: {
      label: "FreeBSD",
      title: "FreeBSD 14+ (amd64 · .pkg & .tar.xz)",
      subtitle: "Built and verified on native FreeBSD 14 CI with full offline test coverage.",
      buttons: [
        { label: "Download FreeBSD amd64 (.pkg)", file: `tenerus-${RELEASE}-freebsd-amd64.pkg`, primary: true },
        { label: "Download FreeBSD amd64 (.tar.xz)", file: `tenerus-${RELEASE}-freebsd-amd64.tar.xz`, primary: false },
      ],
      cmd: `sha256 -c SHA256SUMS.txt\nsudo pkg add ./tenerus-${RELEASE}-freebsd-amd64.pkg`,
      note: "Portable .tar.xz archive is also included for non-root installation.",
    },
  };

  function detectOS() {
    const ua = (navigator.userAgent || "").toLowerCase();
    const plat = (navigator.platform || "").toLowerCase();
    if (ua.includes("freebsd")) return "freebsd";
    if (ua.includes("win") || plat.includes("win")) return "windows";
    if (ua.includes("mac") || plat.includes("mac")) return "macos";
    if (ua.includes("ubuntu") || ua.includes("debian")) return "ubuntu";
    if (ua.includes("fedora")) return "fedora";
    if (ua.includes("arch")) return "arch";
    if (ua.includes("linux") || plat.includes("linux")) return "ubuntu";
    return "macos";
  }

  const dlTitle = document.getElementById("dl-title");
  const dlSub = document.getElementById("dl-subtitle");
  const dlBtns = document.getElementById("dl-buttons");
  const dlCmd = document.getElementById("dl-cmd");
  const dlNote = document.getElementById("dl-note");
  const heroCta = document.getElementById("hero-download-cta");

  function renderPlatform(key) {
    const p = PLATFORMS[key] || PLATFORMS.macos;
    document.querySelectorAll("[data-os]").forEach((tab) => {
      tab.classList.toggle("active", tab.dataset.os === key);
      tab.setAttribute("aria-selected", tab.dataset.os === key ? "true" : "false");
    });
    if (dlTitle) dlTitle.textContent = p.title;
    if (dlSub) dlSub.textContent = p.subtitle;
    if (dlCmd) dlCmd.textContent = p.cmd;
    if (dlNote) dlNote.textContent = p.note;
    if (dlBtns) {
      dlBtns.innerHTML = "";
      p.buttons.forEach((b) => {
        const a = document.createElement("a");
        a.className = b.primary ? "btn btn-primary" : "btn btn-secondary";
        a.href = `${RELEASE_BASE}/${b.file}`;
        a.textContent = b.label;
        dlBtns.appendChild(a);
      });
    }
  }

  const detected = detectOS();
  renderPlatform(detected);
  if (heroCta && PLATFORMS[detected]) {
    heroCta.textContent = `Download for ${PLATFORMS[detected].label}`;
  }

  document.querySelectorAll("[data-os]").forEach((tab) => {
    tab.addEventListener("click", () => renderPlatform(tab.dataset.os));
  });

  /* ---------- Copy Code Buttons ---------- */
  document.querySelectorAll("[data-copy-target]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const target = document.getElementById(btn.dataset.copyTarget);
      if (!target || !navigator.clipboard) return;
      navigator.clipboard.writeText(target.textContent || "").then(() => {
        const prev = btn.textContent;
        btn.textContent = "Copied";
        setTimeout(() => {
          btn.textContent = prev;
        }, 1600);
      });
    });
  });
})();
