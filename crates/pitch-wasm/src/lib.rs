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
