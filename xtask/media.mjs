// Screenshots and the walkthrough film of the household page.
//
// Drives the Chrome installed on the machine over its DevTools protocol — no
// npm package, no browser download — against `hemsd demo`, with `?capture`, so
// every image is the real page exactly as a box on a wall shows it, rendering
// a real simulated day. The walkthrough follows esales's: a caption bar paced
// to be read, a dot where the finger lands, chapter cards; one continuous take
// encoded with ffmpeg.
//
//   cargo run -p hemsd -- demo &
//   node xtask/media.mjs screenshots [base] [out-dir]     # site/static/screens
//   HEMS_TOKEN=… node xtask/media.mjs box [base] [out-dir]  # a running box: setup, handover
//   node xtask/media.mjs walkthrough [base] [out.mp4]     # site/static/media/walkthrough.mp4
//
// Both fail if any view scrolls sideways at phone width.
import { verifyIndustrialReplay } from "./industrial_replay.mjs";
import { X509Certificate, createHash, generateKeyPairSync, sign } from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { readFileSync, mkdirSync, writeFileSync, rmSync, mkdtempSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";

const [mode = "screenshots", base = "http://127.0.0.1:8080", out] = process.argv.slice(2);
const buildingBase = process.env.BEMSD_URL ?? "http://127.0.0.1:8081";
const industrialBase = process.env.CEMSD_URL ?? "http://127.0.0.1:8082";
const chrome = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const PHONE = { width: 390, height: 844, deviceScaleFactor: 3, mobile: true };
const TABLET = { width: 1280, height: 800, deviceScaleFactor: 2, mobile: true };

// ── Chrome over DevTools ────────────────────────────────────────────────────
const profile = mkdtempSync(join(tmpdir(), "hems-media-"));
const trustArgs = [];
if (["handover-test", "commissioning-review", "first-run-install"].includes(mode) ||
    (mode === "price-sheet" && new URL(base).protocol === "https:")) {
  const url = new URL(base);
  if (url.protocol !== "https:" || url.hostname !== "127.0.0.1") throw Error("Expected an isolated loopback gateway");
  const certificate = new X509Certificate(readFileSync(process.env.GATEWAY_CERT));
  const key = certificate.publicKey.export({type: "spki", format: "der"});
  trustArgs.push("--ignore-certificate-errors-spki-list=" + createHash("sha256").update(key).digest("base64"));
}
const proc = spawn(chrome, [...trustArgs, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run",
  "--no-default-browser-check", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "about:blank"], { stdio: "ignore" });
let target;
for (let i = 0; i < 50 && !target; i++) {
  try {
    const port = Number(readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n", 1)[0]);
    target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === "page");
  } catch { await sleep(200); }
}
if (!target) { proc.kill(); throw new Error("Chrome did not start"); }
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let next = 0;
const pending = new Map(), listeners = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
  else if (msg.method && listeners.has(msg.method)) listeners.get(msg.method)(msg.params);
};
const send = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++next; pending.set(id, (msg) => (msg.error ? reject(new Error(`${method}: ${msg.error.message}`)) : resolve(msg.result)));
  ws.send(JSON.stringify({ id, method, params }));
});
const js = async (expression) => (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result.value;
const done = () => { ws.close(); proc.kill(); try { rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); } catch {} };

await send("Page.enable");
await send("Emulation.setDeviceMetricsOverride", PHONE);
await send("Emulation.setTimezoneOverride", { timezoneId: "Europe/Berlin" });

const problems = [];
async function open(view, { lang = "en", scheme = "light", at = 1050, device = PHONE, scroll } = {}) {
  await send("Emulation.setDeviceMetricsOverride", device);
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: scheme }] });
  await send("Page.navigate", { url: `${base}/?capture&lang=${lang}#${view}` });
  await sleep(900);
  await js(`window.hemsAt(${at})`);
  await sleep(scroll || view === "day" ? 900 : 300);
  if (scroll) { await js(`document.querySelector(${JSON.stringify(scroll)})?.scrollIntoView({ block: "start" }); window.scrollBy(0, -70)`); await sleep(300); }
  const overflow = await js(`document.documentElement.scrollWidth - document.documentElement.clientWidth`);
  if (overflow > 0) problems.push(`${view} (${lang}, ${scheme}): scrolls sideways by ${overflow}px`);
  if (view === "glance") {
    if (await js("document.documentElement.scrollHeight > innerHeight")) problems.push("glance: scrolls vertically");
    if (!await js("parseFloat(getComputedStyle(document.querySelector('.face .figure')).fontSize) >= innerHeight * .18")) problems.push("glance: figure smaller than 18vh");
    if (await js("!!document.querySelector('.face :is(form,button,input,img,textarea,select), .face [data-approve]') || /[a-f0-9]{40,}/i.test(document.querySelector('.face').textContent)")) problems.push("glance: unexpected control or commissioning data");
  }
}

