# pitch-detector

Real-time monophonic pitch detection for a tuner and a play-along note checker
that scores you against a MIDI or Guitar Pro file, with the tab on screen. The core is a dependency-free Rust crate
so it can be compiled to WASM for the web and reused on iOS and Android.

## Layout

```
crates/pitch-core/   pure detection library (no I/O, no platform code)
crates/pitch-song/   reference songs: MIDI import, chord reduction, scorer
  src/midi.rs        Standard MIDI File -> per-track NoteEvents with a tempo map
  src/scorer.rs      judges each target note from the stream of readings
crates/pitch-wasm/   wasm-bindgen wrapper: Tracker, Reading, Song, Judge
web/                 tuner page: AudioWorklet -> Worker (WASM) -> UI; alphaTab for Guitar Pro
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

## Play-along

Load a `.mid` or Guitar Pro file (`.gp`, `.gp3`, `.gp4`, `.gp5`, `.gpx`), pick a track (bass tracks are picked by default: General
MIDI programs 32–39, or a track named "bass", or else the lowest track), and
press Play. Four clicks count you in, then the roll scrolls: grey notes are
coming, blue is now, green was hit, red was missed, and white dots are what
the detector heard. "Hear the part" plays the track as a synth. Use
headphones for that, or the mic scores the synth.

- **Chords collapse to their lowest note** (`pitch_song::monophonic`); the
  detector is monophonic, and the root is what the ear judges anyway.
- **Scoring** (`pitch_song::Scorer`): each note has a window from 100 ms
  before its start to 100 ms after its end. Readings matching the note inside
  the window accumulate matched time; the note is a hit at 60 ms of matches
  (or half its duration for shorter notes). Misses record the wrong note
  heard most often. Times are on the audio clock, and the detector's window
  centre plus median delay is subtracted, so scoring is on the beat even at
  100 ms of latency.
- **Tempo changes** anywhere in the file are honoured; SMPTE-timed files are
  not supported.
- **Parts, not tracks.** Each MIDI track is split into one part per channel,
  labelled with its General MIDI instrument. Single-track (format 0) files
  put every instrument together, and without the split the kick drum at
  MIDI 36 was the "lowest note" of every chord and the bass line vanished.
  Guitar Pro conversions skip the split, since alphaTab puts a track's bent
  notes on a second channel.
- **"Hear the part"** is a filtered sawtooth with a plucked envelope. Bass
  fundamentals are below what phone speakers reproduce; the harmonics carry
  the pitch.
- **Guitar Pro** files go through [alphaTab](https://alphatab.net) in the
  browser: it parses the score, generates a standard MIDI file from it (one
  MIDI track per score track, bends as pitch bends) and that file takes the
  same path as a `.mid`. alphaTab also renders the selected track as tab
  above the roll, and a cursor follows the audio clock through it using
  alphaTab's tick and bounds lookups. No alphaTab player or soundfont is
  used; our click and synth are the playback.

## Web tuner

Audio path: `getUserMedia` (echo cancellation, noise suppression and auto
gain off) -> `AudioWorklet` that forwards 128-sample blocks over a
`MessagePort` -> dedicated `Worker` running the WASM tracker -> readings to
the main thread, drawn on `requestAnimationFrame`. The worklet cannot host
the WASM itself: its global scope lacks `fetch` and `TextDecoder`.

```
wasm-pack build crates/pitch-wasm --target web --out-dir ../../web/pkg --release
(cd web && npm install)          # alphaTab, for Guitar Pro files and tab rendering
(cd web && python3 -m http.server 8765)
# open http://localhost:8765/
```

Devices: the "In" picker lists every input the OS exposes, so an audio
interface appears by name (CoreAudio on macOS, WASAPI on Windows, PipeWire
on Linux; browsers have no ASIO path). "ch" picks one side of a stereo
interface or a mix. "Out" appears where `AudioContext.setSinkId` exists
(Chromium; Safari routes to the system default). Choices persist in the
browser.

Query parameters for testing: `?instrument=guitar&autostart=1&debug=1` logs
every reading to the console; `?smooth=<ms>` sets the display smoothing.

## Develop

```
cargo test                      # unit + synthetic-signal + MIDI/scorer tests
cargo clippy --all-targets
cargo fmt
node scripts/e2e.mjs            # tuner, MIDI play-along, Guitar Pro in headless chromium
```

Rust is managed with [mise](https://mise.jdx.dev) on the dev machine
(`mise use -g rust@stable`).
