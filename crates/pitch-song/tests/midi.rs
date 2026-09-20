use midly::{
    num::*, Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind,
};
use pitch_song::{monophonic, MidiSong, NoteEvent};

const TPB: u16 = 480;

fn note_on(delta: u32, ch: u8, key: u8, vel: u8) -> TrackEvent<'static> {
    TrackEvent {
        delta: u28::new(delta),
        kind: TrackEventKind::Midi {
            channel: u4::new(ch),
            message: MidiMessage::NoteOn {
                key: u7::new(key),
                vel: u7::new(vel),
            },
        },
    }
}

fn note_off(delta: u32, ch: u8, key: u8) -> TrackEvent<'static> {
    TrackEvent {
        delta: u28::new(delta),
        kind: TrackEventKind::Midi {
            channel: u4::new(ch),
            message: MidiMessage::NoteOff {
                key: u7::new(key),
                vel: u7::new(0),
            },
        },
    }
}

fn meta(delta: u32, m: MetaMessage<'static>) -> TrackEvent<'static> {
    TrackEvent {
        delta: u28::new(delta),
        kind: TrackEventKind::Meta(m),
    }
}

fn tempo(delta: u32, bpm: u32) -> TrackEvent<'static> {
    meta(delta, MetaMessage::Tempo(u24::new(60_000_000 / bpm)))
}

fn write(smf: &Smf) -> Vec<u8> {
    let mut out = Vec::new();
    smf.write(&mut out).unwrap();
    out
}

/// A format-1 file: tempo track, then a bass line E1 A1 D2 G2, quarter notes at 120 bpm.
fn bass_file(bpm: u32) -> Vec<u8> {
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(TPB)),
    ));
    smf.tracks
        .push(vec![tempo(0, bpm), meta(0, MetaMessage::EndOfTrack)]);
    let mut bass = vec![meta(0, MetaMessage::TrackName(b"Bass"))];
    bass.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Midi {
            channel: u4::new(1),
            message: MidiMessage::ProgramChange {
                program: u7::new(33),
            },
        },
    });
    for key in [28u8, 33, 38, 43] {
        bass.push(note_on(0, 1, key, 100));
        bass.push(note_off(TPB as u32, 1, key));
    }
    bass.push(meta(0, MetaMessage::EndOfTrack));
    smf.tracks.push(bass);
    write(&smf)
}

#[test]
fn parses_a_bass_line_with_names_and_timing() {
    let song = MidiSong::parse(&bass_file(120)).unwrap();
    // The tempo track has no notes, so only the bass part is listed.
    assert_eq!(song.tracks().len(), 1);
    let bass = &song.tracks()[0];
    assert_eq!(bass.name, "Bass");
    assert_eq!((bass.midi_track, bass.channel), (1, 1));
    assert_eq!(bass.program_name(), "Electric Bass (finger)");
    assert_eq!(bass.note_count, 4);
    assert_eq!((bass.lowest_midi, bass.highest_midi), (28, 43));
    assert_eq!(bass.program, Some(33));
    assert!(!bass.is_drums);
    assert!((song.initial_bpm() - 120.0).abs() < 1e-3);

    let ev = song.events(0);
    let starts: Vec<f32> = ev.iter().map(|e| e.start_secs).collect();
    assert_eq!(starts, vec![0.0, 0.5, 1.0, 1.5]);
    assert!(ev.iter().all(|e| (e.duration_secs - 0.5).abs() < 1e-6));
    assert_eq!(ev[0].midi, 28);
    assert!((song.duration_secs() - 2.0).abs() < 1e-6);
}

#[test]
fn tempo_changes_mid_song_are_honoured() {
    // 120 bpm for one beat, then 60 bpm.
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(TPB)),
    ));
    smf.tracks.push(vec![
        tempo(0, 120),
        tempo(TPB as u32, 60),
        meta(0, MetaMessage::EndOfTrack),
    ]);
    smf.tracks.push(vec![
        note_on(0, 0, 40, 90),
        note_off(TPB as u32, 0, 40), // beat 1: 0.5 s long
        note_on(0, 0, 45, 90),
        note_off(TPB as u32, 0, 45), // beat 2: 1.0 s long
        meta(0, MetaMessage::EndOfTrack),
    ]);
    let song = MidiSong::parse(&write(&smf)).unwrap();
    let ev = song.events(0);
    assert!((ev[0].duration_secs - 0.5).abs() < 1e-6);
    assert!((ev[1].start_secs - 0.5).abs() < 1e-6);
    assert!((ev[1].duration_secs - 1.0).abs() < 1e-6);
    let beats = song.beat_times();
    assert_eq!(beats.len(), 3);
    assert!((beats[1] - 0.5).abs() < 1e-6 && (beats[2] - 1.5).abs() < 1e-6);
}

#[test]
fn note_on_with_zero_velocity_ends_a_note_and_drums_are_flagged() {
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(TPB)),
    ));
    smf.tracks.push(vec![
        note_on(0, 9, 36, 100),
        note_on(240, 9, 36, 0),
        meta(0, MetaMessage::EndOfTrack),
    ]);
    let song = MidiSong::parse(&write(&smf)).unwrap();
    assert!(song.tracks()[0].is_drums);
    assert!((song.events(0)[0].duration_secs - 0.25).abs() < 1e-6);
}

