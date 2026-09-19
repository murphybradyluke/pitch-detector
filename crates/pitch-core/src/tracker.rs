//! Stateful pipeline: sliding window -> RMS gate -> YIN -> confidence gate ->
//! median filter.
//!
//! Samples arrive in whatever chunk size the platform delivers (128 in Web
//! Audio, a few hundred to a few thousand on mobile). They go into a ring
//! buffer the size of one analysis frame, and every `hop` samples the tracker
//! analyses the most recent frame. Sliding rather than stepping the window is
//! what keeps latency low: the reading refreshes every few milliseconds, and
//! the median filter spans a few hops rather than a few whole frames.

use crate::instrument::Instrument;
use crate::note::A4_DEFAULT;
use crate::smoothing::{rms, MedianFilter};
use crate::yin::{PitchResult, Yin};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackerOptions {
    /// Sets the search range and, through it, the window length.
    pub instrument: Instrument,
    /// Reference pitch for the search range, Hz.
    pub a4: f32,
    /// Samples between analyses. 256 at 48 kHz is ~5 ms.
    pub hop: usize,
    /// YIN absolute threshold.
    pub threshold: f32,
    /// Frames with RMS below this are silence. Samples are in `[-1, 1]`.
    pub rms_gate: f32,
    /// Minimum YIN confidence to accept a frame as voiced.
    pub min_confidence: f32,
    /// Median filter length in analyses (hops), not seconds.
    pub median_frames: usize,
}

impl Default for TrackerOptions {
    fn default() -> Self {
        Self {
            instrument: Instrument::default(),
            a4: A4_DEFAULT,
            hop: 256,
            threshold: 0.15,
            rms_gate: 0.01,
            min_confidence: 0.8,
            median_frames: 5,
        }
    }
}

impl TrackerOptions {
    pub fn for_instrument(instrument: Instrument) -> Self {
        Self {
            instrument,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackedPitch {
    /// Median-filtered frequency in Hz, or `0.0` when unvoiced.
    pub frequency: f32,
    /// Confidence of the raw estimate for this analysis.
    pub confidence: f32,
    /// RMS level of the analysed frame.
    pub rms: f32,
    /// True when the frame passed both the level gate and the confidence gate.
    pub voiced: bool,
}

impl TrackedPitch {
    const UNVOICED: TrackedPitch = TrackedPitch {
        frequency: 0.0,
        confidence: 0.0,
        rms: 0.0,
        voiced: false,
    };
}

/// Turns a sample stream into a stable pitch readout.
///
/// The median only sees voiced analyses, so a burst of silence does not drag
/// the reading toward zero. A silence as long as the median window resets the
/// filter so the next note starts fresh instead of blending with the last.
#[derive(Debug, Clone)]
pub struct PitchTracker {
    yin: Yin,
    median: MedianFilter,
    rms_gate: f32,
    min_confidence: f32,
    silent_frames: usize,
    hop: usize,
    sample_rate: f32,
    /// Ring buffer of the most recent `frame_len` samples.
    ring: Vec<f32>,
    /// Next write position in `ring`.
    write: usize,
    /// Samples written so far, saturating at `frame_len`.
    filled: usize,
    /// Samples written since the last analysis.
    since_analysis: usize,
    /// Chronologically ordered copy of the ring for analysis.
    frame: Vec<f32>,
}

impl PitchTracker {
    pub fn new(sample_rate: f32, options: TrackerOptions) -> Self {
        assert!(options.hop > 0, "hop must be positive");
        let frame_len = options.instrument.frame_len(sample_rate);
        let yin_opts = options
            .instrument
            .yin_options(options.threshold, options.a4);
        Self {
            yin: Yin::new(frame_len, sample_rate, yin_opts),
            median: MedianFilter::new(options.median_frames),
            rms_gate: options.rms_gate,
            min_confidence: options.min_confidence,
            silent_frames: 0,
            hop: options.hop,
            sample_rate,
            ring: vec![0.0; frame_len],
            write: 0,
            filled: 0,
            since_analysis: 0,
            frame: vec![0.0; frame_len],
        }
    }

    pub fn frame_len(&self) -> usize {
        self.ring.len()
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Nominal time from a steady note starting to a settled reading, in
    /// seconds, excluding the platform's capture buffer: one full window so
    /// the note fills it, up to one hop of waiting, and half the median window.
    pub fn nominal_latency_secs(&self) -> f32 {
        let median_delay = (self.median.size() / 2) * self.hop;
        (self.frame_len() + self.hop + median_delay) as f32 / self.sample_rate
    }

    /// Feed samples of any length. Returns the newest reading if at least one
    /// analysis ran, or `None` if the stream has not yet reached the next hop.
    ///
    /// Allocation-free.
    pub fn push(&mut self, samples: &[f32]) -> Option<TrackedPitch> {
        let mut latest = None;
        let mut rest = samples;
        while !rest.is_empty() {
            let room = self.hop - self.since_analysis;
            let take = room.min(rest.len());
            self.write_ring(&rest[..take]);
            rest = &rest[take..];
            self.since_analysis += take;
            if self.since_analysis == self.hop {
                self.since_analysis = 0;
                if self.filled == self.ring.len() {
                    latest = Some(self.analyze_ring());
                }
            }
        }
        latest
    }

    /// Analyse one complete frame directly, bypassing the ring buffer. The
    /// frame must be exactly [`PitchTracker::frame_len`] samples.
    pub fn analyze(&mut self, frame: &[f32]) -> TrackedPitch {
        let level = rms(frame);
        if level < self.rms_gate {
            return self.unvoiced(level);
        }
        let raw = self.yin.detect(frame);
        if raw.is_none() || raw.confidence < self.min_confidence {
            return self.unvoiced(level);
        }
        self.silent_frames = 0;
        let frequency = self.median.push(raw.frequency);
        TrackedPitch {
            frequency,
            confidence: raw.confidence,
            rms: level,
            voiced: true,
        }
    }

    pub fn reset(&mut self) {
        self.median.reset();
        self.silent_frames = 0;
        self.write = 0;
        self.filled = 0;
        self.since_analysis = 0;
        self.ring.iter_mut().for_each(|s| *s = 0.0);
    }

    fn write_ring(&mut self, samples: &[f32]) {
        let n = self.ring.len();
        for &s in samples {
            self.ring[self.write] = s;
            self.write = (self.write + 1) % n;
        }
        self.filled = (self.filled + samples.len()).min(n);
    }

    fn analyze_ring(&mut self) -> TrackedPitch {
        // Oldest sample is at `write`; copy the two halves into order.
        let (tail, head) = self.ring.split_at(self.write);
        self.frame[..head.len()].copy_from_slice(head);
        self.frame[head.len()..].copy_from_slice(tail);
        // Borrow dance: analyze() needs &mut self and &self.frame at once.
        let frame = std::mem::take(&mut self.frame);
        let result = self.analyze(&frame);
        self.frame = frame;
        result
    }

    fn unvoiced(&mut self, level: f32) -> TrackedPitch {
        self.silent_frames += 1;
        if self.silent_frames >= self.median.size() {
            self.median.reset();
        }
        let PitchResult {
            frequency,
            confidence,
        } = PitchResult::NONE;
        TrackedPitch {
            frequency,
            confidence,
            rms: level,
            ..TrackedPitch::UNVOICED
        }
    }
}
