#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openpt_audio_io::AudioBuffer;
use openpt_mix::render_range;
use openpt_model::{ChannelFormat, Clip, Insert, Route, SendSlot, Session, SourceAudio, SourceId, TrackKind};
use openpt_time::Range;
use std::sync::Arc;

const LEN: usize = 48_000;

fn fixture(key_on: bool, listen: bool, kick_first: bool) -> Session {
    let mut s = Session::default();
    let tone: Vec<f32> = (0..LEN).map(|i| 0.1 * (i as f32 * 0.05).sin()).collect();
    let burst: Vec<f32> = (0..LEN).map(|i| if i >= LEN / 2 { 0.9 * (i as f32 * 0.2).sin() } else { 0.0 }).collect();
    s.pool.insert(SourceId(1), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![tone] })));
    s.pool.insert(SourceId(2), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![burst] })));
    let key = s.add_bus("Key", ChannelFormat::Mono);
    let mk = |s: &mut Session, name: &str| s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some(name));
    let (bass, kick) = if kick_first {
        let k = mk(&mut s, "Kick");
        (mk(&mut s, "Bass"), k)
    } else {
        let b = mk(&mut s, "Bass");
        (b, mk(&mut s, "Kick"))
    };
    for (t, src) in [(bass, 1), (kick, 2)] {
        let c = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(c, "c", SourceId(src), 0, 0, LEN as i64));
    }
    let k = s.track_mut(kick).unwrap();
    k.mixer.output = Route::None;
    let mut send = SendSlot::new(Route::Bus(key));
    send.level_db = 0.0;
    k.mixer.sends[0] = Some(send);
    let mut comp = Insert::new("compressor");
    comp.params.insert("threshold".into(), -30.0);
    comp.params.insert("ratio".into(), 20.0);
    comp.params.insert("attack".into(), 0.1);
    comp.key = Some(key);
    comp.key_on = key_on;
    comp.key_listen = listen;
    s.track_mut(bass).unwrap().mixer.inserts[0] = Some(comp);
    s
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn halves(s: &Session) -> (f32, f32) {
    let out = render_range(s, Range::new(0, LEN as i64), 512);
    let l = &out[0];
    (rms(&l[LEN / 8..LEN / 2]), rms(&l[LEN / 2 + LEN / 8..]))
}

#[test]
fn the_key_ducks_the_bass_only_while_the_kick_plays() {
    for kick_first in [false, true] {
        let (quiet, loud) = halves(&fixture(true, false, kick_first));
        assert!(quiet > 0.03, "no kick: the bass passes ({quiet})");
        assert!(loud < quiet * 0.3, "kick on the key: the bass ducks ({loud} vs {quiet}, kick first: {kick_first})");
    }
}

#[test]
fn without_external_key_the_compressor_hears_its_own_track() {
    let (a, b) = halves(&fixture(false, false, false));
    assert!((a - b).abs() < a * 0.05, "the bass level does not follow the kick ({a} vs {b})");
}

#[test]
fn key_listen_plays_the_key() {
    let (quiet, loud) = halves(&fixture(true, true, false));
    assert!(quiet < 1e-4, "the key is silent before the kick ({quiet})");
    assert!(loud > 0.3, "the key is heard ({loud})");
}
