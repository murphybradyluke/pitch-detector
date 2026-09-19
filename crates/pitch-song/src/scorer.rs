//! Judges each target note from the stream of detector readings.
//!
//! Readings arrive every few milliseconds as `(time, detected note)`. For
//! each target note there is a window from a little before its start to a
//! little after its end. Readings inside the window that match the target
//! accumulate matched time; when the window has passed, the note is a hit if
//! enough matched time was seen. "Enough" is `min_match_secs`, or half the
//! note's duration for very short notes, whichever is smaller.
//!
//! Times are on the audio clock. The caller subtracts detector latency
//! before feeding, or sets `latency_secs` here and passes raw reading times.

use crate::NoteEvent;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScorerOptions {
    /// How early a reading may match a note, before its written start.
    pub early_tolerance_secs: f32,
    /// How late a reading may still match, after its written end.
    pub late_tolerance_secs: f32,
    /// Matched time needed for a hit (capped at half the note's duration).
    pub min_match_secs: f32,
    /// Time between readings; each matching reading counts this much.
    pub reading_interval_secs: f32,
    /// Subtracted from every reading time before matching.
    pub latency_secs: f32,
}

impl Default for ScorerOptions {
    fn default() -> Self {
        Self {
            early_tolerance_secs: 0.10,
            late_tolerance_secs: 0.10,
            min_match_secs: 0.06,
            reading_interval_secs: 256.0 / 48_000.0,
            latency_secs: 0.0,
        }
    }
}

/// Verdict on one target note, produced once its window has passed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NoteOutcome {
    pub index: usize,
    pub hit: bool,
    /// Seconds of readings that matched the target.
    pub matched_secs: f32,
    /// Most common wrong note heard in the window, if any.
    pub wrong_midi: Option<u8>,
    /// Time of the first matching reading relative to the written start,
    /// negative when early. `None` when nothing matched.
    pub onset_offset_secs: Option<f32>,
}

#[derive(Debug, Clone, Default)]
struct NoteState {
    matched_secs: f32,
    first_match: Option<f32>,
    /// (midi, readings) for wrong notes heard.
    wrong: Vec<(u8, u32)>,
}

#[derive(Debug, Clone)]
pub struct Scorer {
    targets: Vec<NoteEvent>,
    options: ScorerOptions,
    state: Vec<NoteState>,
    /// First target whose window has not yet been finalized.
    next: usize,
    hits: usize,
    finished: usize,
}

impl Scorer {
    pub fn new(targets: Vec<NoteEvent>, options: ScorerOptions) -> Self {
        let state = vec![NoteState::default(); targets.len()];
        Self {
            targets,
            options,
            state,
            next: 0,
            hits: 0,
            finished: 0,
        }
    }

    pub fn targets(&self) -> &[NoteEvent] {
        &self.targets
    }

    pub fn hits(&self) -> usize {
        self.hits
    }

    /// Notes whose windows have passed.
    pub fn finished(&self) -> usize {
        self.finished
    }

    pub fn total(&self) -> usize {
        self.targets.len()
    }

    fn window(&self, i: usize) -> (f32, f32) {
        let n = &self.targets[i];
        (
            n.start_secs - self.options.early_tolerance_secs,
            n.end_secs() + self.options.late_tolerance_secs,
        )
    }

    /// Feed one reading. `detected` is the nearest MIDI note, or `None` when
    /// unvoiced. Returns outcomes for every note whose window closed at or
    /// before this time, in order.
    pub fn feed(&mut self, time_secs: f32, detected: Option<u8>) -> Vec<NoteOutcome> {
        let t = time_secs - self.options.latency_secs;

        // Credit every target whose window contains t. Windows of fast
        // passages overlap, so look past `next` until windows start after t.
        let mut i = self.next;
        while i < self.targets.len() {
            let (lo, hi) = self.window(i);
            if lo > t {
                break;
            }
            if t <= hi {
                if let Some(d) = detected {
                    let st = &mut self.state[i];
                    if d == self.targets[i].midi {
                        st.matched_secs += self.options.reading_interval_secs;
                        st.first_match.get_or_insert(t - self.targets[i].start_secs);
                    } else if let Some(w) = st.wrong.iter_mut().find(|w| w.0 == d) {
                        w.1 += 1;
                    } else {
                        st.wrong.push((d, 1));
                    }
                }
            }
            i += 1;
        }

        let mut out = Vec::new();
        while self.next < self.targets.len() && self.window(self.next).1 < t {
            out.push(self.finalize(self.next));
            self.next += 1;
        }
        out
    }

    /// Finalize everything that is still open, e.g. when the song ends.
    pub fn finish(&mut self) -> Vec<NoteOutcome> {
        let mut out = Vec::new();
        while self.next < self.targets.len() {
            out.push(self.finalize(self.next));
            self.next += 1;
        }
        out
    }

    fn finalize(&mut self, i: usize) -> NoteOutcome {
        let n = self.targets[i];
        let st = std::mem::take(&mut self.state[i]);
        let needed = self
            .options
            .min_match_secs
            .min(n.duration_secs * 0.5)
            .max(self.options.reading_interval_secs);
        let hit = st.matched_secs >= needed;
        if hit {
            self.hits += 1;
        }
        self.finished += 1;
        let wrong_midi = st.wrong.iter().max_by_key(|w| w.1).map(|w| w.0);
        NoteOutcome {
            index: i,
            hit,
            matched_secs: st.matched_secs,
            wrong_midi,
            onset_offset_secs: st.first_match,
        }
    }
}