#[test]
fn format_0_file_is_split_by_channel_so_drums_do_not_swallow_the_bass() {
    // One track holding a piano chord (ch 0), a bass line (ch 1) and a kick
    // drum (ch 9) all starting together. Before channel splitting, the kick
    // at MIDI 36 was the "lowest note of the chord" and the bass vanished.
    let mut smf = Smf::new(Header::new(
        Format::SingleTrack,
        Timing::Metrical(u15::new(TPB)),
    ));
    let mut t = vec![meta(0, MetaMessage::TrackName(b"Song"))];
    for (ch, prog) in [(0u8, 0u8), (1, 34)] {
        t.push(TrackEvent {
            delta: u28::new(0),
            kind: TrackEventKind::Midi {
                channel: u4::new(ch),
                message: MidiMessage::ProgramChange {
                    program: u7::new(prog),
                },
            },
        });
    }
    for beat in 0..4u32 {
        let bass_key = 28 + beat as u8 * 5;
        t.push(note_on(0, 0, 60, 80));
        t.push(note_on(0, 0, 64, 80));
        t.push(note_on(0, 1, bass_key, 100));
        t.push(note_on(0, 9, 36, 110));
        t.push(note_off(TPB as u32 / 2, 9, 36));
        t.push(note_off(TPB as u32 / 2, 0, 60));
        t.push(note_off(0, 0, 64));
        t.push(note_off(0, 1, bass_key));
    }
    t.push(meta(0, MetaMessage::EndOfTrack));
    smf.tracks.push(t);
    let song = MidiSong::parse(&write(&smf)).unwrap();

    let parts: Vec<(u8, &str, usize, bool)> = song
        .tracks()
        .iter()
        .map(|p| (p.channel, p.name.as_str(), p.note_count, p.is_drums))
        .collect();
    assert_eq!(
        parts,
        vec![
            (0, "Song · Acoustic Grand Piano", 8, false),
            (1, "Song · Electric Bass (pick)", 4, false),
            (9, "Song · Drums", 4, true),
        ]
    );
    let bass = song.events(1);
    assert_eq!(
        bass.iter().map(|e| e.midi).collect::<Vec<_>>(),
        vec![28, 33, 38, 43]
    );
    assert_eq!(
        monophonic(bass, 0.03).len(),
        4,
        "every bass note survives reduction"
    );
}

#[test]
fn without_splitting_a_two_channel_track_stays_one_part() {
    // Like alphaTab's output: a bass track whose bent note sits on channel 1.
    let mut smf = Smf::new(Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(TPB)),
    ));
    smf.tracks.push(vec![
        meta(0, MetaMessage::TrackName(b"Bass")),
        note_on(0, 0, 28, 100),
        note_off(TPB as u32, 0, 28),
        note_on(0, 1, 33, 100),
        note_off(TPB as u32, 1, 33),
        note_on(0, 0, 38, 100),
        note_off(TPB as u32, 0, 38),
        meta(0, MetaMessage::EndOfTrack),
    ]);
    let bytes = write(&smf);
    let split = MidiSong::parse(&bytes).unwrap();
    assert_eq!(split.tracks().len(), 2);
    let whole = MidiSong::parse_with(&bytes, false).unwrap();
    assert_eq!(whole.tracks().len(), 1);
    assert_eq!(whole.tracks()[0].name, "Bass");
    assert_eq!(
        whole.tracks()[0].channel,
        0,
        "labelled by the busier channel"
    );
    assert!(!whole.tracks()[0].is_drums);
    assert_eq!(
        whole.events(0).iter().map(|e| e.midi).collect::<Vec<_>>(),
        vec![28, 33, 38]
    );
}

#[test]
fn garbage_is_rejected() {
    assert!(MidiSong::parse(b"not midi").is_err());
}

fn ev(midi: u8, start: f32, dur: f32) -> NoteEvent {
    NoteEvent {
        midi,
        start_secs: start,
        duration_secs: dur,
        velocity: 100,
    }
}

#[test]
fn monophonic_keeps_the_lowest_note_of_a_chord() {
    // Power chord E2 + B2 + E3, then a single G2.
    let poly = vec![
        ev(52, 0.0, 1.0),
        ev(40, 0.0, 1.0),
        ev(47, 0.01, 1.0),
        ev(43, 1.0, 0.5),
    ];
    let mono = monophonic(&poly, 0.03);
    assert_eq!(
        mono.iter().map(|e| e.midi).collect::<Vec<_>>(),
        vec![40, 43]
    );
}

#[test]
fn monophonic_clips_overlaps_to_the_next_note() {
    let ringing = vec![ev(28, 0.0, 2.0), ev(33, 0.5, 0.5)];
    let mono = monophonic(&ringing, 0.03);
    assert!((mono[0].duration_secs - 0.5).abs() < 1e-6);
    assert!((mono[1].duration_secs - 0.5).abs() < 1e-6);
}
