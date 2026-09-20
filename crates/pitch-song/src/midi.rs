//! Standard MIDI File import.
//!
//! Handles format 0 and 1 files with metrical timing. Tempo changes from any
//! track are merged into one tempo map so ticks convert to seconds correctly
//! even when a file puts them on a track other than the first.

use crate::NoteEvent;
use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug)]
pub enum MidiError {
    Parse(String),
    /// SMPTE timecode timing is not supported.
    Timecode,
    NoTracks,
}

impl fmt::Display for MidiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MidiError::Parse(e) => write!(f, "not a valid MIDI file: {e}"),
            MidiError::Timecode => write!(f, "SMPTE-timed MIDI files are not supported"),
            MidiError::NoTracks => write!(f, "MIDI file has no tracks"),
        }
    }
}

impl std::error::Error for MidiError {}

/// Summary of one part, for a track picker.
///
/// A part is one MIDI channel within one MIDI track. Format-1 files usually
/// have one channel per track, so parts and tracks coincide. Format-0 files
/// put every instrument in a single track, and splitting by channel is what
/// keeps the drums (channel 10, whose kick sits below any bass note) from
/// being mistaken for the lowest note of the bass line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    /// Index into [`MidiSong::tracks`] and the argument to [`MidiSong::events`].
    pub index: usize,
    /// MIDI track this part came from.
    pub midi_track: usize,
    /// MIDI channel, 0-based (9 is percussion).
    pub channel: u8,
    /// Track name from the file, with the instrument appended when a track
    /// holds several channels.
    pub name: String,
    pub note_count: usize,
    pub lowest_midi: u8,
    pub highest_midi: u8,
    /// General MIDI program number of the first program change on this channel.
    pub program: Option<u8>,
    /// True for channel 10 (percussion).
    pub is_drums: bool,
}

impl TrackInfo {
    /// General MIDI name of the program, or "Drums" for the percussion channel.
    pub fn program_name(&self) -> &'static str {
        if self.is_drums {
            "Drums"
        } else {
            self.program.map(gm_program_name).unwrap_or("")
        }
    }
}

/// General MIDI level 1 program names, indexed by program number.
pub fn gm_program_name(program: u8) -> &'static str {
    GM_PROGRAMS.get(program as usize).copied().unwrap_or("")
}

#[rustfmt::skip]
const GM_PROGRAMS: [&str; 128] = [
    "Acoustic Grand Piano", "Bright Acoustic Piano", "Electric Grand Piano", "Honky-tonk Piano",
    "Electric Piano 1", "Electric Piano 2", "Harpsichord", "Clavinet",
    "Celesta", "Glockenspiel", "Music Box", "Vibraphone", "Marimba", "Xylophone", "Tubular Bells", "Dulcimer",
    "Drawbar Organ", "Percussive Organ", "Rock Organ", "Church Organ", "Reed Organ", "Accordion", "Harmonica", "Tango Accordion",
    "Acoustic Guitar (nylon)", "Acoustic Guitar (steel)", "Electric Guitar (jazz)", "Electric Guitar (clean)",
    "Electric Guitar (muted)", "Overdriven Guitar", "Distortion Guitar", "Guitar Harmonics",
    "Acoustic Bass", "Electric Bass (finger)", "Electric Bass (pick)", "Fretless Bass",
    "Slap Bass 1", "Slap Bass 2", "Synth Bass 1", "Synth Bass 2",
    "Violin", "Viola", "Cello", "Contrabass", "Tremolo Strings", "Pizzicato Strings", "Orchestral Harp", "Timpani",
    "String Ensemble 1", "String Ensemble 2", "Synth Strings 1", "Synth Strings 2",
    "Choir Aahs", "Voice Oohs", "Synth Voice", "Orchestra Hit",
    "Trumpet", "Trombone", "Tuba", "Muted Trumpet", "French Horn", "Brass Section", "Synth Brass 1", "Synth Brass 2",
    "Soprano Sax", "Alto Sax", "Tenor Sax", "Baritone Sax", "Oboe", "English Horn", "Bassoon", "Clarinet",
    "Piccolo", "Flute", "Recorder", "Pan Flute", "Blown Bottle", "Shakuhachi", "Whistle", "Ocarina",
    "Lead 1 (square)", "Lead 2 (sawtooth)", "Lead 3 (calliope)", "Lead 4 (chiff)",
    "Lead 5 (charang)", "Lead 6 (voice)", "Lead 7 (fifths)", "Lead 8 (bass + lead)",
    "Pad 1 (new age)", "Pad 2 (warm)", "Pad 3 (polysynth)", "Pad 4 (choir)",
    "Pad 5 (bowed)", "Pad 6 (metallic)", "Pad 7 (halo)", "Pad 8 (sweep)",
    "FX 1 (rain)", "FX 2 (soundtrack)", "FX 3 (crystal)", "FX 4 (atmosphere)",
    "FX 5 (brightness)", "FX 6 (goblins)", "FX 7 (echoes)", "FX 8 (sci-fi)",
    "Sitar", "Banjo", "Shamisen", "Koto", "Kalimba", "Bag pipe", "Fiddle", "Shanai",
    "Tinkle Bell", "Agogo", "Steel Drums", "Woodblock", "Taiko Drum", "Melodic Tom", "Synth Drum", "Reverse Cymbal",
    "Guitar Fret Noise", "Breath Noise", "Seashore", "Bird Tweet", "Telephone Ring", "Helicopter", "Applause", "Gunshot",
];

