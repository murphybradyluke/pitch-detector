# pitch-detector

Real-time monophonic pitch detection for a tuner and, later, a play-along note
checker. The core is a dependency-free Rust crate so it can be compiled to WASM
for the web and reused on iOS and Android.

## Layout

```
crates/pitch-core/   pure detection library (no I/O, no platform code)
crates/pitch-wasm/   wasm-bindgen wrapper: Tracker, Reading, instrument_names()
web/                 tuner page: AudioWorklet -> Worker (WASM) -> UI
scripts/e2e.mjs      headless-Chromium end-to-end test with a WAV as the mic
  src/yin.rs         YIN estimator: &[f32] + sample rate -> { frequency, confidence }
  src/note.rs        frequency <-> MIDI note number and cents
  src/instrument.rs  range presets (guitar, 4/5-string bass, drop tunings, ...)
  src/smoothing.rs   RMS level, median filter
  src/tracker.rs     sliding window -> RMS gate -> YIN -> confidence gate -> median
  tests/             synthetic sine / harmonic / noise tests
```

## Design decisions

- **YIN, not FFT peak-picking.** The loudest spectral peak is often a harmonic,
  which gives octave errors. YIN's absolute-threshold rule picks the smallest
  lag whose normalised difference dips under the threshold, so it prefers the
  fundamental. Tests cover a loud second harmonic and a missing fundamental.
- **Pure function core.** `detect_pitch(&[f32], sample_rate)` has no state. A
  `Yin` struct pre-allocates scratch space so the audio thread never allocates.
- **Pick the instrument up front.** The window must hold ~2 periods of the
  lowest note searched for (measured: below ~1.9 periods the lowest notes
  collapse to a harmonic; a weak or missing fundamental makes no difference).
  So the lowest note sets the latency floor, and nobody needs drop-A bass and
  C7 on one instrument. `Instrument` presets size the window: ~30 ms for
  guitar, ~85 ms for a drop-A five-string. The search range extends one
  semitone past each end so a badly flat string still registers.
- **Slide the window, don't step it.** `PitchTracker` keeps a ring buffer one
  frame long and analyses every `hop` samples (256 = 5 ms at 48 kHz). The
  reading refreshes every hop, and the median filter spans a few hops instead
  of a few whole frames. Compute rises 8x but stays trivial natively.
- **Gate, then smooth.** Frames below an RMS floor or a confidence floor are
  dropped, then the rest are median-filtered over ~5 analyses. A silence as
  long as the median window resets it so a new note is not blended with the
  last.
- **MIDI and cents.** `midi = 69 + 12 * log2(f / 440)`; the fractional part times
  100 is the cents offset.
- **Latency.** Nominal onset-to-settled-reading, excluding the platform's
  capture buffer, is window + hop + half the median: ~45 ms for guitar, ~95 ms
  for drop-A bass. `PitchTracker::nominal_latency_secs()` reports it.
- **High-note limit.** At 48 kHz a 2.5 kHz note has a 19-sample period, so one
  sample is 90 cents and sub-sample interpolation no longer holds 3 cents.
  Presets stop at C7.

## Web tuner

Audio path: `getUserMedia` (echo cancellation, noise suppression and auto
gain off) -> `AudioWorklet` that forwards 128-sample blocks over a
`MessagePort` -> dedicated `Worker` running the WASM tracker -> readings to
the main thread, drawn on `requestAnimationFrame`. The worklet cannot host
the WASM itself: its global scope lacks `fetch` and `TextDecoder`.

```
wasm-pack build crates/pitch-wasm --target web --out-dir ../../web/pkg --release
(cd web && python3 -m http.server 8765)
# open http://localhost:8765/
```

Query parameters for testing: `?instrument=guitar&autostart=1&debug=1` logs
every reading to the console.

## Develop

```
cargo test                      # unit + synthetic-signal tests
cargo clippy --all-targets
cargo fmt
node scripts/e2e.mjs            # needs the wasm build, the static server, and chromium
```

Rust is managed with [mise](https://mise.jdx.dev) on the dev machine
(`mise use -g rust@stable`).
