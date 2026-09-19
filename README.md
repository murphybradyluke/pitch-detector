# pitch-detector

Real-time monophonic pitch detection for a tuner and, later, a play-along note
checker. The core is a dependency-free Rust crate so it can be compiled to WASM
for the web and reused on iOS and Android.

## Layout

```
crates/pitch-core/   pure detection library (no I/O, no platform code)
  src/yin.rs         YIN estimator: &[f32] + sample rate -> { frequency, confidence }
  src/note.rs        frequency <-> MIDI note number and cents
  src/smoothing.rs   RMS level, median filter
  src/tracker.rs     RMS gate -> YIN -> confidence gate -> median filter
  tests/             synthetic sine / harmonic / noise tests
```

## Design decisions

- **YIN, not FFT peak-picking.** The loudest spectral peak is often a harmonic,
  which gives octave errors. YIN's absolute-threshold rule picks the smallest
  lag whose normalised difference dips under the threshold, so it prefers the
  fundamental. Tests cover a loud second harmonic and a missing fundamental.
- **Pure function core.** `detect_pitch(&[f32], sample_rate)` has no state. A
  `Yin` struct pre-allocates scratch space so the audio thread never allocates.
- **Gate, then smooth.** `PitchTracker` drops frames below an RMS floor or a
  confidence floor, then median-filters over ~5 voiced frames. A silence as long
  as the window resets the filter so a new note is not blended with the last.
- **MIDI and cents.** `midi = 69 + 12 * log2(f / 440)`; the fractional part times
  100 is the cents offset.
- **Latency.** Budget is ~60-80 ms and it is dominated by window length (about
  two periods of the lowest note) and the capture buffer, not compute. A 2048
  sample frame at 48 kHz is ~43 ms and reaches down to ~47 Hz.

## Develop

```
cargo test
cargo clippy --all-targets
cargo fmt
```

Rust is managed with [mise](https://mise.jdx.dev) on the dev machine
(`mise use -g rust@stable`).
