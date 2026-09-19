//! Monophonic pitch detection for tuners and play-along note checkers.
//!
//! The detector is a pure function over a slice of samples plus a sample
//! rate. It knows nothing about microphones, threads, or timing. That keeps it
//! unit-testable on synthetic signals and portable to WASM, iOS, and Android
//! behind a thin platform wrapper.
//!
//! Layers, bottom up:
//!
//! - [`yin`]: the estimator. `&[f32]` + sample rate -> [`PitchResult`].
//! - [`note`]: frequency <-> MIDI note number and cents.
//! - [`smoothing`]: RMS level and a small median filter.
//! - [`tracker`]: stateful pipeline that gates on level and confidence and
//!   median-filters the result, for a stable readout.

pub mod note;
pub mod smoothing;
pub mod tracker;
pub mod yin;

pub use note::{describe_frequency, frequency_to_midi, midi_to_frequency, NoteInfo, A4_DEFAULT};
pub use smoothing::{rms, MedianFilter};
pub use tracker::{PitchTracker, TrackedPitch, TrackerOptions};
pub use yin::{detect_pitch, PitchResult, Yin, YinOptions};
