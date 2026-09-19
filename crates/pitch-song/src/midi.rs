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

/// Summary of one track, for a track picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    pub index: usize,
    pub name: String,
    pub note_count: usize,
    pub lowest_midi: u8,
    pub highest_midi: u8,
    /// General MIDI program number of the first program change, if any.
    pub program: Option<u8>,
    /// True when every note is on channel 10 (percussion).
    pub is_drums: bool,
}

/// A parsed MIDI file: tempo map plus per-track note events.
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
    pub fn parse(bytes: &[u8]) -> Result<MidiSong, MidiError> {
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

        // Pass 2: notes per track.
        let mut tracks = Vec::new();
        let mut events = Vec::new();
        for (index, track) in smf.tracks.iter().enumerate() {
            let (info, notes) = song.read_track(index, track);
            tracks.push(info);
            events.push(notes);
        }
        Ok(MidiSong {
            tracks,
            events,
            ..song
        })
    }

    fn read_track(&self, index: usize, track: &[midly::TrackEvent]) -> (TrackInfo, Vec<NoteEvent>) {
        let mut tick = 0u32;
        let mut name = String::new();
        let mut program = None;
        let mut open: HashMap<(u8, u8), (u32, u8)> = HashMap::new();
        let mut notes = Vec::new();
        let mut channels_seen = [false; 16];

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
                        MidiMessage::ProgramChange { program: p } => {
                            if program.is_none() {
                                program = Some(p.as_int());
                            }
                        }
                        MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                            channels_seen[ch as usize] = true;
                            // A retriggered key ends the previous note.
                            if let Some((start, v)) = open.remove(&(ch, key.as_int())) {
                                notes.push(self.note(key.as_int(), start, tick, v));
                            }
                            open.insert((ch, key.as_int()), (tick, vel.as_int()));
                        }
                        MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                            if let Some((start, v)) = open.remove(&(ch, key.as_int())) {
                                notes.push(self.note(key.as_int(), start, tick, v));
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        for ((_, key), (start, v)) in open {
            notes.push(self.note(key, start, tick, v));
        }
        notes.sort_by(|a, b| {
            a.start_secs
                .partial_cmp(&b.start_secs)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let only_drums =
            channels_seen[9] && channels_seen.iter().enumerate().all(|(c, &s)| !s || c == 9);
        let info = TrackInfo {
            index,
            name: if name.is_empty() {
                format!("Track {}", index + 1)
            } else {
                name
            },
            note_count: notes.len(),
            lowest_midi: notes.iter().map(|n| n.midi).min().unwrap_or(0),
            highest_midi: notes.iter().map(|n| n.midi).max().unwrap_or(0),
            program,
            is_drums: only_drums && !notes.is_empty(),
        };
        (info, notes)
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

    pub fn tracks(&self) -> &[TrackInfo] {
        &self.tracks
    }

    /// Raw (possibly polyphonic) notes of a track.
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
