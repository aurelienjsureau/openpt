use openpt_model::{ClipContent, Session};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};

const PAGE: usize = 1024;
const AHEAD_SECONDS: f64 = 8.0;
const BEHIND_SECONDS: f64 = 1.0;

struct Watched {
    session: Arc<Session>,
    shared: Weak<crate::Shared>,
}

static WATCHED: Mutex<Option<Watched>> = Mutex::new(None);
static STARTED: std::sync::Once = std::sync::Once::new();
pub static TOUCHED: AtomicI64 = AtomicI64::new(0);

pub(crate) fn watch(session: Arc<Session>, shared: &Arc<crate::Shared>) {
    if !session.pool.ids().any(|id| session.pool.get(id).is_some_and(|a| a.is_mapped())) {
        *WATCHED.lock().unwrap_or_else(PoisonError::into_inner) = None;
        return;
    }
    *WATCHED.lock().unwrap_or_else(PoisonError::into_inner) = Some(Watched { session, shared: Arc::downgrade(shared) });
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("openpt-readahead".into()).spawn(run);
    });
}

fn run() {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(40));
        let snapshot = WATCHED.lock().unwrap_or_else(PoisonError::into_inner).as_ref().and_then(|w| Some((Arc::clone(&w.session), w.shared.upgrade()?)));
        let Some((s, shared)) = snapshot else { continue };
        let n = warm(&s, shared.position.load(Ordering::Relaxed));
        TOUCHED.fetch_add(n, Ordering::Relaxed);
    }
}

pub fn warm(s: &Session, pos: i64) -> i64 {
    let rate = s.sample_rate.as_f64();
    let from = pos.saturating_sub((BEHIND_SECONDS * rate) as i64);
    let to = pos.saturating_add((AHEAD_SECONDS * rate) as i64);
    let mut touched = 0i64;
    let mut sum = 0.0f32;
    for t in s.tracks.iter().filter(|t| !t.inactive) {
        for c in t.clips() {
            let ClipContent::Audio { source, offset } = c.content else { continue };
            let (a, b) = (c.start.max(from), c.end().min(to));
            if a >= b {
                continue;
            }
            let Some(audio) = s.pool.get(source).filter(|x| x.is_mapped()) else { continue };
            let stretch = if c.stretch.is_finite() && c.stretch > 0.0 { c.stretch } else { 1.0 };
            let margin = rate as i64;
            let s0 = offset.saturating_add(((a - c.start) as f64 / stretch) as i64).saturating_sub(margin).max(0);
            let s1 = offset.saturating_add(((b - c.start) as f64 / stretch) as i64).saturating_add(margin).max(0);
            let (s0, s1) = (usize::try_from(s0).unwrap_or(0), usize::try_from(s1).unwrap_or(0).min(audio.frames()));
            for ch in audio.channels() {
                let mut i = s0 - s0 % PAGE;
                while i < s1 {
                    sum += ch.get(i).copied().unwrap_or(0.0);
                    touched += 1;
                    i += PAGE;
                }
            }
        }
    }
    std::hint::black_box(sum);
    touched
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpt_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};

    #[test]
    fn only_the_window_around_the_playhead_of_mapped_sources_is_read() {
        let dir = std::env::temp_dir().join(format!("openpt-readahead-{}", std::process::id()));
        let file = dir.join("a.optpcm");
        let frames = 48_000 * 60;
        let data = vec![0.25f32; frames];
        openpt_mapped::write(&file, 48_000, &[data.as_slice()], 1).unwrap();
        let mut s = Session::default();
        s.pool.insert(SourceId(1), Arc::new(SourceAudio::mapped(openpt_mapped::MappedSamples::open(&file, 1).unwrap())));
        s.pool.insert(SourceId(2), Arc::new(SourceAudio::new(openpt_audio_io::AudioBuffer { sample_rate: 48_000, channels: vec![data.clone()] })));
        for src in [1, 2] {
            let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
            let c = s.new_clip_id();
            s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(c, "c", SourceId(src), 0, 0, frames as i64));
        }
        let n = warm(&s, 48_000 * 10);
        let window = ((AHEAD_SECONDS + BEHIND_SECONDS + 2.0) * 48_000.0) as i64 / PAGE as i64;
        assert!(n > 0 && n <= window + 2, "{n} pages read, at most {window}");
        assert_eq!(warm(&s, 48_000 * 120), 0, "past the end of every clip");
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
