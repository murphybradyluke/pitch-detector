const $ = (id) => document.getElementById(id);
const ui = {
  instrument: $("instrument"), a4: $("a4"), start: $("start"), note: $("note"),
  needle: document.querySelector("#meter .needle"), cents: $("cents"), freq: $("freq"),
  conf: $("conf"), level: document.querySelector("#level div"), status: $("status"),
  tabTuner: $("tab-tuner"), tabSong: $("tab-song"), tuner: $("tuner"), song: $("song"),
  midifile: $("midifile"), track: $("track"), hear: $("hear"), click: $("click"), play: $("play"),
  songinfo: $("songinfo"), countin: $("countin"), target: $("target"), roll: $("roll"), scoreline: $("scoreline"),
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

// ---------------------------------------------------------------------------
// Tuner display: hold and smoothing
// ---------------------------------------------------------------------------

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
const noteName = (midi) => NOTE_NAMES[((midi % 12) + 12) % 12] + (Math.floor(midi / 12) - 1);
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
    note: noteName(shownMidi),
    cents: (smoothedMidi - shownMidi) * 100,
    frequency: a4 * Math.pow(2, (smoothedMidi - 69) / 12),
    confidence: msg.confidence,
    rms: msg.rms,
  };
}

// ---------------------------------------------------------------------------
// Worker messages
// ---------------------------------------------------------------------------

worker.onmessage = (e) => {
  const msg = e.data;
  switch (msg.type) {
    case "ready":
      for (const name of msg.instruments) {
        const opt = document.createElement("option");
        opt.value = opt.textContent = name;
        ui.instrument.appendChild(opt);
      }
      ui.instrument.value = params.get("instrument") || localStorage.getItem("instrument") || msg.instruments[0];
      ui.start.disabled = false;
      if (params.has("autostart")) start();
      break;
    case "started": {
      const capture = audio.baseLatency ? ` + ${(audio.baseLatency * 1000).toFixed(0)} ms capture` : "";
      ui.status.textContent =
        `${audio.sampleRate} Hz, window ${msg.frameLen} samples, hop ${msg.hop}. ` +
        `Nominal latency ${(msg.nominalLatencySecs * 1000).toFixed(0)} ms${capture}.`;
      if (debug) console.log(`[started] ${ui.status.textContent}`);
      break;
    }
    case "reading":
      latest = msg;
      if (msg.voiced) {
        smooth(msg);
        lastVoicedAt = performance.now();
        if (playing) trail.push({ t: msg.time - playing.startTime, midi: msg.midi + msg.cents / 100 });
      }
      if (msg.outcomes) applyOutcomes(msg.outcomes, msg.score);
      if (debug && msg.voiced) {
        console.log(`[reading] ${msg.note} ${msg.frequency.toFixed(2)} Hz ${msg.cents.toFixed(1)} cents conf ${msg.confidence.toFixed(2)}`);
      }
      break;
    case "song_loaded":
      onSongLoaded(msg);
      break;
    case "song_error":
      ui.songinfo.textContent = `Could not read MIDI file: ${msg.message}`;
      break;
    case "track_events":
      onTrackEvents(msg);
      break;
    case "song_finished":
      applyOutcomes(msg.outcomes, msg.score);
      showFinalScore(msg.score);
      break;
  }
};

// ---------------------------------------------------------------------------
// Audio start/stop
// ---------------------------------------------------------------------------

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
  updatePlayButton();
  requestAnimationFrame(draw);
}

function stop() {
  if (playing) stopSong();
  running = false;
  worker.postMessage({ type: "stop" });
  if (stream) stream.getTracks().forEach((t) => t.stop());
  if (audio) audio.close();
  stream = audio = latest = display = smoothedMidi = shownMidi = null;
  ui.start.textContent = "Start";
  ui.start.classList.remove("on");
  ui.status.textContent = "Stopped.";
  updatePlayButton();
  render(null, false);
}

// ---------------------------------------------------------------------------
// Tabs
// ---------------------------------------------------------------------------

