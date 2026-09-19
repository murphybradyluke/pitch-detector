use pitch_core::{describe_frequency, frequency_to_midi, midi_to_frequency, A4_DEFAULT};

fn approx(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

#[test]
fn a4_is_midi_69_with_zero_cents() {
    let n = describe_frequency(440.0, A4_DEFAULT).unwrap();
    assert_eq!(n.midi, 69);
    assert_eq!(n.label(), "A4");
    assert!(approx(n.cents, 0.0, 1e-3));
    assert!(approx(n.target_frequency, 440.0, 1e-3));
}

#[test]
fn octaves_and_pitch_classes() {
    assert_eq!(
        describe_frequency(261.63, A4_DEFAULT).unwrap().label(),
        "C4"
    );
    assert_eq!(
        describe_frequency(130.81, A4_DEFAULT).unwrap().label(),
        "C3"
    );
    assert_eq!(describe_frequency(82.41, A4_DEFAULT).unwrap().label(), "E2");
    assert_eq!(
        describe_frequency(466.16, A4_DEFAULT).unwrap().label(),
        "A#4"
    );
    assert_eq!(describe_frequency(16.35, A4_DEFAULT).unwrap().label(), "C0");
}

#[test]
fn sharp_and_flat_cents_are_signed() {
    // 445 Hz is ~19.56 cents sharp of A4.
    let sharp = describe_frequency(445.0, A4_DEFAULT).unwrap();
    assert_eq!(sharp.midi, 69);
    assert!(approx(sharp.cents, 19.56, 0.05), "{}", sharp.cents);

    // 435 Hz is ~19.79 cents flat of A4.
    let flat = describe_frequency(435.0, A4_DEFAULT).unwrap();
    assert_eq!(flat.midi, 69);
    assert!(approx(flat.cents, -19.79, 0.05), "{}", flat.cents);
}

#[test]
fn quarter_tone_rounds_to_nearest_and_cents_stay_in_range() {
    // Exactly 50 cents sharp of A4 rounds up (f32::round is away from zero).
    let f = midi_to_frequency(69.5, A4_DEFAULT);
    let n = describe_frequency(f, A4_DEFAULT).unwrap();
    assert!(n.cents >= -50.0 && n.cents <= 50.0, "{}", n.cents);
    assert!(n.midi == 69 || n.midi == 70);
}

#[test]
fn alternate_reference_pitch() {
    let n = describe_frequency(442.0, 442.0).unwrap();
    assert_eq!(n.midi, 69);
    assert!(approx(n.cents, 0.0, 1e-3));
}

#[test]
fn midi_round_trips_through_frequency() {
    for midi in 0..128 {
        let f = midi_to_frequency(midi as f32, A4_DEFAULT);
        assert!(approx(frequency_to_midi(f, A4_DEFAULT), midi as f32, 1e-3));
    }
}

#[test]
fn invalid_frequencies_are_none() {
    assert!(describe_frequency(0.0, A4_DEFAULT).is_none());
    assert!(describe_frequency(-10.0, A4_DEFAULT).is_none());
    assert!(describe_frequency(f32::NAN, A4_DEFAULT).is_none());
    assert!(describe_frequency(f32::INFINITY, A4_DEFAULT).is_none());
}