if (mode === "handover-test") {
  const until = async (expression) => {
    const end = Date.now() + 40_000;
    while (Date.now() < end) {
      const value = await js(expression);
      if (value) return value;
      await sleep(150);
    }
    throw Error("Commissioning timed out at " + expression + " (" + await js("document.title") + ")");
  };
  try {
    await send("Page.navigate", {url: base + "/?lang=en#setup"});
    await until('location.origin === ' + JSON.stringify(base) + ' && typeof signIn === "function"');
    if (!await js('signIn(' + JSON.stringify(process.env.HEMS_TOKEN) + ')')) throw Error("Household sign-in failed");
    await send("Page.reload");
    await until('document.querySelector("a[href=\'#handover\']") !== null');
    // Exercise each presentation state against the signed-in Setup view. The
    // series regression drives the actual store through invalidation and repair.
    await send("Emulation.setDeviceMetricsOverride", { ...PHONE, width: 320 });
    const statesChecked = await js(`(() => {
      const saved = trust, language = lang;
      try {
        for (const locale of ["en", "de"]) {
          lang = locale;
          for (const state of ["repair_pending", "busy", "unavailable", "current", "disabled"]) {
            trust = { ...saved, history: state };
            render(true);
            const warning = document.querySelector("[data-history-status]");
            if (["current", "disabled"].includes(state)) {
              if (warning) throw Error("Healthy history displayed an alert");
            } else if (warning?.textContent !== t("history_" + state)) {
              throw Error("Missing history diagnostic: " + state + " / " + locale);
            }
            if (warning && warning.getBoundingClientRect().right > innerWidth) throw Error("History diagnostic overflows the screen");
          }
        }
      } finally { trust = saved; lang = language; render(true); }
      return true;
    })()`);
    if (statesChecked !== true) throw Error("Setup history diagnostics failed at 320 px");
    if (process.env.PAIR_SKI) {
      await until('(async () => (await fetch("/metrics").then(r => r.text())).includes("hems_control_box_needs_attention 1\\n"))()');
      const expected = process.env.PAIR_SKI.replace(/[^a-f0-9]/gi, "").toLowerCase();
      await until('Array.from(document.querySelectorAll("[data-approve]")).find(b => b.dataset.approve.replace(/[^a-f0-9]/gi, "").toLowerCase() === ' + JSON.stringify(expected) + ')?.outerHTML');
      await js('Array.from(document.querySelectorAll("[data-approve]")).find(b => b.dataset.approve.replace(/[^a-f0-9]/gi, "").toLowerCase() === ' + JSON.stringify(expected) + ').click()');
      await until('(async () => !document.querySelector("[data-approve]") && (await fetch("/v1/pairing").then(r => r.json())).trusted.length === 1)()');
    }
    await until(`(async () => {
      const trust = await fetch("/v1/trust").then(r => r.json());
      const metrics = await fetch("/metrics").then(r => r.text());
      return trust.control_box.state === "heard" && Number.isInteger(trust.heartbeat_age_s)
        && metrics.split("\\n").includes("hems_control_box_needs_attention 0");
    })()`);
    await js('location.hash="handover"');
    await until('document.querySelectorAll(".sheet .ski").length === 2 && Array.from(document.querySelectorAll(".sheet img")).every(i => i.complete && i.naturalWidth > 0)');
    const ids = await js('Array.from(document.querySelectorAll(".sheet .ski"), e=>e.textContent.replace(/\\s/g,"").toLowerCase())');
    if (!/^[a-f0-9]{40}$/.test(ids[0]) || !/^[a-f0-9]{64}$/.test(ids[1])) throw Error("Printable identities are incomplete");
    const {data} = await send("Page.printToPDF", {preferCSSPageSize: true});
    if (Buffer.from(data, "base64").subarray(0, 5).toString() !== "%PDF-") throw Error("Print failed");
    console.log(JSON.stringify(ids));
  } finally {
    done();
  }
  process.exit(0);
}

