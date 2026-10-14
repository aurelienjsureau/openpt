use crate::pcm::{be_u32, be_u64};
use crate::{AudioError, Result};

#[derive(Clone, Copy)]
enum Level {
    Top,
    Moov,
    Trak,
    Mdia,
    Minf,
    Stbl,
    Udta,
    Meta,
    Ilst,
    Tag,
}

pub(crate) fn check(b: &[u8], at: usize) -> Result<()> {
    atoms(b, at, b.len(), Level::Top).map(|_| ())
}

fn malformed(what: &str) -> AudioError {
    AudioError::Malformed(format!("MP4 {what} is larger than the file"))
}

fn atoms(b: &[u8], mut pos: usize, end: usize, level: Level) -> Result<bool> {
    while pos < end {
        let (Some(size), Some(kind)) = (be_u32(b, pos), b.get(pos + 4..pos + 8)) else { return Ok(false) };
        let head = if size == 1 { 16 } else { 8 };
        let len = match size {
            0 => head,
            1 => be_u64(b, pos + 8).unwrap_or(0),
            n => u64::from(n),
        };
        if len < head {
            return Ok(false);
        }
        let body = pos + head as usize;
        let data_end = usize::try_from(len).ok().and_then(|n| pos.checked_add(n));
        let next = if size == 0 { Some(end) } else { data_end };
        let inner = data_end.map_or(b.len(), |n| n.min(b.len()));
        let more = match (level, kind) {
            (Level::Top, b"moov") => atoms(b, body, inner, Level::Moov)?,
            (Level::Top | Level::Udta, b"meta") => atoms(b, body + 4, inner, Level::Meta)?,
            (Level::Moov, b"trak") => atoms(b, body, inner, Level::Trak)?,
            (Level::Moov, b"udta") => atoms(b, body, inner, Level::Udta)?,
            (Level::Trak, b"mdia") => atoms(b, body, inner, Level::Mdia)?,
            (Level::Mdia, b"minf") => atoms(b, body, inner, Level::Minf)?,
            (Level::Minf, b"stbl") => atoms(b, body, inner, Level::Stbl)?,
            (Level::Meta, b"ilst") => atoms(b, body, inner, Level::Ilst)?,
            (Level::Ilst, _) => atoms(b, body, inner, Level::Tag)?,
            (Level::Stbl, b"stts" | b"co64") => table(b, body + 4, 8, inner)?,
            (Level::Stbl, b"stsc") => table(b, body + 4, 12, inner)?,
            (Level::Stbl, b"stco") => table(b, body + 4, 4, inner)?,
            (Level::Stbl, b"stsz") => sample_sizes(b, body, inner)?,
            (Level::Mdia, b"hdlr") | (Level::Tag, b"data" | b"name" | b"mean") if len > (b.len() - pos) as u64 => {
                return Err(malformed(&format!("`{}` atom", String::from_utf8_lossy(kind))));
            }
            _ => true,
        };
        match next {
            Some(n) if more && n <= end => pos = n,
            _ => return Ok(false),
        }
    }
    Ok(true)
}

fn table(b: &[u8], count: usize, entry: u64, end: usize) -> Result<bool> {
    let Some(n) = be_u32(b, count) else { return Ok(false) };
    let (first, bytes) = (count + 4, u64::from(n) * entry);
    if bytes > b.len().saturating_sub(first) as u64 {
        return Err(malformed("sample table"));
    }
    Ok(first as u64 + bytes <= end as u64)
}

fn sample_sizes(b: &[u8], body: usize, end: usize) -> Result<bool> {
    let (Some(size), Some(count)) = (be_u32(b, body + 4), be_u32(b, body + 8)) else { return Ok(false) };
    let file = b.len() as u64;
    if u64::from(size) > file {
        return Err(malformed("sample"));
    }
    if size != 0 {
        return Ok(true);
    }
    let fits = table(b, body + 8, 4, end)?;
    let (sizes, _) = b.get(body + 12..end).unwrap_or_default().as_chunks::<4>();
    if sizes.iter().take(count as usize).any(|s| u64::from(u32::from_be_bytes(*s)) > file) {
        return Err(malformed("sample"));
    }
    Ok(fits)
}
