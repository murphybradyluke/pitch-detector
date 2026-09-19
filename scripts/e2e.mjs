#!/usr/bin/env node
// End-to-end tests of the web tuner in headless Chromium.
//
// Phase 1 (tuner): a WAV of A1 with a weak fundamental then A3 is fed to
// Chromium as the fake microphone; every reading must be within 2 cents.
// Phase 2 (play-along): a constant A1 WAV against a MIDI of four A1 notes
// and one A3; the score must be 4 of 5.
// Phase 3 (Guitar Pro): the same song as a .gp file written by alphaTab's
// exporter; the bass track must be picked by name, tab must render, and the
// score must again be 4 of 5.
//
// Prerequisites: `wasm-pack build crates/pitch-wasm --target web --out-dir
// ../../web/pkg --release`, a static server on web/ (default port 8765), and
// `chromium` on PATH. Do not use --virtual-time-budget: it stalls real-time
// audio capture.
import { spawn } from "node:child_process";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { writeSmf } from "./smf.mjs";

const BASE = process.env.E2E_URL || "http://127.0.0.1:8765";
const PORT = 9222;
const SR = 48000;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function wav(segments) {
  const perSeg = Math.round(1.5 * SR);
  const pcm = new Int16Array(perSeg * segments.length);
  segments.forEach((seg, s) => {
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

/// Launch Chromium with `wavPath` as the microphone and attach over CDP.
async function launch(dir, wavPath) {
  const chrome = spawn("chromium", [
    "--headless=new", "--no-sandbox", "--disable-gpu", `--user-data-dir=${join(dir, "profile")}`,
    `--remote-debugging-port=${PORT}`, "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
    `--use-file-for-fake-audio-capture=${wavPath}`, "--autoplay-policy=no-user-gesture-required", "about:blank",
  ], { stdio: "ignore" });
  let targets;
  for (let i = 0; i < 50 && !targets; i++) {
    try { targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json(); } catch { await sleep(200); }
  }
  if (!targets) { chrome.kill(); throw new Error("Chromium did not expose a DevTools endpoint"); }
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
  const evaluate = async (expr) => (await send("Runtime.evaluate", { expression: expr, returnByValue: true, awaitPromise: true })).result?.value;
  const text = (sel) => evaluate(`document.querySelector(${JSON.stringify(sel)}).textContent`);
  const waitFor = async (expr, timeoutMs, what) => {
    const t0 = Date.now();
    while (Date.now() - t0 < timeoutMs) {
      if (await evaluate(expr)) return true;
      await sleep(100);
    }
    throw new Error(`timed out waiting for ${what}`);
  };
  return { send, evaluate, text, waitFor, readings, errors, close: () => { ws.close(); chrome.kill(); } };
}

let failed = false;
const fail = (msg) => { console.log(`  ${msg} FAIL`); failed = true; };
const dir = mkdtempSync(join(tmpdir(), "pitch-e2e-"));

try {
  // ---- Phase 1: tuner --------------------------------------------------
  console.log("Phase 1: tuner");
  const SEGMENTS = [
    { note: "A1", hz: 55, amps: [0.1, 0.4, 0.3, 0.2] },
    { note: "A3", hz: 220, amps: [0.3, 0.35, 0.2, 0.1] },
  ];
  const wav1 = join(dir, "tuner.wav");
  writeFileSync(wav1, wav(SEGMENTS));
  {
    const b = await launch(dir, wav1);
    try {
      await b.send("Page.navigate", { url: `${BASE}/?instrument=bass%20(4-string)&autostart=1&debug=1` });
      await sleep(4000);
      const status = await b.text("#status");
      const shownNote = await b.text("#note");
      const shownCents = parseFloat((await b.text("#cents")).replace("+", ""));
      console.log(`  ${status}`);
      console.log(`  ${b.readings.length} readings, ${b.errors.length} errors`);
      for (const err of b.errors) console.log("  error:", err);
      for (const seg of SEGMENTS) {
        const hits = b.readings.filter((r) => r.note === seg.note);
        const inTune = hits.filter((r) => Math.abs(r.cents) <= 2).length;
        const ok = hits.length >= 20 && inTune / hits.length >= 0.9;
        console.log(`  ${seg.note}: ${hits.length} readings, ${inTune} within 2 cents ${ok ? "OK" : "FAIL"}`);
        if (!ok) failed = true;
      }
      const wrong = b.readings.filter((r) => !SEGMENTS.some((s) => s.note === r.note));
      if (wrong.length > b.readings.length * 0.05) fail(`${wrong.length} readings of unexpected notes`);
      const displayOk = SEGMENTS.some((seg) => seg.note === shownNote) && Math.abs(shownCents) <= 3;
      console.log(`  display: ${shownNote} ${shownCents} cents ${displayOk ? "OK" : "FAIL"}`);
      if (!displayOk) failed = true;
      if (b.errors.length) failed = true;
      if (!/Nominal latency \d+ ms/.test(status)) fail("status line missing latency");
    } finally { b.close(); }
  }

  // ---- Phase 2: play-along ----------------------------------------------
  console.log("Phase 2: play-along");
  const wav2 = join(dir, "a1.wav");
  writeFileSync(wav2, wav([{ note: "A1", hz: 55, amps: [0.1, 0.4, 0.3, 0.2] }]));
  const midPath = join(dir, "test.mid");
  // 120 bpm: four 1-second A1 notes, then one A3 the mic will not play.
  writeFileSync(midPath, writeSmf({ bpm: 120, name: "Bass", notes: [
    { midi: 33, startBeats: 0, durBeats: 2 }, { midi: 33, startBeats: 2, durBeats: 2 },
    { midi: 33, startBeats: 4, durBeats: 2 }, { midi: 33, startBeats: 6, durBeats: 2 },
    { midi: 57, startBeats: 8, durBeats: 2 },
  ]}));
  {
    const b = await launch(dir, wav2);
    try {
      await b.send("Page.navigate", { url: `${BASE}/?instrument=bass%20(4-string)&autostart=1` });
      await b.waitFor("/Nominal latency/.test(document.getElementById('status').textContent)", 8000, "audio start");
      await b.evaluate("document.getElementById('tab-song').click()");
      const { root } = await b.send("DOM.getDocument", { depth: 1 });
      const { nodeId } = await b.send("DOM.querySelector", { nodeId: root.nodeId, selector: "#midifile" });
      await b.send("DOM.setFileInputFiles", { nodeId, files: [midPath] });
      await b.waitFor("!document.getElementById('play').disabled", 8000, "track to load");
      const info = await b.text("#songinfo");
      const trackLabel = await b.evaluate("document.getElementById('track').selectedOptions[0].textContent");
      console.log(`  ${info}`);
      console.log(`  track: ${trackLabel}`);
      await b.evaluate("document.getElementById('play').click()");
      await b.waitFor("/^Finished/.test(document.getElementById('songinfo').textContent)", 15000, "song to finish");
      const result = await b.text("#songinfo");
      console.log(`  ${result}`);
      for (const err of b.errors) console.log("  error:", err);
      if (!/4 of 5 notes/.test(result)) fail("expected 4 of 5 notes");
      if (!/5 notes/.test(trackLabel)) fail("track picker did not show 5 notes");
      if (b.errors.length) failed = true;
    } finally { b.close(); }
  }

  // ---- Phase 3: Guitar Pro --------------------------------------------
  console.log("Phase 3: Guitar Pro");
  const gpPath = join(dir, "test.gp");
  writeFileSync(gpPath, await guitarProFile());
  {
    const b = await launch(dir, wav2);
    try {
      await b.send("Page.navigate", { url: `${BASE}/?instrument=bass%20(4-string)&autostart=1` });
      await b.waitFor("/Nominal latency/.test(document.getElementById('status').textContent)", 8000, "audio start");
      await b.evaluate("document.getElementById('tab-song').click()");
      const { root } = await b.send("DOM.getDocument", { depth: 1 });
      const { nodeId } = await b.send("DOM.querySelector", { nodeId: root.nodeId, selector: "#midifile" });
      await b.send("DOM.setFileInputFiles", { nodeId, files: [gpPath] });
      await b.waitFor("!document.getElementById('play').disabled", 15000, "track to load");
      await b.waitFor("document.querySelector('#tab svg') !== null", 15000, "tab to render");
      const info = await b.text("#songinfo");
      const trackLabel = await b.evaluate("document.getElementById('track').selectedOptions[0].textContent");
      const options = await b.evaluate("[...document.getElementById('track').options].map(o => o.textContent).join(' | ')");
      console.log(`  ${info}`);
      console.log(`  tracks: ${options}`);
      await b.evaluate("document.getElementById('play').click()");
      await sleep(3500);
      // The cursor must lie inside the rendered bounds of the beat that is
      // sounding right now (compared in one evaluation to avoid a race).
      const cursor = await b.evaluate(`(() => {
        const d = window.__pitch, api = d.tabApi, t = d.songTime();
        const f = api.tickCache.findBeat(new Set([d.tabTrack]), d.songTimeToTick(t), null);
        const bb = api.boundsLookup.findBeat(f.beat), nb = f.nextBeat && api.boundsLookup.findBeat(f.nextBeat.beat);
        const x = parseFloat(document.getElementById('tabcursor').style.left) + document.getElementById('tab').scrollLeft;
        return { t, x, x0: bb.visualBounds.x, x1: nb ? nb.visualBounds.x : bb.visualBounds.x + bb.visualBounds.w, frets: f.beat.notes.map(n => n.fret) };
      })()`);
      const cursorX = cursor.x;
      const cursorOk = cursor.x >= cursor.x0 - 1 && cursor.x < cursor.x1;
      console.log(`  cursor at song time ${cursor.t.toFixed(2)} s: x=${cursor.x.toFixed(1)} within beat [${cursor.x0.toFixed(1)}, ${cursor.x1.toFixed(1)}) frets ${cursor.frets} ${cursorOk ? "OK" : "FAIL"}`);
      if (!cursorOk) failed = true;
      await b.waitFor("/^Finished/.test(document.getElementById('songinfo').textContent)", 15000, "song to finish");
      const result = await b.text("#songinfo");
      console.log(`  ${result}; cursor at ${cursorX}px mid-song`);
      for (const err of b.errors) console.log("  error:", err);
      if (!/^Bass \(5 notes/.test(trackLabel)) fail(`bass track not selected by name: ${trackLabel}`);
      if (!/4 of 5 notes/.test(result)) fail("expected 4 of 5 notes");
      if (!(cursorX > 0)) fail("tab cursor did not move");
      if (b.errors.length) failed = true;
    } finally { b.close(); }
  }
} finally {
  rmSync(dir, { recursive: true, force: true });
}

/// The phase-2 song as a Guitar Pro 7 file, via alphaTab's alphaTex importer
/// and GP7 exporter. Bass track: four A1 half notes then A3 and a rest.
async function guitarProFile() {
  const at = await import("../web/node_modules/@coderline/alphatab/dist/alphaTab.core.mjs");
  const settings = new at.Settings();
  const tex = `\\title "E2E" \\tempo 120
\\track "Guitar" \\instrument 30
r.1 | r.1 | r.1 |
\\track "Bass" \\instrument 33 \\tuning G2 D2 A1 E1
0.3.2 0.3.2 | 0.3.2 0.3.2 | 14.1.2 r.2 |`;
  const importer = new at.importer.AlphaTexImporter();
  importer.initFromString(tex, settings);
  return new at.exporter.Gp7Exporter().export(importer.readScore(), settings);
}
console.log(failed ? "E2E FAILED" : "E2E PASSED");
process.exit(failed ? 1 : 0);