// Exercise the real goal controls with two quotes delivered out of order.
// Only the quote responses are scheduled here. The demo handles the save and readback.
if (mode === "release-preview") {
  let restoreGoal;
  try {
    let ready = false;
    // Debug builds simulate the reference day before the HTTP listener opens.
    const startupDeadline = Date.now() + 180_000;
    while (!ready && Date.now() < startupDeadline) {
      try {
        const response = await fetch(new URL("/v1/home", base), {signal: AbortSignal.timeout(1000)});
        ready = response.ok && (await response.json()).demo === true;
      } catch { /* The simulator may still be recording the day. */ }
      if (!ready) await sleep(500);
    }
    if (!ready) throw Error("Household demo did not become ready at " + base + " within three minutes; check the hemsd startup log");
    const initial = await (await fetch(new URL("/v1/home?at=1050", base))).json();
    const carId = initial.devices.find(device => device.kind === "car").id;
    const goalUrl = new URL("/v1/goals/" + encodeURIComponent(carId), base);
    const savedGoal = initial.goal?.[0];
    restoreGoal = async () => {
      const options = savedGoal ? {method:"PUT", headers:{"content-type":"application/json"}, body:JSON.stringify(savedGoal)} : {method:"DELETE"};
      if (!(await fetch(goalUrl, options)).ok) throw Error("Could not restore the demo departure goal");
    };
    if (savedGoal && !(await fetch(goalUrl, {method:"DELETE"})).ok) throw Error("Could not prepare the winter fixture");
    await open("day", {lang: "en", device: {...PHONE, width: 320}});
    const receipt = initial.receipt;
    if (!receipt?.metered_bill || receipt.money != null || !receipt.comparison_gap || receipt.comparison != null) {
      throw Error("Winter replay must expose its tariff bill and service exclusion");
    }
    if (!await js(`document.querySelector(".receipt").textContent.includes(t(${JSON.stringify("gap_" + receipt.comparison_gap.reason)}))`)) {
      throw Error("Day omitted the simulated service exclusion");
    }
    const forecastDays = (await (await fetch(new URL("/v1/days", base))).json()).days;
    const scoredCount = forecastDays.filter(day => day.forecast != null).length;
    if (scoredCount < 14 && !await js(`document.body.innerText.includes(${JSON.stringify("Forecast learning · " + scoredCount + " of 14 days scored")})`)) {
      throw Error("Day learning count disagrees with closed receipts");
    }
    const displayedMonth = forecastDays.at(-1)?.day.slice(0, 7);
    const scoredDay = forecastDays.findLast(day => day.forecast != null && day.day.startsWith(displayedMonth));
    if (scoredDay) {
      const selected = await js(`(() => {
        const bar = Array.from(document.querySelectorAll("[data-day]")).find(el => el.dataset.day === ${JSON.stringify(scoredDay.day)});
        if (!bar) return false;
        bar.dispatchEvent(new MouseEvent("click", {bubbles: true}));
        return document.body.innerText.includes(${JSON.stringify("Solar " + scoredDay.forecast.pv_pct + "% · household load " + scoredDay.forecast.load_pct + "%")});
      })()`);
      if (!selected) throw Error("Selected day disagrees with its measured forecast coverage");
    }
    const matchedReceipt = await js(`(() => {
      const previousDays = days, previousPick = picked;
      try {
        const d = {...home.receipt, day: "2026-11-04", money: null,
          metered_bill: {energy_ct: 200, fixed_ct: 20, watched_intervals: 96, expected_intervals: 96},
          comparison: {with_plan_ct: 219, without_plan_ct: 220, error_ct: 2},
          arbitrage: {blocks: 1, bought_wh: 1000, served_wh: 800, bought_ct: 30, avoided_ct: 28, buy_rate_mct_per_kwh: 30000, use_rate_mct_per_kwh: 35000}};
        const other = {...d, day: "2026-11-05", arbitrage: null, comparison: {with_plan_ct: 225, without_plan_ct: 220, error_ct: 2}};
        days = [d, other]; picked = d.day;
        const sheet = document.createElement("div");
        sheet.innerHTML = viewDay({...home, demo: false});
        return sheet.textContent.includes("Within model error")
          && sheet.textContent.includes("With plan (model)")
          && sheet.textContent.includes("Margin after storage losses")
          && sheet.textContent.includes("Conclusive comparison days · 1")
          && !!sheet.querySelector('rect[fill="var(--ink3)"]')
          && !!sheet.querySelector('rect[fill="var(--bad)"]');
      } finally { days = previousDays; picked = previousPick; }
    })()`);
    if (!matchedReceipt) throw Error("Matched-day cost, uncertainty, loss or battery-margin rendering is incorrect");
    // Compare every visible number on Now at the winter reduction, including
    // device explanations and chart ticks. Read the API independently of Chrome.
    const minute = 1050;
    const home = await (await fetch(new URL("/v1/home?at=" + minute, base))).json();
    const device = kind => home.devices.find(d => d.kind === kind);
    const localTime = iso => new Date(iso).toLocaleTimeString("en-GB", {
      hour: "2-digit", minute: "2-digit", timeZone: "Europe/Berlin",
    });
    const sorted = home.plan.map(p => p.price_ct).filter(p => p != null).sort((a, b) => a - b);
    const cheapLimit = sorted[Math.floor(sorted.length / 3) - 1];
    const start = home.plan.findIndex(p => p.price_ct != null && p.price_ct <= cheapLimit + 1e-9);
    const length = home.plan.slice(start).findIndex(p => p.price_ct == null || p.price_ct > cheapLimit + 1e-9);
    if (home.at_minute !== minute || start <= 0 || length <= 0) throw Error(`Unexpected winter reference window (minute ${home.at_minute}, start ${start}, length ${length})`);
    const prices = [device("heat_pump"), device("hot_water")].map(d => d.why.params);
    const pack = device("battery");
    const expectedFigures = [
      Math.abs(home.grid_kw).toFixed(1), Math.round(home.price_ct), "14", home.reduction.minimum_kw.toFixed(1),
      "24", localTime(home.plan[start].start), Math.round(length / 4),
      ...[0, 24, 48, 72].map(slot => localTime(home.plan[slot].start)), Math.round(device("car").soc * 100),
      prices[0].at, prices[0].then_ct, pack.kw.toFixed(1), Number(pack.why.params.kw).toFixed(1),
      pack.why.params.price_ct, prices[1].at, prices[1].then_ct,
    ].join(" ");
    const figures = text => text.match(/-?\d+(?:[.:]\d+)?/g);
    await open("now", {at: minute});
    const shownFigures = figures(await js('document.querySelector("#screen").innerText'));
    if (JSON.stringify(shownFigures) !== JSON.stringify(figures(expectedFigures))) {
      throw Error("Winter Now differs from /v1/home: " + shownFigures + " (expected " + expectedFigures + ")");
    }
    console.log("Household Now figures agree with the fixed API minute");
    for (const [lang, store, sell] of [["de", "in den Speicher", "wird eingespeist"], ["en", "goes into the battery", "it is exported"]]) {
      await open("now", {lang, at: minute, device: {...PHONE, width: 320}});
      const why = await js("Array.from(document.querySelectorAll('.why'), el => el.textContent)");
      const price = pack.why.params.price_ct;
      if (pack.why.key !== "plan.discharge" || !why.some(text =>
        text.includes(lang === "de" ? "damit Sie zu " + price + " ct nichts kaufen" : "so you buy nothing at " + price + " ct"))) {
        throw Error("Winter battery reason disagrees with /v1/home in " + lang);
      }
      const keys = async (key, code) => {
        for (const type of ["keyDown", "keyUp"]) await send("Input.dispatchKeyEvent", {type, key, code:key, windowsVirtualKeyCode:code});
      };
      const sequence = [];
      for (let n = 0; n < 30; n++) {
        await keys("Tab", 9);
        sequence.push(await js("document.activeElement.textContent.trim() || document.activeElement.tagName"));
        if (await js("document.activeElement.matches('a[data-v=plan]')")) break;
      }
      if (!await js("document.activeElement.matches('a[data-v=plan]')")) throw Error("Keyboard missed Plan: " + sequence);
      await keys("Enter", 13);
      for (let n = 0; n < 40 && !await js("!!document.querySelector('.say.plan')"); n++) await sleep(25);
      const first = home.plan.find(slot => slot.kw[pack.id] < -0.05);
      if (!first || !await js("document.querySelector('.say.plan')?.textContent.includes(" + JSON.stringify(localTime(first.start)) + ")")) {
        throw Error("Battery discharge time disagrees with /v1/home in " + lang);
      }
      if (await js("document.documentElement.scrollWidth > innerWidth")) throw Error("Plan overflows at 320px in " + lang);
      console.log("320px winter keyboard journey (" + lang + "): " + sequence.join(" → "));
      const expectedPlan = lang === "de"
        ? "Der Speicher versorgt das Haus 17:30–21:00. Die Wärmepumpe läuft 19:30–23:45. Das Warmwasser heizt 22:15–00:00."
        : "The battery carries the house 17:30–21:00. The heat pump runs 19:30–23:45. Hot water heats 22:15–00:00.";
      if (!await js(`document.querySelector(".say.plan")?.textContent.startsWith(${JSON.stringify(expectedPlan)})`)) throw Error("Unexpected winter plan in " + lang);
      const limit = await js('reductionChip({ceiling_kw:4.2, minimum_kw:null, failsafe:false})');
      if (!limit.includes(lang === "de" ? "Begrenzung auf 4,2" : "limited to 4.2")) throw Error("Missing guarantee shown as a guaranteed value");
      const compared = await js(`[worth({worth: {battery_ct: 20, export_ct: 8}}), worth({worth: {battery_ct: 5, export_ct: 8}}), worth({})]`);
      if (!compared[0].includes(store) || !compared[1].includes(sell) || compared[2] !== "") throw Error("Unexpected export comparison in " + lang);
    }
    for (let i = 0; i < 100; i++) {
      if (await js('Boolean(document.querySelector("[data-goal]"))')) break;
      await sleep(50);
    }
    const result = await send("Runtime.evaluate", {
      awaitPromise: true, returnByValue: true,
      expression: `(async () => {
        const card = document.querySelector('[data-goal][data-tank=""]');
        const range = card.querySelector("input[type=range]");
        const original = window.fetch;
        const pendingQuotes = [];
        let writes = 0;
        window.fetch = async (url, init) => {
          if (String(url).endsWith("/quote")) {
            return new Promise(resolve => pendingQuotes.push(price => resolve(
              Response.json({verdict:"feasible", extra_eur:price}))));
          }
          if (init?.method === "PUT") {
            writes++;
            const saved = await original(url, init);
            if (!saved.ok) throw Error("demo refused save");
            throw TypeError("response lost");
          }
          return original(url, init);
        };
        const waitFor = async (test) => {
          for (let i=0; i<150; i++) {
            if (test()) return;
            await new Promise(resolve => setTimeout(resolve, 30));
          }
          throw Error("timed out waiting for replay state");
        };
        const set = value => { range.value = value; range.dispatchEvent(new Event("change")); };
        const quote = card.querySelector("[data-quote]");
        set(80); set(75);
        pendingQuotes[1](0.42); pendingQuotes[0](0.97);
        await waitFor(() => quote.querySelector("button"));
        if (!quote.textContent.includes("0.42") || quote.textContent.includes("0.97")) throw Error("stale quote displayed");
        range.focus();
        await window.hemsAt(1051);
        if (!card.isConnected || document.activeElement !== range) throw Error("poll discarded draft or focus");
        quote.querySelector("button").click();
        await waitFor(() => pendingQuotes.length === 3);
        pendingQuotes[2](0.55);
        await waitFor(() => quote.textContent.includes("0.55"));
        if (writes !== 0) throw Error("changed quote was committed");
        quote.querySelector("button").click();
        await waitFor(() => pendingQuotes.length === 4);
        pendingQuotes[3](0.55);
        await waitFor(() => !card.isConnected);
        if (writes !== 1 || !document.querySelector('[data-goal] .cap').textContent.includes('75')) throw Error("save was not reconciled");
        const nextCard = document.querySelector('[data-goal][data-tank=""]');
        const slider = nextCard.querySelector("input[type=range]");
        const draftPrice = nextCard.querySelector("[data-quote]");
        const device = home.devices.find(d => d.id === nextCard.dataset.goal);
        // The demo may have no plugged-in car at this minute. Supply a known
        // slope to check the visible arithmetic and release transition.
        device.goal_eur_per_unit = 0.04;
        slider.value = "85";
        slider.dispatchEvent(new Event("input"));
        if (!draftPrice.textContent.includes("About") || !draftPrice.textContent.includes("0.40") ||
            draftPrice.querySelector("button") || pendingQuotes.length !== 4) throw Error("drag estimate or premature quote");
        const amount = nextCard.querySelector("[data-amount]");
        amount.value = "78";
        amount.dispatchEvent(new Event("input"));
        amount.dispatchEvent(new Event("change"));
        const deadline = nextCard.querySelector("select");
        deadline.value = "480";
        deadline.dispatchEvent(new Event("change"));
        if (draftPrice.textContent.includes("About")) throw Error("drag estimate survived release");
        pendingQuotes[5](0.63); pendingQuotes[4](0.91);
        const currentQuote = nextCard.querySelector("[data-quote]");
        await waitFor(() => currentQuote.querySelector("button"));
        if (!currentQuote.textContent.includes("0.63") || !currentQuote.textContent.includes("78 % · 08:00")) throw Error("deadline-only change or typed target lost");
        nextCard.querySelector("[data-cancel]").click();
        if (writes !== 1 || !document.querySelector('[data-goal] .cap').textContent.includes('75')) throw Error("discard mutated saved goal");
        const revised = document.querySelector('[data-goal][data-tank=""]');
        const cap = revised.querySelector("[data-cap]");
        cap.value = "30";
        cap.dispatchEvent(new Event("change"));
        const current = pendingQuotes.length - 1;
        cap.value = "25";
        cap.dispatchEvent(new Event("change"));
        pendingQuotes[current + 1](0.18);
        pendingQuotes[current](0.91);
        await waitFor(() => revised.querySelector("[data-quote] button"));
        if (!revised.querySelector("[data-quote] button").textContent.includes("25 ct/kWh")) throw Error("cap-only change reused an old price");
        revised.querySelector("[data-cancel]").click();
        // Give the ordinary Plan controls a connected charge point and a
        // deterministic price. The solver and stop conditions run in Rust tests.
        const state = {...home, demo:false, charging_now:false, goal:[home.goal[0], {verdict:"feasible", needed_kwh:10}]};
        home = state;
        window.fetch = async (url, init) => {
          if (String(url).includes("/quote?charge_now=true")) return Response.json({verdict:"feasible", extra_eur:0.31, boost_until:1800000000});
          if (String(url).endsWith("/boost")) {
            if (init.method === "PUT") {
              const approved = JSON.parse(init.body);
              if (approved.extra_eur !== 0.31 || approved.goal.target_soc !== 0.75) throw Error("wrong boost approved");
              state.charging_now = true;
            } else state.charging_now = false;
            return new Response(null, {status:204});
          }
          if (String(url) === "/v1/home") return Response.json(state);
          return original(url, init);
        };
        render();
        document.querySelector("[data-boost]").click();
        const boostCard = document.querySelector("[data-goal]");
        const boostQuote = boostCard.querySelector("[data-quote]");
        await waitFor(() => boostQuote.querySelector("button"));
        if (!boostQuote.textContent.includes("0.31") || state.charging_now) throw Error("boost preview committed or unpriced");
        boostQuote.querySelector("button").click();
        await waitFor(() => document.querySelector("[data-boost]")?.textContent.includes("End immediate charging"));
        document.querySelector("[data-boost]").click();
        await waitFor(() => !state.charging_now && document.querySelector("[data-boost]")?.textContent.includes("Preview"));
        return "goal slope, price cap, quote race, focus and interrupted save passed";
      })()`,
    });
    if (result.exceptionDetails) throw Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    console.log(result.result.value);
    await send("Emulation.setDeviceMetricsOverride", { ...PHONE, width: 320 });
    for (const [scenario, expected] of [["confirmed","Ladegrenze eingehalten"],["above-request","26 kW gemessen"],["unconfirmed","Lieferung unbestätigt"],["stale","Netzmessung veraltet"],["stale-charger","Lieferung unbestätigt"],["over-limit","3 kW über Anschlussgrenze"]]) {
      await send("Page.navigate", {url: buildingBase + "/?case=" + scenario});
      let matched = false;
      for (let i = 0; i < 100 && !matched; i++) {
        await sleep(30);
        matched = await js('document.readyState === "complete" && location.search === ' + JSON.stringify("?case="+scenario) + ' && document.body.innerText.includes(' + JSON.stringify(expected) + ')');
      }
      if (!matched) throw Error("Building replay failed: " + scenario);
      const reading = await (await fetch(buildingBase + "/v1/demo/building?case=" + scenario)).json();
      if (scenario === "stale-charger" || scenario === "above-request") {
        const missing = scenario === "stale-charger";
        const [first, second] = reading.stations;
        if (first.id !== "West-01" || second.id !== "West-02" ||
            !first.profile_accepted || !second.profile_accepted ||
            first.final_kw !== (missing ? 12 : 13) || second.final_kw !== (missing ? null : 13) ||
            reading.final_sample_kw !== (missing ? null : 26)) throw Error("Incorrect station evidence");
        await js('document.querySelector("details summary").click()');
        const evidence = await js('document.querySelector("details[open]").innerText');
        if (!evidence.includes("West-02") || !evidence.includes(missing ? "Kein zeitgleicher Messwert" : "13 kW gemessen")) throw Error("Station evidence missing from handover");
      }
      const verdicts = {
        "within-limit": "Ladegrenze eingehalten",
        "above-limit": "Ladegrenze überschritten",
        "unconfirmed": "Lieferung unbestätigt",
      };
      if (!await js('document.querySelector("main strong")?.textContent === ' + JSON.stringify(verdicts[reading.delivery]) + ' && document.documentElement.scrollWidth <= innerWidth')) {
        throw Error("Building verdict or layout disagrees: " + scenario);
      }
    }
    console.log("Building incident cases passed");
    for (const [moment, required, residual, exposure] of [
      ["early", 120, 1100, 0], ["delayed", 200, 1116, 1600], ["closed", null, null, null],
    ]) {
      const query = "?moment=" + moment;
      const response = await fetch(industrialBase + "/v1/demo/industrial" + query);
      const reading = await response.json();
      if (!response.ok || reading.required_kw !== required || reading.residual_peak_kw !== residual ||
          reading.residual_exposure_eur !== exposure || reading.closed_mean_kw !== (moment === "closed" ? 1140 : null)) {
        throw Error("Industrial calculation disagrees at " + moment);
      }
      await send("Page.navigate", {url: industrialBase + "/" + query});
      const expected = moment === "closed" ? "1140 kW im abgeschlossenen Intervall" : residual + " kW mit Batterie";
      let matches = false;
      for (let i = 0; i < 100 && !matches; i++) {
        await sleep(30);
        matches = await js('location.search === ' + JSON.stringify(query) + ' && document.body.innerText.includes(' + JSON.stringify(expected) + ') && document.documentElement.scrollWidth <= innerWidth');
      }
      if (!matches) throw Error("Industrial screen disagrees at " + moment);
    }
    for (const [query, expected] of [
      ["moment=unavoidable", {minimum_mean_kw:1120, required_kw:null, residual_peak_kw:null}],
      ["evidence=limited-battery", {achievable_kw:60, residual_peak_kw:1120, residual_exposure_eur:2000}],
      ["evidence=battery-unknown", {projected_kw:1140, achievable_kw:null, residual_peak_kw:null}],
      ["evidence=tariff-review", {residual_peak_kw:1100, residual_exposure_eur:null}],
      ["evidence=stale-meter", {import_kwh:null, projected_kw:null, prior_peak_kw:1100}],
      ["moment=closed&evidence=stale-meter", {closed_mean_kw:null, projected_kw:null}],
    ]) {
      const response = await fetch(industrialBase + "/v1/demo/industrial?" + query);
      const reading = await response.json();
      for (const [field, value] of Object.entries(expected)) {
        if (reading[field] !== value) throw Error(query + ": " + field + " expected " + value + ", got " + reading[field]);
      }
    }
    for (const route of ["/", "/v1/demo/industrial"]) {
      const unknown = await fetch(industrialBase + route + "?moment=unknown");
      const write = await fetch(industrialBase + route, {method: "POST"});
      if (unknown.status !== 400 || write.status !== 405) throw Error("Unexpected industrial route boundary: " + route);
    }
    await verifyIndustrialReplay(industrialBase, send, js);
    console.log("Industrial shift replay passed");
    for (const [url, foreignRoutes] of [
      [base, ["/v1/demo/building", "/v1/demo/industrial"]],
      [buildingBase, ["/v1/home", "/v1/demo/industrial"]],
      [industrialBase, ["/v1/home", "/v1/demo/building"]],
    ]) {
      for (const route of foreignRoutes) {
        if ((await fetch(url + route)).status !== 404) throw Error(url + " exposes " + route);
      }
    }

  } catch (error) { problems.push(String(error)); }
  finally { try { await restoreGoal?.(); } catch (error) { problems.push(String(error)); } }
}

