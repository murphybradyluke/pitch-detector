use pitch_song::{NoteEvent, NoteOutcome, Scorer, ScorerOptions};

const HOP: f32 = 256.0 / 48_000.0;

fn ev(midi: u8, start: f32, dur: f32) -> NoteEvent {
    NoteEvent {
        midi,
        start_secs: start,
        duration_secs: dur,
        velocity: 100,
    }
}

/// Bass line: E1 A1 D2 G2, half a second each.
fn line() -> Vec<NoteEvent> {
    vec![
        ev(28, 0.0, 0.5),
        ev(33, 0.5, 0.5),
        ev(38, 1.0, 0.5),
        ev(43, 1.5, 0.5),
    ]
}

/// Simulate a player: `played(t)` gives what the detector hears at time t.
fn run(scorer: &mut Scorer, until: f32, played: impl Fn(f32) -> Option<u8>) -> Vec<NoteOutcome> {
    let mut out = Vec::new();
    let mut t = 0.0;
    while t <= until {
        out.extend(scorer.feed(t, played(t)));
        t += HOP;
    }
    out.extend(scorer.finish());
    out
}

#[test]
fn perfect_performance_hits_everything() {
    let mut s = Scorer::new(line(), ScorerOptions::default());
    let out = run(&mut s, 2.2, |t| {
        line()
            .iter()
            .find(|n| t >= n.start_secs && t < n.end_secs())
            .map(|n| n.midi)
    });
    assert_eq!(out.len(), 4);
    assert!(out.iter().all(|o| o.hit), "{out:?}");
    assert_eq!(s.hits(), 4);
    assert!(out[0].onset_offset_secs.unwrap().abs() < HOP);
}

#[test]
fn wrong_note_is_a_miss_and_names_what_was_heard() {
    let mut s = Scorer::new(line(), ScorerOptions::default());
    // Plays A1 (33) for the whole song.
    let out = run(&mut s, 2.2, |_| Some(33));
    assert!(!out[0].hit && out[0].wrong_midi == Some(33));
    assert!(out[1].hit);
    assert!(!out[2].hit && !out[3].hit);
    assert_eq!(s.hits(), 1);
}

#[test]
fn silence_is_a_miss_with_no_wrong_note() {
    let mut s = Scorer::new(line(), ScorerOptions::default());
    let out = run(&mut s, 2.2, |_| None);
    assert!(out
        .iter()
        .all(|o| !o.hit && o.wrong_midi.is_none() && o.onset_offset_secs.is_none()));
}

#[test]
fn playing_slightly_early_or_late_still_hits() {
    // Every note 80 ms late (within the 100 ms tolerance).
    let mut s = Scorer::new(line(), ScorerOptions::default());
    let out = run(&mut s, 2.3, |t| {
        let t = t - 0.08;
        line()
            .iter()
            .find(|n| t >= n.start_secs && t < n.end_secs())
            .map(|n| n.midi)
    });
    assert!(out.iter().all(|o| o.hit), "{out:?}");
    assert!((out[1].onset_offset_secs.unwrap() - 0.08).abs() < 2.0 * HOP);
}

#[test]
fn a_brief_flicker_of_the_right_note_is_not_a_hit() {
    let mut s = Scorer::new(line(), ScorerOptions::default());
    // Correct note heard for only two readings (~10 ms) per note.
    let out = run(&mut s, 2.2, |t| {
        let n = line()
            .into_iter()
            .find(|n| t >= n.start_secs && t < n.end_secs())?;
        if t - n.start_secs < 2.0 * HOP {
            Some(n.midi)
        } else {
            None
        }
    });
    assert!(out.iter().all(|o| !o.hit), "{out:?}");
}

#[test]
fn short_notes_need_proportionally_less_matched_time() {
    // 60 ms notes: need 30 ms matched, not the full 60 ms minimum.
    let fast: Vec<NoteEvent> = (0..4).map(|i| ev(40 + i, i as f32 * 0.06, 0.06)).collect();
    let mut s = Scorer::new(fast.clone(), ScorerOptions::default());
    let out = run(&mut s, 0.5, |t| {
        fast.iter()
            .find(|n| t >= n.start_secs && t < n.end_secs())
            .map(|n| n.midi)
    });
    assert!(out.iter().all(|o| o.hit), "{out:?}");
}

#[test]
fn latency_option_shifts_reading_times() {
    // Player is exact, but readings arrive 100 ms late from the detector.
    let opts = ScorerOptions {
        latency_secs: 0.1,
        ..Default::default()
    };
    let mut s = Scorer::new(line(), opts);
    let out = run(&mut s, 2.3, |t| {
        let t = t - 0.1;
        line()
            .iter()
            .find(|n| t >= n.start_secs && t < n.end_secs())
            .map(|n| n.midi)
    });
    assert!(out.iter().all(|o| o.hit));
    assert!(
        out[2].onset_offset_secs.unwrap().abs() < 2.0 * HOP,
        "{:?}",
        out[2]
    );
}

#[test]
fn outcomes_arrive_in_order_as_windows_close() {
    let mut s = Scorer::new(line(), ScorerOptions::default());
    let mut t = 0.0;
    while t < 0.4 {
        assert!(s.feed(t, Some(28)).is_empty(), "window still open at {t}");
        t += HOP;
    }
    let closed = s.feed(0.7, Some(33));
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].index, 0);
    assert!(closed[0].hit);
    assert_eq!(s.finished(), 1);
}