/// A parsed MIDI file: tempo map plus note events per part.
#[derive(Debug, Clone)]
pub struct MidiSong {
    ticks_per_beat: u16,
    /// (tick, seconds at that tick, microseconds per beat from that tick).
    tempo: Vec<(u32, f64, u32)>,
    tracks: Vec<TrackInfo>,
    events: Vec<Vec<NoteEvent>>,
    end_tick: u32,
}

impl MidiSong {
    /// Parse, splitting each MIDI track into one part per channel.
    pub fn parse(bytes: &[u8]) -> Result<MidiSong, MidiError> {
        Self::parse_with(bytes, true)
    }

    /// Parse with control over channel splitting. Turn it off for files
    /// where one instrument deliberately spans channels, such as the MIDI
    /// alphaTab generates from Guitar Pro (bent notes go to a second
    /// channel of the same track).
    pub fn parse_with(bytes: &[u8], split_channels: bool) -> Result<MidiSong, MidiError> {
        let smf = Smf::parse(bytes).map_err(|e| MidiError::Parse(e.to_string()))?;
        let ticks_per_beat = match smf.header.timing {
            Timing::Metrical(t) => t.as_int(),
            Timing::Timecode(..) => return Err(MidiError::Timecode),
        };
        if smf.tracks.is_empty() {
            return Err(MidiError::NoTracks);
        }

        // Pass 1: tempo map and song length, across all tracks.
        let mut changes: Vec<(u32, u32)> = Vec::new();
        let mut end_tick = 0u32;
        for track in &smf.tracks {
            let mut tick = 0u32;
            for ev in track {
                tick += ev.delta.as_int();
                if let TrackEventKind::Meta(MetaMessage::Tempo(us)) = ev.kind {
                    changes.push((tick, us.as_int()));
                }
            }
            end_tick = end_tick.max(tick);
        }
        changes.sort_by_key(|c| c.0);
        let tempo = build_tempo_map(&changes, ticks_per_beat);
        let song = MidiSong {
            ticks_per_beat,
            tempo,
            tracks: Vec::new(),
            events: Vec::new(),
            end_tick,
        };

        // Pass 2: notes per track, split by channel into parts.
        let mut tracks = Vec::new();
        let mut events = Vec::new();
        for (midi_track, track) in smf.tracks.iter().enumerate() {
            for (mut info, notes) in song.read_track(midi_track, track, split_channels) {
                info.index = tracks.len();
                tracks.push(info);
                events.push(notes);
            }
        }
        Ok(MidiSong {
            tracks,
            events,
            ..song
        })
    }

