//! Bounded, actual tick locations, wire-compatible with upstream HistoricalObject.
//! This is a delivery buffer for the latest batch, not an unlimited audit log.
use crate::{MotionHistory, Player, Snapshot};

struct Field {
    raw: u32,
    initial: i64,
    times: Vec<u64>,
    values: Vec<i64>,
}
struct Resident {
    id: u64,
    fields: Vec<Field>,
}
pub struct Capture {
    start: u64,
    residents: Vec<Resident>,
}

fn raw(p: &Player, field: usize) -> u32 {
    match field {
        0 => p.x,
        1 => p.y,
        2 => p.face_dx,
        3 => p.face_dy,
        _ => p.speed,
    }
}
fn quantized(value: u32, field: usize) -> i64 {
    let factor = if field == 4 { 65536 } else { 256 };
    let v = if field == 2 || field == 3 {
        value as i64 - 1000
    } else {
        value as i64
    };
    let n = v * factor;
    if n < 0 { (n - 999) / 1000 } else { n / 1000 }
}
impl Capture {
    pub(crate) fn new(s: &Snapshot) -> Self {
        let mut residents = Vec::new();
        let mut i = 0;
        while i < s.players.len() {
            let p = &s.players[i];
            let mut fields = Vec::new();
            let mut f = 0;
            while f < 5 {
                fields.push(Field {
                    raw: raw(p, f),
                    initial: quantized(raw(p, f), f),
                    times: Vec::new(),
                    values: Vec::new(),
                });
                f += 1;
            }
            residents.push(Resident { id: p.id, fields });
            i += 1;
        }
        Self {
            start: s.meta.now_ms,
            residents,
        }
    }
    pub(crate) fn record(&mut self, s: &Snapshot) {
        let mut i = 0;
        let mut cursor = 0;
        // Ticks only remove residents, preserving their order (idle humans).
        while i < s.players.len() {
            let id = s.players[i].id;
            cursor = resident_index(&self.residents, id, cursor);
            if cursor < self.residents.len() {
                let mut f = 0;
                while f < 5 {
                    let field = &mut self.residents[cursor].fields[f];
                    let value = raw(&s.players[i], f);
                    if field.raw != value {
                        field.times.push(s.meta.now_ms);
                        field.values.push(quantized(value, f));
                        field.raw = value;
                    }
                    f += 1;
                }
            }
            i += 1;
        }
    }
    pub fn finish(self, end: u64) -> MotionHistory {
        let mut data = Vec::new();
        let mut i = 0;
        while i < self.residents.len() {
            let resident = &self.residents[i];
            let packet = pack(resident);
            if packet.len() > 4 {
                little(&mut data, resident.id, 8);
                little(&mut data, packet.len() as u64, 4);
                let mut j = 0;
                while j < packet.len() {
                    data.push(packet[j]);
                    j += 1;
                }
            }
            i += 1;
        }
        MotionHistory {
            start_ms: self.start,
            end_ms: end,
            data,
        }
    }
}
fn little(out: &mut Vec<u8>, mut value: u64, bytes: usize) {
    let mut i = 0;
    while i < bytes {
        out.push((value & 255) as u8);
        value >>= 8;
        i += 1;
    }
}
fn compressed(values: &Vec<i64>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < values.len() {
        let v = values[i];
        let mut n = if v < 0 {
            ((-v) as u64) * 2 - 1
        } else {
            v as u64 * 2
        };
        while n >= 128 {
            out.push(((n & 127) | 128) as u8);
            n >>= 7;
        }
        out.push(n as u8);
        i += 1;
    }
    out
}
fn rle(values: &Vec<i64>) -> Vec<i64> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < values.len() {
        let value = values[i];
        let mut count = 1;
        i += 1;
        while i < values.len() && values[i] == value {
            count += 1;
            i += 1;
        }
        out.push(value);
        out.push(count);
    }
    out
}
fn pack(resident: &Resident) -> Vec<u8> {
    // xxHash32 of upstream's version-1 locationFields configuration.
    let mut out = Vec::new();
    little(&mut out, 134607547, 4);
    let mut f = 0;
    while f < resident.fields.len() {
        pack_field(&resident.fields[f], f, &mut out);
        f += 1;
    }
    out
}

