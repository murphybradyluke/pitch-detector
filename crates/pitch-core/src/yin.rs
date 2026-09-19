//! YIN fundamental-frequency estimator.
//!
//! Reference: de Cheveigné & Kawahara, "YIN, a fundamental frequency estimator
//! for speech and music", JASA 2002.
//!
//! Why YIN rather than FFT peak-picking: the loudest spectral peak is often a
//! harmonic rather than the fundamental, which produces octave errors. YIN
//! works in the time domain on the difference function, which has a minimum at
//! the true period and at its multiples. The absolute-threshold rule picks the
//! *smallest* lag that dips below the threshold, so it prefers the fundamental
//! over sub-harmonics.

/// Result of one pitch estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchResult {
    /// Estimated fundamental in Hz. `0.0` when no estimate could be made.
    pub frequency: f32,
    /// `1 - CMNDF` at the chosen lag, clamped to `[0, 1]`. Near 1 for a clean
    /// periodic signal, near 0 for noise or silence. Callers should gate on it.
    pub confidence: f32,
}

impl PitchResult {
    /// The "nothing found" result.
    pub const NONE: PitchResult = PitchResult {
        frequency: 0.0,
        confidence: 0.0,
    };

    pub fn is_none(&self) -> bool {
        self.frequency == 0.0
    }
}

/// Tunable parameters for the estimator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YinOptions {
    /// Absolute threshold on the cumulative mean normalised difference. The
    /// paper suggests ~0.1; 0.15 tolerates a little inharmonicity and noise.
    pub threshold: f32,
    /// Lowest frequency to search for, Hz. Sets the largest lag.
    pub min_frequency: f32,
    /// Highest frequency to search for, Hz. Sets the smallest lag.
    pub max_frequency: f32,
}

impl Default for YinOptions {
    fn default() -> Self {
        Self {
            threshold: 0.15,
            min_frequency: 50.0,
            max_frequency: 2500.0,
        }
    }
}

/// A YIN estimator with pre-allocated scratch space.
///
/// Use this on an audio thread: after construction, [`Yin::detect`] does no
/// heap allocation. For one-off calls, [`detect_pitch`] is more convenient.
#[derive(Debug, Clone)]
pub struct Yin {
    sample_rate: f32,
    options: YinOptions,
    tau_min: usize,
    tau_max: usize,
    window: usize,
    /// Difference function, indexed by lag. Accumulated in f64 because the
    /// window can be a few thousand terms.
    diff: Vec<f64>,
    /// Cumulative mean normalised difference, indexed by lag.
    cmndf: Vec<f32>,
}

impl Yin {
    /// Build an estimator for buffers of exactly `buffer_len` samples.
    ///
    /// The buffer should hold at least two periods of the lowest frequency
    /// you care about. At 48 kHz, 2048 samples (~43 ms) reaches down to ~47 Hz.
    pub fn new(buffer_len: usize, sample_rate: f32, options: YinOptions) -> Self {
        let half = buffer_len / 2;
        let tau_max = ((sample_rate / options.min_frequency).ceil() as usize).min(half);
        let tau_min = ((sample_rate / options.max_frequency).floor() as usize).max(1);
        let window = buffer_len.saturating_sub(tau_max);
        Self {
            sample_rate,
            options,
            tau_min,
            tau_max,
            window,
            diff: vec![0.0; tau_max + 1],
            cmndf: vec![0.0; tau_max + 1],
        }
    }

    pub fn buffer_len(&self) -> usize {
        self.window + self.tau_max
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn options(&self) -> &YinOptions {
        &self.options
    }

    /// Estimate the fundamental of `buffer`.
    ///
    /// # Panics
    /// If `buffer.len()` differs from the length given to [`Yin::new`].
    pub fn detect(&mut self, buffer: &[f32]) -> PitchResult {
        assert_eq!(buffer.len(), self.buffer_len(), "buffer length mismatch");
        let (tau_min, tau_max, w) = (self.tau_min, self.tau_max, self.window);
        if tau_max <= tau_min + 1 {
            return PitchResult::NONE;
        }

        // Step 1: difference function d(tau) = sum_j (x[j] - x[j + tau])^2.
        for tau in 1..=tau_max {
            let (a, b) = (&buffer[..w], &buffer[tau..tau + w]);
            let sum: f64 = a
                .iter()
                .zip(b)
                .map(|(&x, &y)| {
                    let d = (x - y) as f64;
                    d * d
                })
                .sum();
            self.diff[tau] = sum;
        }

        // Step 2: cumulative mean normalised difference.
        // d'(0) = 1, d'(tau) = d(tau) * tau / sum_{j=1..tau} d(j).
        // This removes the bias toward tau = 0 and makes the threshold absolute.
        self.cmndf[0] = 1.0;
        let mut running = 0.0f64;
        for tau in 1..=tau_max {
            running += self.diff[tau];
            self.cmndf[tau] = if running == 0.0 {
                1.0
            } else {
                (self.diff[tau] * tau as f64 / running) as f32
            };
        }

        // Step 3: absolute threshold. Take the first lag under the threshold,
        // then slide down to the bottom of that dip. If nothing crosses, fall
        // back to the global minimum; the low confidence tells the caller.
        let cmndf = &self.cmndf;
        let threshold = self.options.threshold;
        let mut tau = None;
        for t in tau_min..=tau_max {
            if cmndf[t] < threshold {
                let mut t = t;
                while t < tau_max && cmndf[t + 1] < cmndf[t] {
                    t += 1;
                }
                tau = Some(t);
                break;
            }
        }
        let tau = tau.unwrap_or_else(|| {
            (tau_min..=tau_max)
                .min_by(|&a, &b| {
                    cmndf[a]
                        .partial_cmp(&cmndf[b])
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(tau_min)
        });

        // Step 4: parabolic interpolation for a sub-sample period estimate.
        let refined = parabolic_minimum(cmndf, tau);
        let frequency = self.sample_rate / refined;
        if !frequency.is_finite() || frequency <= 0.0 {
            return PitchResult::NONE;
        }
        let confidence = (1.0 - cmndf[tau]).clamp(0.0, 1.0);
        PitchResult {
            frequency,
            confidence,
        }
    }
}

/// One-shot estimate. Allocates scratch space each call; see [`Yin`] for the
/// allocation-free form.
pub fn detect_pitch(buffer: &[f32], sample_rate: f32, options: YinOptions) -> PitchResult {
    Yin::new(buffer.len(), sample_rate, options).detect(buffer)
}

/// Fit a parabola through `(tau-1, tau, tau+1)` and return the x of its vertex.
fn parabolic_minimum(values: &[f32], tau: usize) -> f32 {
    if tau == 0 || tau + 1 >= values.len() {
        return tau as f32;
    }
    let (y0, y1, y2) = (values[tau - 1], values[tau], values[tau + 1]);
    let denom = y0 - 2.0 * y1 + y2;
    if denom == 0.0 {
        return tau as f32;
    }
    let shift = (y0 - y2) / (2.0 * denom);
    // A degenerate fit can throw the vertex outside the bin; ignore it then.
    if shift.abs() <= 1.0 {
        tau as f32 + shift
    } else {
        tau as f32
    }
}
