//! Thin wasm-bindgen surface over `pitch-core`.
//!
//! Keeps the JS boundary small: one `Tracker` object that eats sample chunks
//! and returns a flat `Reading`. All the signal processing stays in Rust.

use pitch_core::{describe_frequency, Instrument, PitchTracker, TrackerOptions, A4_DEFAULT};
use wasm_bindgen::prelude::*;

/// One pitch reading, ready to display.
#[wasm_bindgen(getter_with_clone)]
#[derive(Debug, Clone)]
pub struct Reading {
    pub voiced: bool,
    /// Median-filtered frequency in Hz, 0 when unvoiced.
    pub frequency: f32,
    pub confidence: f32,
    pub rms: f32,
    /// Nearest MIDI note, or -1 when unvoiced.
    pub midi: i32,
    /// Signed cents from the nearest note, 0 when unvoiced.
    pub cents: f32,
    /// e.g. "A4", empty when unvoiced.
    pub note: String,
}

#[wasm_bindgen]
pub struct Tracker {
    inner: PitchTracker,
    a4: f32,
}

#[wasm_bindgen]
impl Tracker {
    /// `instrument` is one of the names from [`instrument_names`].
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: f32, instrument: &str, a4: Option<f32>) -> Result<Tracker, JsError> {
        let preset = Instrument::PRESETS
            .into_iter()
            .find(|p| p.name == instrument)
            .ok_or_else(|| JsError::new(&format!("unknown instrument: {instrument}")))?;
        let a4 = a4.unwrap_or(A4_DEFAULT);
        let options = TrackerOptions {
            instrument: preset,
            a4,
            ..Default::default()
        };
        Ok(Tracker {
            inner: PitchTracker::new(sample_rate, options),
            a4,
        })
    }

    /// Feed samples in `[-1, 1]` of any length. Returns the newest reading if
    /// at least one analysis ran, otherwise `undefined`.
    pub fn push(&mut self, samples: &[f32]) -> Option<Reading> {
        let r = self.inner.push(samples)?;
        let (midi, cents, note) = if r.voiced {
            match describe_frequency(r.frequency, self.a4) {
                Some(n) => (n.midi, n.cents, n.label()),
                None => (-1, 0.0, String::new()),
            }
        } else {
            (-1, 0.0, String::new())
        };
        Some(Reading {
            voiced: r.voiced,
            frequency: r.frequency,
            confidence: r.confidence,
            rms: r.rms,
            midi,
            cents,
            note,
        })
    }

    pub fn reset(&mut self) {
        self.inner.reset();
    }

    #[wasm_bindgen(getter)]
    pub fn frame_len(&self) -> usize {
        self.inner.frame_len()
    }

    #[wasm_bindgen(getter)]
    pub fn hop(&self) -> usize {
        self.inner.hop()
    }

    /// Nominal onset-to-settled latency in seconds, excluding capture buffer.
    #[wasm_bindgen(getter)]
    pub fn nominal_latency_secs(&self) -> f32 {
        self.inner.nominal_latency_secs()
    }
}

