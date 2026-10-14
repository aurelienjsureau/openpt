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
    Aux,
    Master,
    Vca,
    Folder,
    Other(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    pub stereo: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PtxSession {
    pub app: String,
    pub sample_rate: Option<u32>,
    pub bit_depth: Option<u8>,
    pub tracks: Vec<Track>,
    pub audio_files: Vec<String>,
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

fn walk<'a>(list: &'a [Block], out: &mut Vec<&'a Block>) {
    for b in list {
        out.push(b);
        walk(&b.children, out);
    }
}

pub fn read(bytes: &[u8]) -> Result<PtxSession, PtxError> {
    let d = decrypt(bytes)?;
    let tree = blocks(&d, HEADER, d.len(), 6);
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
    for b in all.iter().filter(|b| b.content_type == 0x251A) {
        let Some(code) = d.get(b.start).copied() else { continue };
        let Some((name, after)) = string_at(&d, b.start + 2) else { continue };
        if s.tracks.iter().any(|t| t.name == name) {
            continue;
        }
        let kind = match code {
            0x00 => TrackKind::Audio,
            0x02 => TrackKind::Aux,
            0x05 => TrackKind::Master,
            0x09 => TrackKind::Vca,
            0x0B => TrackKind::Folder,
            c => TrackKind::Other(c),
        };
        s.tracks.push(Track { name, kind, stereo: d.get(after).copied() == Some(1) });
    }
    for b in all.iter().filter(|b| b.content_type == 0x103A) {
        let mut at = b.start;
        while at + 8 < b.end {
            if let Some((name, after)) = string_at(&d, at)
                && after + 4 <= b.end
                && matches!(d.get(after..after + 4), Some(b"EVAW" | b"FFIA"))
            {
                s.audio_files.push(name);
                at = after + 4;
            } else {
                at += 1;
            }
        }
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

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
