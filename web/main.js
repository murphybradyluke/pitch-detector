const $ = (id) => document.getElementById(id);
const ui = {
  instrument: $("instrument"), a4: $("a4"), start: $("start"), note: $("note"),
  needle: document.querySelector("#meter .needle"), cents: $("cents"), freq: $("freq"),
  conf: $("conf"), level: document.querySelector("#level div"), status: $("status"),
};

const worker = new Worker("./worker.js", { type: "module" });
let audio = null;
let stream = null;
let latest = null;
let running = false;

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
    if (debug && msg.voiced) {
      console.log(`[reading] ${msg.note} ${msg.frequency.toFixed(2)} Hz ${msg.cents.toFixed(1)} cents conf ${msg.confidence.toFixed(2)}`);
    }
  }
};

// ?instrument=<name>&autostart=1&debug=1 for headless end-to-end tests.
const params = new URLSearchParams(location.search);
const debug = params.has("debug");

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
  stream = audio = latest = null;
  ui.start.textContent = "Start";
  ui.start.classList.remove("on");
  ui.status.textContent = "Stopped.";
  render(null);
}

function draw() {
  if (!running) return;
  render(latest);
  requestAnimationFrame(draw);
}

function render(r) {
  if (!r || !r.voiced) {
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
  ui.note.className = Math.abs(r.cents) <= 5 ? "ok" : r.cents < 0 ? "flat" : "sharp";
  // Meter spans -50..+50 cents across the width.
  ui.needle.style.left = `${50 + r.cents}%`;
  ui.cents.textContent = `${r.cents >= 0 ? "+" : ""}${r.cents.toFixed(0)} ¢`;
  ui.freq.textContent = `${r.frequency.toFixed(1)} Hz`;
  ui.conf.textContent = `conf ${r.confidence.toFixed(2)}`;
  ui.level.style.width = `${Math.min(100, r.rms * 300)}%`;
}
