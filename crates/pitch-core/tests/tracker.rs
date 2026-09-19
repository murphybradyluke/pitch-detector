mod common;

use common::*;
use pitch_core::{rms, Instrument, MedianFilter, PitchTracker, TrackerOptions};

const SR: f32 = 48_000.0;
/// Web Audio delivers 128-sample blocks; feed the tracker the same way.
const BLOCK: usize = 128;

fn guitar_tracker() -> PitchTracker {
    PitchTracker::new(SR, TrackerOptions::for_instrument(Instrument::GUITAR))
}

/// Push `signal` in BLOCK-sized chunks, collecting every reading with the
/// sample index at which it was produced.
fn feed(t: &mut PitchTracker, signal: &[f32]) -> Vec<(usize, pitch_core::TrackedPitch)> {
    let mut out = Vec::new();
    for (i, chunk) in signal.chunks(BLOCK).enumerate() {
        if let Some(r) = t.push(chunk) {
            out.push(((i + 1) * BLOCK, r));
        }
    }
    out
}

#[test]
fn rms_of_sine_is_amplitude_over_root_two() {
    let buf = sine(440.0, SR, 48_000, 1.0);
    assert!((rms(&buf) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
    assert_eq!(rms(&[]), 0.0);
}

#[test]
fn median_filter_rejects_a_single_outlier() {
    let mut m = MedianFilter::new(5);
    for v in [440.0, 441.0, 440.5] {
        m.push(v);
    }
    assert_eq!(m.push(880.0), 440.75); // even window: mean of the middle two
    assert_eq!(m.push(440.2), 440.5);
    assert_eq!(m.len(), 5);
    m.reset();
    assert!(m.is_empty());
    assert!(m.median().is_none());
}

#[test]
fn median_filter_slides() {
    let mut m = MedianFilter::new(3);
    m.push(1.0);
    m.push(2.0);
    m.push(3.0);
    assert_eq!(m.push(100.0), 3.0); // window is [2, 3, 100]
    assert_eq!(m.push(100.0), 100.0); // window is [3, 100, 100]
}

#[test]
fn readings_arrive_once_per_hop_after_the_window_fills() {
    let mut t = guitar_tracker();
    let n = t.frame_len() + 10 * t.hop();
    let readings = feed(&mut t, &sine(440.0, SR, n, 0.3));
    assert_eq!(
        readings.len(),
        10,
        "one reading per hop once the ring is full"
    );
    for w in readings.windows(2) {
        assert_eq!(w[1].0 - w[0].0, t.hop());
    }
    assert!(readings.iter().all(|(_, r)| r.voiced));
}

#[test]
fn push_accepts_any_chunk_size_and_gives_the_same_readings() {
    let signal = sine(330.0, SR, 6000, 0.3);
    let run = |chunk: usize| {
        let mut t = guitar_tracker();
        let mut out = Vec::new();
        for c in signal.chunks(chunk) {
            if let Some(r) = t.push(c) {
                out.push(r.frequency);
            }
        }
        out
    };
    let a = run(1);
    let b = run(128);
    let c = run(1000);
    assert!(!a.is_empty());
    assert_eq!(a, b);
    // A 1000-sample chunk spans several hops; push returns only the newest,
    // so it has fewer readings but each must be one of the fine-grained ones.
    assert!(c.iter().all(|f| a.contains(f)));
}

#[test]
fn note_onset_to_first_reading_is_within_one_window_plus_one_hop() {
    let mut t = guitar_tracker();
    let silence = t.frame_len() * 2;
    let note = sine(196.0, SR, t.frame_len() * 2, 0.3);
    let signal = [vec![0.0; silence], note].concat();
    let readings = feed(&mut t, &signal);
    let first = readings
        .iter()
        .find(|(_, r)| r.voiced)
        .expect("note should be detected");
    let delay = first.0 - silence;
    assert!(
        delay <= t.frame_len() + t.hop(),
        "first voiced reading {delay} samples after onset, window {} + hop {}",
        t.frame_len(),
        t.hop()
    );
    // The first frame still holds a sliver of silence, so it may be a few
    // cents off; it must at least be the right note, not an octave error.
    assert!(
        cents_between(first.1.frequency, 196.0).abs() < 10.0,
        "got {}",
        first.1.frequency
    );

    // By the nominal latency the median has settled to the real pitch.
    let settle_at = silence + (t.nominal_latency_secs() * SR) as usize;
    let settled = readings
        .iter()
        .find(|(at, _)| *at >= settle_at)
        .expect("reading after settle");
    assert!(settled.1.voiced);
    assert!(
        cents_between(settled.1.frequency, 196.0).abs() < 2.0,
        "settled reading {} Hz at {} samples after onset",
        settled.1.frequency,
        settled.0 - silence
    );
}

#[test]
fn nominal_latency_for_guitar_is_under_fifty_ms_and_bass_under_a_hundred() {
    let g = guitar_tracker();
    let b = PitchTracker::new(
        SR,
        TrackerOptions::for_instrument(Instrument::BASS_5_DROP_A),
    );
    assert!(
        g.nominal_latency_secs() < 0.050,
        "guitar {}",
        g.nominal_latency_secs()
    );
    assert!(
        b.nominal_latency_secs() < 0.100,
        "bass {}",
        b.nominal_latency_secs()
    );
    assert!(b.nominal_latency_secs() > g.nominal_latency_secs());
}

#[test]
fn tracker_gates_silence_and_reports_voiced_frames() {
    let mut t = guitar_tracker();
    let n = t.frame_len();
    let quiet = t.analyze(&vec![0.0; n]);
    assert!(!quiet.voiced);
    assert_eq!(quiet.frequency, 0.0);

    let loud = t.analyze(&sine(440.0, SR, n, 0.3));
    assert!(loud.voiced);
    assert!(cents_between(loud.frequency, 440.0).abs() < 1.0);
    assert!((loud.rms - 0.3 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-2);
}

#[test]
fn tracker_smooths_a_one_analysis_octave_glitch() {
    let mut t = guitar_tracker();
    let n = t.frame_len();
    let a = sine(220.0, SR, n, 0.3);
    let glitch = sine(440.0, SR, n, 0.3);
    for _ in 0..4 {
        t.analyze(&a);
    }
    let during = t.analyze(&glitch);
    assert!(during.voiced);
    assert!(
        cents_between(during.frequency, 220.0).abs() < 2.0,
        "got {}",
        during.frequency
    );
}

#[test]
fn tracker_resets_after_a_long_silence_so_next_note_is_not_blended() {
    let opts = TrackerOptions {
        median_frames: 3,
        ..TrackerOptions::for_instrument(Instrument::GUITAR)
    };
    let mut t = PitchTracker::new(SR, opts);
    let n = t.frame_len();
    let low = sine(220.0, SR, n, 0.3);
    let high = sine(660.0, SR, n, 0.3);
    for _ in 0..3 {
        t.analyze(&low);
    }
    for _ in 0..3 {
        t.analyze(&vec![0.0; n]);
    }
    let first = t.analyze(&high);
    assert!(first.voiced);
    assert!(
        cents_between(first.frequency, 660.0).abs() < 2.0,
        "got {}",
        first.frequency
    );
}

#[test]
fn tracker_rejects_noise_by_confidence() {
    let mut t = guitar_tracker();
    let r = t.analyze(&noise(t.frame_len(), 0.5, 99));
    assert!(!r.voiced, "{r:?}");
    assert!(r.rms > 0.01, "noise is loud enough to pass the RMS gate");
}

#[test]
fn reset_clears_the_ring_so_old_audio_is_not_analysed() {
    let mut t = guitar_tracker();
    let n = t.frame_len();
    let warmup = n + t.hop();
    feed(&mut t, &sine(440.0, SR, warmup, 0.3));
    t.reset();
    // After reset the ring must fill again before any reading appears.
    let readings = feed(&mut t, &sine(440.0, SR, n - BLOCK, 0.3));
    assert!(
        readings.is_empty(),
        "got {} readings before the ring refilled",
        readings.len()
    );
}

#[test]
fn bass_drop_a_is_tracked_through_the_sliding_window() {
    let mut t = PitchTracker::new(
        SR,
        TrackerOptions::for_instrument(Instrument::BASS_5_DROP_A),
    );
    let f = 27.5; // A0
    let n = t.frame_len() + 8 * t.hop();
    let signal = harmonics(f, SR, n, &[0.1, 0.4, 0.3, 0.2, 0.1]);
    let readings = feed(&mut t, &signal);
    assert!(readings.len() >= 8);
    for (_, r) in &readings {
        assert!(r.voiced);
        assert!(
            cents_between(r.frequency, f).abs() < 2.0,
            "got {}",
            r.frequency
        );
    }
}