// ── Screenshots ─────────────────────────────────────────────────────────────
if (mode === "screenshots") {
  const dir = out ?? "site/static/screens";
  mkdirSync(dir, { recursive: true });
  const shots = [
    ["now-reduced", "now", { at: 1050 }],
    ["now-reduced-dark", "now", { at: 1050, scheme: "dark" }],
    ["now-night", "now", { at: 240 }],
    ["plan", "plan", { at: 1000 }],
    ["day", "day", { at: 1430 }],
    ["month", "day", { at: 1430, scroll: ".month" }],
    ["goals", "plan", { at: 1000, scroll: "[data-goal]" }],
    ["setup", "setup", { at: 600 }],
    ["now-de", "now", { at: 1050, lang: "de" }],
    ["glance", "glance", { at: 840, device: TABLET }],
    ["glance-reduced", "glance", { at: 1050, device: TABLET }],
    ["glance-night", "glance", { at: 240, device: TABLET }],
  ];
  for (const [name, view, opts] of shots) {
    await open(view, opts);
    const { data } = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(join(dir, `${name}.png`), Buffer.from(data, "base64"));
    console.log(`  ${dir}/${name}.png`);
  }
}

// ── A real box ──────────────────────────────────────────────────────────────
// The views a replay cannot show — pairing state, discovery, notifications, the
// handover sheet — from a running box (`hemsd house` and `hemsd run` beside it),
// signed in with HEMS_TOKEN.
if (mode === "price-sheet") {
  try {
    if (new URL(base).hostname !== "127.0.0.1") throw Error("Use an isolated loopback household gateway");
    const waitFor = async (condition) => {
      for (let i = 0; i < 250; i++) {
        if (await js(condition)) return;
        await sleep(100);
      }
      throw Error("Tariff page did not reach " + condition);
    };
    const sheet = JSON.parse(readFileSync(new URL("../crates/ems-tariff/examples/preisblatt-fixed.json", import.meta.url)));
    const year = Number(new Intl.DateTimeFormat("en", {year:"numeric",timeZone:"Europe/Berlin"}).format(new Date())) + 2;
    Object.assign(sheet, {contract:"HOUSEHOLD-BROWSER-TEST",valid_from:year + "-01-01",priced_for_year:year});
    const make = (doc) => {
      const {privateKey,publicKey} = generateKeyPairSync("ed25519");
      const jwk = publicKey.export({format:"jwk"});
      const encode = (value) => Buffer.from(JSON.stringify(value)).toString("base64url");
      const input = encode({alg:"Ed25519",kid:"Browser fixture",jwk}) + "." + encode(doc);
      return input + "." + sign(null,Buffer.from(input),privateKey).toString("base64url");
    };
    const signed = make(sheet);
    const upload = (content) => js(`document.querySelector("#supplyFile").files = (() => { const d = new DataTransfer(); d.items.add(new File([${JSON.stringify(content)}],"supplier.jws",{type:"application/jose"}));return d.files; })(); document.querySelector("#supplyFile").dispatchEvent(new Event("change", {bubbles:true}));`);
    await send("Emulation.setDeviceMetricsOverride",{...PHONE,width:320});
    await send("Page.navigate",{url:base + "/?lang=en#t=" + encodeURIComponent(process.env.HEMS_TOKEN)});
    await waitFor('typeof home !== "undefined" && home && view === "now"');
    await js('location.hash = "setup"');
    await waitFor('document.querySelector("#supplyFile") && supplyStatus.can_import');
    if (!await js('supplyStatus.active == null && supplyStatus.next == null')) throw Error("Tariff acceptance needs an empty household store");
    const dynamic = JSON.parse(readFileSync(new URL("../crates/ems-tariff/examples/preisblatt-dynamic.json", import.meta.url)));
    Object.assign(dynamic, {contract:"DYNAMIC-BROWSER-TEST", valid_from:sheet.valid_from, priced_for_year:year});
    await upload(make(dynamic));
    await waitFor('supplyDraft?.review.terms.contract === "DYNAMIC-BROWSER-TEST"');
    if (!await js('supplyDraft.review.price_ct === null && document.querySelector("#supplyCard").innerText.includes("Market price pending")')) throw Error("A future dynamic contract needs an explicit missing price");
    await js('document.querySelector("#supplyCancel").click()');
    await waitFor('document.activeElement?.id === "supplyFile" && !supplyDraft');

    const parts = signed.split(".");
    await upload(parts[0] + "." + Buffer.from(JSON.stringify({...sheet,contract:"MODIFIED"})).toString("base64url") + "." + parts[2]);
    await waitFor('supplyMessage && !supplyDraft && !supplyBusy');
    await upload(signed);
    await waitFor('supplyDraft?.review.terms.contract === "HOUSEHOLD-BROWSER-TEST"');
    if (!await js('document.querySelector("#supplyCard code").textContent === supplyDraft.review.terms.fingerprint && document.documentElement.scrollWidth <= innerWidth')) throw Error("Tariff review or phone width");
    if (!await js('document.querySelector("#supplyCard").innerText.includes("33.71389 ct/kWh")')) throw Error("The confirmation must show the exact signed price");
    await js('document.querySelector("#lang").focus(); document.querySelector("#lang").click()');
    if (!await js('document.querySelector("#supplyCard").innerText.includes("33,71389 ct/kWh")')) throw Error("German review lost decimal precision");
    await js('document.querySelector("#lang").focus(); document.querySelector("#lang").click()');
    const other = make(sheet);
    const status = await js(`fetch("/v1/tariff", {method:"PUT",headers:{"Content-Type":"application/json"}, body:JSON.stringify({signed:${JSON.stringify(other)}, confirmation:supplyDraft.review})}).then(r => r.status)`);
    if (status !== 409) throw Error("Changed signer under the same key ID was accepted");
    await js('document.querySelector("#supplyUse").click()');
    await waitFor('supplyStatus.next?.contract === "HOUSEHOLD-BROWSER-TEST" && !supplyBusy');
    console.log("Household tariff upload, tamper refusal, signer confirmation and readback passed");
  } catch (error) { problems.push(String(error)); }
}

