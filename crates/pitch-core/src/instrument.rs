//! Instrument range presets.
//!
//! The analysis window must hold about two periods of the lowest note the
//! detector is asked to find (measured, see `tests/instrument.rs`). That makes
//! the lowest note the single biggest latency cost: a drop-A five-string bass
//! needs an ~80 ms window while a guitar needs ~30 ms. Nobody plays both
//! ranges on one instrument, so the caller picks a preset up front and the
//! tracker sizes its window to match.
//!
//! The search range extends one semitone past each end so a badly flat or
//! sharp string still registers on the tuner.

use crate::note::{midi_to_frequency, A4_DEFAULT};
use crate::yin::YinOptions;

/// Window length as a multiple of the lowest search period. Anything under
/// ~1.9 lets the lowest note collapse to a harmonic; 2.1 leaves a margin.
pub const WINDOW_PERIODS: f32 = 2.1;

/// Frame lengths are rounded up to a multiple of this. 128 is the Web Audio
/// render quantum, so a frame is always a whole number of capture blocks.
pub const FRAME_GRANULE: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instrument {
    pub name: &'static str,
    /// Lowest playable note, MIDI number.
    pub lowest_midi: u8,
    /// Highest playable note, MIDI number.
    pub highest_midi: u8,
}

impl Instrument {
    /// Six-string guitar, standard tuning, 24 frets: E2 to E6.
    pub const GUITAR: Self = Self::new("guitar", 40, 88);
    /// Guitar with the low E dropped to D2.
    pub const GUITAR_DROP_D: Self = Self::new("guitar (drop D)", 38, 88);
    /// Four-string bass, standard tuning, 24 frets: E1 to G4.
    pub const BASS_4: Self = Self::new("bass (4-string)", 28, 67);
    /// Four-string bass with the low E dropped to D1.
    pub const BASS_4_DROP_D: Self = Self::new("bass (4-string, drop D)", 26, 67);
    /// Five-string bass, standard tuning, 24 frets: B0 to G4.
    pub const BASS_5: Self = Self::new("bass (5-string)", 23, 67);
    /// Five-string bass with the low B dropped to A0.
    pub const BASS_5_DROP_A: Self = Self::new("bass (5-string, drop A)", 21, 67);
    /// Ukulele, re-entrant GCEA, 15 frets: C4 to C6.
    pub const UKULELE: Self = Self::new("ukulele", 60, 84);
    /// Violin: G3 to C7. Above ~2 kHz at 48 kHz the period is under 24
    /// samples and sub-sample interpolation no longer holds 3 cents.
    pub const VIOLIN: Self = Self::new("violin", 55, 96);

    pub const PRESETS: [Instrument; 8] = [
        Self::GUITAR,
        Self::GUITAR_DROP_D,
        Self::BASS_4,
        Self::BASS_4_DROP_D,
        Self::BASS_5,
        Self::BASS_5_DROP_A,
        Self::UKULELE,
        Self::VIOLIN,
    ];

    /// A custom range. `lowest_midi` must not exceed `highest_midi`.
    pub const fn new(name: &'static str, lowest_midi: u8, highest_midi: u8) -> Self {
        assert!(lowest_midi <= highest_midi, "lowest note above highest");
        Self {
            name,
            lowest_midi,
            highest_midi,
        }
    }

    /// Search floor: one semitone below the lowest note.
    pub fn min_frequency(&self, a4: f32) -> f32 {
        midi_to_frequency(self.lowest_midi as f32 - 1.0, a4)
    }

    /// Search ceiling: one semitone above the highest note.
    pub fn max_frequency(&self, a4: f32) -> f32 {
        midi_to_frequency(self.highest_midi as f32 + 1.0, a4)
    }

    /// Analysis window in samples for this range at `sample_rate`.
    pub fn frame_len(&self, sample_rate: f32) -> usize {
        let raw = (WINDOW_PERIODS * sample_rate / self.min_frequency(A4_DEFAULT)).ceil() as usize;
        raw.div_ceil(FRAME_GRANULE) * FRAME_GRANULE
    }

    /// Window length in seconds; the floor on onset-to-reading latency.
    pub fn frame_secs(&self, sample_rate: f32) -> f32 {
        self.frame_len(sample_rate) as f32 / sample_rate
    }

    pub fn yin_options(&self, threshold: f32, a4: f32) -> YinOptions {
        YinOptions {
            threshold,
            min_frequency: self.min_frequency(a4),
            max_frequency: self.max_frequency(a4),
        }
    }
}

impl Default for Instrument {
    fn default() -> Self {
        Self::GUITAR
    }
}
