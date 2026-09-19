mod common;

use common::*;
use pitch_core::{midi_to_frequency, Instrument, Yin, A4_DEFAULT};

const SR: f32 = 48_000.0;

/// Bass and guitar pickups roll off the fundamental; the 2nd and 3rd
/// harmonics usually dominate. YIN must find the period from the harmonics.
const WEAK_FUNDAMENTAL: [f32; 6] = [0.1, 0.4, 0.3, 0.2, 0.1, 0.05];
const NEARLY_MISSING_FUNDAMENTAL: [f32; 5] = [0.02, 0.4, 0.35, 0.2, 0.1];

/// Every semitone in the preset's range, with a little noise, must resolve
/// within 3 cents with high confidence using the preset's own frame length.
fn assert_preset_covers_its_range(preset: Instrument, spectrum: &[f32]) {
    let frame = preset.frame_len(SR);
    let mut yin = Yin::new(frame, SR, preset.yin_options(0.15, A4_DEFAULT));
    for midi in preset.lowest_midi..=preset.highest_midi {
        let f = midi_to_frequency(midi as f32, A4_DEFAULT);
        let buf = add(
            &harmonics(f, SR, frame, spectrum),
            &noise(frame, 0.02, midi as u64),
        );
        let r = yin.detect(&buf);
        let err = cents_between(r.frequency, f);
        assert!(
            err.abs() <= 3.0,
            "{}: MIDI {midi} ({f:.1} Hz) with frame {frame}: got {:.2} Hz ({err:+.1} cents)",
            preset.name,
            r.frequency
        );
        assert!(
            r.confidence > 0.9,
            "{}: MIDI {midi}: confidence {}",
            preset.name,
            r.confidence
        );
    }
}

#[test]
fn every_preset_covers_its_range_with_a_weak_fundamental() {
    for preset in Instrument::PRESETS {
        assert_preset_covers_its_range(preset, &WEAK_FUNDAMENTAL);
    }
}

#[test]
fn bass_presets_cope_with_a_nearly_missing_fundamental() {
    for preset in [
        Instrument::BASS_5_DROP_A,
        Instrument::BASS_5,
        Instrument::BASS_4,
    ] {
        assert_preset_covers_its_range(preset, &NEARLY_MISSING_FUNDAMENTAL);
    }
}

#[test]
fn frame_lengths_scale_with_the_lowest_note() {
    let guitar = Instrument::GUITAR.frame_len(SR);
    let bass4 = Instrument::BASS_4.frame_len(SR);
    let bass5a = Instrument::BASS_5_DROP_A.frame_len(SR);
    assert!(
        guitar < bass4 && bass4 < bass5a,
        "{guitar} {bass4} {bass5a}"
    );
    // Guitar should be around 30 ms, drop-A bass around 80 ms.
    assert!((25.0..35.0).contains(&(Instrument::GUITAR.frame_secs(SR) * 1000.0)));
    assert!((75.0..90.0).contains(&(Instrument::BASS_5_DROP_A.frame_secs(SR) * 1000.0)));
    for p in Instrument::PRESETS {
        assert_eq!(
            p.frame_len(SR) % 128,
            0,
            "{}: frame not a multiple of 128",
            p.name
        );
    }
}

#[test]
fn search_range_extends_a_semitone_past_each_end() {
    let g = Instrument::GUITAR;
    let e2 = midi_to_frequency(40.0, A4_DEFAULT);
    let e6 = midi_to_frequency(88.0, A4_DEFAULT);
    assert!(g.min_frequency(A4_DEFAULT) < e2 && g.min_frequency(A4_DEFAULT) > e2 * 0.9);
    assert!(g.max_frequency(A4_DEFAULT) > e6 && g.max_frequency(A4_DEFAULT) < e6 * 1.1);
}

#[test]
fn a_string_tuned_forty_cents_flat_still_registers_on_the_lowest_note() {
    let p = Instrument::BASS_5_DROP_A;
    let frame = p.frame_len(SR);
    let mut yin = Yin::new(frame, SR, p.yin_options(0.15, A4_DEFAULT));
    let f = midi_to_frequency(21.0 - 0.4, A4_DEFAULT);
    let buf = harmonics(f, SR, frame, &WEAK_FUNDAMENTAL);
    let r = yin.detect(&buf);
    assert!(
        cents_between(r.frequency, f).abs() < 3.0,
        "got {}",
        r.frequency
    );
}