function showTab(name) {
  ui.tuner.classList.toggle("active", name === "tuner");
  ui.song.classList.toggle("active", name === "song");
  ui.tabTuner.classList.toggle("active", name === "tuner");
  ui.tabSong.classList.toggle("active", name === "song");
  localStorage.setItem("tab", name);
  if (name === "song") sizeRoll();
}
ui.tabTuner.onclick = () => showTab("tuner");
ui.tabSong.onclick = () => showTab("song");
showTab(localStorage.getItem("tab") || "tuner");

// ---------------------------------------------------------------------------
// Play-along: song loading
// ---------------------------------------------------------------------------

let songMeta = null; // { name, tracks, durationSecs, bpm }
let track = null; // { index, events: [{midi,start,dur,state}], beats, low, high }
let playing = null; // { startTime, nextBeat, nextNote, countInBeats }
let trail = []; // recent detected pitches, song-relative time
let score = { hits: 0, finished: 0, total: 0 };

ui.midifile.onchange = async () => {
  const file = ui.midifile.files[0];
  if (!file) return;
  const bytes = await file.arrayBuffer();
  ui.songinfo.textContent = `Reading ${file.name}…`;
  worker.postMessage({ type: "load_song", name: file.name, bytes }, [bytes]);
};

function onSongLoaded(msg) {
  songMeta = msg;
  track = null;
  ui.track.innerHTML = "";
  const usable = msg.tracks.filter((t) => t.noteCount > 0 && !t.isDrums);
  if (usable.length === 0) {
    ui.songinfo.textContent = `${msg.name}: no melodic tracks found.`;
    ui.track.disabled = true;
    updatePlayButton();
    return;
  }
  for (const t of usable) {
    const opt = document.createElement("option");
    opt.value = t.index;
    opt.textContent = `${t.name} (${t.noteCount} notes, ${noteName(t.lowestMidi)}–${noteName(t.highestMidi)})`;
    ui.track.appendChild(opt);
  }
  ui.track.disabled = false;
  // Default to the track that looks most like a bass part: GM programs
  // 32-39 are basses; failing that, the lowest-pitched melodic track.
  const bass = usable.find((t) => t.program >= 32 && t.program <= 39) || usable.find((t) => /bass/i.test(t.name));
  const lowest = usable.reduce((a, b) => (b.lowestMidi < a.lowestMidi ? b : a));
  ui.track.value = (bass || lowest).index;
  ui.songinfo.textContent = `${msg.name}: ${msg.tracks.length} tracks, ${msg.bpm.toFixed(0)} bpm, ${formatTime(msg.durationSecs)}.`;
  selectTrack();
}

ui.track.onchange = selectTrack;
function selectTrack() {
  worker.postMessage({ type: "select_track", index: Number(ui.track.value) });
}

function onTrackEvents(msg) {
  const events = [];
  for (let i = 0; i < msg.events.length; i += 3) {
    events.push({ midi: msg.events[i], start: msg.events[i + 1], dur: msg.events[i + 2], state: "pending", wrong: -1 });
  }
  const midis = events.map((e) => e.midi);
  track = {
    index: msg.index, events, beats: msg.beats,
    low: Math.min(...midis, 127) - 2, high: Math.max(...midis, 0) + 2,
  };
  score = { hits: 0, finished: 0, total: events.length };
  ui.scoreline.textContent = "";
  updatePlayButton();
  drawRoll(0);
}

function updatePlayButton() {
  ui.play.disabled = !(running && track);
  ui.play.textContent = playing ? "Stop" : "Play";
  ui.play.classList.toggle("on", !!playing);
  ui.play.classList.toggle("primary", !playing);
}

// ---------------------------------------------------------------------------
// Play-along: playback, click, scheduling
// ---------------------------------------------------------------------------

const COUNT_IN_BEATS = 4;
const SCHEDULE_AHEAD = 0.5; // seconds of clicks/notes scheduled per frame

ui.play.onclick = () => (playing ? stopSong() : playSong());

