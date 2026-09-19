//! Reference songs for the play-along checker.
//!
//! The scorer consumes one thing: a flat list of [`NoteEvent`]s, each a MIDI
//! note with a start time and duration in seconds. Every import format is a
//! converter into that list. Today that is Standard MIDI Files ([`midi`]);
//! tablature formats can join later by producing the same events.
//!
//! The detector is monophonic, so a track is reduced to one note at a time
//! before scoring ([`monophonic`]): when several notes start together, the
//! lowest wins. Power chords therefore score against the root.

pub mod midi;
pub mod scorer;

pub use midi::{MidiSong, TrackInfo};
pub use scorer::{NoteOutcome, Scorer, ScorerOptions};

/// One target note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteEvent {
    pub midi: u8,
    pub start_secs: f32,
    pub duration_secs: f32,
    pub velocity: u8,
}

impl NoteEvent {
    pub fn end_secs(&self) -> f32 {
        self.start_secs + self.duration_secs
    }
}

/// Reduce possibly polyphonic events to a single line.
///
/// Notes starting within `chord_window_secs` of each other are one chord;
/// only the lowest is kept. Each kept note is then clipped so it ends no
/// later than the next note starts.
pub fn monophonic(events: &[NoteEvent], chord_window_secs: f32) -> Vec<NoteEvent> {
    let mut sorted = events.to_vec();
    sorted.sort_by(|a, b| {
        a.start_secs
            .partial_cmp(&b.start_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.midi.cmp(&b.midi))
    });

    let mut out: Vec<NoteEvent> = Vec::with_capacity(sorted.len());
    let mut i = 0;
    while i < sorted.len() {
        let group_start = sorted[i].start_secs;
        let mut lowest = sorted[i];
        let mut j = i + 1;
        while j < sorted.len() && sorted[j].start_secs - group_start <= chord_window_secs {
            if sorted[j].midi < lowest.midi {
                lowest = sorted[j];
            }
            j += 1;
        }
        out.push(lowest);
        i = j;
    }

    for k in 0..out.len().saturating_sub(1) {
        let next_start = out[k + 1].start_secs;
        if out[k].end_secs() > next_start {
            out[k].duration_secs = (next_start - out[k].start_secs).max(0.0);
        }
    }
    out
}