fn resident_index(residents: &Vec<Resident>, id: u64, start: usize) -> usize {
    let mut cursor = start;
    while cursor < residents.len() {
        if residents[cursor].id == id {
            return cursor;
        }
        cursor += 1;
    }
    cursor
}
fn pack_field(field: &Field, f: usize, out: &mut Vec<u8>) {
    if field.times.len() > 0 {
        let mut timestamps = Vec::new();
        let mut i = 1;
        while i < field.times.len() {
            let current = field.times[i];
            let previous = field.times[i - 1];
            timestamps.push((current - previous) as i64);
            i += 1;
        }
        let times = compressed(&rle(&timestamps));
        let mut values = Vec::new();
        values.push(field.initial);
        let mut previous = field.initial;
        i = 0;
        while i < field.values.len() {
            values.push(field.values[i] - previous);
            previous = field.values[i];
            i += 1;
        }
        let runs = rle(&values);
        let use_rle = runs.len() < values.len();
        let bytes = if use_rle {
            compressed(&runs)
        } else {
            compressed(&values)
        };
        out.push(f as u8 | if use_rle { 16 } else { 0 });
        little(out, field.times[0], 8);
        little(out, times.len() as u64, 2);
        i = 0;
        while i < times.len() {
            out.push(times[i]);
            i += 1;
        }
        little(out, bytes.len() as u64, 2);
        i = 0;
        while i < bytes.len() {
            out.push(bytes[i]);
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn player(id: u64) -> Player {
        Player {
            id,
            owner: 1,
            is_human: false,
            x: 0,
            y: 0,
            face_dx: 2000,
            face_dy: 1000,
            speed: 0,
            dest_x: 0,
            dest_y: 0,
            pf: 0,
            pf_started: 0,
            wait_until: 0,
            last_input: 0,
            activity: 0,
            activity_until: 0,
            character: 0,
            name: Vec::new(),
        }
    }
    #[test]
    fn packets_match_upstream_reference_vectors() {
        let mut s = Snapshot::default();
        s.meta.now_ms = 1_000_000;
        s.players.push(Player {
            id: 1,
            x: 1000,
            face_dx: 2000,
            face_dy: 1000,
            ..player(1)
        });
        let mut c = Capture::new(&s);
        s.meta.now_ms += 16;
        s.players[0].x = 1012;
        c.record(&s);
        s.meta.now_ms += 16;
        s.players[0].x = 1024;
        c.record(&s);
        let h = c.finish(s.meta.now_ms);
        assert_eq!(
            &h.data[12..],
            &[
                187, 242, 5, 8, 0, 80, 66, 15, 0, 0, 0, 0, 0, 2, 0, 32, 2, 4, 0, 128, 4, 6, 6
            ]
        );
        s.players[0].x = 1000;
        s.meta.now_ms = 1_000_000;
        let mut c = Capture::new(&s);
        s.meta.now_ms += 16;
        s.players[0].face_dx = 0;
        s.players[0].speed = 750;
        c.record(&s);
        s.meta.now_ms += 16;
        s.players[0].speed = 0;
        c.record(&s);
        let h = c.finish(s.meta.now_ms);
        assert_eq!(
            &h.data[12..],
            &[
                187, 242, 5, 8, 2, 80, 66, 15, 0, 0, 0, 0, 0, 0, 0, 4, 0, 128, 4, 255, 7, 4, 80,
                66, 15, 0, 0, 0, 0, 0, 2, 0, 32, 2, 7, 0, 0, 128, 128, 6, 255, 255, 5
            ]
        );
        assert!(Capture::new(&s).finish(s.meta.now_ms).data.is_empty());
    }
    #[test]
    fn removed_residents_do_not_shift_tracks() {
        let mut s = Snapshot::default();
        s.players = vec![player(1), player(2)];
        let mut c = Capture::new(&s);
        s.players.remove(0);
        s.meta.now_ms = 16;
        s.players[0].x = 12;
        c.record(&s);
        let h = c.finish(16);
        assert_eq!(u64::from_le_bytes(h.data[..8].try_into().unwrap()), 2);
    }
}
