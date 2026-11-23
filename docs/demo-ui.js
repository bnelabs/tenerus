/* Tenerus Interactive Browser Demo Controller */
(function () {
  "use strict";

  const E = window.TenerusEngine;
  if (!E) return;

  /* ---------- Theme Toggle ---------- */
  const root = document.documentElement;
  const themeBtn = document.getElementById("theme-toggle");

  function preferredTheme() {
    try {
      const s = localStorage.getItem("tenerus-site-theme");
      if (s === "light" || s === "dark") return s;
    } catch (_) {}
    return window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches
      ? "dark"
      : "light";
  }

  function applyTheme(theme) {
    root.classList.add("theme-switching");
    root.setAttribute("data-theme", theme);
    if (themeBtn) {
      themeBtn.setAttribute("aria-label", theme === "dark" ? "Switch to light mode" : "Switch to dark mode");
      themeBtn.innerHTML =
        theme === "dark"
          ? '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.41 1.41m11.32 11.32l1.41 1.41M2 12h2m16 0h2M4.93 19.07l1.41-1.41m11.32-11.32l1.41-1.41"/></svg>'
          : '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>';
    }
    if (typeof state !== "undefined" && state.activeScenario) drawFanChart();
    void root.offsetHeight;
    requestAnimationFrame(() => root.classList.remove("theme-switching"));
  }

  window.__setTenerusTheme = applyTheme;
  if (themeBtn) {
    themeBtn.addEventListener("click", () => {
      const next = root.getAttribute("data-theme") === "dark" ? "light" : "dark";
      try {
        localStorage.setItem("tenerus-site-theme", next);
      } catch (_) {}
      applyTheme(next);
    });
  }

  /* ---------- State ---------- */
  const sp500Sample = E.sampleHistory("^GSPC");
  const histories = new Map();
  for (const t of E.DEMO_TICKERS) {
    histories.set(t.ticker, t.ticker === "^GSPC" ? sp500Sample : E.sampleHistory(t.ticker));
  }

  const SAMPLE_HEADLINES = {
    NVDA: [
      "NVIDIA beats quarterly revenue outlook on record data-center growth",
      "Analysts upgrade semiconductor leaders despite new export tariff concerns",
      "Cloud demand accelerates ahead of upcoming Q4 earnings results",
    ],
    AAPL: [
      "Apple expands buyback program as services revenue posts steady growth",
      "Supply chain report flags minor product launch delay in key region",
      "Analyst reiterates outperform rating ahead of quarterly results",
    ],
    TSLA: [
      "Tesla faces analyst downgrade after delivery shortfall warning",
      "EV maker expands energy storage partnership across North America",
      "Regulator opens probe into driver-assistance software recall",
    ],
    DEFAULT: [
      "Company beats quarterly EPS expectations and raises full-year guidance",
      "Analyst notes growth is not weak despite macro interest rate pressure",
      "Board approves expanded share repurchase program",
    ],
  };

  const state = {
    ticker: "NVDA",
    horizon: 20,
    depth: "expert", // beginner | expert | market-god
    replayAt: null, // null = latest close, else timestamp
    replayRevealed: false,
    stressPct: -8.0,
    report: null,
    replayData: null,
    activeScenario: null,
    lastNewsSignal: null,
    chartMeta: null,
  };

  /* ---------- DOM References ---------- */
  const el = (id) => document.getElementById(id);
  const tickerStrip = el("ticker-strip");
  const provBadge = el("prov-badge");
  const wsName = el("ws-name");
  const wsPrice = el("ws-price");
  const wsDelta = el("ws-delta");
  const wsSub = el("ws-sub");
  const replayBar = el("replay-bar");
  const replaySelect = el("replay-select");
  const revealBtn = el("reveal-btn");
  const replayResult = el("replay-result");
  const summaryLine = el("summary-line");
  const engineError = el("engine-error");
  const fanSvg = el("fan-svg");
  const chartBox = el("chart-box");
  const chartTooltip = el("chart-tooltip");
  const legendRealized = el("legend-realized");
  const evalSection = el("eval-section");
  const stressBox = el("stress-box");
  const stressSlider = el("stress-slider");
  const stressVal = el("stress-val");
  const stressSummary = el("stress-summary");
  const newsInput = el("news-input");
  const earningsDaysInput = el("earnings-days");
  const newsOutput = el("news-output");

  /* ---------- Ticker Strip ---------- */
  function renderTickerStrip() {
    tickerStrip.innerHTML = "";
    for (const [sym, hist] of histories.entries()) {
      const last = hist.closes[hist.closes.length - 1];
      const pct = hist.pct || 0;
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "ticker-chip" + (sym === state.ticker ? " active" : "");
      btn.setAttribute("role", "tab");
      btn.setAttribute("aria-selected", sym === state.ticker ? "true" : "false");
      btn.innerHTML = `
        <span class="ticker-chip-sym">${escapeHtml(sym)}</span>
        <span class="ticker-chip-price">${escapeHtml(E.formatPrice(last.price, hist.currency))}</span>
        <span class="ticker-chip-pct ${pct >= 0 ? "pos" : "neg"}">${pct >= 0 ? "+" : ""}${pct.toFixed(2)}%</span>
      `;
      btn.addEventListener("click", () => {
        state.ticker = sym;
        state.replayAt = null;
        state.replayRevealed = false;
        loadTickerHeadlines(sym);
        computeAndRender();
      });
      tickerStrip.appendChild(btn);
    }
  }

  function loadTickerHeadlines(sym) {
    const lines = SAMPLE_HEADLINES[sym] || SAMPLE_HEADLINES.DEFAULT;
    newsInput.value = lines.join("\n");
    earningsDaysInput.value = sym === "NVDA" ? "14" : "";
    renderNewsSignal();
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  /* ---------- Compute & Render Workspace ---------- */
  function computeAndRender() {
    renderTickerStrip();
    const history = histories.get(state.ticker);
    if (!history) return;

    const last = history.closes[history.closes.length - 1];
    const pct = history.pct || 0;
    wsName.textContent = history.name || history.ticker;
    wsPrice.textContent = E.formatPrice(last.price, history.currency);
    wsDelta.textContent = `${pct >= 0 ? "+" : ""}${pct.toFixed(2)}%`;
    wsDelta.className = "ws-delta " + (pct >= 0 ? "pos" : "neg");
    wsSub.textContent = `${history.ticker} · ${history.source} · Close ${E.formatDate(last.time)}`;

    provBadge.className = "prov-badge" + (history.sample ? "" : " live");
    provBadge.textContent = history.sample
      ? "● SAMPLE DATA · Simulated 10y daily closes (runs 100% locally in your browser)"
      : `● USER DATA · ${history.source} (${history.closes.length} daily closes · processed locally)`;

    // Update depth & horizon pills
    document.querySelectorAll("[data-depth]").forEach((b) => {
      b.classList.toggle("active", b.dataset.depth === state.depth);
    });
    document.querySelectorAll("[data-horizon]").forEach((b) => {
      b.classList.toggle("active", Number(b.dataset.horizon) === state.horizon);
    });

    const regime = history.sample ? sp500Sample : histories.get("^GSPC") || null;

    try {
      state.report = E.runReport(history, state.horizon, regime);
      engineError.classList.add("hidden");
    } catch (err) {
      state.report = null;
      state.activeScenario = null;
      engineError.textContent = err.message || String(err);
      engineError.classList.remove("hidden");
      summaryLine.textContent = "";
      fanSvg.innerHTML = "";
      return;
    }

    // Populate replay selector
    replaySelect.innerHTML = '<option value="">Latest close</option>';
    for (const t of state.report.trials) {
      const opt = document.createElement("option");
      opt.value = String(t.at);
      opt.textContent = E.formatDate(t.at);
      if (state.replayAt === t.at) opt.selected = true;
      replaySelect.appendChild(opt);
    }

    const isMarketGod = state.depth === "market-god";
    replayBar.classList.toggle("hidden", !isMarketGod);
    stressBox.classList.toggle("hidden", !isMarketGod);

    if (isMarketGod && state.replayAt !== null) {
      const trial = state.report.trials.find((t) => t.at === state.replayAt);
      if (trial) {
        const rep = E.runReplay(history, state.replayAt, state.horizon, regime);
        state.replayData = rep ? { trial, ...rep } : null;
      } else {
        state.replayAt = null;
        state.replayData = null;
      }
    } else {
      state.replayData = null;
      state.replayRevealed = false;
    }

    const active = state.replayData ? state.replayData.scenario : state.report.current;
    state.activeScenario = active;

    // Replay bar controls
    if (state.replayData) {
      revealBtn.classList.toggle("hidden", state.replayRevealed);
      if (state.replayRevealed) {
        const t = state.replayData.trial;
        replayResult.textContent = `${E.formatPct(t.result)} on ${E.formatDate(t.through)} · ${t.inside ? "Inside" : "Outside"} range`;
        replayResult.classList.remove("hidden");
      } else {
        replayResult.classList.add("hidden");
      }
    } else {
      revealBtn.classList.add("hidden");
      replayResult.classList.add("hidden");
    }

    const end = active.end;
    const h = active.fan.length - 1;
    const prefix = state.replayData
      ? `Blind replay from ${E.formatDate(active.anchor.time)} (strictly prior history only)`
      : `From the ${E.formatDate(active.anchor.time)} close`;
    summaryLine.textContent = `${prefix}, ${active.upCount} of ${active.matches.length} similar periods ended higher. Calibrated 80% range: ${E.formatPct(end.low)} to ${E.formatPct(end.high)} over ${h} sessions.`;

    const anchor = active.anchor.price;
    const cur = history.currency;
    el("out-low-price").textContent = E.formatPrice(anchor * (1 + end.low), cur);
    el("out-low-pct").textContent = E.formatPct(end.low);
    el("out-mid-price").textContent = E.formatPrice(anchor * (1 + end.mid), cur);
    el("out-mid-pct").textContent = E.formatPct(end.mid);
    el("out-mid-pct").className = "outcome-pct " + (end.mid >= 0 ? "pos" : "neg");
    el("out-high-price").textContent = E.formatPrice(anchor * (1 + end.high), cur);
    el("out-high-pct").textContent = E.formatPct(end.high);

    // Watch levels & similar periods
    el("watch-low").textContent = E.formatPrice(anchor * (1 + end.low), cur);
    el("watch-high").textContent = E.formatPrice(anchor * (1 + end.high), cur);
    el("watch-note").textContent = `A close outside ${E.formatPrice(anchor * (1 + end.low), cur)}–${E.formatPrice(anchor * (1 + end.high), cur)} after ${h} sessions falls outside the calibrated 80% band.`;

    const analogRows = el("analog-rows");
    analogRows.innerHTML = active.matches
      .slice(0, state.depth === "beginner" ? 3 : 5)
      .map(
        (m) => `
        <div class="analog-row">
          <span class="analog-dates">${E.formatDate(m.start)} → ${E.formatDate(m.end)}</span>
          <strong class="${m.realized >= 0 ? "pos" : "neg"}">${E.formatPct(m.realized)}</strong>
        </div>`
      )
      .join("");

    // Walk-forward evaluation
    evalSection.classList.toggle("hidden", state.depth === "beginner");
    if (state.depth !== "beginner") {
      renderEvaluation(active, state.replayData ? null : state.report.trials);
    }

    // Stress readout
    if (isMarketGod) renderStressReadout(active);

    legendRealized.classList.toggle("hidden", !(state.replayData && state.replayRevealed));
    drawFanChart();
    renderNewsSignal();
  }

  function renderEvaluation(scenario, trials) {
    const cal = scenario.calibration;
    if (trials && trials.length) {
      const n = trials.length;
      const mae = (trials.reduce((a, t) => a + t.absoluteError, 0) / n) * 100;
      const baseMae = (trials.reduce((a, t) => a + t.baselineError, 0) / n) * 100;
      el("m-coverage").textContent = `${cal.calibratedCovered}/${n} (${Math.round((100 * cal.calibratedCovered) / n)}%)`;
      el("m-coverage-sub").textContent = `Baseline ${cal.baselineCovered}/${n} · Raw ${cal.rawCovered}/${n}`;
      el("m-pinball").textContent = `${(cal.ensemblePinball * 100).toFixed(2)} pp`;
      el("m-pinball-sub").textContent = `vs ${(cal.baselinePinball * 100).toFixed(2)} pp (${cal.pinballSkill >= 0 ? "+" : ""}${(cal.pinballSkill * 100).toFixed(1)}% skill)`;
      el("m-conformal").textContent = `${cal.conformalDelta >= 0 ? "+" : ""}${(cal.conformalDelta * 100).toFixed(1)} pp`;
      el("m-mae-sub").textContent = `Median MAE ${mae.toFixed(1)} vs ${baseMae.toFixed(1)} pp`;
    } else {
      el("m-coverage").textContent = "Hidden in replay";
      el("m-coverage-sub").textContent = "Prior trials used strictly causally";
      el("m-pinball").textContent = "—";
      el("m-pinball-sub").textContent = "Exit replay for aggregate skill";
      el("m-conformal").textContent = `${cal.conformalDelta >= 0 ? "+" : ""}${(cal.conformalDelta * 100).toFixed(1)} pp`;
      el("m-mae-sub").textContent = "Split-conformal adjustment at cutoff";
    }

    const items = [
      { name: "Kernel Analogs", w: cal.weights.analogs, pb: cal.analogPinball, color: "var(--accent)" },
      { name: "Online QR (10f)", w: cal.weights.onlineQr, pb: cal.onlineQrPinball, color: "var(--blue)" },
      { name: "GARCH(1,1)", w: cal.weights.garch, pb: cal.garchPinball, color: "var(--teal)" },
      { name: "Baseline", w: cal.weights.baseline, pb: cal.baselinePinball, color: "var(--faint)" },
    ];
    el("weights-list").innerHTML = items
      .map(
        (m) => `
        <div class="weight-row">
          <span>${m.name}</span>
          <div class="weight-track"><div class="weight-fill" style="width:${(m.w * 100).toFixed(1)}%;background:${m.color};"></div></div>
          <span class="weight-pct">${Math.round(m.w * 100)}%</span>
          <span class="weight-pb">${(m.pb * 100).toFixed(2)} pp</span>
        </div>`
      )
      .join("");

    const s = scenario.setup;
    const regimePart =
      s.relStrength20 !== null && s.marketVol20 !== null
        ? ` · vs ^GSPC ${E.formatPct(s.relStrength20)} (mkt vol ${(s.marketVol20 * 100).toFixed(2)}%)`
        : "";
    const volPart = s.volumeRatio !== null ? ` · vol 5d/20d ${s.volumeRatio.toFixed(2)}x` : "";
    el("setup-strip").textContent =
      `Setup · 5d/20d/60d ${E.formatPct(s.momentum5)}, ${E.formatPct(s.momentum)}, ${E.formatPct(s.momentum60)} · daily vol ${(s.dailyVolatility * 100).toFixed(2)}% (${s.volRatio.toFixed(2)}x) · 60d drawdown ${E.formatPct(s.drawdown)}${volPart}${regimePart}`;
  }

  function renderStressReadout(scenario) {
    const target = state.stressPct / 100;
    const matched = scenario.matches.filter((m) => m.realized <= target).length;
    const base = scenario.baselineReturns.filter((r) => r <= target).length;
    stressVal.textContent = E.formatPct(target);
    stressSummary.textContent = `${E.formatPct(target)} or worse happened in ${matched}/${scenario.matches.length} similar periods and ${base}/${scenario.baselineReturns.length} unfiltered historical periods.`;
  }

  /* ---------- Interactive Responsive SVG Fan Chart ---------- */
  function cssVar(name) {
    return getComputedStyle(root).getPropertyValue(name).trim();
  }

  function niceStep(raw) {
    if (!Number.isFinite(raw) || raw <= 0) return 1;
    const base = Math.pow(10, Math.floor(Math.log10(raw)));
    const f = raw / base;
    return base * (f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10);
  }

  function drawFanChart() {
    const s = state.activeScenario;
    const history = histories.get(state.ticker);
    if (!s || !history) return;

    const W = 760,
      H = 320;
    const padL = 62,
      padR = 76,
      padT = 20,
      padB = 34;
    const plotW = W - padL - padR;
    const plotH = H - padT - padB;
    const pivot = padL + plotW * 0.54;
    const anchor = s.anchor.price;
    const proj = (r) => anchor * (1 + r);
    const realized = state.replayData && state.replayRevealed ? state.replayData.realized : null;
    const stressRet = state.depth === "market-god" ? state.stressPct / 100 : null;

    let min = anchor,
      max = anchor;
    for (const b of s.past) {
      if (b.price < min) min = b.price;
      if (b.price > max) max = b.price;
    }
    for (const b of s.fan) {
      const lo = proj(b.low),
        hi = proj(b.high);
      if (lo < min) min = lo;
      if (hi > max) max = hi;
    }
    if (realized) {
      for (const r of realized) {
        const v = proj(r);
        if (v < min) min = v;
        if (v > max) max = v;
      }
    }
    const padY = Math.max((max - min) * 0.09, anchor * 0.008);
    min = Math.max(0, min - padY);
    max = max + padY;

    const y = (val) => padT + plotH - ((val - min) / (max - min)) * plotH;
    const pastX = (idx) => padL + (idx / Math.max(1, s.past.length - 1)) * (pivot - padL);
    const horizon = s.fan.length - 1;
    const fwdX = (step) => pivot + (step / Math.max(1, horizon)) * (padL + plotW - pivot);

    const isDark = root.getAttribute("data-theme") === "dark";
    const cText = isDark ? "#eef4f0" : "#14221b";
    const cMuted = isDark ? "#9eb3a8" : "#52645a";
    const cFaint = isDark ? "#73887d" : "#7e9085";
    const cBorder = isDark ? "#23332b" : "#dfe5e0";
    const cGreen = isDark ? "#4cc38f" : "#1d634a";
    const cRed = isDark ? "#e57d6e" : "#ab493b";
    const cSurface = isDark ? "#141f1b" : "#ffffff";
    const fanFill = isDark ? "rgba(76, 195, 143, 0.19)" : "rgba(29, 99, 74, 0.16)";
    const cPurple = isDark ? "#c8a4ee" : "#734a99";

    const parts = [];
    // Gridlines
    const step = niceStep((max - min) / 4.5);
    const startVal = Math.ceil(min / step) * step;
    for (let v = startVal; v <= max; v += step) {
      const yy = y(v);
      parts.push(`<line x1="${padL}" y1="${yy.toFixed(1)}" x2="${padL + plotW}" y2="${yy.toFixed(1)}" stroke="${cBorder}" stroke-width="1"/>`);
      const lbl = step < 0.1 ? v.toFixed(2) : v >= 10000 ? `${(v / 1000).toFixed(1)}k` : v.toFixed(0);
      parts.push(`<text x="${padL - 8}" y="${(yy + 3.5).toFixed(1)}" text-anchor="end" font-family="ui-monospace,monospace" font-size="10.5" fill="${cMuted}">${lbl}</text>`);
    }

    // Pivot dashed line
    parts.push(`<line x1="${pivot.toFixed(1)}" y1="${padT}" x2="${pivot.toFixed(1)}" y2="${padT + plotH}" stroke="${cFaint}" stroke-width="1.2" stroke-dasharray="4 4"/>`);

    // Stress horizontal target line
    if (stressRet !== null) {
      const sv = proj(stressRet);
      if (sv > min && sv < max) {
        const sy = y(sv);
        parts.push(`<line x1="${pivot.toFixed(1)}" y1="${sy.toFixed(1)}" x2="${padL + plotW}" y2="${sy.toFixed(1)}" stroke="${cPurple}" stroke-opacity="0.7" stroke-width="1.3" stroke-dasharray="5 3"/>`);
      }
    }

    // Shaded calibrated 80% fan polygon
    const topPts = s.fan.map((b, idx) => `${fwdX(idx).toFixed(1)},${y(proj(b.high)).toFixed(1)}`);
    const botPts = s.fan
      .slice()
      .reverse()
      .map((b, rIdx) => `${fwdX(horizon - rIdx).toFixed(1)},${y(proj(b.low)).toFixed(1)}`);
    parts.push(`<polygon points="${topPts.concat(botPts).join(" ")}" fill="${fanFill}"/>`);

    // Past close line
    const pastPts = s.past.map((b, idx) => `${pastX(idx).toFixed(1)},${y(b.price).toFixed(1)}`).join(" ");
    parts.push(`<polyline points="${pastPts}" fill="none" stroke="${cText}" stroke-width="2.2" stroke-linejoin="round" stroke-linecap="round"/>`);

    // Fan 90th / 50th / 10th lines
    const hiPts = s.fan.map((b, idx) => `${fwdX(idx).toFixed(1)},${y(proj(b.high)).toFixed(1)}`).join(" ");
    const mdPts = s.fan.map((b, idx) => `${fwdX(idx).toFixed(1)},${y(proj(b.mid)).toFixed(1)}`).join(" ");
    const loPts = s.fan.map((b, idx) => `${fwdX(idx).toFixed(1)},${y(proj(b.low)).toFixed(1)}`).join(" ");
    parts.push(`<polyline points="${hiPts}" fill="none" stroke="${cGreen}" stroke-width="1.6"/>`);
    parts.push(`<polyline points="${mdPts}" fill="none" stroke="${cText}" stroke-width="2.2"/>`);
    parts.push(`<polyline points="${loPts}" fill="none" stroke="${cRed}" stroke-width="1.6"/>`);

    // Revealed realized path
    if (realized) {
      const rPts = realized.map((r, idx) => `${fwdX(idx).toFixed(1)},${y(proj(r)).toFixed(1)}`).join(" ");
      parts.push(`<polyline points="${rPts}" fill="none" stroke="${cPurple}" stroke-width="2.6" stroke-linejoin="round" stroke-linecap="round"/>`);
    }

    // Anchor & endpoint dots
    parts.push(`<circle cx="${pivot.toFixed(1)}" cy="${y(anchor).toFixed(1)}" r="4.2" fill="${cText}"/>`);
    const endX = padL + plotW;
    const endLabels = [
      { yPos: y(proj(s.end.high)), ret: s.end.high, col: cGreen },
      { yPos: y(proj(s.end.mid)), ret: s.end.mid, col: cText },
      { yPos: y(proj(s.end.low)), ret: s.end.low, col: cRed },
    ];
    // Prevent label collision on right edge
    const minGap = 16;
    endLabels[0].ly = Math.min(endLabels[0].yPos, padT + plotH - 2 * minGap);
    endLabels[1].ly = Math.min(Math.max(endLabels[1].yPos, endLabels[0].ly + minGap), padT + plotH - minGap);
    endLabels[2].ly = Math.max(endLabels[2].yPos, endLabels[1].ly + minGap);

    for (const item of endLabels) {
      parts.push(`<circle cx="${endX.toFixed(1)}" cy="${item.yPos.toFixed(1)}" r="3.4" fill="${item.col}"/>`);
      const txt = E.formatPrice(proj(item.ret), history.currency);
      parts.push(`<rect x="${(endX + 4).toFixed(1)}" y="${(item.ly - 8.5).toFixed(1)}" width="68" height="16" rx="4" fill="${cSurface}" fill-opacity="0.9"/>`);
      parts.push(`<text x="${(endX + 7).toFixed(1)}" y="${(item.ly + 3.5).toFixed(1)}" font-family="ui-monospace,monospace" font-size="10.5" font-weight="600" fill="${item.col}">${escapeHtml(txt)}</text>`);
    }

    // X-axis captions
    const bottomY = H - 10;
    parts.push(`<text x="${padL}" y="${bottomY}" font-family="ui-monospace,monospace" font-size="10.5" fill="${cMuted}">${E.formatDate(s.past[0].time)}</text>`);
    parts.push(`<text x="${pivot.toFixed(1)}" y="${bottomY}" text-anchor="middle" font-family="ui-monospace,monospace" font-size="10.5" fill="${cMuted}">${state.replayData ? "Replay cutoff" : "Latest close"}</text>`);
    parts.push(`<text x="${endX}" y="${bottomY}" text-anchor="end" font-family="ui-monospace,monospace" font-size="10.5" fill="${cMuted}">+${horizon} sessions</text>`);

    // Crosshair group (updated on pointer move)
    parts.push(`<line id="svg-xhair" x1="0" y1="${padT}" x2="0" y2="${padT + plotH}" stroke="${cFaint}" stroke-width="1" stroke-dasharray="3 3" visibility="hidden"/>`);

    fanSvg.innerHTML = parts.join("");
    state.chartMeta = { W, H, padL, plotW, padT, plotH, pivot, horizon, s, history, realized, pastX, fwdX };
  }

  function handleChartPointer(clientX) {
    const m = state.chartMeta;
    if (!m) return;
    const rect = fanSvg.getBoundingClientRect();
    const xSvg = ((clientX - rect.left) / rect.width) * m.W;
    if (xSvg < m.padL || xSvg > m.padL + m.plotW) {
      hideTooltip();
      return;
    }
    const xhair = document.getElementById("svg-xhair");
    const cur = m.history.currency;
    const anchor = m.s.anchor.price;
    let snapX, title, body;
    if (xSvg >= m.pivot) {
      const idx = Math.max(0, Math.min(m.horizon, Math.round(((xSvg - m.pivot) / (m.padL + m.plotW - m.pivot)) * m.horizon)));
      snapX = m.fwdX(idx);
      const b = m.s.fan[idx];
      title = idx === 0 ? `Anchor · ${E.formatDate(m.s.anchor.time)}` : `+${idx} trading sessions`;
      body = `Range: ${E.formatPrice(anchor * (1 + b.low), cur)} – ${E.formatPrice(anchor * (1 + b.high), cur)}<br>Median: ${E.formatPrice(anchor * (1 + b.mid), cur)} (${E.formatPct(b.mid)})`;
      if (m.realized && idx < m.realized.length) {
        body += `<br>Realized: <strong>${E.formatPrice(anchor * (1 + m.realized[idx]), cur)} (${E.formatPct(m.realized[idx])})</strong>`;
      }
    } else {
      const idx = Math.max(0, Math.min(m.s.past.length - 1, Math.round(((xSvg - m.padL) / (m.pivot - m.padL)) * (m.s.past.length - 1))));
      snapX = m.pastX(idx);
      const bar = m.s.past[idx];
      title = E.formatDate(bar.time);
      body = `Close: <strong>${E.formatPrice(bar.price, cur)}</strong>`;
    }
    if (xhair) {
      xhair.setAttribute("x1", snapX.toFixed(1));
      xhair.setAttribute("x2", snapX.toFixed(1));
      xhair.setAttribute("visibility", "visible");
    }
    chartTooltip.innerHTML = `<strong>${title}</strong>${body}`;
    chartTooltip.style.display = "block";
    // Flip tooltip side if pointer is near left edge
    if (xSvg < m.W * 0.45) {
      chartTooltip.style.left = "auto";
      chartTooltip.style.right = "12px";
    } else {
      chartTooltip.style.right = "auto";
      chartTooltip.style.left = "12px";
    }
  }

  function hideTooltip() {
    const xhair = document.getElementById("svg-xhair");
    if (xhair) xhair.setAttribute("visibility", "hidden");
    chartTooltip.style.display = "none";
  }

  chartBox.addEventListener("pointermove", (e) => handleChartPointer(e.clientX));
  chartBox.addEventListener("pointerdown", (e) => handleChartPointer(e.clientX));
  chartBox.addEventListener("pointerleave", hideTooltip);

  /* ---------- Controls Listeners ---------- */
  document.querySelectorAll("[data-depth]").forEach((btn) => {
    btn.addEventListener("click", () => {
      state.depth = btn.dataset.depth;
      if (state.depth !== "market-god") {
        state.replayAt = null;
        state.replayRevealed = false;
      }
      computeAndRender();
    });
  });

  document.querySelectorAll("[data-horizon]").forEach((btn) => {
    btn.addEventListener("click", () => {
      state.horizon = Number(btn.dataset.horizon);
      state.replayAt = null;
      state.replayRevealed = false;
      computeAndRender();
    });
  });

  replaySelect.addEventListener("change", () => {
    state.replayAt = replaySelect.value ? Number(replaySelect.value) : null;
    state.replayRevealed = false;
    computeAndRender();
  });

  revealBtn.addEventListener("click", () => {
    state.replayRevealed = true;
    computeAndRender();
  });

  stressSlider.addEventListener("input", () => {
    state.stressPct = Number(stressSlider.value);
    if (state.activeScenario) {
      renderStressReadout(state.activeScenario);
      drawFanChart();
    }
  });

  /* ---------- Local Headline Tone & Earnings Risk ---------- */
  function renderNewsSignal() {
    const lines = (newsInput.value || "").split(/\r?\n/);
    const earnRaw = earningsDaysInput.value.trim();
    const earnDays = earnRaw === "" ? NaN : Number(earnRaw);
    const sig = E.analyzeHeadlines(lines, earnDays, state.horizon);
    state.lastNewsSignal = sig;

    const toneCls = sig.score >= 0.18 ? "" : sig.score <= -0.18 ? "neg" : "blue";
    let html = `
      <div class="tone-row">
        <span class="pill-tag ${toneCls}">${escapeHtml(sig.label)} (${sig.score >= 0 ? "+" : ""}${sig.score.toFixed(2)})</span>
        <span style="font-size: 11.5px; color: var(--muted);">${sig.articleCount} headlines (+${sig.positiveCount} / −${sig.negativeCount})</span>
      </div>`;
    if (sig.eventTags.length) {
      html += `<div style="font-size: 11.5px; color: var(--muted);">Topics · ${escapeHtml(sig.eventTags.join(" · "))}</div>`;
    }
    if (sig.earningsInHorizon) {
      html += `<div class="earnings-alert">Expected earnings in ~${sig.earningsDays}d (inside ${state.horizon}-session window) · elevated gap risk.</div>`;
    }
    newsOutput.innerHTML = html;
  }

  newsInput.addEventListener("input", renderNewsSignal);
  earningsDaysInput.addEventListener("input", renderNewsSignal);

  /* ---------- Evidence Pack Copy + Optional BYO AI API ---------- */
  const copyEvBtn = el("copy-evidence-btn");
  const copyEvStatus = el("copy-evidence-status");

  copyEvBtn.addEventListener("click", () => {
    if (!state.activeScenario) return;
    const hist = histories.get(state.ticker);
    const pack = E.buildEvidencePack(
      hist,
      state.activeScenario,
      state.replayData ? null : state.report.trials,
      state.lastNewsSignal
    );
    if (navigator.clipboard) {
      navigator.clipboard.writeText(pack).then(() => {
        copyEvStatus.textContent = "Copied E1–E7 evidence pack — paste into ChatGPT, Claude, Codex, or Antigravity.";
        copyEvStatus.classList.remove("hidden");
        setTimeout(() => copyEvStatus.classList.add("hidden"), 4000);
      });
    }
  });

  const AI_PRESETS = {
    gemini: {
      endpoint: "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent",
      model: "gemini-2.5-flash",
    },
    openai: {
      endpoint: "https://api.openai.com/v1/chat/completions",
      model: "gpt-4.1-mini",
    },
    openrouter: {
      endpoint: "https://openrouter.ai/api/v1/chat/completions",
      model: "openai/gpt-4.1-mini",
    },
  };

  const aiPreset = el("ai-preset");
  const aiEndpoint = el("ai-endpoint");
  const aiModel = el("ai-model");
  const aiKey = el("ai-key");
  const aiRunBtn = el("ai-run-btn");
  const aiStatus = el("ai-status");
  const aiOutput = el("ai-output");

  function syncAiPreset() {
    const p = AI_PRESETS[aiPreset.value] || AI_PRESETS.gemini;
    aiEndpoint.value = p.endpoint;
    aiModel.value = p.model;
  }
  syncAiPreset();
  aiPreset.addEventListener("change", syncAiPreset);

  aiRunBtn.addEventListener("click", async () => {
    if (!state.activeScenario) return;
    const endpoint = aiEndpoint.value.trim();
    const model = aiModel.value.trim();
    const key = aiKey.value.trim();
    if (!endpoint || !key) {
      aiStatus.textContent = "Enter an endpoint URL and session API key first.";
      aiStatus.className = "status-msg err";
      aiStatus.classList.remove("hidden");
      return;
    }
    try {
      const u = new URL(endpoint);
      const isLocal = u.hostname === "localhost" || u.hostname === "127.0.0.1";
      if (u.protocol !== "https:" && !isLocal) {
        throw new Error("Remote AI endpoints must use HTTPS.");
      }
      aiRunBtn.disabled = true;
      aiStatus.textContent = "Requesting structured read directly from your provider…";
      aiStatus.className = "status-msg";
      aiStatus.classList.remove("hidden");
      aiOutput.classList.add("hidden");

      const hist = histories.get(state.ticker);
      const prompt = E.buildEvidencePack(
        hist,
        state.activeScenario,
        state.replayData ? null : state.report.trials,
        state.lastNewsSignal
      );

      let replyText = "";
      if (aiPreset.value === "gemini" || endpoint.includes("generativelanguage.googleapis.com")) {
        const res = await fetch(endpoint, {
          method: "POST",
          headers: { "Content-Type": "application/json", "x-goog-api-key": key },
          body: JSON.stringify({ contents: [{ parts: [{ text: prompt }] }] }),
        });
        const data = await res.json();
        if (!res.ok) throw new Error(data?.error?.message || `HTTP ${res.status}`);
        replyText = (data?.candidates?.[0]?.content?.parts || []).map((p) => p.text || "").join("\n");
      } else {
        const res = await fetch(endpoint, {
          method: "POST",
          headers: { "Content-Type": "application/json", Authorization: `Bearer ${key}` },
          body: JSON.stringify({
            model,
            messages: [
              { role: "system", content: "You are a cautious, evidence-cited educational market research assistant. Never give financial advice." },
              { role: "user", content: prompt },
            ],
          }),
        });
        const data = await res.json();
        if (!res.ok) throw new Error(data?.error?.message || `HTTP ${res.status}`);
        replyText = data?.choices?.[0]?.message?.content || "";
      }
      aiOutput.textContent = replyText.trim() || "Provider returned an empty reply.";
      aiOutput.classList.remove("hidden");
      aiStatus.textContent = "Completed (educational context only — verify against primary filings).";
      aiStatus.className = "status-msg ok";
    } catch (err) {
      aiStatus.textContent = `Request failed: ${err.message}. (Some providers block browser CORS; Copy Evidence Pack always works.)`;
      aiStatus.className = "status-msg err";
    } finally {
      aiRunBtn.disabled = false;
    }
  });

  /* ---------- CSV / JSON Local Upload & Optional Live Fetch ---------- */
  const dropzone = el("dropzone");
  const fileInput = el("file-input");
  const uploadStatus = el("upload-status");
  const quickUploadBtn = el("quick-upload-btn");

  function handleFile(file) {
    if (!file) return;
    if (file.size > 5 * 1024 * 1024) {
      uploadStatus.textContent = "File exceeds 5 MB limit.";
      uploadStatus.className = "status-msg err";
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      try {
        const series = E.parseUploadedSeries(String(reader.result || ""), file.name);
        histories.set(series.ticker, series);
        state.ticker = series.ticker;
        state.replayAt = null;
        state.replayRevealed = false;
        uploadStatus.textContent = `Loaded ${series.closes.length} closes`;
        uploadStatus.className = "status-msg ok";
        computeAndRender();
      } catch (err) {
        uploadStatus.textContent = err.message || String(err);
        uploadStatus.className = "status-msg err";
      }
    };
    reader.readAsText(file);
  }

  dropzone.addEventListener("click", () => fileInput.click());
  dropzone.addEventListener("keydown", (e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      fileInput.click();
    }
  });
  if (quickUploadBtn) quickUploadBtn.addEventListener("click", () => fileInput.click());
  fileInput.addEventListener("change", () => handleFile(fileInput.files[0]));
  dropzone.addEventListener("dragover", (e) => {
    e.preventDefault();
    dropzone.classList.add("dragover");
  });
  dropzone.addEventListener("dragleave", () => dropzone.classList.remove("dragover"));
  dropzone.addEventListener("drop", (e) => {
    e.preventDefault();
    dropzone.classList.remove("dragover");
    handleFile(e.dataTransfer?.files?.[0]);
  });

  el("download-csv-tpl").addEventListener("click", () => {
    const sample = histories.get("NVDA");
    const rows = [
      "Date,Open,High,Low,Close,Volume",
      ...sample.closes
        .slice(-500)
        .map((b) => `${E.formatDate(b.time)},${b.price.toFixed(2)},${b.high.toFixed(2)},${b.low.toFixed(2)},${b.price.toFixed(2)},${Math.round(b.volume)}`),
    ].join("\n");
    const blob = new Blob([rows], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "tenerus-sample-nvda.csv";
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  });

  const feedProvider = el("feed-provider");
  const feedTicker = el("feed-ticker");
  const feedKey = el("feed-key");
  const feedBtn = el("feed-fetch-btn");
  const feedStatus = el("feed-status");

  feedBtn.addEventListener("click", async () => {
    const sym = feedTicker.value.trim().toUpperCase().replace(/[^A-Z0-9.\-^=]/g, "");
    const key = feedKey.value.trim();
    if (!sym || !key) {
      feedStatus.textContent = "Enter both a ticker symbol and API key.";
      feedStatus.className = "status-msg err";
      feedStatus.classList.remove("hidden");
      return;
    }
    feedBtn.disabled = true;
    feedStatus.textContent = `Fetching daily OHLCV closes for ${sym}…`;
    feedStatus.className = "status-msg";
    feedStatus.classList.remove("hidden");
    try {
      let rows = [];
      let sourceLabel = "";
      if (feedProvider.value === "twelvedata") {
        sourceLabel = "Twelve Data (live)";
        const url = `https://api.twelvedata.com/time_series?symbol=${encodeURIComponent(sym)}&interval=1day&outputsize=1500&apikey=${encodeURIComponent(key)}`;
        const res = await fetch(url);
        const data = await res.json();
        if (data.status === "error" || !Array.isArray(data.values)) {
          throw new Error(data.message || "Twelve Data returned no daily rows.");
        }
        rows = data.values.map((v) => ({
          date: v.datetime,
          close: Number(v.close),
          high: Number(v.high),
          low: Number(v.low),
          volume: Number(v.volume),
        }));
      } else {
        sourceLabel = "Alpha Vantage (live)";
        const url = `https://www.alphavantage.co/query?function=TIME_SERIES_DAILY&symbol=${encodeURIComponent(sym)}&outputsize=full&apikey=${encodeURIComponent(key)}`;
        const res = await fetch(url);
        const data = await res.json();
        const ts = data["Time Series (Daily)"];
        if (!ts) throw new Error(data.Note || data["Error Message"] || "Alpha Vantage returned no daily series.");
        rows = Object.entries(ts).map(([d, v]) => ({
          date: d,
          close: Number(v["4. close"]),
          high: Number(v["2. high"]),
          low: Number(v["3. low"]),
          volume: Number(v["5. volume"]),
        }));
      }
      const series = E.parseUploadedSeries(JSON.stringify(rows), sym);
      series.ticker = sym;
      series.name = `${sym} (${sourceLabel})`;
      series.source = sourceLabel;
      histories.set(sym, series);
      state.ticker = sym;
      state.replayAt = null;
      state.replayRevealed = false;
      feedStatus.textContent = `Loaded ${series.closes.length} live daily closes for ${sym}.`;
      feedStatus.className = "status-msg ok";
      computeAndRender();
    } catch (err) {
      feedStatus.textContent = `Fetch failed: ${err.message}`;
      feedStatus.className = "status-msg err";
    } finally {
      feedBtn.disabled = false;
    }
  });

  /* ---------- Initial Boot ---------- */
  applyTheme(preferredTheme());
  loadTickerHeadlines(state.ticker);
  computeAndRender();
})();
