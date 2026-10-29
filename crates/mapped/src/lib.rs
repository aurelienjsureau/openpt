use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

#[cfg(target_endian = "big")]
compile_error!("openpt-mapped stores little-endian samples");

const MAGIC: &[u8; 8] = b"OPTPCM01";
const HEADER: usize = 64;

pub struct MappedSamples {
    map: memmap2::Mmap,
    channels: usize,
    frames: usize,
    sample_rate: u32,
}

impl std::fmt::Debug for MappedSamples {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedSamples").field("channels", &self.channels).field("frames", &self.frames).field("sample_rate", &self.sample_rate).finish()
    }
}

fn header(channels: usize, frames: usize, sample_rate: u32, key: u64) -> [u8; HEADER] {
    let mut h = [0u8; HEADER];
    h[..8].copy_from_slice(MAGIC);
    h[8..16].copy_from_slice(&(channels as u64).to_le_bytes());
    h[16..24].copy_from_slice(&(frames as u64).to_le_bytes());
    h[24..28].copy_from_slice(&sample_rate.to_le_bytes());
    h[32..40].copy_from_slice(&key.to_le_bytes());
    h
}

fn read_u64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    if let Some(s) = b.get(at..at + 8) {
        a.copy_from_slice(s);
    }
    u64::from_le_bytes(a)
}

pub fn write(path: &Path, sample_rate: u32, channels: &[&[f32]], key: u64) -> io::Result<()> {
    let frames = channels.iter().map(|c| c.len()).max().unwrap_or(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        w.write_all(&header(channels.len(), frames, sample_rate, key))?;
        let mut chunk = Vec::with_capacity(1 << 16);
        for ch in channels {
            for block in ch.chunks(1 << 14) {
                chunk.clear();
                for x in block {
                    chunk.extend_from_slice(&x.to_le_bytes());
                }
                w.write_all(&chunk)?;
            }
            for _ in ch.len()..frames {
                w.write_all(&0f32.to_le_bytes())?;
            }
        }
        w.flush()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            if path.exists() { Ok(()) } else { Err(e) }
        }
    }
}

impl MappedSamples {
    pub fn open(path: &Path, key: u64) -> io::Result<MappedSamples> {
        let file = File::open(path)?;
        #[allow(unsafe_code)]
        let map = unsafe { memmap2::Mmap::map(&file)? };
        let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {what}", path.display()));
        if map.get(..8) != Some(&MAGIC[..]) {
            return Err(bad("not an audio cache file"));
        }
        if read_u64(&map, 32) != key {
            return Err(bad("stale cache"));
        }
        let channels = usize::try_from(read_u64(&map, 8)).map_err(|_| bad("bad header"))?;
        let frames = usize::try_from(read_u64(&map, 16)).map_err(|_| bad("bad header"))?;
        let mut sr = [0u8; 4];
        sr.copy_from_slice(map.get(24..28).unwrap_or(&[0; 4]));
        let want = channels.checked_mul(frames).and_then(|n| n.checked_mul(4)).and_then(|n| n.checked_add(HEADER)).ok_or_else(|| bad("bad header"))?;
        if map.len() != want || channels == 0 {
            return Err(bad("truncated"));
        }
        if (map.as_ptr() as usize) % std::mem::align_of::<f32>() != 0 {
            return Err(bad("unaligned map"));
        }
        Ok(MappedSamples { map, channels, frames, sample_rate: u32::from_le_bytes(sr) })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channel(&self, ch: usize) -> Option<&[f32]> {
        if ch >= self.channels {
            return None;
        }
        let start = HEADER + ch * self.frames * 4;
        let bytes = self.map.get(start..start + self.frames * 4)?;
        #[allow(unsafe_code)]
        let samples = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<f32>(), self.frames) };
        Some(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_samples_read_back_through_the_map() {
        let dir = std::env::temp_dir().join(format!("openpt-mapped-{}", std::process::id()));
        let path = dir.join("a.optpcm");
        let chans: [&[f32]; 2] = [&[0.5, -0.25, 1.0], &[0.1, 0.2]];
        write(&path, 48_000, &chans, 42).unwrap();
        let m = MappedSamples::open(&path, 42).unwrap();
        assert_eq!((m.channels(), m.frames(), m.sample_rate()), (2, 3, 48_000));
        assert_eq!(m.channel(0).unwrap(), &[0.5, -0.25, 1.0]);
        assert_eq!(m.channel(1).unwrap(), &[0.1, 0.2, 0.0]);
        assert!(m.channel(2).is_none());
        assert!(MappedSamples::open(&path, 7).is_err(), "another key is stale");
        drop(m);
        std::fs::write(&path, b"OPTPCM01short").unwrap();
        assert!(MappedSamples::open(&path, 42).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
