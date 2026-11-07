use openpt_model::SourceAudio;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

const VERSION: u64 = 1;
const MIN_BYTES: usize = 4 << 20;
const MAX_TOTAL: u64 = 64 << 30;

static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_dir(dir: Option<PathBuf>) {
    *DIR.lock().unwrap_or_else(PoisonError::into_inner) = dir;
}

pub fn default_dir() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(d) = env("OPENPT_AUDIO_CACHE_DIR") {
        return Some(d);
    }
    if cfg!(windows) {
        env("LOCALAPPDATA").map(|d| d.join("OpenPT").join("AudioCache"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Caches/OpenPT/AudioCache"))
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache"))).map(|d| d.join("openpt").join("AudioCache"))
    }
}

fn dir() -> Option<PathBuf> {
    DIR.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn key_for(p: &Path, rate: u32) -> Option<(PathBuf, u64)> {
    use std::hash::{Hash, Hasher};
    let d = dir()?;
    let meta = std::fs::metadata(p).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_nanos();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (VERSION, crate::io::file_key(p), meta.len(), mtime, rate).hash(&mut h);
    let key = h.finish();
    Some((d.join(format!("{key:016x}.optpcm")), key))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn cached(p: &Path, rate: u32) -> Option<Arc<SourceAudio>> {
    let (file, key) = key_for(p, rate)?;
    let m = openpt_mapped::MappedSamples::open(&file, key).ok()?;
    if let Ok(f) = std::fs::File::options().write(true).open(&file) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
    Some(Arc::new(SourceAudio::mapped(m)))
}

#[cfg(target_arch = "wasm32")]
pub fn cached(_: &Path, _: u32) -> Option<Arc<SourceAudio>> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn store(p: &Path, rate: u32, audio: Arc<SourceAudio>) -> Arc<SourceAudio> {
    let bytes = audio.frames().saturating_mul(audio.num_channels()).saturating_mul(4);
    if bytes < MIN_BYTES || audio.is_mapped() {
        return audio;
    }
    let Some((file, key)) = key_for(p, rate) else { return audio };
    if let Err(e) = openpt_mapped::write(&file, audio.sample_rate(), &audio.channels(), key) {
        log::warn!("audio cache: {}: {e}", file.display());
        return audio;
    }
    prune(&file);
    match openpt_mapped::MappedSamples::open(&file, key) {
        Ok(m) => Arc::new(SourceAudio::mapped(m)),
        Err(e) => {
            log::warn!("audio cache: {}: {e}", file.display());
            audio
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub fn store(_: &Path, _: u32, audio: Arc<SourceAudio>) -> Arc<SourceAudio> {
    audio
}

fn prune(keep: &Path) {
    let Some(d) = keep.parent() else { return };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let m = e.metadata().ok()?;
            let stale_tmp = p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.starts_with("tmp"))
                && m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 3600);
            if stale_tmp {
                let _ = std::fs::remove_file(&p);
                return None;
            }
            (p.extension().and_then(|x| x.to_str()) == Some("optpcm")).then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), p))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= MAX_TOTAL {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, p) in files {
        if total <= MAX_TOTAL * 3 / 4 {
            break;
        }
        if p != keep && std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use openpt_audio_io::AudioBuffer;

    #[test]
    fn big_sources_come_back_mapped_and_reopen_from_the_cache() {
        let root = std::env::temp_dir().join(format!("openpt-audio-cache-{}", std::process::id()));
        let src = root.join("take.wav");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&src, b"stand-in for the original file").unwrap();
        set_dir(Some(root.join("cache")));
        let frames = MIN_BYTES / 4 + 10;
        let owned =
            Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![(0..frames).map(|i| (i % 100) as f32 / 100.0).collect()] }));
        let mapped = store(&src, 48_000, Arc::clone(&owned));
        assert!(mapped.is_mapped());
        assert_eq!(mapped.channel(0), owned.channel(0));
        let again = cached(&src, 48_000).expect("reopened from the cache");
        assert_eq!(again.sample(0, 1234), owned.sample(0, 1234));
        assert!(cached(&src, 44_100).is_none(), "another rate is another entry");
        let small = Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.5; 100]] }));
        assert!(!store(&src, 48_000, small).is_mapped());
        set_dir(None);
        drop((mapped, again));
        let _ = std::fs::remove_dir_all(&root);
    }
}
