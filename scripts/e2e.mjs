#!/usr/bin/env node
// End-to-end test of the web tuner in headless Chromium.
//
// Generates a WAV (1.5 s of A1 with a weak fundamental, then 1.5 s of A3),
// feeds it to Chromium as the fake microphone, drives the page over the
// DevTools protocol, and checks that both notes were reported in tune.
//
// Prerequisites: `wasm-pack build crates/pitch-wasm --target web --out-dir
// ../../web/pkg --release`, a static server on web/ (default port 8765), and
// `chromium` on PATH. Do not use --virtual-time-budget: it stalls real-time
// audio capture.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BASE = process.env.E2E_URL || "http://127.0.0.1:8765";
const PORT = 9222;
const SR = 48000;
const SEGMENTS = [
  { note: "A1", hz: 55, amps: [0.1, 0.4, 0.3, 0.2] },
  { note: "A3", hz: 220, amps: [0.3, 0.35, 0.2, 0.1] },
];

function wav() {
  const perSeg = Math.round(1.5 * SR);
  const pcm = new Int16Array(perSeg * SEGMENTS.length);
  SEGMENTS.forEach((seg, s) => {
    for (let i = 0; i < perSeg; i++) {
      const t = i / SR;
      let v = 0;
      seg.amps.forEach((a, k) => (v += a * Math.sin(2 * Math.PI * seg.hz * (k + 1) * t)));
      pcm[s * perSeg + i] = Math.max(-1, Math.min(1, v)) * 32767;
    }
  });
  const data = Buffer.from(pcm.buffer);
  const h = Buffer.alloc(44);
  h.write("RIFF", 0); h.writeUInt32LE(36 + data.length, 4); h.write("WAVE", 8);
  h.write("fmt ", 12); h.writeUInt32LE(16, 16); h.writeUInt16LE(1, 20); h.writeUInt16LE(1, 22);
  h.writeUInt32LE(SR, 24); h.writeUInt32LE(SR * 2, 28); h.writeUInt16LE(2, 32); h.writeUInt16LE(16, 34);
  h.write("data", 36); h.writeUInt32LE(data.length, 40);
  return Buffer.concat([h, data]);
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const dir = mkdtempSync(join(tmpdir(), "pitch-e2e-"));
const wavPath = join(dir, "test.wav");
writeFileSync(wavPath, wav());

const chrome = spawn("chromium", [
  "--headless=new", "--no-sandbox", "--disable-gpu", `--user-data-dir=${join(dir, "profile")}`,
  `--remote-debugging-port=${PORT}`, "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
  `--use-file-for-fake-audio-capture=${wavPath}`, "--autoplay-policy=no-user-gesture-required", "about:blank",
], { stdio: "ignore" });

let failed = false;
try {
  let targets;
  for (let i = 0; i < 50 && !targets; i++) {
    try { targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json(); } catch { await sleep(200); }
  }
  if (!targets) throw new Error("Chromium did not expose a DevTools endpoint");
  const page = targets.find((t) => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let id = 0; const pending = new Map();
  const send = (method, params = {}) => new Promise((res) => { pending.set(++id, res); ws.send(JSON.stringify({ id, method, params })); });
  const readings = []; const errors = [];
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.id) return pending.get(m.id)?.(m.result);
    if (m.method === "Runtime.consoleAPICalled") {
      const text = m.params.args.map((a) => a.value ?? a.description).join(" ");
      const r = /^\[reading\] (\S+) ([\d.]+) Hz (-?[\d.]+) cents/.exec(text);
      if (r) readings.push({ note: r[1], hz: +r[2], cents: +r[3] });
      else if (m.params.type === "error") errors.push(text);
    }
    if (m.method === "Runtime.exceptionThrown") errors.push(m.params.exceptionDetails.exception?.description || m.params.exceptionDetails.text);
  };
  await send("Runtime.enable");
  await send("Page.navigate", { url: `${BASE}/?instrument=bass%20(4-string)&autostart=1&debug=1` });
  await sleep(4000);
  const text = async (id) => (await send("Runtime.evaluate", { expression: `document.getElementById('${id}').textContent`, returnByValue: true })).result.value;
  const status = await text("status");
  const shownNote = await text("note");
  const shownCents = parseFloat((await text("cents")).replace("+", ""));
  ws.close();

  console.log(status);
  console.log(`${readings.length} readings, ${errors.length} errors`);
  for (const err of errors) console.log("  error:", err);
  for (const seg of SEGMENTS) {
    const hits = readings.filter((r) => r.note === seg.note);
    const inTune = hits.filter((r) => Math.abs(r.cents) <= 2).length;
    const ok = hits.length >= 20 && inTune / hits.length >= 0.9;
    console.log(`  ${seg.note}: ${hits.length} readings, ${inTune} within 2 cents ${ok ? "OK" : "FAIL"}`);
    if (!ok) failed = true;
  }
  const wrong = readings.filter((r) => !SEGMENTS.some((s) => s.note === r.note));
  if (wrong.length > readings.length * 0.05) { console.log(`  ${wrong.length} readings of unexpected notes FAIL`); failed = true; }
  // The smoothed display must show one of the test notes, close to in tune.
  const displayOk = SEGMENTS.some((seg) => seg.note === shownNote) && Math.abs(shownCents) <= 3;
  console.log(`  display: ${shownNote} ${shownCents} cents ${displayOk ? "OK" : "FAIL"}`);
  if (!displayOk) failed = true;
  if (errors.length) failed = true;
  if (!/Nominal latency \d+ ms/.test(status)) { console.log("  status line missing latency FAIL"); failed = true; }
} finally {
  chrome.kill();
  rmSync(dir, { recursive: true, force: true });
}
console.log(failed ? "E2E FAILED" : "E2E PASSED");
process.exit(failed ? 1 : 0);
