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
import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, rmSync, mkdtempSync } from "node:fs";
import { join, dirname } from "node:path";
import { tmpdir } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";

const [mode = "screenshots", base = "http://127.0.0.1:8080", out] = process.argv.slice(2);
const chrome = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const port = 9333 + (process.pid % 500);
const PHONE = { width: 390, height: 844, deviceScaleFactor: 3, mobile: true };
const TABLET = { width: 1180, height: 820, deviceScaleFactor: 2, mobile: true };

// ── Chrome over DevTools ────────────────────────────────────────────────────
const profile = mkdtempSync(join(tmpdir(), "hems-media-"));
const proc = spawn(chrome, ["--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run",
  "--no-default-browser-check", `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "about:blank"], { stdio: "ignore" });
let target;
for (let i = 0; i < 50 && !target; i++) {
  try { target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find((t) => t.type === "page"); } catch { await sleep(200); }
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
  // A wall display has no one to scroll it.
  if (view === "glance" && await js(`document.documentElement.scrollHeight - document.documentElement.clientHeight`) > 0) problems.push(`glance (${lang}): scrolls`);
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
  await say("Am Ende des Tages ein Kassenbon: was der Tag gekostet hat — und was dasselbe Haus ohne hems gekostet hätte.");
  await js(`document.querySelector('.month')?.scrollIntoView({ block: 'center', behavior: 'smooth' })`);
  await sleep(700);
  await say("Darunter der Monat: ein Balken je Tag, und eine Summe, die auf den Cent den Kassenbons entspricht.");
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
