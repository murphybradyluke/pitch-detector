// Dedicated worker that owns the WASM tracker. Receives raw sample blocks from
// the AudioWorklet over a transferred MessagePort and posts readings to the
// main thread. Keeping this off the main thread means UI jank cannot delay
// analysis, and off the audio thread means a slow analysis cannot glitch audio.
import init, { Tracker, instrument_names } from "./pkg/pitch_wasm.js";

let tracker = null;

const ready = init().then(() => {
  postMessage({ type: "ready", instruments: instrument_names() });
});

onmessage = async (e) => {
  const msg = e.data;
  await ready;
  if (msg.type === "start") {
    tracker = new Tracker(msg.sampleRate, msg.instrument, msg.a4);
    postMessage({
      type: "started",
      frameLen: tracker.frame_len,
      hop: tracker.hop,
      nominalLatencySecs: tracker.nominal_latency_secs,
    });
    msg.port.onmessage = (ev) => {
      if (!tracker) return;
      const reading = tracker.push(ev.data);
      if (reading) {
        postMessage({
          type: "reading",
          voiced: reading.voiced,
          frequency: reading.frequency,
          confidence: reading.confidence,
          rms: reading.rms,
          midi: reading.midi,
          cents: reading.cents,
          note: reading.note,
        });
        reading.free();
      }
    };
  } else if (msg.type === "stop") {
    if (tracker) tracker.free();
    tracker = null;
  }
};