    /// Notes of one MIDI track, grouped by channel (or all together when
    /// `split_channels` is false). Tracks without notes yield nothing.
    fn read_track(
        &self,
        midi_track: usize,
        track: &[midly::TrackEvent],
        split_channels: bool,
    ) -> Vec<(TrackInfo, Vec<NoteEvent>)> {
        let mut tick = 0u32;
        let mut name = String::new();
        let mut programs: HashMap<u8, u8> = HashMap::new();
        let mut open: HashMap<(u8, u8), (u32, u8)> = HashMap::new();
        let mut notes: Vec<(u8, NoteEvent)> = Vec::new();

        for ev in track {
            tick += ev.delta.as_int();
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::TrackName(n)) if name.is_empty() => {
                    name = String::from_utf8_lossy(n).trim().to_string();
                }
                TrackEventKind::Meta(MetaMessage::InstrumentName(n)) if name.is_empty() => {
                    name = String::from_utf8_lossy(n).trim().to_string();
                }
                TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        MidiMessage::ProgramChange { program } => {
                            programs.entry(ch).or_insert(program.as_int());
                        }
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            // A retriggered key ends the previous note.
                            if let Some((start, v)) = open.remove(&(ch, key.as_int())) {
                                notes.push((ch, self.note(key.as_int(), start, tick, v)));
                            }
                            open.insert((ch, key.as_int()), (tick, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some((start, v)) = open.remove(&(ch, key.as_int())) {
                                notes.push((ch, self.note(key.as_int(), start, tick, v)));
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        for ((ch, key), (start, v)) in open {
            notes.push((ch, self.note(key, start, tick, v)));
        }

        let mut channels: Vec<u8> = notes.iter().map(|(ch, _)| *ch).collect();
        channels.sort_unstable();
        channels.dedup();
        let multi = split_channels && channels.len() > 1;
        // Without splitting, the whole track is one part labelled by its
        // most-used channel, and it is drums only if every note is drums.
        let groups: Vec<(u8, bool)> = if split_channels {
            channels.iter().map(|&ch| (ch, ch == 9)).collect()
        } else if channels.is_empty() {
            Vec::new()
        } else {
            let main = *channels
                .iter()
                .max_by_key(|&&ch| notes.iter().filter(|(c, _)| *c == ch).count())
                .unwrap();
            vec![(main, channels.iter().all(|&ch| ch == 9))]
        };

        groups
            .into_iter()
            .map(|(ch, is_drums)| {
                let mut part: Vec<NoteEvent> = notes
                    .iter()
                    .filter(|(c, _)| !split_channels || *c == ch)
                    .map(|(_, n)| *n)
                    .collect();
                part.sort_by(|a, b| {
                    a.start_secs
                        .partial_cmp(&b.start_secs)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let mut info = TrackInfo {
                    index: 0,
                    midi_track,
                    channel: ch,
                    name: String::new(),
                    note_count: part.len(),
                    lowest_midi: part.iter().map(|n| n.midi).min().unwrap_or(0),
                    highest_midi: part.iter().map(|n| n.midi).max().unwrap_or(0),
                    program: programs.get(&ch).copied(),
                    is_drums,
                };
                let base = if name.is_empty() {
                    format!("Track {}", midi_track + 1)
                } else {
                    name.clone()
                };
                info.name = if multi {
                    let instrument = info.program_name();
                    if instrument.is_empty() {
                        format!("{base} ch{}", ch + 1)
                    } else {
                        format!("{base} · {instrument}")
                    }
                } else {
                    base
                };
                (info, part)
            })
            .collect()
    }

    fn note(&self, midi: u8, start_tick: u32, end_tick: u32, velocity: u8) -> NoteEvent {
        let start = self.secs_at(start_tick);
        let end = self.secs_at(end_tick.max(start_tick));
        NoteEvent {
            midi,
            start_secs: start as f32,
            duration_secs: (end - start) as f32,
            velocity,
        }
    }

    /// Seconds from the start of the file to `tick`.
    pub fn secs_at(&self, tick: u32) -> f64 {
        let idx = self
            .tempo
            .partition_point(|&(t, _, _)| t <= tick)
            .saturating_sub(1);
        let (t0, s0, uspb) = self.tempo[idx];
        s0 + (tick - t0) as f64 * uspb as f64 / 1e6 / self.ticks_per_beat as f64
    }

    /// Parts (one per channel per MIDI track) that contain notes.
    pub fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    /// Raw (possibly polyphonic) notes of a part, by [`TrackInfo::index`].
    pub fn events(&self, track: usize) -> &[NoteEvent] {
        self.events.get(track).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn duration_secs(&self) -> f32 {
        self.secs_at(self.end_tick) as f32
    }

    /// Time of every beat from the start to the end of the song.
    pub fn beat_times(&self) -> Vec<f32> {
        let mut out = Vec::new();
        let mut tick = 0u32;
        while tick <= self.end_tick {
            out.push(self.secs_at(tick) as f32);
            tick += self.ticks_per_beat as u32;
        }
        out
    }

    /// Tempo in beats per minute at the start of the song.
    pub fn initial_bpm(&self) -> f32 {
        60_000_000.0 / self.tempo[0].2 as f32
    }
}

fn build_tempo_map(changes: &[(u32, u32)], ticks_per_beat: u16) -> Vec<(u32, f64, u32)> {
    const DEFAULT_USPB: u32 = 500_000; // 120 bpm
    let mut map: Vec<(u32, f64, u32)> = vec![(0, 0.0, DEFAULT_USPB)];
    for &(tick, uspb) in changes {
        let (t0, s0, prev) = *map.last().unwrap();
        if tick == t0 {
            map.last_mut().unwrap().2 = uspb;
            continue;
        }
        let secs = s0 + (tick - t0) as f64 * prev as f64 / 1e6 / ticks_per_beat as f64;
        map.push((tick, secs, uspb));
    }
    map
}
