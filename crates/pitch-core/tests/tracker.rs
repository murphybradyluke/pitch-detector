mod common;

use common::*;
use pitch_core::{rms, MedianFilter, PitchTracker, TrackerOptions};

const SR: f32 = 48_000.0;
const FRAME: usize = 2048;

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
fn tracker_gates_silence_and_reports_voiced_frames() {
    let mut t = PitchTracker::new(FRAME, SR, TrackerOptions::default());
    let quiet = t.process(&vec![0.0; FRAME]);
    assert!(!quiet.voiced);
    assert_eq!(quiet.frequency, 0.0);

    let loud = t.process(&sine(440.0, SR, FRAME, 0.3));
    assert!(loud.voiced);
    assert!(cents_between(loud.frequency, 440.0).abs() < 1.0);
    assert!((loud.rms - 0.3 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-2);
}

#[test]
fn tracker_smooths_a_one_frame_octave_glitch() {
    let mut t = PitchTracker::new(FRAME, SR, TrackerOptions::default());
    let a = sine(220.0, SR, FRAME, 0.3);
    let glitch = sine(440.0, SR, FRAME, 0.3);
    for _ in 0..4 {
        t.process(&a);
    }
    let during = t.process(&glitch);
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
        ..Default::default()
    };
    let mut t = PitchTracker::new(FRAME, SR, opts);
    let low = sine(220.0, SR, FRAME, 0.3);
    let high = sine(660.0, SR, FRAME, 0.3);
    for _ in 0..3 {
        t.process(&low);
    }
    for _ in 0..3 {
        t.process(&vec![0.0; FRAME]);
    }
    let first = t.process(&high);
    assert!(first.voiced);
    assert!(
        cents_between(first.frequency, 660.0).abs() < 2.0,
        "got {}",
        first.frequency
    );
}

#[test]
fn tracker_rejects_noise_by_confidence() {
    let mut t = PitchTracker::new(FRAME, SR, TrackerOptions::default());
    let r = t.process(&noise(FRAME, 0.5, 99));
    assert!(!r.voiced, "{r:?}");
    assert!(r.rms > 0.01, "noise is loud enough to pass the RMS gate");
}