/// Names of the built-in instrument presets, in display order.
#[wasm_bindgen]
pub fn instrument_names() -> Vec<String> {
    Instrument::PRESETS
        .iter()
        .map(|p| p.name.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Songs and scoring (play-along)
// ---------------------------------------------------------------------------

use pitch_song::{monophonic, MidiSong, NoteEvent, Scorer, ScorerOptions};

/// One track of a loaded MIDI file, for a track picker.
#[wasm_bindgen(getter_with_clone)]
#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub index: usize,
    pub midi_track: usize,
    pub channel: u8,
    pub name: String,
    /// General MIDI instrument name, "Drums", or empty.
    pub program_name: String,
    pub note_count: usize,
    pub lowest_midi: u8,
    pub highest_midi: u8,
    /// General MIDI program, or -1 when the track never sets one.
    pub program: i32,
    pub is_drums: bool,
}

/// A parsed MIDI file.
#[wasm_bindgen]
pub struct Song {
    inner: MidiSong,
}

#[wasm_bindgen]
impl Song {
    /// `split_channels` (default true) lists one part per channel per track,
    /// which keeps drums apart from the bass in single-track files. Pass
    /// false for MIDI generated from Guitar Pro, where a track's bent notes
    /// live on a second channel.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], split_channels: Option<bool>) -> Result<Song, JsError> {
        MidiSong::parse_with(bytes, split_channels.unwrap_or(true))
            .map(|inner| Song { inner })
            .map_err(|e| JsError::new(&e.to_string()))
    }

    pub fn tracks(&self) -> Vec<TrackInfo> {
        self.inner
            .tracks()
            .iter()
            .map(|t| TrackInfo {
                index: t.index,
                midi_track: t.midi_track,
                channel: t.channel,
                name: t.name.clone(),
                program_name: t.program_name().to_string(),
                note_count: t.note_count,
                lowest_midi: t.lowest_midi,
                highest_midi: t.highest_midi,
                program: t.program.map(|p| p as i32).unwrap_or(-1),
                is_drums: t.is_drums,
            })
            .collect()
    }

    /// Monophonic events of a track as a flat array: `[midi, start, duration]`
    /// per note, times in seconds. Chords collapse to their lowest note.
    pub fn events(&self, track: usize, chord_window_secs: f32) -> Vec<f32> {
        monophonic(self.inner.events(track), chord_window_secs)
            .iter()
            .flat_map(|e| [e.midi as f32, e.start_secs, e.duration_secs])
            .collect()
    }

    pub fn beat_times(&self) -> Vec<f32> {
        self.inner.beat_times()
    }

    #[wasm_bindgen(getter)]
    pub fn duration_secs(&self) -> f32 {
        self.inner.duration_secs()
    }

    #[wasm_bindgen(getter)]
    pub fn initial_bpm(&self) -> f32 {
        self.inner.initial_bpm()
    }
}

/// Verdict on one target note.
#[wasm_bindgen]
#[derive(Debug, Clone, Copy)]
pub struct Outcome {
    pub index: usize,
    pub hit: bool,
    pub matched_secs: f32,
    /// Most common wrong note heard, or -1.
    pub wrong_midi: i32,
    /// First matching reading relative to the written start, or NaN.
    pub onset_offset_secs: f32,
}

/// Scores readings against a flat event list from [`Song::events`].
#[wasm_bindgen]
pub struct Judge {
    inner: Scorer,
}

#[wasm_bindgen]
impl Judge {
    /// `latency_secs` is subtracted from every reading time; pass the
    /// detector's effective delay so scoring lines up with the written notes.
    #[wasm_bindgen(constructor)]
    pub fn new(events: &[f32], latency_secs: f32, reading_interval_secs: f32) -> Judge {
        let targets: Vec<NoteEvent> = events
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| NoteEvent {
                midi: c[0] as u8,
                start_secs: c[1],
                duration_secs: c[2],
                velocity: 100,
            })
            .collect();
        let options = ScorerOptions {
            latency_secs,
            reading_interval_secs,
            ..Default::default()
        };
        Judge {
            inner: Scorer::new(targets, options),
        }
    }

    /// `midi` is the detected note or -1 when unvoiced. Returns outcomes for
    /// notes whose windows closed.
    pub fn feed(&mut self, song_time_secs: f32, midi: i32) -> Vec<Outcome> {
        let detected = if midi >= 0 { Some(midi as u8) } else { None };
        self.inner
            .feed(song_time_secs, detected)
            .into_iter()
            .map(convert)
            .collect()
    }

    pub fn finish(&mut self) -> Vec<Outcome> {
        self.inner.finish().into_iter().map(convert).collect()
    }

    #[wasm_bindgen(getter)]
    pub fn hits(&self) -> usize {
        self.inner.hits()
    }

    #[wasm_bindgen(getter)]
    pub fn finished(&self) -> usize {
        self.inner.finished()
    }

    #[wasm_bindgen(getter)]
    pub fn total(&self) -> usize {
        self.inner.total()
    }
}

fn convert(o: pitch_song::NoteOutcome) -> Outcome {
    Outcome {
        index: o.index,
        hit: o.hit,
        matched_secs: o.matched_secs,
        wrong_midi: o.wrong_midi.map(|m| m as i32).unwrap_or(-1),
        onset_offset_secs: o.onset_offset_secs.unwrap_or(f32::NAN),
    }
}
