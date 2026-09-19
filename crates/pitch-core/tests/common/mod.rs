//! Synthetic signal generators shared by the integration tests.

#![allow(dead_code)]

use std::f32::consts::TAU;

/// A pure sine of `frequency` Hz.
pub fn sine(frequency: f32, sample_rate: f32, len: usize, amplitude: f32) -> Vec<f32> {
    (0..len)
        .map(|i| amplitude * (TAU * frequency * i as f32 / sample_rate).sin())
        .collect()
}

/// A sum of harmonics. `amplitudes[k]` is the amplitude of harmonic `k + 1`.
pub fn harmonics(frequency: f32, sample_rate: f32, len: usize, amplitudes: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0f32; len];
    for (k, &amp) in amplitudes.iter().enumerate() {
        let f = frequency * (k + 1) as f32;
        for (i, s) in out.iter_mut().enumerate() {
            *s += amp * (TAU * f * i as f32 / sample_rate).sin();
        }
    }
    out
}

/// Deterministic uniform noise in `[-amplitude, amplitude]` (xorshift, no deps).
pub fn noise(len: usize, amplitude: f32, seed: u64) -> Vec<f32> {
    let mut state = seed.max(1);
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let unit = (state >> 11) as f32 / (1u64 << 53) as f32;
            amplitude * (2.0 * unit - 1.0)
        })
        .collect()
}

pub fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x + y).collect()
}

/// Signed cents between two frequencies.
pub fn cents_between(measured: f32, expected: f32) -> f32 {
    1200.0 * (measured / expected).log2()
}
