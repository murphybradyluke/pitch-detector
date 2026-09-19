//! Helpers between raw YIN output and the display: an RMS level and a short
//! median filter. Kept out of `yin.rs` so the detector stays a pure function.

/// Root-mean-square level of a buffer, in the same units as the samples.
pub fn rms(buffer: &[f32]) -> f32 {
    if buffer.is_empty() {
        return 0.0;
    }
    let sum: f64 = buffer.iter().map(|&x| (x as f64) * (x as f64)).sum();
    (sum / buffer.len() as f64).sqrt() as f32
}

/// Sliding-window median.
///
/// A median rejects single-frame outliers (an octave glitch, a spurious
/// estimate on a note onset) without the lag a moving average adds. Around 5
/// frames is a good balance for a tuner.
#[derive(Debug, Clone)]
pub struct MedianFilter {
    size: usize,
    values: Vec<f32>,
    scratch: Vec<f32>,
}

impl MedianFilter {
    /// # Panics
    /// If `size` is zero.
    pub fn new(size: usize) -> Self {
        assert!(size > 0, "median filter size must be positive");
        Self {
            size,
            values: Vec::with_capacity(size),
            scratch: Vec::with_capacity(size),
        }
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Push a value and return the median of the window so far.
    pub fn push(&mut self, value: f32) -> f32 {
        if self.values.len() == self.size {
            self.values.remove(0);
        }
        self.values.push(value);
        self.median().expect("window is non-empty after push")
    }

    /// Median of the current window, or `None` when empty.
    pub fn median(&mut self) -> Option<f32> {
        if self.values.is_empty() {
            return None;
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.values);
        self.scratch.sort_by(|a, b| a.total_cmp(b));
        let n = self.scratch.len();
        Some(if n % 2 == 1 {
            self.scratch[n / 2]
        } else {
            (self.scratch[n / 2 - 1] + self.scratch[n / 2]) / 2.0
        })
    }

    pub fn reset(&mut self) {
        self.values.clear();
    }
}