function playSong() {
  if (!running || !track) return;
  for (const e of track.events) { e.state = "pending"; e.wrong = -1; }
  score = { hits: 0, finished: 0, total: track.events.length };
  trail = [];
  const beat = 60 / songMeta.bpm;
  const startTime = audio.currentTime + 0.2 + COUNT_IN_BEATS * beat;
  playing = { startTime, beat, nextBeat: 0, nextNote: 0, countInScheduled: 0 };
  worker.postMessage({ type: "start_song", events: flatEvents(), startTime });
  ui.scoreline.textContent = "";
  updatePlayButton();
}

function stopSong() {
  if (!playing) return;
  playing = null;
  worker.postMessage({ type: "stop_song" });
  ui.countin.textContent = "";
  ui.target.textContent = "";
  updatePlayButton();
}

function flatEvents() {
  const out = new Float32Array(track.events.length * 3);
  track.events.forEach((e, i) => { out[i * 3] = e.midi; out[i * 3 + 1] = e.start; out[i * 3 + 2] = e.dur; });
  return out;
}

function click(at, accent) {
  if (!ui.click.checked) return;
  const osc = audio.createOscillator();
  const gain = audio.createGain();
  osc.frequency.value = accent ? 1600 : 1100;
  gain.gain.setValueAtTime(0.4, at);
  gain.gain.exponentialRampToValueAtTime(0.001, at + 0.04);
  osc.connect(gain).connect(audio.destination);
  osc.start(at);
  osc.stop(at + 0.05);
}

function playNote(midi, at, dur) {
  if (!ui.hear.checked) return;
  const a4 = Number(ui.a4.value) || 440;
  const osc = audio.createOscillator();
  const gain = audio.createGain();
  osc.type = "triangle";
  osc.frequency.value = a4 * Math.pow(2, (midi - 69) / 12);
  const end = at + Math.max(0.05, dur - 0.02);
  gain.gain.setValueAtTime(0.0001, at);
  gain.gain.exponentialRampToValueAtTime(0.25, at + 0.01);
  gain.gain.setValueAtTime(0.25, Math.max(at + 0.01, end - 0.03));
  gain.gain.exponentialRampToValueAtTime(0.0001, end);
  osc.connect(gain).connect(audio.destination);
  osc.start(at);
  osc.stop(end + 0.01);
}

function schedule() {
  const p = playing;
  const horizon = audio.currentTime + SCHEDULE_AHEAD;
  // Count-in clicks before the song start.
  while (p.countInScheduled < COUNT_IN_BEATS) {
    const at = p.startTime - (COUNT_IN_BEATS - p.countInScheduled) * p.beat;
    if (at > horizon) break;
    click(at, p.countInScheduled === 0);
    p.countInScheduled++;
  }
  while (p.nextBeat < track.beats.length && p.startTime + track.beats[p.nextBeat] <= horizon) {
    click(p.startTime + track.beats[p.nextBeat], p.nextBeat % 4 === 0);
    p.nextBeat++;
  }
  while (p.nextNote < track.events.length && p.startTime + track.events[p.nextNote].start <= horizon) {
    const e = track.events[p.nextNote];
    playNote(e.midi, p.startTime + e.start, e.dur);
    p.nextNote++;
  }
}

function applyOutcomes(outcomes, s) {
  if (!track) return;
  for (const o of outcomes) {
    const e = track.events[o.index];
    if (!e) continue;
    e.state = o.hit ? "hit" : "miss";
    e.wrong = o.wrongMidi;
  }
  if (s) score = s;
  ui.scoreline.innerHTML =
    `<span class="hit">${score.hits}</span> / ${score.finished} of ${score.total}` +
    (score.finished ? ` · ${Math.round((100 * score.hits) / score.finished)}%` : "");
}

