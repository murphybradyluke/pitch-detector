//! Stateful pipeline: RMS gate -> YIN -> confidence gate -> median filter.

use crate::smoothing::{rms, MedianFilter};
use crate::yin::{PitchResult, Yin, YinOptions};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackerOptions {
    pub yin: YinOptions,
    /// Frames with RMS below this are treated as silence. Samples are in `[-1, 1]`.
    pub rms_gate: f32,
    /// Minimum YIN confidence to accept a frame as voiced.
    pub min_confidence: f32,
    /// Median filter length in frames.
    pub median_frames: usize,
}

impl Default for TrackerOptions {
    fn default() -> Self {
        Self {
            yin: YinOptions::default(),
            rms_gate: 0.01,
            min_confidence: 0.8,
            median_frames: 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackedPitch {
    /// Median-filtered frequency in Hz, or `0.0` when unvoiced.
    pub frequency: f32,
    /// Confidence of the raw estimate for this frame.
    pub confidence: f32,
    /// RMS level of the frame.
    pub rms: f32,
    /// True when the frame passed both the level gate and the confidence gate.
    pub voiced: bool,
}

/// Turns a stream of fixed-size frames into a stable pitch readout.
///
/// The median only sees voiced frames, so a burst of silence does not drag the
/// reading toward zero. A silence as long as the median window resets the
/// filter so the next note starts fresh instead of blending with the last.
#[derive(Debug, Clone)]
pub struct PitchTracker {
    yin: Yin,
    median: MedianFilter,
    rms_gate: f32,
    min_confidence: f32,
    silent_frames: usize,
}

impl PitchTracker {
    pub fn new(frame_len: usize, sample_rate: f32, options: TrackerOptions) -> Self {
        Self {
            yin: Yin::new(frame_len, sample_rate, options.yin),
            median: MedianFilter::new(options.median_frames),
            rms_gate: options.rms_gate,
            min_confidence: options.min_confidence,
            silent_frames: 0,
        }
    }

    pub fn frame_len(&self) -> usize {
        self.yin.buffer_len()
    }

    /// Process one frame. Allocation-free after construction.
    pub fn process(&mut self, frame: &[f32]) -> TrackedPitch {
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
            voiced: false,
        }
    }

    pub fn reset(&mut self) {
        self.median.reset();
        self.silent_frames = 0;
    }
}
