//! Frequency <-> musical note conversions.
//!
//! MIDI note 69 is A4. Each semitone is a factor of 2^(1/12), so
//! `midi = 69 + 12 * log2(f / a4)`. The fractional part of that number, times
//! 100, is the offset in cents from the nearest equal-tempered note.

pub const A4_DEFAULT: f32 = 440.0;

pub const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// A frequency resolved to its nearest equal-tempered note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteInfo {
    /// Nearest MIDI note number.
    pub midi: i32,
    /// Signed offset from that note in cents, in `[-50, 50)`.
    pub cents: f32,
    /// Frequency of the nearest note at the given reference, Hz.
    pub target_frequency: f32,
}

impl NoteInfo {
    /// Pitch class name using sharps, e.g. `"A#"`.
    pub fn name(&self) -> &'static str {
        NOTE_NAMES[self.midi.rem_euclid(12) as usize]
    }

    /// Scientific pitch notation octave, so MIDI 69 is octave 4.
    pub fn octave(&self) -> i32 {
        self.midi.div_euclid(12) - 1
    }

    /// e.g. `"A4"`.
    pub fn label(&self) -> String {
        format!("{}{}", self.name(), self.octave())
    }
}

/// Fractional MIDI note number for a frequency.
pub fn frequency_to_midi(frequency: f32, a4: f32) -> f32 {
    69.0 + 12.0 * (frequency / a4).log2()
}

pub fn midi_to_frequency(midi: f32, a4: f32) -> f32 {
    a4 * 2f32.powf((midi - 69.0) / 12.0)
}

/// Resolve a frequency to the nearest note and cents offset.
///
/// Returns `None` for a non-positive or non-finite frequency.
pub fn describe_frequency(frequency: f32, a4: f32) -> Option<NoteInfo> {
    if frequency <= 0.0 || !frequency.is_finite() {
        return None;
    }
    let exact = frequency_to_midi(frequency, a4);
    let midi = exact.round();
    Some(NoteInfo {
        midi: midi as i32,
        cents: (exact - midi) * 100.0,
        target_frequency: midi_to_frequency(midi, a4),
    })
}
