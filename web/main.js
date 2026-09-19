const $ = (id) => document.getElementById(id);
const ui = {
  instrument: $("instrument"), a4: $("a4"), start: $("start"), note: $("note"),
  needle: document.querySelector("#meter .needle"), cents: $("cents"), freq: $("freq"),
  conf: $("conf"), level: document.querySelector("#level div"), status: $("status"),
};

// ?instrument=<name>&autostart=1&debug=1 for headless end-to-end tests.
// ?smooth=<ms> overrides the display smoothing time constant.
const params = new URLSearchParams(location.search);
const debug = params.has("debug");

const worker = new Worker("./worker.js", { type: "module" });
let audio = null;
let stream = null;
let latest = null;
let running = false;

// A real instrument's confidence dips constantly (decay, a wobbling harmonic),
// and the tracker reports those analyses as unvoiced. Blanking the display on
// each one makes the reading flicker. Instead, hold the last good reading for
// HOLD_MS after it was seen, dimmed once it is older than FRESH_MS.
const HOLD_MS = 1500;
const FRESH_MS = 250;
let lastVoicedAt = 0;

// Smoothing. Raw readings arrive every ~5 ms and wobble by a few cents; an
// exponential average with a ~150 ms time constant steadies the needle
// without visible lag. It works on fractional MIDI (log frequency) so it is
// the same in cents at every pitch. A jump of more than SNAP_SEMITONES is a
// new note and resets the average instead of gliding to it. The displayed
// note name has hysteresis: it changes only once the smoothed pitch is more
// than NOTE_HYSTERESIS_CENTS past the boundary, so playing near a boundary
// does not flip the name back and forth.
const SMOOTH_MS = Number(params.get("smooth")) || 150;
const SNAP_SEMITONES = 0.6;
const NOTE_HYSTERESIS_CENTS = 60;
const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
let smoothedMidi = null; // fractional MIDI after smoothing
let shownMidi = null; // integer MIDI currently named on screen
let smoothedAt = 0;
let display = null; // what render() draws: { note, cents, frequency, confidence, rms }

function smooth(msg) {
  const now = performance.now();
  const exact = msg.midi + msg.cents / 100;
  if (smoothedMidi === null || Math.abs(exact - smoothedMidi) > SNAP_SEMITONES) {
    smoothedMidi = exact;
    shownMidi = Math.round(exact);
  } else {
    const dt = now - smoothedAt;
    const alpha = 1 - Math.exp(-dt / SMOOTH_MS);
    smoothedMidi += alpha * (exact - smoothedMidi);
    if (Math.abs(smoothedMidi - shownMidi) * 100 > NOTE_HYSTERESIS_CENTS) {
      shownMidi = Math.round(smoothedMidi);
    }
  }
  smoothedAt = now;
  const a4 = Number(ui.a4.value) || 440;
  display = {
    note: NOTE_NAMES[((shownMidi % 12) + 12) % 12] + (Math.floor(shownMidi / 12) - 1),
    cents: (smoothedMidi - shownMidi) * 100,
    frequency: a4 * Math.pow(2, (smoothedMidi - 69) / 12),
    confidence: msg.confidence,
    rms: msg.rms,
  };
}

worker.onmessage = (e) => {
  const msg = e.data;
  if (msg.type === "ready") {
    for (const name of msg.instruments) {
      const opt = document.createElement("option");
      opt.value = opt.textContent = name;
      ui.instrument.appendChild(opt);
    }
    ui.instrument.value = params.get("instrument") || localStorage.getItem("instrument") || msg.instruments[0];
    ui.start.disabled = false;
    if (params.has("autostart")) start();
  } else if (msg.type === "started") {
    const capture = audio.baseLatency ? ` + ${(audio.baseLatency * 1000).toFixed(0)} ms capture` : "";
    ui.status.textContent =
      `${audio.sampleRate} Hz, window ${msg.frameLen} samples, hop ${msg.hop}. ` +
      `Nominal latency ${(msg.nominalLatencySecs * 1000).toFixed(0)} ms${capture}.`;
    if (debug) console.log(`[started] ${ui.status.textContent}`);
  } else if (msg.type === "reading") {
    latest = msg;
    if (msg.voiced) {
      smooth(msg);
      lastVoicedAt = performance.now();
    }
    if (debug && msg.voiced) {
      console.log(`[reading] ${msg.note} ${msg.frequency.toFixed(2)} Hz ${msg.cents.toFixed(1)} cents conf ${msg.confidence.toFixed(2)}`);
    }
  }
};

