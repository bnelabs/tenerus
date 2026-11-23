/**
 * Tenerus In-Browser Quantitative Engine (100% client-side port of src/scenarios.rs & src/news_signal.rs)
 *
 * Implements the exact strictly-causal 4-model walk-forward ensemble from Tenerus v0.8.0:
 *   M1: Kernel- & recency-weighted multivariable OHLCV + ^GSPC regime analogues
 *   M2: 10-feature Online L2-regularized Quantile Regression (sequential AdaGrad) + news ablation
 *   M3: Stationary mean-reverting GARCH(1,1) + Cornish-Fisher fat-tailed skew-adjusted volatility
 *   M4: Unconditional non-overlapping historical baseline
 * Blended via out-of-sample Pinball-Loss Hedge weighting + finite-sample split-conformal calibration.
 */
(function (root, factory) {
  if (typeof module === "object" && module.exports) {
    module.exports = factory();
  } else {
    root.TenerusEngine = factory();
  }
})(typeof self !== "undefined" ? self : this, function () {
  "use strict";

  const DAY = 86400;
  const LOOKBACK = 60;
  const MATCH_TARGET = 24;
  const MATCH_MINIMUM = 10;
  const REPLAY_LIMIT = 36;
  const HORIZONS = [5, 20, 60];
  const GARCH_ALPHA = 0.08;
  const GARCH_BETA = 0.88;
  const GARCH_PERSISTENCE = GARCH_ALPHA + GARCH_BETA;
  const SAMPLE_END_TS = 1767220200; // 2025-12-31 close (matches src/model.rs)

  const DEMO_TICKERS = [
    { ticker: "NVDA", name: "NVIDIA Corporation", price: 186.52, pct: 2.34, currency: "USD" },
    { ticker: "AAPL", name: "Apple Inc.", price: 247.79, pct: 0.78, currency: "USD" },
    { ticker: "MSFT", name: "Microsoft Corporation", price: 514.23, pct: 1.12, currency: "USD" },
    { ticker: "GOOGL", name: "Alphabet Inc.", price: 201.42, pct: -0.42, currency: "USD" },
    { ticker: "AMZN", name: "Amazon.com, Inc.", price: 231.84, pct: 1.45, currency: "USD" },
    { ticker: "TSLA", name: "Tesla, Inc.", price: 352.60, pct: -1.27, currency: "USD" },
    { ticker: "META", name: "Meta Platforms, Inc.", price: 746.18, pct: 1.86, currency: "USD" },
    { ticker: "AMD", name: "Advanced Micro Devices", price: 168.24, pct: 3.12, currency: "USD" },
    { ticker: "^GSPC", name: "S&P 500 Index", price: 6584.29, pct: 0.64, currency: "USD" },
    { ticker: "BTC-USD", name: "Bitcoin USD", price: 113420.68, pct: 1.92, currency: "USD" },
  ];

  const MASK64 = 0xffffffffffffffffn;

  function clamp(v, lo, hi) {
    return v < lo ? lo : v > hi ? hi : v;
  }

  function change(from, to) {
    return (to - from) / from;
  }

  function formatDate(ts) {
    const d = new Date(ts * 1000);
    const y = d.getUTCFullYear();
    const m = String(d.getUTCMonth() + 1).padStart(2, "0");
    const day = String(d.getUTCDate()).padStart(2, "0");
    return `${y}-${m}-${day}`;
  }

  function formatPct(v, decimals = 1) {
    const p = v * 100;
    const sign = p >= 0 ? "+" : "";
    return `${sign}${p.toFixed(decimals)}%`;
  }

  function formatPrice(value, currency = "USD") {
    if (!Number.isFinite(value)) return "—";
    const abs = Math.abs(value);
    const num =
      abs > 0 && abs < 0.0001
        ? value.toExponential(3)
        : abs >= 1000
          ? value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })
          : value.toFixed(2);
    if (currency === "USD") return `$${num}`;
    if (currency === "EUR") return `€${num}`;
    if (currency === "GBP") return `£${num}`;
    return currency ? `${num} ${currency}` : num;
  }

  /** Bit-exact port of Rust's History::sample(ticker) */
  function sampleHistory(ticker) {
    const meta = DEMO_TICKERS.find((x) => x.ticker === ticker);
    if (!meta) return null;
    const dates = [];
    const isCrypto = ticker.endsWith("-USD");
    for (let age = 0; age < 3650; age++) {
      const t = SAMPLE_END_TS - age * DAY;
      const wd = (((Math.floor(t / DAY) + 3) % 7) + 7) % 7;
      if (isCrypto || wd < 5) dates.push(t);
    }
    dates.reverse();

    let noise = 17n;
    for (let i = 0; i < ticker.length; i++) {
      noise = (noise * 31n + BigInt(ticker.charCodeAt(i))) & MASK64;
    }

    let logPrice = 3.0;
    const len = dates.length;
    const closes = new Array(len);
    for (let i = 0; i < len; i++) {
      noise = (noise * 6364136223846793005n + 1n) & MASK64;
      const hi32 = Number(noise >> 32n);
      const shock = (hi32 / 4294967295.0 - 0.5) * 2.0;
      const phase = i / 58.0;
      let trend, swing;
      if (i + 38 >= len) {
        trend = 0.0027;
        swing = 0.018;
      } else {
        trend = 0.00065 + 0.0019 * Math.sin(phase);
        swing = 0.018 + 0.01 * Math.cos(phase * 0.6);
      }
      logPrice += trend + swing * shock;
      const price = Math.exp(logPrice);
      const spread = Math.max(swing * (0.45 + Math.abs(shock) * 0.65), 0.004);
      const volume = 25000000.0 * (0.75 + 0.55 * Math.abs(shock) + 0.2 * Math.abs(Math.sin(phase)));
      closes[i] = {
        time: dates[i],
        price,
        high: price * (1 + spread),
        low: Math.max(price * (1 - spread), 0.0001),
        volume,
      };
    }

    const ratio = meta.price / closes[len - 1].price;
    for (let i = 0; i < len; i++) {
      closes[i].price *= ratio;
      closes[i].high *= ratio;
      closes[i].low *= ratio;
    }

    return {
      ticker: meta.ticker,
      name: meta.name,
      currency: meta.currency,
      fetched: SAMPLE_END_TS,
      sample: true,
      source: "Simulated sample data",
      pct: meta.pct,
      closes,
    };
  }

  function percentile(sorted, p) {
    const rank = p * (sorted.length - 1);
    const lo = Math.floor(rank);
    const hi = Math.ceil(rank);
    return sorted[lo] + (sorted[hi] - sorted[lo]) * (rank - lo);
  }

  function weightedPercentile(sortedPairs, p) {
    if (sortedPairs.length === 1) return sortedPairs[0][0];
    let total = 0;
    for (let i = 0; i < sortedPairs.length; i++) total += sortedPairs[i][1];
    if (total <= 0) {
      return percentile(sortedPairs.map((x) => x[0]), p);
    }
    const target = p * total;
    let acc = 0;
    let prevV = sortedPairs[0][0];
    let prevC = 0;
    for (let i = 0; i < sortedPairs.length; i++) {
      const v = sortedPairs[i][0];
      const w = sortedPairs[i][1];
      const midC = acc + 0.5 * w;
      if (midC >= target) {
        if (midC <= prevC + 1e-12) return v;
        const t = clamp((target - prevC) / (midC - prevC), 0, 1);
        return prevV + (v - prevV) * t;
      }
      prevV = v;
      prevC = midC;
      acc += w;
    }
    return sortedPairs[sortedPairs.length - 1][0];
  }

  function orderedBand(low, mid, high) {
    let lo = Math.max(Math.min(low, mid, high), -0.99);
    let hi = Math.max(low, mid, high);
    let m = clamp(mid, lo, hi);
    if (!Number.isFinite(lo) || !Number.isFinite(m) || !Number.isFinite(hi)) {
      return { low: 0, mid: 0, high: 0 };
    }
    return { low: lo, mid: clamp(m, lo, hi), high: hi };
  }

  function bandFromReturns(returns) {
    returns.sort((a, b) => a - b);
    return {
      low: percentile(returns, 0.1),
      mid: percentile(returns, 0.5),
      high: percentile(returns, 0.9),
    };
  }

  function bandFromWeightedReturns(pairs) {
    pairs.sort((a, b) => a[0] - b[0]);
    return {
      low: weightedPercentile(pairs, 0.1),
      mid: weightedPercentile(pairs, 0.5),
      high: weightedPercentile(pairs, 0.9),
    };
  }

  function blendBands(bands, weights) {
    let low = 0,
      mid = 0,
      high = 0;
    for (let i = 0; i < 4; i++) {
      low += weights[i] * bands[i].low;
      mid += weights[i] * bands[i].mid;
      high += weights[i] * bands[i].high;
    }
    return orderedBand(low, mid, high);
  }

  function calibrateBand(band, delta) {
    const halfWidth = (band.high - band.low) * 0.5;
    const effDelta = delta < 0 ? Math.max(delta, -halfWidth * 0.35) : delta;
    const low = Math.max(Math.min(band.low - effDelta, band.mid), -0.99);
    const high = Math.max(band.high + effDelta, band.mid);
    return orderedBand(low, band.mid, high);
  }

  function pinball(y, q, alpha) {
    const diff = y - q;
    return diff >= 0 ? alpha * diff : (alpha - 1) * diff;
  }

  function bandPinballLoss(band, y) {
    return (pinball(y, band.low, 0.1) + pinball(y, band.mid, 0.5) + pinball(y, band.high, 0.9)) / 3.0;
  }

  function bandContains(band, y) {
    return y >= band.low && y <= band.high;
  }

  function partitionPoint(arr, pred) {
    let lo = 0,
      hi = arr.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (pred(arr[mid])) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  function causalSentimentAt(entries, atTime) {
    if (!entries || !entries.length) return null;
    let num = 0,
      den = 0;
    for (let i = 0; i < entries.length; i++) {
      const e = entries[i];
      if (e.day_time > atTime) continue;
      const ageDays = (atTime - e.day_time) / DAY;
      if (ageDays > 14.0 || !Number.isFinite(e.score) || !e.article_count) continue;
      const w = Math.exp(-ageDays / 5.0) * Math.min(e.article_count, 10);
      num += w * clamp(e.score, -1, 1);
      den += w;
    }
    return den > 0 ? clamp(num / den, -1, 1) : null;
  }

  function computeSetup(closes, i, regimeCloses, sentimentEntries) {
    const cNow = closes[i].price;
    const momentum5 = change(closes[i - 5].price, cNow);
    const momentum = change(closes[i - 20].price, cNow);
    const momentum60 = change(closes[i - LOOKBACK].price, cNow);

    const meanLog20 = Math.log(cNow / closes[i - 20].price) / 20.0;
    let var20 = 0;
    for (let k = i - 20; k < i; k++) {
      const d = Math.log(closes[k + 1].price / closes[k].price) - meanLog20;
      var20 += d * d;
    }
    var20 /= 19.0;
    const dailyVolatility = Math.sqrt(var20);

    const meanLog5 = Math.log(cNow / closes[i - 5].price) / 5.0;
    let var5 = 0;
    for (let k = i - 5; k < i; k++) {
      const d = Math.log(closes[k + 1].price / closes[k].price) - meanLog5;
      var5 += d * d;
    }
    var5 /= 4.0;
    const volRatio = dailyVolatility > 1e-6 ? clamp(Math.sqrt(var5) / dailyVolatility, 0.25, 4.0) : 1.0;

    let high60 = 0;
    for (let k = i + 1 - LOOKBACK; k <= i; k++) {
      if (closes[k].price > high60) high60 = closes[k].price;
    }
    const drawdown = change(high60, cNow);

    // Strictly causal GARCH(1,1)
    const lrStart = Math.max(0, i - 252);
    const lrRets = [];
    for (let k = lrStart; k < i; k++) {
      const r = Math.log(closes[k + 1].price / closes[k].price);
      if (Number.isFinite(r)) lrRets.push(r);
    }
    let longRunVar;
    if (lrRets.length >= 10) {
      let sum = 0;
      for (let k = 0; k < lrRets.length; k++) sum += lrRets[k];
      const meanLr = sum / lrRets.length;
      let s2 = 0;
      for (let k = 0; k < lrRets.length; k++) {
        const d = lrRets[k] - meanLr;
        s2 += d * d;
      }
      longRunVar = clamp(s2 / (lrRets.length - 1), 0.002 * 0.002, 0.25 * 0.25);
    } else {
      longRunVar = clamp(var20, 0.002 * 0.002, 0.25 * 0.25);
    }

    const omega = (1.0 - GARCH_PERSISTENCE) * longRunVar;
    let garchVar = longRunVar;
    const rets60 = [];
    for (let k = i - LOOKBACK; k < i; k++) {
      const r = Math.log(closes[k + 1].price / closes[k].price);
      if (Number.isFinite(r)) {
        rets60.push(r);
        garchVar = clamp(omega + GARCH_ALPHA * r * r + GARCH_BETA * garchVar, 0.002 * 0.002, 0.25 * 0.25);
      }
    }
    const meanLog60 = Math.log(cNow / closes[i - LOOKBACK].price) / LOOKBACK;
    const garchDrift = clamp(meanLog60 * 0.25, -0.0025, 0.0025);

    let returnSkew = 0;
    if (rets60.length >= 20) {
      let m = 0;
      for (let k = 0; k < rets60.length; k++) m += rets60[k];
      m /= rets60.length;
      let v = 0;
      for (let k = 0; k < rets60.length; k++) {
        const d = rets60[k] - m;
        v += d * d;
      }
      const sd = Math.sqrt(v / rets60.length);
      if (sd > 1e-6) {
        let m3 = 0;
        for (let k = 0; k < rets60.length; k++) {
          const z = (rets60[k] - m) / sd;
          m3 += z * z * z;
        }
        returnSkew = clamp(m3 / rets60.length, -0.5, 0.5);
      }
    }

    // Optional volume ratio
    let volumeRatio = null;
    let v20Sum = 0,
      v20Cnt = 0,
      v5Sum = 0,
      v5Cnt = 0;
    for (let k = i + 1 - 20; k <= i; k++) {
      const v = closes[k].volume;
      if (Number.isFinite(v) && v > 0) {
        v20Sum += v;
        v20Cnt++;
        if (k >= i + 1 - 5) {
          v5Sum += v;
          v5Cnt++;
        }
      }
    }
    if (v20Cnt >= 15 && v5Cnt >= 4 && v20Sum > 0) {
      volumeRatio = clamp(v5Sum / v5Cnt / (v20Sum / v20Cnt), 0.2, 5.0);
    }

    // Optional OHLC range ratio
    let rangeRatio = null;
    let r20Sum = 0,
      r20Cnt = 0,
      r5Sum = 0,
      r5Cnt = 0;
    for (let k = i + 1 - 20; k <= i; k++) {
      const b = closes[k];
      if (Number.isFinite(b.high) && Number.isFinite(b.low) && b.high >= b.price && b.low > 0 && b.low <= b.price) {
        const rr = Math.max((b.high - b.low) / b.price, 1e-5);
        r20Sum += rr;
        r20Cnt++;
        if (k >= i + 1 - 5) {
          r5Sum += rr;
          r5Cnt++;
        }
      }
    }
    if (r20Cnt >= 15 && r5Cnt >= 4 && r20Sum > 0) {
      rangeRatio = clamp(r5Sum / r5Cnt / (r20Sum / r20Cnt), 0.25, 4.0);
    }

    // Strictly causal S&P 500 regime lookup
    let relStrength20 = null;
    let marketVol20 = null;
    if (regimeCloses && regimeCloses.length >= 21) {
      const t = closes[i].time;
      const pos = partitionPoint(regimeCloses, (b) => b.time <= t);
      const mi = pos - 1;
      if (mi >= 20 && t - regimeCloses[mi].time <= 7 * DAY) {
        const spMom = change(regimeCloses[mi - 20].price, regimeCloses[mi].price);
        const spMean = Math.log(regimeCloses[mi].price / regimeCloses[mi - 20].price) / 20.0;
        let spVar = 0;
        for (let k = mi - 20; k < mi; k++) {
          const d = Math.log(regimeCloses[k + 1].price / regimeCloses[k].price) - spMean;
          spVar += d * d;
        }
        spVar /= 19.0;
        relStrength20 = clamp(momentum - spMom, -1.0, 1.0);
        marketVol20 = clamp(Math.sqrt(spVar), 0.002, 0.25);
      }
    }

    const newsSentiment = causalSentimentAt(sentimentEntries, closes[i].time);

    return {
      momentum5,
      momentum,
      momentum60,
      dailyVolatility,
      volRatio,
      drawdown,
      volumeRatio,
      rangeRatio,
      relStrength20,
      marketVol20,
      newsSentiment,
      garchVar,
      longRunVar,
      garchDrift,
      returnSkew,
    };
  }

  function setupDistance(a, b) {
    const vol = Math.max(a.dailyVolatility, 0.002);
    const oVol = Math.max(b.dailyVolatility, 0.002);
    let d =
      0.55 * Math.pow((a.momentum5 / vol - b.momentum5 / oVol) / Math.sqrt(5), 2) +
      Math.pow((a.momentum / vol - b.momentum / oVol) / Math.sqrt(20), 2) +
      0.7 * Math.pow((a.momentum60 / vol - b.momentum60 / oVol) / Math.sqrt(60), 2) +
      Math.pow(Math.log(vol / oVol), 2) +
      0.45 * Math.pow(Math.log(a.volRatio / b.volRatio), 2) +
      Math.pow((a.drawdown / vol - b.drawdown / oVol) / Math.sqrt(60), 2);
    if (a.volumeRatio !== null && b.volumeRatio !== null) {
      d += 0.35 * Math.pow(Math.log(a.volumeRatio / b.volumeRatio), 2);
    }
    if (a.rangeRatio !== null && b.rangeRatio !== null) {
      d += 0.25 * Math.pow(Math.log(a.rangeRatio / b.rangeRatio), 2);
    }
    if (a.relStrength20 !== null && b.relStrength20 !== null) {
      d += 0.5 * Math.pow((a.relStrength20 - b.relStrength20) / 0.1, 2);
    }
    if (a.marketVol20 !== null && b.marketVol20 !== null) {
      d += 0.4 * Math.pow(Math.log(a.marketVol20 / b.marketVol20), 2);
    }
    return d;
  }

  function featureVec(s, includeSentiment) {
    return [
      clamp(s.momentum5, -0.6, 0.6),
      clamp(s.momentum, -1.0, 1.0),
      clamp(s.momentum60, -1.5, 2.0),
      Math.log(Math.max(s.dailyVolatility, 0.002)),
      Math.log(s.volRatio),
      clamp(s.drawdown, -0.9, 0.0),
      Math.log(s.volumeRatio !== null ? s.volumeRatio : 1.0),
      clamp(s.relStrength20 !== null ? s.relStrength20 : 0.0, -1.0, 1.0),
      s.marketVol20 !== null ? Math.log(Math.max(s.marketVol20, 0.002) / 0.01) : 0.0,
      includeSentiment && s.newsSentiment !== null ? clamp(s.newsSentiment, -1.0, 1.0) : 0.0,
    ];
  }

  function garchBand(setup, step) {
    if (step === 0) return { low: 0, mid: 0, high: 0 };
    const h = step;
    const drift = setup.garchDrift * h;
    const decay = (1.0 - Math.pow(GARCH_PERSISTENCE, step)) / (1.0 - GARCH_PERSISTENCE);
    const vH = Math.max(h * setup.longRunVar + (setup.garchVar - setup.longRunVar) * decay, h * 0.002 * 0.002);
    const sdH = Math.sqrt(vH);
    const z = 1.34;
    const cfSkew = ((z * z - 1.0) / 6.0) * setup.returnSkew;
    const low = Math.exp(drift + (-z + cfSkew) * sdH) - 1.0;
    const mid = Math.exp(drift - 0.05 * setup.returnSkew * sdH) - 1.0;
    const high = Math.exp(drift + (z + cfSkew) * sdH) - 1.0;
    return orderedBand(low, mid, high);
  }

  function onlineQrBand(closes, setups, baseline, current, horizon, baselineBand, includeSentiment) {
    if (baseline.length < MATCH_MINIMUM) return baselineBand;
    const D = 10;
    const mean = new Float64Array(D);
    let minRet = Infinity,
      maxRet = -Infinity;
    const samples = [];
    for (let idx = baseline.length - 1; idx >= 0; idx--) {
      const j = baseline[idx];
      const f = featureVec(setups[j - LOOKBACK], includeSentiment);
      const y = change(closes[j].price, closes[j + horizon].price);
      if (!Number.isFinite(y)) continue;
      for (let k = 0; k < D; k++) mean[k] += f[k];
      if (y < minRet) minRet = y;
      if (y > maxRet) maxRet = y;
      samples.push([f, y]);
    }
    if (samples.length < MATCH_MINIMUM) return baselineBand;
    const nf = samples.length;
    for (let k = 0; k < D; k++) mean[k] /= nf;
    const std = new Float64Array(D);
    for (let s = 0; s < nf; s++) {
      const f = samples[s][0];
      for (let k = 0; k < D; k++) {
        const d = f[k] - mean[k];
        std[k] += d * d;
      }
    }
    for (let k = 0; k < D; k++) std[k] = Math.max(Math.sqrt(std[k] / nf), 1e-4);

    const standardize = (raw) => {
      const x = new Float64Array(D + 1);
      x[0] = 1.0;
      for (let k = 0; k < D; k++) x[k + 1] = clamp((raw[k] - mean[k]) / std[k], -3.0, 3.0);
      return x;
    };

    const xs = samples.map(([f, y]) => [standardize(f), y]);
    const curX = standardize(featureVec(current, includeSentiment));
    const init = [baselineBand.low, baselineBand.mid, baselineBand.high];
    const alphas = [0.1, 0.5, 0.9];
    const l2 = 0.08;
    const lr = 0.045;
    const preds = [0, 0, 0];

    for (let q = 0; q < 3; q++) {
      const alpha = alphas[q];
      const w = new Float64Array(D + 1);
      w[0] = init[q];
      const g2 = new Float64Array(D + 1).fill(1e-4);
      for (let epoch = 0; epoch < 3; epoch++) {
        for (let s = 0; s < xs.length; s++) {
          const x = xs[s][0];
          const y = xs[s][1];
          let pred = 0;
          for (let k = 0; k <= D; k++) pred += w[k] * x[k];
          const err = y - pred;
          const dLoss = err > 0 ? -alpha : err < 0 ? 1.0 - alpha : 0.0;
          for (let k = 0; k <= D; k++) {
            const reg = k === 0 ? 0 : l2 * w[k];
            const grad = dLoss * x[k] + reg;
            g2[k] += grad * grad;
            w[k] -= (lr / Math.sqrt(g2[k])) * grad;
          }
        }
      }
      let pred = 0;
      for (let k = 0; k <= D; k++) pred += w[k] * curX[k];
      const span = Math.max(maxRet - minRet, 0.05);
      preds[q] = clamp(pred, Math.max(minRet - 0.35 * span, -0.98), maxRet + 0.35 * span);
    }
    return orderedBand(preds[0], preds[1], preds[2]);
  }

  function candidates(setups, i, horizon) {
    const end = i - (horizon + 1);
    if (end < LOOKBACK || i < LOOKBACK || i - LOOKBACK >= setups.length) return null;
    const current = setups[i - LOOKBACK];
    const baseline = [];
    for (let j = end; j >= LOOKBACK; j -= horizon) baseline.push(j);
    if (baseline.length < MATCH_MINIMUM) return null;

    const ranked = [];
    for (let j = LOOKBACK; j <= end; j++) {
      ranked.push([j, setupDistance(current, setups[j - LOOKBACK])]);
    }
    ranked.sort((a, b) => (a[1] !== b[1] ? a[1] - b[1] : b[0] - a[0]));

    let selected = [];
    for (let r = 0; r < ranked.length; r++) {
      const [j, dist] = ranked[r];
      let ok = true;
      for (let s = 0; s < selected.length; s++) {
        if (Math.abs(j - selected[s][0]) < horizon) {
          ok = false;
          break;
        }
      }
      if (ok) {
        selected.push([j, dist]);
        if (selected.length === MATCH_TARGET) break;
      }
    }
    if (selected.length < MATCH_MINIMUM) {
      selected = ranked.filter(([j]) => (end - j) % horizon === 0).slice(0, MATCH_TARGET);
    }
    return { selected, baseline };
  }

  function matchWeights(selected, i) {
    if (!selected.length) return { weights: [], nEff: 0 };
    const dists = selected.map((x) => x[1]).sort((a, b) => a - b);
    const tau = Math.max(percentile(dists, 0.5), 0.25);
    const raw = selected.map(([j, dist]) => {
      const ageYears = Math.max(0, i - j) / 252.0;
      return Math.max(Math.exp(-dist / tau) * Math.exp(-0.08 * ageYears), 1e-6);
    });
    let sum = 0;
    for (let k = 0; k < raw.length; k++) sum += raw[k];
    if (sum > 0) for (let k = 0; k < raw.length; k++) raw[k] /= sum;
    let sumSq = 0;
    for (let k = 0; k < raw.length; k++) sumSq += raw[k] * raw[k];
    return { weights: raw, nEff: sumSq > 0 ? 1.0 / sumSq : 0 };
  }

  function componentEndBands(closes, setups, i, horizon) {
    const cands = candidates(setups, i, horizon);
    if (!cands) return null;
    const { selected, baseline } = cands;
    const { weights, nEff } = matchWeights(selected, i);
    const analogPairs = selected.map(([j], k) => [change(closes[j].price, closes[j + horizon].price), weights[k]]);
    const analog = bandFromWeightedReturns(analogPairs);
    const baseRets = baseline.map((j) => change(closes[j].price, closes[j + horizon].price));
    const baselineBand = bandFromReturns(baseRets);
    const current = setups[i - LOOKBACK];
    const qr = onlineQrBand(closes, setups, baseline, current, horizon, baselineBand, true);
    const garch = garchBand(current, horizon);
    return { bands: [analog, qr, garch, baselineBand], selected, weights, nEff, baseline };
  }

  function hedgeWeights(losses, count) {
    if (count < 3) return [0.25, 0.25, 0.25, 0.25];
    const scale = Math.max(losses[3] / count, 0.005);
    const eta = 3.2;
    const avg = losses.map((l) => l / count);
    const minL = Math.min(...avg);
    const w = [0, 0, 0, 0];
    let sum = 0;
    for (let m = 0; m < 4; m++) {
      w[m] = Math.max(Math.exp((-eta * (avg[m] - minL)) / scale), 0.03);
      sum += w[m];
    }
    for (let m = 0; m < 4; m++) w[m] /= sum;
    return w;
  }

  function conformalAdjustment(scores) {
    if (scores.length < 5) return 0.0;
    const sorted = scores.slice().sort((a, b) => a - b);
    const n = sorted.length;
    const rank = Math.min(Math.max(Math.ceil((n + 1) * 0.8) - 1, 0), n - 1);
    return clamp(sorted[rank], -0.04, 0.15);
  }

  function walkForward(closes, setups, atIndex, horizon) {
    if (atIndex < horizon) {
      return {
        trials: [],
        calibration: {
          weights: { analogs: 0.25, onlineQr: 0.25, garch: 0.25, baseline: 0.25 },
          conformalDelta: 0,
          effectiveMatches: 0,
          ensemblePinball: 0,
          baselinePinball: 0,
          analogPinball: 0,
          onlineQrPinball: 0,
          garchPinball: 0,
          trialCount: 0,
          rawCovered: 0,
          calibratedCovered: 0,
          baselineCovered: 0,
          pinballSkill: 0,
          regimeActive: false,
          sentimentAblationTrials: 0,
          sentimentPinballDelta: null,
        },
      };
    }
    const indices = [];
    for (let idx = atIndex - horizon; idx >= 0 && indices.length < REPLAY_LIMIT; idx -= horizon) {
      indices.push(idx);
    }
    indices.reverse();

    const cumLosses = [0, 0, 0, 0];
    let priorTrials = 0;
    const conformalScores = [];
    let ensPinballSum = 0;
    let rawCovered = 0,
      calibratedCovered = 0,
      baselineCovered = 0;
    let ablationTrials = 0,
      ablationWithPb = 0,
      ablationWithoutPb = 0;
    const trials = [];

    for (let k = 0; k < indices.length; k++) {
      const idx = indices[k];
      const comp = componentEndBands(closes, setups, idx, horizon);
      if (!comp) continue;
      const { bands, baseline } = comp;
      const actual = change(closes[idx].price, closes[idx + horizon].price);
      const wArr = hedgeWeights(cumLosses, priorTrials);
      const rawEns = blendBands(bands, wArr);
      const delta = conformalAdjustment(conformalScores);
      const calEns = calibrateBand(rawEns, delta);

      const ensPb = bandPinballLoss(calEns, actual);
      const basePb = bandPinballLoss(bands[3], actual);
      const inside = bandContains(calEns, actual);
      const rawInside = bandContains(rawEns, actual);
      const baselineInside = bandContains(bands[3], actual);

      const current = setups[idx - LOOKBACK];
      if (current.newsSentiment !== null) {
        let priorSent = 0;
        for (let b = 0; b < baseline.length; b++) {
          if (setups[baseline[b] - LOOKBACK].newsSentiment !== null) priorSent++;
        }
        if (priorSent >= 5) {
          const qrNoSent = onlineQrBand(closes, setups, baseline, current, horizon, bands[3], false);
          ablationTrials++;
          ablationWithPb += bandPinballLoss(bands[1], actual);
          ablationWithoutPb += bandPinballLoss(qrNoSent, actual);
        }
      }

      for (let m = 0; m < 4; m++) cumLosses[m] += bandPinballLoss(bands[m], actual);
      conformalScores.push(Math.max(rawEns.low - actual, actual - rawEns.high));
      priorTrials++;
      ensPinballSum += ensPb;
      if (rawInside) rawCovered++;
      if (inside) calibratedCovered++;
      if (baselineInside) baselineCovered++;

      trials.push({
        index: idx,
        at: closes[idx].time,
        through: closes[idx + horizon].time,
        result: actual,
        inside,
        rawInside,
        baselineInside,
        absoluteError: Math.abs(actual - calEns.mid),
        baselineError: Math.abs(actual - bands[3].mid),
        pinballLoss: ensPb,
        baselinePinballLoss: basePb,
      });
    }

    trials.reverse();
    const wFinal = hedgeWeights(cumLosses, priorTrials);
    const nf = priorTrials || 1;
    const ensPbAvg = priorTrials ? ensPinballSum / nf : 0;
    const basePbAvg = priorTrials ? cumLosses[3] / nf : 0;
    const pinballSkill = basePbAvg > 1e-9 && priorTrials > 0 ? 1.0 - ensPbAvg / basePbAvg : 0.0;

    return {
      trials,
      calibration: {
        weights: { analogs: wFinal[0], onlineQr: wFinal[1], garch: wFinal[2], baseline: wFinal[3] },
        conformalDelta: conformalAdjustment(conformalScores),
        effectiveMatches: 0,
        ensemblePinball: ensPbAvg,
        baselinePinball: basePbAvg,
        analogPinball: priorTrials ? cumLosses[0] / nf : 0,
        onlineQrPinball: priorTrials ? cumLosses[1] / nf : 0,
        garchPinball: priorTrials ? cumLosses[2] / nf : 0,
        trialCount: priorTrials,
        rawCovered,
        calibratedCovered,
        baselineCovered,
        pinballSkill,
        regimeActive: false,
        sentimentAblationTrials: ablationTrials,
        sentimentPinballDelta: ablationTrials >= 5 ? (ablationWithPb - ablationWithoutPb) / ablationTrials : null,
      },
    };
  }

  function buildSnapshot(closes, setups, i, horizon, cal) {
    const comp = componentEndBands(closes, setups, i, horizon);
    if (!comp) return null;
    const { bands, selected, weights, nEff, baseline } = comp;
    cal.effectiveMatches = nEff;
    const wArr = [cal.weights.analogs, cal.weights.onlineQr, cal.weights.garch, cal.weights.baseline];
    const current = setups[i - LOOKBACK];
    const qrEnd = bands[1];

    const fan = new Array(horizon + 1);
    fan[0] = { low: 0, mid: 0, high: 0 };
    for (let step = 1; step <= horizon; step++) {
      const frac = Math.sqrt(step / horizon);
      const analogPairs = selected.map(([j], k) => [change(closes[j].price, closes[j + step].price), weights[k]]);
      const analogStep = bandFromWeightedReturns(analogPairs);
      const qrStep = orderedBand(qrEnd.low * frac, qrEnd.mid * (step / horizon), qrEnd.high * frac);
      const garchStep = garchBand(current, step);
      const baseStepRets = baseline.map((j) => change(closes[j].price, closes[j + step].price));
      const baseStep = bandFromReturns(baseStepRets);
      const blended = blendBands([analogStep, qrStep, garchStep, baseStep], wArr);
      fan[step] = calibrateBand(blended, cal.conformalDelta * frac);
    }

    const matches = selected.map(([j], k) => ({
      start: closes[j].time,
      end: closes[j + horizon].time,
      realized: change(closes[j].price, closes[j + horizon].price),
      weight: weights[k],
    }));

    return {
      anchor: closes[i],
      setup: current,
      past: closes.slice(Math.max(0, i - 40), i + 1),
      fan,
      end: fan[horizon],
      upCount: matches.filter((m) => m.realized > 0).length,
      matches,
      baselineReturns: baseline.map((j) => change(closes[j].price, closes[j + horizon].price)),
      calibration: cal,
    };
  }

  function buildSetups(closes, regimeCloses, sentimentEntries) {
    const out = new Array(Math.max(0, closes.length - LOOKBACK));
    for (let i = LOOKBACK; i < closes.length; i++) {
      out[i - LOOKBACK] = computeSetup(closes, i, regimeCloses, sentimentEntries);
    }
    return out;
  }

  function runReport(history, horizon, regimeHistory, sentimentEntries) {
    if (!HORIZONS.includes(horizon)) throw new Error("Choose a 5, 20, or 60 session horizon.");
    if (!history || !history.closes || history.closes.length < LOOKBACK + MATCH_MINIMUM * horizon + 2) {
      throw new Error(
        `At least ${LOOKBACK + MATCH_MINIMUM * horizon + 2} completed daily closes are required for a ${horizon}-session outlook.`
      );
    }
    const closes = history.closes;
    const last = closes.length - 1;
    const regimeCloses =
      regimeHistory && regimeHistory.closes && regimeHistory.closes.length >= 21 && regimeHistory.ticker !== history.ticker
        ? regimeHistory.closes
        : null;
    const allSetups = buildSetups(closes, regimeCloses, sentimentEntries || null);
    const { trials, calibration } = walkForward(closes, allSetups, last, horizon);
    calibration.regimeActive = Boolean(regimeCloses);
    const current = buildSnapshot(closes, allSetups, last, horizon, calibration);
    if (!current) {
      throw new Error("Too few non-overlapping historical periods for this horizon. Try a shorter horizon.");
    }
    return { current, trials, _setups: allSetups, _regimeActive: Boolean(regimeCloses) };
  }

  function runReplay(history, atTime, horizon, regimeHistory, sentimentEntries) {
    const closes = history.closes;
    let i = -1;
    for (let k = 0; k < closes.length; k++) {
      if (closes[k].time === atTime) {
        i = k;
        break;
      }
    }
    if (i < 0 || i + horizon >= closes.length) return null;
    const realized = [];
    for (let k = 0; k <= horizon; k++) {
      realized.push(change(closes[i].price, closes[i + k].price));
    }
    const regimeCloses =
      regimeHistory && regimeHistory.closes && regimeHistory.closes.length >= 21 && regimeHistory.ticker !== history.ticker
        ? regimeHistory.closes
        : null;
    const allSetups = buildSetups(closes, regimeCloses, sentimentEntries || null);
    const { calibration } = walkForward(closes, allSetups, i, horizon);
    calibration.regimeActive = Boolean(regimeCloses);
    const scenario = buildSnapshot(closes, allSetups, i, horizon, calibration);
    if (!scenario) return null;
    return { scenario, realized };
  }

  /* ==================== LOCAL NEWS SIGNAL ENGINE (src/news_signal.rs) ==================== */
  const POS_TERMS = new Map([
    ["beats", 1.0], ["beat", 0.9], ["tops", 0.8], ["surges", 1.0], ["surge", 0.9], ["soars", 1.0],
    ["rallies", 0.9], ["rally", 0.8], ["jumps", 0.8], ["gains", 0.7], ["outperforms", 1.0], ["outperform", 0.9],
    ["upgrade", 1.0], ["upgraded", 1.0], ["upgrades", 1.0], ["raises", 0.9], ["raised", 0.8], ["record", 0.6],
    ["strong", 0.7], ["growth", 0.6], ["breakout", 0.8], ["buyback", 0.7], ["repurchase", 0.7], ["approval", 0.9],
    ["approved", 0.9], ["partnership", 0.6], ["expands", 0.6], ["profit", 0.6], ["bullish", 0.9], ["boom", 0.8],
    ["accelerate", 0.7], ["accelerates", 0.7],
  ]);

  const NEG_TERMS = new Map([
    ["misses", 1.0], ["miss", 0.9], ["plunges", 1.0], ["plunge", 1.0], ["tumbles", 1.0], ["slumps", 1.0],
    ["drops", 0.8], ["falls", 0.7], ["sinks", 0.9], ["crashes", 1.0], ["downgrade", 1.0], ["downgraded", 1.0],
    ["downgrades", 1.0], ["cuts", 0.8], ["cut", 0.7], ["slashed", 1.0], ["slashes", 1.0], ["warning", 0.9],
    ["warns", 0.9], ["weak", 0.8], ["loss", 0.8], ["losses", 0.8], ["lawsuit", 0.9], ["sued", 0.9],
    ["probe", 0.9], ["investigation", 0.9], ["subpoena", 1.0], ["recall", 0.9], ["layoffs", 0.8], ["layoff", 0.8],
    ["antitrust", 0.8], ["tariff", 0.6], ["tariffs", 0.6], ["sanctions", 0.8], ["bearish", 0.9], ["default", 1.0],
    ["bankruptcy", 1.0], ["delay", 0.7], ["delays", 0.7], ["decline", 0.7], ["declines", 0.7], ["shortfall", 0.9],
  ]);

  const NEGATORS = new Set(["not", "no", "never", "without", "fails", "failed", "below"]);

  const EVENT_RULES = [
    ["Earnings", ["earnings", "eps", "quarter", "quarterly", "q1", "q2", "q3", "q4", "revenue", "results", "profit"]],
    ["Guidance", ["guidance", "outlook", "forecast", "raises", "cuts", "targets"]],
    ["Analyst", ["upgrade", "upgraded", "downgrade", "downgraded", "price target", "rating", "outperform", "underperform", "overweight", "underweight", "analyst"]],
    ["M&A / Capital", ["acquire", "acquisition", "merger", "buyout", "buyback", "repurchase", "dividend", "split", "offering", "ipo", "spin-off"]],
    ["Legal / Regulatory", ["sec", "ftc", "doj", "fda", "antitrust", "lawsuit", "sued", "probe", "investigation", "regulator", "subpoena", "recall", "fine", "settlement"]],
    ["Macro / Policy", ["fed", "inflation", "cpi", "interest rate", "rates", "tariff", "tariffs", "sanctions", "recession", "treasury", "jobs report"]],
  ];

  function tokenize(text) {
    return text
      .split(/[^a-zA-Z0-9\-']+/)
      .map((t) => t.replace(/^'+|'+$/g, "").toLowerCase())
      .filter(Boolean);
  }

  function scoreHeadline(title) {
    const tokens = tokenize(title);
    if (!tokens.length) return 0;
    let sum = 0,
      hits = 0;
    for (let i = 0; i < tokens.length; i++) {
      const neg =
        (i > 0 && NEGATORS.has(tokens[i - 1])) || (i > 1 && NEGATORS.has(tokens[i - 2]));
      const sign = neg ? -1 : 1;
      if (POS_TERMS.has(tokens[i])) {
        const w = POS_TERMS.get(tokens[i]);
        sum += sign * w;
        hits += w;
      } else if (NEG_TERMS.has(tokens[i])) {
        const w = NEG_TERMS.get(tokens[i]);
        sum -= sign * w;
        hits += w;
      }
    }
    return hits === 0 ? 0 : clamp(sum / (hits + 0.5), -1, 1);
  }

  function analyzeHeadlines(lines, earningsDays, horizonSessions) {
    const clean = lines.map((l) => l.trim()).filter(Boolean).slice(0, 15);
    let total = 0,
      pos = 0,
      neg = 0,
      neu = 0;
    const tags = new Set();
    const scored = clean.map((title) => {
      const score = scoreHeadline(title);
      total += score;
      if (score >= 0.15) pos++;
      else if (score <= -0.15) neg++;
      else neu++;
      const lower = title.toLowerCase();
      const tokens = tokenize(title);
      for (const [tag, kws] of EVENT_RULES) {
        if (kws.some((kw) => (kw.includes(" ") ? lower.includes(kw) : tokens.includes(kw)))) {
          tags.add(tag);
        }
      }
      return { title, score };
    });

    const n = clean.length;
    const score = n ? clamp(total / n, -1, 1) : 0;
    const label =
      n === 0
        ? "No headlines"
        : score >= 0.18
          ? "Positive tone"
          : score <= -0.18
            ? "Negative tone"
            : pos > 0 && neg > 0
              ? "Mixed tone"
              : "Neutral tone";
    const calDays = Math.max(Math.floor((horizonSessions * 7) / 5) + 3, 7);
    const earningsInHorizon =
      Number.isFinite(earningsDays) && earningsDays >= 0 && earningsDays <= calDays;

    return {
      articleCount: n,
      score,
      label,
      positiveCount: pos,
      negativeCount: neg,
      neutralCount: neu,
      eventTags: Array.from(tags),
      earningsInHorizon,
      earningsDays: Number.isFinite(earningsDays) ? earningsDays : null,
      scored,
    };
  }

  /* ==================== CSV / JSON IMPORTER ==================== */
  function parseTimestamp(raw) {
    if (typeof raw === "number" && Number.isFinite(raw)) {
      return raw > 1e12 ? Math.floor(raw / 1000) : Math.floor(raw);
    }
    const s = String(raw || "").trim();
    if (!s) return null;
    if (/^\d{9,13}$/.test(s)) {
      const n = Number(s);
      return n > 1e12 ? Math.floor(n / 1000) : Math.floor(n);
    }
    const ms = Date.parse(s.length === 10 ? `${s}T16:00:00Z` : s);
    return Number.isFinite(ms) ? Math.floor(ms / 1000) : null;
  }

  function parseUploadedSeries(text, filename) {
    const trimmed = text.trim();
    const baseTicker = (filename || "CUSTOM")
      .replace(/\.[^.]+$/, "")
      .toUpperCase()
      .replace(/[^A-Z0-9.\-^=]/g, "")
      .slice(0, 12) || "CUSTOM";

    let rawRows = [];
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      const parsed = JSON.parse(trimmed);
      const arr = Array.isArray(parsed)
        ? parsed
        : parsed.closes || parsed.values || parsed.data || parsed.prices || [];
      if (!Array.isArray(arr) || !arr.length) throw new Error("JSON does not contain a non-empty price array.");
      rawRows = arr.map((r) => ({
        time: parseTimestamp(r.time ?? r.date ?? r.datetime ?? r.timestamp),
        price: Number(r.close ?? r.adj_close ?? r.adjClose ?? r.price),
        high: Number(r.high),
        low: Number(r.low),
        volume: Number(r.volume),
      }));
    } else {
      const lines = trimmed.split(/\r?\n/).filter((l) => l.trim());
      if (lines.length < 2) throw new Error("CSV needs a header row and daily price rows.");
      const delim = lines[0].includes(";") && !lines[0].includes(",") ? ";" : ",";
      const headers = lines[0].split(delim).map((h) => h.trim().toLowerCase().replace(/['"]/g, ""));
      const findCol = (names) => headers.findIndex((h) => names.includes(h));
      const iDate = findCol(["date", "datetime", "timestamp", "time"]);
      const iClose = findCol(["adj close", "adj_close", "close", "price", "last"]);
      const iHigh = findCol(["high"]);
      const iLow = findCol(["low"]);
      const iVol = findCol(["volume", "vol"]);
      if (iDate < 0 || iClose < 0) {
        throw new Error("CSV must include a Date column and a Close (or Price) column.");
      }
      for (let r = 1; r < lines.length; r++) {
        const cols = lines[r].split(delim).map((c) => c.trim().replace(/['"]/g, ""));
        const t = parseTimestamp(cols[iDate]);
        const p = Number(cols[iClose]);
        if (!t || !Number.isFinite(p) || p <= 0) continue;
        const h = iHigh >= 0 ? Number(cols[iHigh]) : NaN;
        const l = iLow >= 0 ? Number(cols[iLow]) : NaN;
        const v = iVol >= 0 ? Number(cols[iVol]) : NaN;
        rawRows.push({
          time: t,
          price: p,
          high: Number.isFinite(h) && h >= p ? h : null,
          low: Number.isFinite(l) && l > 0 && l <= p ? l : null,
          volume: Number.isFinite(v) && v > 0 ? v : null,
        });
      }
    }

    const byDay = new Map();
    for (const r of rawRows) {
      if (!r.time || !Number.isFinite(r.price) || r.price <= 0) continue;
      const dayKey = Math.floor(r.time / DAY);
      byDay.set(dayKey, {
        time: r.time,
        price: r.price,
        high: Number.isFinite(r.high) && r.high >= r.price ? r.high : null,
        low: Number.isFinite(r.low) && r.low > 0 && r.low <= r.price ? r.low : null,
        volume: Number.isFinite(r.volume) && r.volume > 0 ? r.volume : null,
      });
    }
    const closes = Array.from(byDay.values())
      .sort((a, b) => a.time - b.time)
      .slice(-4000);
    if (closes.length < 120) {
      throw new Error(`Parsed ${closes.length} valid daily rows — at least 120 daily closes are needed for walk-forward modeling.`);
    }
    const last = closes[closes.length - 1];
    const prev = closes[closes.length - 2];
    return {
      ticker: baseTicker,
      name: `${baseTicker} (Uploaded file)`,
      currency: "USD",
      fetched: last.time,
      sample: false,
      source: `Local file (${filename || "CSV/JSON"})`,
      pct: prev ? change(prev.price, last.price) * 100 : 0,
      closes,
    };
  }

  /* ==================== EVIDENCE PACK GENERATOR (matches src/briefing.rs) ==================== */
  function buildEvidencePack(history, scenario, trials, newsSignal) {
    const end = scenario.end;
    const s = scenario.setup;
    const cal = scenario.calibration;
    const horizon = scenario.fan.length - 1;
    const close = scenario.anchor.price;
    const cur = history.currency || "USD";
    const evidence = [
      {
        id: "E1",
        label: "Reference close",
        fact: `${cur} ${formatPrice(close, "")} on ${formatDate(scenario.anchor.time)}. Horizon: ${horizon} completed trading sessions. Source: ${history.source}.`,
      },
      {
        id: "E2",
        label: "Current setup",
        fact: `5d/20d/60d momentum ${formatPct(s.momentum5)}, ${formatPct(s.momentum)}, ${formatPct(s.momentum60)}; daily vol ${(s.dailyVolatility * 100).toFixed(2)}% (5d/20d ratio ${s.volRatio.toFixed(2)}x); 60d drawdown ${formatPct(s.drawdown)}${s.relStrength20 !== null ? `; 20d rel. strength vs ^GSPC ${formatPct(s.relStrength20)} (S&P 500 vol ${(s.marketVol20 * 100).toFixed(2)}%)` : ""}.`,
      },
      {
        id: "E3",
        label: "Self-calibrating 4-model ensemble",
        fact: `Analogs ${(cal.weights.analogs * 100).toFixed(0)}% (${scenario.matches.length} matches, ${cal.effectiveMatches.toFixed(1)} eff.; ${scenario.upCount} higher), Online QR ${(cal.weights.onlineQr * 100).toFixed(0)}%, GARCH(1,1) ${(cal.weights.garch * 100).toFixed(0)}%, Baseline ${(cal.weights.baseline * 100).toFixed(0)}%. Calibrated 10th/50th/90th returns: ${formatPct(end.low)}, ${formatPct(end.mid)}, ${formatPct(end.high)} (conformal ${cal.conformalDelta >= 0 ? "+" : ""}${(cal.conformalDelta * 100).toFixed(1)} pp).`,
      },
      {
        id: "E4",
        label: "Walk-forward track record",
        fact:
          trials && trials.length
            ? `Across ${trials.length} strictly prior past dates, calibrated 80% ensemble caught ${cal.calibratedCovered}/${trials.length} closes vs ${cal.baselineCovered}/${trials.length} baseline (pinball loss ${(cal.ensemblePinball * 100).toFixed(2)} vs ${(cal.baselinePinball * 100).toFixed(2)} pp; ${(cal.pinballSkill * 100).toFixed(1)}% skill).`
            : "Aggregate track record hidden in blind historical replay.",
      },
      {
        id: "E5",
        label: "Range boundaries",
        fact: `Calibrated lower/upper bounds at +${horizon} sessions: ${formatPrice(close * (1 + end.low), cur)} to ${formatPrice(close * (1 + end.high), cur)}.`,
      },
      {
        id: "E6",
        label: "Evidence gaps",
        fact: "Valuations, full financial filings, liquidity, and macro shifts remain unverified. Out-of-sample statistics are descriptive, not financial advice or guarantees.",
      },
    ];
    if (newsSignal && (newsSignal.articleCount > 0 || newsSignal.earningsInHorizon)) {
      evidence.push({
        id: "E7",
        label: "Headlines & event risk",
        fact: `${newsSignal.articleCount} headlines · ${newsSignal.label} (${newsSignal.score >= 0 ? "+" : ""}${newsSignal.score.toFixed(2)})${newsSignal.eventTags.length ? ` · Topics: ${newsSignal.eventTags.join(", ")}` : ""}${newsSignal.earningsInHorizon ? ` · Expected earnings in ~${newsSignal.earningsDays}d (elevated gap risk)` : ""}.`,
      });
    }

    const lines = [
      `Interpret this Tenerus educational market-research snapshot for ${history.ticker} as of ${formatDate(scenario.anchor.time)}.`,
      `Work strictly from the supplied evidence (E1–E${evidence.length}). Give: (1) Plain-language Read, (2) Strongest Countercase referencing E4/E6, and (3) What Would Change It using exact levels from E5. Do not give financial advice or invent facts.`,
      "",
      ...evidence.map((e) => `[${e.id} · ${e.label}] ${e.fact}`),
    ];
    return lines.join("\n");
  }

  return {
    HORIZONS,
    DEMO_TICKERS,
    sampleHistory,
    runReport,
    runReplay,
    scoreHeadline,
    analyzeHeadlines,
    parseUploadedSeries,
    buildEvidencePack,
    formatDate,
    formatPct,
    formatPrice,
    change,
  };
});