if (mode === "first-run-install") {
  const until = async (expression) => {
    for (let i = 0; i < 400; i++) {
      if (await js(expression)) return;
      await sleep(100);
    }
    throw Error("First-run workflow did not reach " + expression + " · " + await js('document.getElementById("status")?.textContent'));
  };
  try {
    await send("Emulation.setDeviceMetricsOverride", {...PHONE, width:320});
    await send("Page.navigate",{url:base + "/?lang=en"});
    await until('document.getElementById("login") && !!document.getElementById("login").onsubmit');
    await js('document.querySelector("[name=token]").value=' + JSON.stringify(process.env.HEMS_TOKEN) + '; document.querySelector("#login").requestSubmit()');
    await until('!document.getElementById("meter").hidden');
    await js(`(() => {
      const f = document.querySelector("#meter").elements;
      f.role.value="installer"; f.role[1].dispatchEvent(new Event("change",{bubbles:true}));
      f.support.value='Elektro "Nord"'; f.phases.checked=true; f.fuse.value=35;
      f.host.value="127.0.0.1"; f.port.value="1502"; f.unit.value=2;
      f.energy.value=20; f.network.value=7; f.standing.value=90;
      for(const [asset,size,power] of [["roof",8,8],["storage",10,5]]) {
        f[asset].click(); f[asset+"_size"].value=size; f[asset+"_power"].value=power; f[asset+"_date"].value="2024-06-01";
      }
      f.controller.value="Commissioned site controller";
      document.querySelector("#meter").requestSubmit();
    })()`);
    await until('!document.getElementById("review").hidden && document.getElementById("block").textContent.includes("listens_only = true")');
    if (!await js('document.documentElement.scrollWidth <= innerWidth')) throw Error("Meter review overflows the phone layout");
    await js('document.getElementById("apply").click()');
    await until('!!document.querySelector("#find")');
    const contact = await js('fetch("/v1/trust").then(r=>r.json()).then(r=>r.support_contact)');
    if (contact !== 'Elektro "Nord"') throw Error("Installer contact lost on activation");
    console.log("Empty site commissioned its meter, PV and observed battery through the browser");
  } catch (error) { problems.push(String(error)); }
}

