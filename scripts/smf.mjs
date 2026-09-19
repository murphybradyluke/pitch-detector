// Minimal Standard MIDI File writer for tests: format 0, one track.
// notes: [{ midi, startBeats, durBeats }], sorted by start, non-overlapping.
export function writeSmf({ bpm = 120, division = 480, name = "Test", notes }) {
  const varlen = (n) => {
    const bytes = [n & 0x7f];
    while ((n >>= 7) > 0) bytes.unshift((n & 0x7f) | 0x80);
    return bytes;
  };
  const events = []; // { tick, bytes }
  const us = Math.round(60_000_000 / bpm);
  events.push({ tick: 0, bytes: [0xff, 0x51, 0x03, (us >> 16) & 0xff, (us >> 8) & 0xff, us & 0xff] });
  events.push({ tick: 0, bytes: [0xff, 0x03, name.length, ...[...name].map((c) => c.charCodeAt(0))] });
  events.push({ tick: 0, bytes: [0xc0, 33] }); // program 33 = electric bass
  for (const n of notes) {
    const on = Math.round(n.startBeats * division), off = Math.round((n.startBeats + n.durBeats) * division);
    events.push({ tick: on, bytes: [0x90, n.midi, 100] });
    events.push({ tick: off, bytes: [0x80, n.midi, 0] });
  }
  events.sort((a, b) => a.tick - b.tick || (a.bytes[0] === 0x80 ? -1 : 1));
  const track = [];
  let last = 0;
  for (const e of events) { track.push(...varlen(e.tick - last), ...e.bytes); last = e.tick; }
  track.push(0, 0xff, 0x2f, 0);
  const u32 = (n) => [(n >>> 24) & 0xff, (n >>> 16) & 0xff, (n >>> 8) & 0xff, n & 0xff];
  const u16 = (n) => [(n >> 8) & 0xff, n & 0xff];
  return Uint8Array.from([
    ...[0x4d, 0x54, 0x68, 0x64], ...u32(6), ...u16(0), ...u16(1), ...u16(division),
    ...[0x4d, 0x54, 0x72, 0x6b], ...u32(track.length), ...track,
  ]);
}
