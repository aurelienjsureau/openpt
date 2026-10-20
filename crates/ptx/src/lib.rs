#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub content_type: u16,
    pub offset: usize,
    pub start: usize,
    pub end: usize,
    pub children: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Midi,
    Aux,
    Master,
    Vca,
    Folder,
    Other(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Insert {
    pub manufacturer: String,
    pub product: String,
    pub name: Option<String>,
    pub state: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    pub stereo: bool,
    pub inserts: Vec<Insert>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub volume_db: f32,
    pub pan: Vec<f32>,
    pub sends: Vec<Send>,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Send {
    pub slot: usize,
    pub target: String,
    pub level_db: f32,
    pub pan: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub name: String,
    pub file: Option<usize>,
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub track: String,
    pub channel: usize,
    pub region: usize,
    pub start: u64,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PtxSession {
    pub app: String,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub tracks: Vec<Track>,
    pub audio_files: Vec<String>,
    pub audio_folders: Vec<Option<String>>,
    pub regions: Vec<Region>,
    pub placements: Vec<Placement>,
    pub busses: Vec<(String, bool)>,
    pub hardware: Vec<String>,
    pub tempos: Vec<(u64, f64)>,
    pub meters: Vec<(u32, u32, u32)>,
    pub markers: Vec<Marker>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub members: Vec<String>,
    pub vca: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marker {
    pub number: u16,
    pub name: String,
    pub start: Position,
    pub end: Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Ticks(u64),
    Samples(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtxError {
    NotPtx,
    Truncated,
}

impl std::fmt::Display for PtxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PtxError::NotPtx => write!(f, "not a Pro Tools session file"),
            PtxError::Truncated => write!(f, "the session file is truncated"),
        }
    }
}

impl std::error::Error for PtxError {}

const MAGIC: &[u8] = b"\x030010111100101011";
const HEADER: usize = 0x14;
const PAGE_SHIFT: usize = 12;

pub fn is_ptx(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

pub fn decrypt(bytes: &[u8]) -> Result<Vec<u8>, PtxError> {
    if !is_ptx(bytes) {
        return Err(PtxError::NotPtx);
    }
    let seed = *bytes.get(0x13).ok_or(PtxError::Truncated)?;
    let k = seed.wrapping_mul(0x5D);
    Ok(bytes.iter().enumerate().map(|(i, b)| if i < HEADER { *b } else { b ^ ((i >> PAGE_SHIFT) as u8).wrapping_mul(k) }).collect())
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

pub fn blocks(d: &[u8], start: usize, end: usize, depth: usize) -> Vec<Block> {
    let end = end.min(d.len());
    let mut out = Vec::new();
    let mut pos = start;
    while pos + 9 <= end {
        let (Some(0x5A), Some(size), Some(ct)) = (d.get(pos).copied(), u32_at(d, pos + 3), u16_at(d, pos + 7)) else {
            pos += 1;
            continue;
        };
        let size = size as usize;
        let block_end = pos.saturating_add(7).saturating_add(size);
        if size < 2 || block_end > end {
            pos += 1;
            continue;
        }
        let children = if depth > 0 { blocks(d, pos + 9, block_end, depth - 1) } else { Vec::new() };
        out.push(Block { content_type: ct, offset: pos, start: pos + 9, end: block_end, children });
        pos = block_end;
    }
    out
}

fn string_at(d: &[u8], at: usize) -> Option<(String, usize)> {
    let n = u32_at(d, at)? as usize;
    if n == 0 || n > 1024 {
        return None;
    }
    let raw = d.get(at + 4..at + 4 + n)?;
    Some((String::from_utf8_lossy(raw).into_owned(), at + 4 + n))
}

fn folders(d: &[u8], b: &Block, refs: &[[u8; 16]], tracks: &mut [Track]) {
    let same = |a: &[u8], t: &[u8; 16]| if a.get(4..8) == Some(&[0x2A, 0, 0, 0][..]) { a.get(8..16) == t.get(8..16) } else { a == t };
    let find = |r: &[u8]| refs.iter().position(|t| r.iter().any(|x| *x != 0) && same(r, t));
    let Some(n) = u32_at(d, b.start) else { return };
    let mut at = b.start + 4;
    let mut best: Vec<Option<(usize, u32)>> = vec![None; tracks.len()];
    for _ in 0..n.min(4096) {
        let (Some(folder), Some(count)) = (d.get(at + 1..at + 17), u32_at(d, at + 17)) else { return };
        if count > 100_000 {
            return;
        }
        let folder = find(folder);
        at += 21;
        for _ in 0..count {
            let Some(m) = d.get(at + 1..at + 17) else { return };
            if let (Some(f), Some(t)) = (folder, find(m))
                && f != t
                && best.get(t).is_some_and(|x| x.is_none_or(|(_, c)| count < c))
            {
                best[t] = Some((f, count));
            }
            at += 17;
        }
    }
    let names: Vec<String> = tracks.iter().map(|t| t.name.clone()).collect();
    for (t, f) in tracks.iter_mut().zip(best) {
        t.folder = f.and_then(|(f, _)| names.get(f).cloned());
    }
}

fn walk<'a>(list: &'a [Block], out: &mut Vec<&'a Block>) {
    for b in list {
        out.push(b);
        walk(&b.children, out);
    }
}

fn insert_at(d: &[u8], b: &Block) -> Option<Insert> {
    let code = d.get(b.start + 16..b.start + 24)?;
    if !code.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
        return None;
    }
    let state = d.get(b.start + 28..b.end).unwrap_or_default().to_vec();
    let text = String::from_utf8_lossy(&state);
    let name = text.find("<PluginName>").and_then(|i| {
        let rest = text.get(i + 12..)?;
        rest.find("</PluginName>").and_then(|j| rest.get(..j)).map(str::to_string)
    });
    Some(Insert {
        manufacturer: String::from_utf8_lossy(code.get(..4)?).into_owned(),
        product: String::from_utf8_lossy(code.get(4..8)?).into_owned(),
        name,
        state,
    })
}

fn uint_at(d: &[u8], at: usize, n: usize) -> Option<u64> {
    let b = d.get(at..at + n.min(8))?;
    Some(b.iter().rev().fold(0u64, |v, x| (v << 8) | u64::from(*x)))
}

fn three_point(d: &[u8], at: usize) -> Option<(u64, u64, u64)> {
    let nib = |k: usize| d.get(at + k).map(|b| usize::from(b >> 4));
    let (no, nl, ns) = (nib(1)?, nib(2)?, nib(3)?);
    if no > 5 || nl > 5 || ns > 5 {
        return None;
    }
    let mut p = at + 5;
    let offset = uint_at(d, p, no)?;
    p += no;
    let length = uint_at(d, p, nl)?;
    p += nl;
    let start = uint_at(d, p, ns)?;
    Some((offset, length, start))
}

const TICK_ZERO: u64 = 1_000_000_000_000;

fn u64_at(d: &[u8], at: usize) -> Option<u64> {
    d.get(at..at + 8).map(|b| b.iter().rev().fold(0u64, |v, x| v << 8 | u64::from(*x)))
}

fn position_at(d: &[u8], at: usize) -> Option<Position> {
    let raw = u64_at(d, at)?;
    let low = raw & 0xFFFF_FFFF_FFFF;
    Some(if raw >> 56 & 0x40 != 0 { Position::Ticks(low.saturating_sub(TICK_ZERO)) } else { Position::Samples(low) })
}

fn timeline(d: &[u8], all: &[&Block], s: &mut PtxSession) {
    if let Some(b) = all.iter().find(|b| b.content_type == 0x2028 && d.get(b.start..b.start + 5) == Some(b"Tempo")) {
        let mut at = b.start;
        while let Some(k) = d.get(at..b.end).and_then(|w| w.windows(3).position(|x| x == b"TMS")) {
            let t = at + k + 3;
            if let (Some(pos), Some(bpm)) = (u64_at(d, t + 16), u64_at(d, t + 26).map(f64::from_bits))
                && bpm.is_finite()
                && (5.0..=1000.0).contains(&bpm)
            {
                s.tempos.push((pos.saturating_sub(TICK_ZERO), bpm));
            }
            at = t;
        }
    }
    if let Some(b) = all.iter().find(|b| b.content_type == 0x2029 && d.get(b.start..b.start + 5) == Some(b"Meter")) {
        let after = b.start + 5;
        let n = u32_at(d, after + 6).unwrap_or(0).min(10_000) as usize;
        for k in 0..n {
            let e = after + 10 + 36 * k;
            if let (Some(bar), Some(num), Some(den)) = (u32_at(d, e + 8), u32_at(d, e + 12), u32_at(d, e + 16))
                && e + 36 <= b.end
            {
                s.meters.push((bar, num, den));
            }
        }
    }
    for b in all.iter().filter(|b| b.content_type == 0x2077) {
        let Some((name, after)) = string_at(d, b.start + 6) else { continue };
        let (Some(number), Some(start), Some(end)) = (u16_at(d, b.start), position_at(d, after), position_at(d, after + 8)) else { continue };
        if !s.markers.iter().any(|m| m.number == number) {
            s.markers.push(Marker { number, name, start, end });
        }
    }
}

fn groups(d: &[u8], all: &[&Block], s: &mut PtxSession) {
    let name_of = |i: u16| s.tracks.get(usize::from(i)).map(|t| t.name.clone());
    let mut out: Vec<Group> = Vec::new();
    for b in all.iter().filter(|b| b.content_type == 0x202A) {
        let Some((name, after)) = string_at(d, b.start) else { continue };
        let Some(n) = u32_at(d, after + 7).filter(|&n| n <= 4096) else { continue };
        let list = after + 11;
        let tail = list + 2 * n as usize;
        if name == "<ALL>" || out.iter().any(|g| g.name == name) || u16_at(d, tail) != Some(0xFFFE) || tail + 12 > b.end {
            continue;
        }
        let members = (0..n as usize).filter_map(|k| u16_at(d, list + 2 * k).and_then(name_of)).collect();
        let vca = u16_at(d, tail + 10).filter(|&v| v != 0xFFFF).and_then(name_of);
        out.push(Group { name, members, vca });
    }
    s.groups = out;
}

fn regions(d: &[u8], all: &[&Block], s: &mut PtxSession) {
    for list in all.iter().filter(|b| b.content_type == 0x262A) {
        for r in list.children.iter().filter(|b| b.content_type == 0x2629) {
            let Some((name, after)) = string_at(d, r.start + 9) else { continue };
            let Some((offset, length, _)) = three_point(d, after) else { continue };
            let file = r.children.first().and_then(|c| u32_at(d, c.end)).map(|f| f as usize);
            s.regions.push(Region { name, file, offset, length });
        }
    }
    let mut channels: Vec<(String, Vec<u16>)> = Vec::new();
    for list in all.iter().filter(|b| b.content_type == 0x1015) {
        for t in list.children.iter().filter(|b| b.content_type == 0x1014) {
            let Some((name, after)) = string_at(d, t.start) else { continue };
            let Some(n) = u32_at(d, after + 1) else { continue };
            let map: Vec<u16> = (0..n.min(16) as usize).filter_map(|k| u16_at(d, after + 5 + 2 * k)).collect();
            channels.push((name, map));
        }
    }
    for map in all.iter().filter(|b| b.content_type == 0x1054) {
        for (index, t) in map.children.iter().filter(|b| b.content_type == 0x1052).enumerate() {
            let label = string_at(d, t.start).map(|x| x.0);
            let owner = match &label {
                Some(n) => s.tracks.iter().any(|tr| &tr.name == n).then(|| (n.clone(), 0)),
                None => channels.iter().find_map(|(name, map)| map.iter().position(|c| usize::from(*c) == index).map(|ch| (name.clone(), ch))),
            };
            let Some((track, channel)) = owner else { continue };
            for e in t.children.iter().filter(|b| b.content_type == 0x1050) {
                if d.get(e.start + 44) == Some(&1) {
                    continue;
                }
                for p in e.children.iter().filter(|b| b.content_type == 0x104F) {
                    let (Some(region), Some(start)) = (u32_at(d, p.start + 2), u32_at(d, p.start + 7)) else { continue };
                    let wide = d.get(p.start + 7..p.start + 13).map_or(0, |b| b.iter().rev().fold(0u64, |v, x| v << 8 | u64::from(*x)));
                    let start = if wide >= 1_000_000_000_000 { wide - 1_000_000_000_000 } else { u64::from(start) };
                    s.placements.push(Placement { track: track.clone(), channel, region: region as usize, start });
                }
            }
        }
    }
}

fn last_string(d: &[u8], start: usize, end: usize) -> Option<String> {
    (start..end.saturating_sub(5)).rev().find_map(|k| {
        let n = u32_at(d, k)? as usize;
        let stop = k + 4 + n;
        (n > 0 && n < 256 && (stop == end || stop + 1 == end))
            .then(|| d.get(k + 4..stop))
            .flatten()
            .filter(|s| s.iter().all(|c| (0x20..0x7F).contains(c)))
            .map(|s| String::from_utf8_lossy(s).into_owned())
    })
}

fn path_name(d: &[u8], start: usize, end: usize) -> Option<String> {
    let id = (start..end.saturating_sub(4)).find(|&k| d.get(k..k + 4) == Some(&[0x2A, 0, 0, 0]))?;
    (id + 12..end.saturating_sub(4)).find_map(|k| {
        let n = u32_at(d, k)? as usize;
        let name = d.get(k + 4..k + 4 + n).filter(|_| k + 4 + n <= end)?;
        (n > 0 && n < 256 && name.iter().all(|c| (0x20..0x7F).contains(c))).then(|| String::from_utf8_lossy(name).into_owned())
    })
}

fn path_ref(d: &[u8], start: usize, end: usize) -> Option<String> {
    (start..end.saturating_sub(9)).find_map(|k| {
        let n = u32_at(d, k)? as usize;
        let name = d.get(k + 4..k + 4 + n)?;
        let ok = n > 0 && n < 256 && name.iter().all(|c| (0x20..0x7F).contains(c)) && d.get(k + 4 + n..k + 9 + n) == Some(&[0, 0, 0, 0, 0x2A]);
        ok.then(|| String::from_utf8_lossy(name).into_owned())
    })
}
pub fn read(bytes: &[u8]) -> Result<PtxSession, PtxError> {
    let d = decrypt(bytes)?;
    let tree = blocks(&d, HEADER, d.len(), 14);
    let mut all = Vec::new();
    walk(&tree, &mut all);
    let mut s = PtxSession::default();
    if let Some(b) = all.iter().find(|b| b.content_type == 0x0003)
        && let Some((app, _)) = string_at(&d, b.start + 1)
    {
        s.app = app;
    }
    if let Some(b) = all.iter().find(|b| b.content_type == 0x1028) {
        s.bit_depth = d.get(b.start + 1).copied();
        s.sample_rate = u32_at(&d, b.start + 2).filter(|r| (8_000..=384_000).contains(r));
    }
    let mut refs: Vec<[u8; 16]> = Vec::new();
    for b in all.iter().filter(|b| b.content_type == 0x251A) {
        let Some(code) = d.get(b.start).copied() else { continue };
        let Some((name, after)) = string_at(&d, b.start + 2) else { continue };
        if s.tracks.iter().any(|t| t.name == name) {
            continue;
        }
        refs.push(d.get(after + 24..after + 40).and_then(|r| r.try_into().ok()).unwrap_or([0; 16]));
        let kind = match code {
            0x00 => TrackKind::Audio,
            0x01 => TrackKind::Midi,
            0x02 => TrackKind::Aux,
            0x05 => TrackKind::Master,
            0x09 => TrackKind::Vca,
            0x0B => TrackKind::Folder,
            c => TrackKind::Other(c),
        };
        s.tracks.push(Track {
            name,
            kind,
            stereo: d.get(after).copied() == Some(1),
            inserts: Vec::new(),
            input: None,
            output: None,
            volume_db: 0.0,
            pan: Vec::new(),
            sends: Vec::new(),
            folder: None,
        });
    }
    if let Some(b) = all.iter().find(|b| b.content_type == 0x210C) {
        folders(&d, b, &refs, &mut s.tracks);
    }
    let rulers = ["Tempo", "Meter", "Key Signature", "Chord Symbols", "Markers"];
    let mut next = 0usize;
    for ch in all.iter().filter(|b| b.content_type == 0x261B) {
        let mut inner = Vec::new();
        walk(&ch.children, &mut inner);
        let Some(head) = inner.iter().find(|b| b.content_type == 0x2619) else { continue };
        let name = string_at(&d, head.start).map(|x| x.0).unwrap_or_default();
        if rulers.contains(&name.as_str()) {
            continue;
        }
        let inserts: Vec<Insert> = inner.iter().filter(|b| b.content_type == 0x1038).filter_map(|b| insert_at(&d, b)).collect();
        let outs: Vec<&Block> = ch.children.iter().filter(|b| b.content_type == 0x260D).collect();
        let target = |o: &Block| {
            o.children
                .iter()
                .find(|b| b.content_type == 0x260E)
                .and_then(|b| path_name(&d, b.start, b.end).or_else(|| last_string(&d, b.start, b.end)))
        };
        let levels = |o: &Block| {
            o.children.iter().find(|b| b.content_type == 0x1029).and_then(|b| {
                let f = |k: usize| u32_at(&d, b.start + k).map(|v| v as i32 as f32);
                Some((f(1)? / 10.0, f(13)? / 100.0, f(41)? / 100.0))
            })
        };
        let output = outs.first().and_then(|o| target(o));
        let input = outs.last().and_then(|o| path_ref(&d, o.end, ch.end));
        let mix = outs.first().and_then(|o| levels(o));
        let sends: Vec<Send> = outs
            .iter()
            .skip(1)
            .take(10)
            .enumerate()
            .filter_map(|(slot, o)| {
                let (level_db, pan, _) = levels(o).unwrap_or((0.0, 0.0, 0.0));
                Some(Send { slot, target: target(o)?, level_db: level_db.max(-144.0), pan })
            })
            .collect();
        let at = if name.is_empty() { Some(next) } else { s.tracks.iter().position(|t| t.name == name) };
        if let Some(i) = at
            && let Some(t) = s.tracks.get_mut(i)
            && t.output.is_none()
            && t.inserts.is_empty()
        {
            t.inserts = inserts;
            t.output = output;
            t.input = input;
            t.sends = sends;
            if let Some(m) = mix {
                (t.volume_db, t.pan) = (m.0, if t.stereo { vec![m.1, m.2] } else { vec![m.1] });
            }
            next = i + 1;
        }
    }
    for b in all.iter().filter(|b| b.content_type == 0x2602) {
        let mut inner = Vec::new();
        walk(&b.children, &mut inner);
        if inner.iter().any(|c| c.content_type == 0x2600)
            && let Some((n, _)) = string_at(&d, b.start + 2)
        {
            s.hardware.push(n);
        }
    }
    let paths: Vec<String> = all.iter().filter(|b| b.content_type == 0x2602).filter_map(|b| string_at(&d, b.start + 2).map(|x| x.0)).collect();
    let channels: std::collections::HashMap<String, u32> = all
        .iter()
        .filter(|b| b.content_type == 0x2602)
        .filter_map(|b| string_at(&d, b.start + 2).and_then(|(n, at)| Some((n, u32_at(&d, at).filter(|c| (1..=64).contains(c))?))))
        .collect();
    for p in &paths {
        let sub_of_hardware = s.hardware.iter().any(|h| p.starts_with(h.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-')));
        let used = s.tracks.iter().any(|t| t.input.as_ref() == Some(p) || t.output.as_ref() == Some(p) || t.sends.iter().any(|x| &x.target == p));
        if !used || sub_of_hardware || p.ends_with(".L") || p.ends_with(".R") || s.hardware.contains(p) || s.busses.iter().any(|b| &b.0 == p) {
            continue;
        }
        let stereo = channels.get(p).map_or_else(|| paths.iter().any(|q| q == &format!("{p}.L")), |n| *n >= 2);
        s.busses.push((p.clone(), stereo));
    }
    for b in all.iter().filter(|b| b.content_type == 0x103A) {
        let mut at = b.start;
        let mut folder: Option<String> = None;
        while at + 8 < b.end {
            match string_at(&d, at) {
                Some((name, after))
                    if !name.chars().any(char::is_control) && after + 4 <= b.end && matches!(d.get(after..after + 4), Some(b"EVAW" | b"FFIA")) =>
                {
                    s.audio_files.push(name);
                    s.audio_folders.push(folder.clone());
                    at = after + 4;
                }
                Some((name, after)) if name.chars().all(|c| !c.is_control()) && !name.ends_with(['\\', ':']) => {
                    folder = Some(name);
                    at = after;
                }
                _ => at += 1,
            }
        }
    }
    regions(&d, &all, &mut s);
    timeline(&d, &all, &mut s);
    groups(&d, &all, &mut s);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_are_musical_when_flagged_and_count_from_ten_to_the_twelfth() {
        let mut d = (TICK_ZERO + 61_440_000).to_le_bytes();
        d[7] = 0x40;
        assert_eq!(position_at(&d, 0), Some(Position::Ticks(61_440_000)));
        assert_eq!(position_at(&630_000u64.to_le_bytes(), 0), Some(Position::Samples(630_000)));
    }

    fn block(ct: u16, content: &[u8]) -> Vec<u8> {
        let mut v = vec![0x5A, 1, 0];
        v.extend_from_slice(&((content.len() + 2) as u32).to_le_bytes());
        v.extend_from_slice(&ct.to_le_bytes());
        v.extend_from_slice(content);
        v
    }

    fn string(s: &str) -> Vec<u8> {
        let mut v = (s.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(s.as_bytes());
        v
    }

    fn track(code: u8, name: &str, stereo: bool) -> Vec<u8> {
        let mut c = vec![code, 0];
        c.extend(string(name));
        c.push(u8::from(stereo));
        c.extend_from_slice(&[0; 8]);
        block(0x251A, &c)
    }

    fn sample(seed: u8) -> Vec<u8> {
        let mut plain = MAGIC.to_vec();
        plain.extend_from_slice(&[0, 5, seed]);
        let mut fmt = vec![3, 24];
        fmt.extend_from_slice(&48_000u32.to_le_bytes());
        plain.extend(block(0x1028, &fmt));
        let mut list = Vec::new();
        for _ in 0..2 {
            list.extend(track(0x05, "Master 1", true));
            list.extend(track(0x00, "Kick", false));
            list.extend(track(0x02, "Verb", true));
        }
        plain.extend(block(0x2519, &list));
        plain.resize(9000, 0);
        let mut files = string("Session");
        for f in ["Kick_01.wav", "Snare_01.wav"] {
            files.extend(string(f));
            files.extend_from_slice(b"EVAW");
        }
        plain.extend(block(0x103A, &files));
        let k = seed.wrapping_mul(0x5D);
        plain.iter().enumerate().map(|(i, b)| if i < HEADER { *b } else { b ^ ((i >> PAGE_SHIFT) as u8).wrapping_mul(k) }).collect()
    }

    #[test]
    fn reads_format_tracks_and_files_across_pages() {
        let s = read(&sample(0xFE)).unwrap();
        assert_eq!(s.sample_rate, Some(48_000));
        assert_eq!(s.bit_depth, Some(24));
        let tracks: Vec<(&str, TrackKind, bool)> = s.tracks.iter().map(|t| (t.name.as_str(), t.kind, t.stereo)).collect();
        assert_eq!(tracks, vec![("Master 1", TrackKind::Master, true), ("Kick", TrackKind::Audio, false), ("Verb", TrackKind::Aux, true)]);
        assert_eq!(s.audio_files, vec!["Kick_01.wav", "Snare_01.wav"]);
    }

    #[test]
    fn the_obfuscation_key_comes_from_the_seed_byte() {
        for (seed, k) in [(0xFEu8, 0x46u8), (0xAB, 0x1F), (0x4D, 0xF9)] {
            assert_eq!(seed.wrapping_mul(0x5D), k);
        }
        assert_eq!(read(b"not a session"), Err(PtxError::NotPtx));
    }
}