ui.start.disabled = true;
ui.start.onclick = () => (running ? stop() : start());
ui.instrument.onchange = () => {
  localStorage.setItem("instrument", ui.instrument.value);
  if (running) { stop(); start(); }
};

async function start() {
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        // These three add processing delay and mangle harmonics; a tuner wants raw input.
        echoCancellation: false, noiseSuppression: false, autoGainControl: false,
        channelCount: 1,
      },
    });
  } catch (err) {
    ui.status.textContent = `Microphone unavailable: ${err.message}`;
    return;
  }
  audio = new AudioContext({ latencyHint: "interactive" });
  await audio.audioWorklet.addModule("./capture-processor.js");
  const source = audio.createMediaStreamSource(stream);
  const node = new AudioWorkletNode(audio, "capture-processor", { numberOfOutputs: 0 });
  source.connect(node);

  // Worklet -> worker channel, bypassing the main thread entirely.
  const channel = new MessageChannel();
  node.port.postMessage({ port: channel.port1 }, [channel.port1]);
  worker.postMessage(
    { type: "start", sampleRate: audio.sampleRate, instrument: ui.instrument.value, a4: Number(ui.a4.value), port: channel.port2 },
    [channel.port2],
  );

  running = true;
  ui.start.textContent = "Stop";
  ui.start.classList.add("on");
  requestAnimationFrame(draw);
}

function stop() {
  running = false;
  worker.postMessage({ type: "stop" });
  if (stream) stream.getTracks().forEach((t) => t.stop());
  if (audio) audio.close();
  stream = audio = latest = display = smoothedMidi = shownMidi = null;
  ui.start.textContent = "Start";
  ui.start.classList.remove("on");
  ui.status.textContent = "Stopped.";
  render(null);
}

function draw() {
  if (!running) return;
  const age = performance.now() - lastVoicedAt;
  if (display && age < HOLD_MS) {
    render({ ...display, rms: latest ? latest.rms : 0 }, age > FRESH_MS);
  } else {
    render(latest ? { ...latest, voiced: false } : null, false);
  }
  requestAnimationFrame(draw);
}

function render(r, held) {
  if (!r || r.voiced === false) {
    ui.note.textContent = "–";
    ui.note.className = "";
    ui.needle.style.left = "50%";
    ui.cents.textContent = "0 ¢";
    ui.freq.textContent = r ? `${r.frequency.toFixed(1)} Hz` : "0.0 Hz";
    ui.conf.textContent = "";
    ui.level.style.width = r ? `${Math.min(100, r.rms * 300)}%` : "0";
    return;
  }
  ui.note.textContent = r.note;
  ui.note.className = (Math.abs(r.cents) <= 5 ? "ok" : r.cents < 0 ? "flat" : "sharp") + (held ? " held" : "");
  // Meter spans -50..+50 cents across the width.
  ui.needle.style.left = `${50 + Math.max(-50, Math.min(50, r.cents))}%`;
  ui.cents.textContent = `${r.cents >= 0 ? "+" : ""}${r.cents.toFixed(0)} ¢`;
  ui.freq.textContent = `${r.frequency.toFixed(1)} Hz`;
  ui.conf.textContent = `conf ${r.confidence.toFixed(2)}`;
  ui.level.style.width = `${Math.min(100, r.rms * 300)}%`;
}
