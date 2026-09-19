// Dedicated worker that owns the WASM tracker, the loaded song, and the
// scorer. Receives raw sample blocks from the AudioWorklet over a transferred
// MessagePort and posts readings (and, while a song plays, note outcomes) to
// the main thread. Keeping this off the main thread means UI jank cannot
// delay analysis, and off the audio thread means analysis cannot glitch audio.
import init, { Tracker, Song, Judge, instrument_names } from "./pkg/pitch_wasm.js";

let tracker = null;
let song = null;
let judge = null;
let songStart = 0; // audio-clock time of song time zero
let sampleRate = 48000;

const ready = init().then(() => {
  postMessage({ type: "ready", instruments: instrument_names() });
});

function readingTime(blockEnd) {
  // A reading describes the window that ends at blockEnd; its content is
  // centred half a window earlier, and the median adds a couple of hops.
  return blockEnd;
}

onmessage = async (e) => {
  const msg = e.data;
  await ready;
  switch (msg.type) {
    case "start": {
      sampleRate = msg.sampleRate;
      tracker = new Tracker(msg.sampleRate, msg.instrument, msg.a4);
      postMessage({
        type: "started",
        frameLen: tracker.frame_len,
        hop: tracker.hop,
        nominalLatencySecs: tracker.nominal_latency_secs,
      });
      msg.port.onmessage = (ev) => {
        if (!tracker) return;
        const { t, s } = ev.data;
        const reading = tracker.push(s);
        if (!reading) return;
        const time = readingTime(t);
        const out = {
          type: "reading", time,
          voiced: reading.voiced, frequency: reading.frequency, confidence: reading.confidence,
          rms: reading.rms, midi: reading.midi, cents: reading.cents, note: reading.note,
        };
        reading.free();
        if (judge) {
          const outcomes = judge.feed(time - songStart, out.voiced ? out.midi : -1);
          if (outcomes.length) {
            out.outcomes = outcomes.map((o) => {
              const r = { index: o.index, hit: o.hit, wrongMidi: o.wrong_midi, onset: o.onset_offset_secs };
              o.free();
              return r;
            });
            out.score = { hits: judge.hits, finished: judge.finished, total: judge.total };
          }
        }
        postMessage(out);
      };
      break;
    }
    case "stop":
      if (tracker) tracker.free();
      tracker = null;
      break;

    case "load_song": {
      if (song) song.free();
      song = null;
      try {
        song = new Song(new Uint8Array(msg.bytes));
        const names = msg.trackNames || [];
        const tracks = song.tracks().map((t) => {
          const r = {
            index: t.index, name: names[t.index] || t.name, noteCount: t.note_count,
            lowestMidi: t.lowest_midi, highestMidi: t.highest_midi, program: t.program, isDrums: t.is_drums,
          };
          t.free();
          return r;
        });
        postMessage({ type: "song_loaded", name: msg.name, tracks, durationSecs: song.duration_secs, bpm: song.initial_bpm });
      } catch (err) {
        postMessage({ type: "song_error", message: String(err.message || err) });
      }
      break;
    }
    case "select_track": {
      if (!song) return;
      const events = song.events(msg.index, 0.03);
      postMessage({ type: "track_events", index: msg.index, events, beats: song.beat_times() });
      break;
    }
    case "start_song": {
      if (judge) judge.free();
      // Scoring latency: the analysed window is centred half a frame before
      // the block that produced the reading, plus a couple of hops of median.
      const latency = tracker ? (tracker.frame_len / 2 + 2 * tracker.hop) / sampleRate : 0;
      const interval = tracker ? tracker.hop / sampleRate : 256 / sampleRate;
      judge = new Judge(msg.events, latency, interval);
      songStart = msg.startTime;
      break;
    }
    case "stop_song": {
      if (!judge) return;
      const outcomes = judge.finish().map((o) => {
        const r = { index: o.index, hit: o.hit, wrongMidi: o.wrong_midi, onset: o.onset_offset_secs };
        o.free();
        return r;
      });
      postMessage({ type: "song_finished", outcomes, score: { hits: judge.hits, finished: judge.finished, total: judge.total } });
      judge.free();
      judge = null;
      break;
    }
  }
};
