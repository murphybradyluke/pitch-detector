mod common;

use common::*;
use pitch_core::{detect_pitch, midi_to_frequency, PitchResult, Yin, YinOptions, A4_DEFAULT};

const RATES: [f32; 2] = [44_100.0, 48_000.0];
const FRAME: usize = 2048;

fn assert_within_cents(result: PitchResult, expected: f32, tolerance: f32, context: &str) {
    assert!(!result.is_none(), "{context}: no pitch found");
    let err = cents_between(result.frequency, expected);
    assert!(
        err.abs() <= tolerance,
        "{context}: expected {expected:.2} Hz, got {:.2} Hz ({err:+.2} cents, tolerance {tolerance})",
        result.frequency
    );
}

#[test]
fn a440_sine_is_found_within_one_cent() {
    for sr in RATES {
        let buf = sine(440.0, sr, FRAME, 0.5);
        let r = detect_pitch(&buf, sr, YinOptions::default());
        assert_within_cents(r, 440.0, 1.0, &format!("{sr} Hz"));
        assert!(r.confidence > 0.95, "confidence {}", r.confidence);
    }
}

#[test]
fn every_semitone_from_e2_to_c7_within_three_cents() {
    // E2 (guitar low E) = MIDI 40, C7 = MIDI 96.
    for sr in RATES {
        let mut yin = Yin::new(FRAME, sr, YinOptions::default());
        for midi in 40..=96 {
            let f = midi_to_frequency(midi as f32, A4_DEFAULT);
            let buf = sine(f, sr, FRAME, 0.3);
            let r = yin.detect(&buf);
            assert_within_cents(r, f, 3.0, &format!("MIDI {midi} at {sr} Hz"));
        }
    }
}

#[test]
fn detuned_note_reports_the_actual_frequency_not_the_nearest_note() {
    // 452 Hz is ~46 cents sharp of A4. A tuner must report 452, not 440.
    let sr = 48_000.0;
    let buf = sine(452.0, sr, FRAME, 0.5);
    let r = detect_pitch(&buf, sr, YinOptions::default());
    assert_within_cents(r, 452.0, 1.0, "452 Hz");
}

#[test]
fn loud_second_harmonic_does_not_cause_an_octave_error() {
    // Many instruments (and voices) have a 2nd harmonic louder than the
    // fundamental. FFT peak-picking would report 220 Hz here; YIN must not.
    let sr = 48_000.0;
    let f = 110.0;
    let buf = harmonics(f, sr, FRAME, &[0.2, 0.5, 0.3, 0.15]);
    let r = detect_pitch(&buf, sr, YinOptions::default());
    assert_within_cents(r, f, 3.0, "110 Hz with loud 2nd harmonic");
}

#[test]
fn missing_fundamental_is_still_resolved_to_the_fundamental() {
    // Only harmonics 2..5 present. The period is still 1/f, so YIN reports f.
    let sr = 44_100.0;
    let f = 196.0; // G3
    let buf = harmonics(f, sr, FRAME, &[0.0, 0.4, 0.3, 0.2, 0.1]);
    let r = detect_pitch(&buf, sr, YinOptions::default());
    assert_within_cents(r, f, 3.0, "missing fundamental");
}

#[test]
fn sine_with_moderate_noise_is_still_found() {
    let sr = 48_000.0;
    let f = 329.63; // E4
    let buf = add(&sine(f, sr, FRAME, 0.4), &noise(FRAME, 0.08, 7));
    let r = detect_pitch(&buf, sr, YinOptions::default());
    assert_within_cents(r, f, 5.0, "E4 + noise");
    assert!(r.confidence > 0.8, "confidence {}", r.confidence);
}

#[test]
fn silence_gives_no_pitch_or_zero_confidence() {
    let sr = 48_000.0;
    let buf = vec![0.0f32; FRAME];
    let r = detect_pitch(&buf, sr, YinOptions::default());
    assert!(r.is_none() || r.confidence == 0.0, "got {r:?}");
}

#[test]
fn white_noise_has_low_confidence() {
    let sr = 48_000.0;
    let buf = noise(FRAME, 0.5, 42);
    let r = detect_pitch(&buf, sr, YinOptions::default());
    assert!(
        r.confidence < 0.5,
        "noise should not look periodic, got {r:?}"
    );
}

#[test]
fn frequency_below_min_frequency_is_not_reported_as_itself() {
    // 30 Hz is below the default 50 Hz floor. Whatever comes back, it must
    // not be a confident 30 Hz reading, because the search range excludes it.
    let sr = 48_000.0;
    let buf = sine(30.0, sr, FRAME, 0.5);
    let r = detect_pitch(&buf, sr, YinOptions::default());
    assert!(
        r.is_none() || r.frequency >= 45.0,
        "should not report below the search floor, got {r:?}"
    );
}

#[test]
fn amplitude_does_not_change_the_estimate() {
    let sr = 48_000.0;
    let f = 261.63; // C4
    let loud = detect_pitch(&sine(f, sr, FRAME, 0.9), sr, YinOptions::default());
    let quiet = detect_pitch(&sine(f, sr, FRAME, 0.02), sr, YinOptions::default());
    assert_within_cents(loud, f, 1.0, "loud");
    assert_within_cents(quiet, f, 1.0, "quiet");
    assert!((loud.frequency - quiet.frequency).abs() < 0.05);
}

#[test]
fn reusable_estimator_matches_one_shot_function() {
    let sr = 44_100.0;
    let buf = sine(523.25, sr, FRAME, 0.5);
    let mut yin = Yin::new(FRAME, sr, YinOptions::default());
    let a = yin.detect(&buf);
    let b = yin.detect(&buf); // state must not leak between calls
    let c = detect_pitch(&buf, sr, YinOptions::default());
    assert_eq!(a, b);
    assert_eq!(a, c);
}

#[test]
#[should_panic(expected = "buffer length mismatch")]
fn reusable_estimator_rejects_wrong_buffer_length() {
    let mut yin = Yin::new(FRAME, 48_000.0, YinOptions::default());
    yin.detect(&[0.0; 100]);
}