if (mode === "commissioning-review") {
  try {
    const waitFor = async (expression) => {
      for (let i = 0; i < 400; i++) {
        if (await js(expression)) return;
        await sleep(100);
      }
      throw Error("Commissioning review did not reach " + expression + " (page " + await js('document.title + ", state " + (typeof view === "undefined" ? "unavailable" : view) + ", summary " + document.activeElement?.tagName + ", open " + !!document.querySelector(".inverter-review[open]") + ", language " + document.documentElement.lang') + ")");
    };
    const enter = async () => {
      for (const type of ["keyDown", "keyUp"]) {
        await send("Input.dispatchKeyEvent", {type, key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: type === "keyDown" ? "\r" : ""});
      }
    };
    await send("Emulation.setDeviceMetricsOverride", { ...PHONE, width: 320 });
    await send("Page.navigate", {url: base + "/?lang=en#t=" + encodeURIComponent(process.env.HEMS_TOKEN)});
    await waitFor('home && view === "now"');
    await js('location.hash = "setup"');
    await waitFor('document.querySelector("#find")');
    await js('$("#hosts").value = "127.0.0.1"; $("#findPort").value = "1502"; $("#find").requestSubmit()');
    await waitFor('document.activeElement.matches(".inverter-review summary")');
    await enter();
    await waitFor('document.querySelector(".inverter-review[open]")?.innerText.includes("Configuration valid")');
    if (!await js('$(".inverter-review pre").textContent.includes("rating_kw") && document.documentElement.scrollWidth <= innerWidth')) throw Error("PV review at phone width");
    await js('document.querySelector("[data-review-close]").focus()');
    await enter();
    await waitFor('!document.querySelector(".inverter-review[open]") && document.activeElement.matches(".inverter-review summary")');
    console.log("Inverter review and keyboard cancellation passed");
    if (process.env.REVIEW_APPLY === "1") {
      listeners.set("Page.javascriptDialogOpening", () => send("Page.handleJavaScriptDialog", {accept:true}));
      await enter();
      await waitFor('document.querySelector(".inverter-review[open] [data-install]")');
      const id = await js('document.querySelector("[data-install]").dataset.install');
      await js('document.querySelector("[data-install]").focus()');
      await enter();
      await waitFor('document.querySelector("[role=status]") && document.body.innerText.includes("Inverter configured")');
      const repeated = await js('api("/v1/commissioning/change",{method:"POST", body:JSON.stringify({id:' + JSON.stringify(id) + '})})');
      if (repeated?.id !== id || repeated.outcome !== "applied") throw Error("Installation replay changed its outcome");
      console.log("Confirmed install survived restart and returned its original outcome on retry");
    }
  } catch (error) { problems.push(String(error)); }
}