function showFinalScore(s) {
  const pct = s.total ? Math.round((100 * s.hits) / s.total) : 0;
  ui.songinfo.textContent = `Finished: ${s.hits} of ${s.total} notes (${pct}%).`;
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

function draw() {
  if (!running) return;
  const age = performance.now() - lastVoicedAt;
  if (display && age < HOLD_MS) {
    render({ ...display, rms: latest ? latest.rms : 0 }, age > FRESH_MS);
  } else {
    render(latest ? { ...latest, voiced: false } : null, false);
  }
  if (playing) {
    schedule();
    const t = audio.currentTime - playing.startTime;
    if (t < 0) {
      ui.countin.textContent = String(Math.ceil(-t / playing.beat));
    } else {
      ui.countin.textContent = "";
      showTarget(t);
      if (t > songMeta.durationSecs + 0.5) stopSong();
    }
    drawRoll(t);
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

function showTarget(t) {
  const ev = track.events;
  let cur = null, next = null;
  for (const e of ev) {
    if (t >= e.start && t < e.start + e.dur) cur = e;
    else if (e.start > t) { next = e; break; }
  }
  ui.target.innerHTML = (cur ? noteName(cur.midi) : "·") + (next ? `<span class="next">then ${noteName(next.midi)}</span>` : "");
}

const PX_PER_SEC = 160;
const PLAYHEAD_FRAC = 0.3;
const TRAIL_SECS = 6;

function sizeRoll() {
  const dpr = window.devicePixelRatio || 1;
  const rect = ui.roll.getBoundingClientRect();
  ui.roll.width = Math.round(rect.width * dpr);
  ui.roll.height = Math.round(rect.height * dpr);
}
window.addEventListener("resize", sizeRoll);

function drawRoll(t) {
  if (!track) return;
  const c = ui.roll.getContext("2d");
  const dpr = window.devicePixelRatio || 1;
  const W = ui.roll.width / dpr, H = ui.roll.height / dpr;
  c.setTransform(dpr, 0, 0, dpr, 0, 0);
  c.clearRect(0, 0, W, H);
  const x0 = W * PLAYHEAD_FRAC;
  const x = (time) => x0 + (time - t) * PX_PER_SEC;
  const span = track.high - track.low || 1;
  const rowH = H / span;
  const y = (midi) => H - (midi - track.low) * rowH;

  // Beat lines.
  c.strokeStyle = "#2a2a2a";
  c.lineWidth = 1;
  for (let i = 0; i < track.beats.length; i++) {
    const bx = x(track.beats[i]);
    if (bx < 0) continue;
    if (bx > W) break;
    c.beginPath(); c.moveTo(bx, 0); c.lineTo(bx, H); c.stroke();
  }

  // Target notes.
  for (const e of track.events) {
    const left = x(e.start), right = x(e.start + e.dur);
    if (right < 0) continue;
    if (left > W) break;
    const active = t >= e.start && t < e.start + e.dur;
    c.fillStyle = e.state === "hit" ? "#4caf50" : e.state === "miss" ? "#e53935" : active ? "#42a5f5" : "#555";
    c.fillRect(left, y(e.midi) - rowH * 0.8, Math.max(2, right - left - 2), rowH * 0.8);
    if (right - left > 28) {
      c.fillStyle = "#000";
      c.font = `${Math.min(12, rowH)}px system-ui`;
      c.fillText(noteName(e.midi), left + 3, y(e.midi) - rowH * 0.15);
    }
  }

  // What was played: dots at the detected pitch.
  while (trail.length && trail[0].t < t - TRAIL_SECS) trail.shift();
  c.fillStyle = "rgba(255,255,255,0.85)";
  for (const p of trail) {
    const px = x(p.t);
    if (px < 0 || px > W) continue;
    c.fillRect(px - 1, y(p.midi) - rowH * 0.4 - 1.5, 3, 3);
  }

  // Playhead.
  c.strokeStyle = "#fff";
  c.lineWidth = 2;
  c.beginPath(); c.moveTo(x0, 0); c.lineTo(x0, H); c.stroke();
}

function formatTime(secs) {
  const m = Math.floor(secs / 60), s = Math.round(secs % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
}

sizeRoll();