if (mode === "box") {
  const dir = out ?? "site/static/screens";
  mkdirSync(dir, { recursive: true });
  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });
  await send("Page.navigate", { url: `${base}/?lang=en#t=${process.env.HEMS_TOKEN ?? ""}` });
  await sleep(1500);
  // Reload: the page forgets the access code, so the sheet is captured without
  // the household's sign-in code. That one is never photographed.
  await send("Page.reload");
  await sleep(1500);
  for (const [name, view, full] of [["box-setup", "setup", true], ["handover", "handover", true], ["record", "record=WO-2026-000417", true]]) {
    await js(`location.hash = '${view}'`);
    await sleep(2500);
    const overflow = await js(`document.documentElement.scrollWidth - document.documentElement.clientWidth`);
    if (overflow > 0) problems.push(`${view}: scrolls sideways by ${overflow}px`);
    const { data } = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: full });
    writeFileSync(join(dir, `${name}.png`), Buffer.from(data, "base64"));
    console.log(`  ${dir}/${name}.png`);
  }
}

// ── Walkthrough ─────────────────────────────────────────────────────────────
if (mode === "walkthrough") {
  const file = out ?? "site/static/media/walkthrough.mp4";
  mkdirSync(dirname(file), { recursive: true });
  const frames = mkdtempSync(join(tmpdir(), "hems-frames-"));
  const shot = [];
  listeners.set("Page.screencastFrame", ({ data, metadata, sessionId }) => {
    const n = String(shot.length).padStart(5, "0");
    writeFileSync(join(frames, `${n}.jpg`), Buffer.from(data, "base64"));
    shot.push(metadata.timestamp);
    send("Page.screencastFrameAck", { sessionId }).catch(() => {});
  });

  // The on-screen helpers, injected after each navigation.
  const helpers = `(() => {
    const css = document.createElement('style');
    css.textContent = '#__cap{position:fixed;left:12px;right:12px;bottom:calc(92px + env(safe-area-inset-bottom));z-index:99;background:rgba(0,0,0,.78);color:#fff;font:600 17px/1.35 -apple-system,system-ui,sans-serif;padding:12px 14px;border-radius:14px;transition:opacity .35s}' +
      '#__dot{position:fixed;width:34px;height:34px;margin:-17px 0 0 -17px;border-radius:50%;background:rgba(0,134,111,.35);border:2px solid #00866f;z-index:100;pointer-events:none;transition:all .45s ease}' +
      '#__card{position:fixed;inset:0;z-index:101;display:grid;place-items:center;text-align:center;background:#f2f2f7;color:#000;font:700 30px/1.2 -apple-system,system-ui,sans-serif;padding:32px}' +
      '#__card small{display:block;font:500 18px/1.4 -apple-system,system-ui,sans-serif;color:rgba(60,60,67,.62);margin-top:12px}';
    document.head.appendChild(css);
  })()`;
  // Styled through attributes: the page's CSP refuses an injected <style>, as it should.
  const CAP = "position:fixed;left:12px;right:12px;bottom:calc(84px + env(safe-area-inset-bottom));z-index:99;background:rgba(0,0,0,.8);color:#fff;font:600 17px/1.35 -apple-system,system-ui,sans-serif;padding:12px 14px;border-radius:14px";
  const DOT = "position:fixed;width:34px;height:34px;margin:-17px 0 0 -17px;border-radius:50%;background:rgba(0,134,111,.35);border:2px solid #00866f;z-index:100;pointer-events:none;transition:all .45s ease";
  const CARD = "position:fixed;inset:0;z-index:101;display:grid;place-items:center;text-align:center;background:#f2f2f7;color:#000;font:700 30px/1.2 -apple-system,system-ui,sans-serif;padding:32px";
  const caption = (text) => js(`(() => { let b = document.getElementById('__cap'); if (!b) { b = document.createElement('div'); b.id = '__cap'; b.setAttribute('style', ${JSON.stringify(CAP)}); document.body.appendChild(b); } b.textContent = ${JSON.stringify(text)}; })()`);
  const say = async (text) => { await caption(text); await sleep(Math.max(2600, text.length * 66)); };
  const card = async (title, sub, hold = 2600) => {
    await js(`(() => { const c = document.createElement('div'); c.id = '__card'; c.setAttribute('style', ${JSON.stringify(CARD)}); c.innerHTML = ${JSON.stringify(`<div>${title}<div style="font:500 18px/1.4 -apple-system,system-ui,sans-serif;color:rgba(60,60,67,.62);margin-top:12px">${sub}</div></div>`)}; document.body.appendChild(c); })()`);
    await sleep(hold);
    await js(`document.getElementById('__card')?.remove()`);
  };
  const tap = async (selector) => {
    const box = await js(`(() => { const r = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect(); return { x: r.left + r.width / 2, y: r.top + r.height / 2 }; })()`);
    await js(`(() => { let d = document.getElementById('__dot'); if (!d) { d = document.createElement('div'); d.id = '__dot'; d.setAttribute('style', ${JSON.stringify(DOT)}); document.body.appendChild(d); } d.style.left = '${box.x}px'; d.style.top = '${box.y}px'; })()`);
    await sleep(600);
    await js(`document.querySelector(${JSON.stringify(selector)}).click()`);
    await sleep(700);
  };
  const go = async (view) => { await js(`location.hash = '${view}'`); await sleep(500); };
  const travel = async (from, to, step = 10, ms = 120) => { for (let m = from; m <= to; m += step) { await js(`window.hemsAt(${m})`); await sleep(ms); } };

  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });
  await send("Page.navigate", { url: `${base}/?capture&lang=de#now` });
  await sleep(900);
  await js(`window.hemsAt(180)`);
  await send("Page.startScreencast", { format: "jpeg", quality: 88, everyNthFrame: 1 });

  await card("Ihr Haus, erklärt.", "hems · ein Wintertag, von der Box selbst gerechnet");
  await say("Oben steht eine Zahl: was das Haus gerade aus dem Netz nimmt. Darunter ein Satz, warum.");
  await travel(180, 330);
  await say("Nachts ist Strom günstig. Das grüne Band zeigt, wann — wie ein Wetterbericht für den Preis.");
  await travel(330, 1020, 15, 60);
  await say("17 Uhr: Der Netzbetreiber dimmt den Anschluss nach § 14a. hems sagt es sofort — und was garantiert bleibt.");
  await travel(1020, 1060, 5, 160);
  await sleep(800);
  await tap('.tabs a[data-v="plan"]');
  await say("Der Plan für die nächsten zwei Tage. Grün ist günstig, violett der Netzbetreiber.");
  await js(`(() => { const s = document.getElementById('strip'); const r = s.getBoundingClientRect(); s.dispatchEvent(new PointerEvent('pointerdown', { clientX: r.left + r.width * 0.62, clientY: r.top + r.height / 2, bubbles: true })); })()`);
  await say("Jede Viertelstunde erklärt sich — mit dem Preis der Alternative.");
  await js(`document.querySelector('[data-goal] input')?.scrollIntoView({ block: 'center', behavior: 'smooth' })`);
  await sleep(700);
  await js(`(() => { const g = document.querySelector('[data-goal] input'); if (!g) return; g.value = 90; g.dispatchEvent(new Event('input')); g.dispatchEvent(new Event('change')); })()`);
  await say("Ein Ziel fürs Auto: hems sagt sofort, ob es machbar ist.");
  await js(`document.querySelectorAll('[data-goal]')[1]?.scrollIntoView({ block: 'center', behavior: 'smooth' })`);
  await sleep(700);
  await js(`(() => { const g = document.querySelectorAll('[data-goal] input')[1]; if (!g) return; g.value = 55; g.dispatchEvent(new Event('input')); g.dispatchEvent(new Event('change')); })()`);
  await say("Dasselbe fürs Warmwasser: 55 Grad bis 18 Uhr.");
  await js(`window.scrollTo({ top: 0, behavior: 'smooth' })`);
  await tap('.tabs a[data-v="day"]');
  await say("Die Tagesrechnung zeigt die Stromkosten. Fehlt vergleichbarer Betrieb, steht hier der erste Grund.");
  await js(`document.querySelector('.month')?.scrollIntoView({ block: 'center', behavior: 'smooth' })`);
  await sleep(700);
  await say("Im Monat sehen Sie, für wie viele Tage ein Vergleich möglich ist. Tippen Sie einen Tag für seine Rechnung an.");
  await js(`window.scrollTo({ top: 0, behavior: 'smooth' })`);
  await tap('.tabs a[data-v="setup"]');
  await say("Einrichtung: die SKI in Vierergruppen, zum Vergleich mit dem Etikett der Steuerbox.");
  await tap('.tabs a[data-v="now"]');
  await card("cargo run -p hemsd -- demo", "Probieren Sie es selbst — ohne Hardware, ohne Konto.", 3200);

  await send("Page.stopScreencast");
  await sleep(300);
  // Each frame lasts until the next one; the screencast sends a frame only
  // when the page changes, so the durations carry the pacing.
  const list = shot.map((t, i) => `file '${join(frames, String(i).padStart(5, "0") + ".jpg")}'\nduration ${Math.max(0.04, (shot[i + 1] ?? t + 1) - t).toFixed(3)}`).join("\n");
  writeFileSync(join(frames, "list.txt"), list + `\nfile '${join(frames, String(shot.length - 1).padStart(5, "0") + ".jpg")}'\n`);
  execFileSync("ffmpeg", ["-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", join(frames, "list.txt"),
    "-vf", "fps=30,scale=780:-2:flags=lanczos,format=yuv420p", "-c:v", "libx264", "-crf", "26", "-preset", "slow",
    "-tune", "animation", "-movflags", "+faststart", file], { stdio: "inherit" });
  rmSync(frames, { recursive: true, force: true });
  console.log(`  ${file}: ${shot.length} frames, ${(shot.at(-1) - shot[0]).toFixed(0)} s`);
}

done();
if (problems.length) { console.error(problems.join("\n")); process.exit(1); }
