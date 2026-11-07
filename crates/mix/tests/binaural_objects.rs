#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openpt_audio_io::AudioBuffer;
use openpt_mix::{
    MixEngine,
    monitor::{Monitor, split_modes},
};
use openpt_model::{
    AtmosMonitor, BINAURAL_CHANNELS, BinauralDistance, ChannelFormat, Clip, Insert, Route, Session, SourceAudio, SourceId, SurroundPan, TrackId,
    TrackKind,
};
use std::sync::Arc;

const BLOCK: usize = 512;
const LEN: usize = 48_000;

fn noise(seed: u32, gain: f32) -> Vec<f32> {
    let mut st = seed;
    (0..LEN)
        .map(|_| {
            st = st.wrapping_mul(1664525).wrapping_add(1013904223);
            ((st >> 8) as f32 / 16_777_216.0 - 0.5) * gain
        })
        .collect()
}

fn add(s: &mut Session, name: &str, source: u64, signal: Vec<f32>, object: bool, x: f32) -> TrackId {
    s.pool.insert(SourceId(source), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![signal] })));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some(name));
    let clip = s.new_clip_id();
    let track = s.track_mut(t).unwrap();
    track.playlist_mut().unwrap().clips.push(Clip::audio(clip, name, SourceId(source), 0, 0, LEN as i64));
    track.mixer.output = Route::Main;
    track.mixer.surround = Some(SurroundPan { x, y: 1.0, ..SurroundPan::default() });
    s.edit.set_flag(&format!("object.{}", t.0), object);
    t
}

fn base() -> Session {
    let mut s = Session::default();
    s.outputs[0].format = ChannelFormat::Atmos714;
    s.atmos.monitor = AtmosMonitor::Binaural;
    s
}

fn run(s: &Session, split: bool, mut each: impl FnMut(&mut MixEngine, &mut Vec<Vec<f32>>)) {
    let mut eng = MixEngine::new(48_000.0, BLOCK);
    eng.split_objects = split;
    let mut out = vec![vec![0.0f32; BLOCK]; 12];
    for pos in (0..LEN - BLOCK).step_by(BLOCK) {
        eng.render(s, pos as i64, BLOCK, &mut out);
        each(&mut eng, &mut out);
    }
}

#[test]
fn a_session_at_one_distance_never_splits() {
    let mut s = base();
    add(&mut s, "bed", 1, noise(1, 0.3), false, 0.0);
    let o = add(&mut s, "obj", 2, noise(2, 0.3), true, -1.0);
    assert_eq!(split_modes(&s), 0);
    s.track_mut(o).unwrap().mixer.binaural_distance = BinauralDistance::Near;
    assert_eq!(split_modes(&s), 1 << BinauralDistance::Near.index());
    s.atmos.monitor = AtmosMonitor::Speakers;
    assert_eq!(split_modes(&s), 0);
}

#[test]
fn the_feed_equals_the_object_in_the_main_mix_after_the_master_fader() {
    let mut s = base();
    let o = add(&mut s, "obj", 2, noise(2, 0.3), true, -1.0);
    s.track_mut(o).unwrap().mixer.binaural_distance = BinauralDistance::Near;
    let m = s.add_track(TrackKind::Master, ChannelFormat::Atmos714, Some("Master"));
    s.track_mut(m).unwrap().mixer.volume_db = -6.0;
    let mut energy = 0.0f32;
    run(&s, true, |eng, out| {
        let feed = &eng.object_feeds[BinauralDistance::Near.index()];
        for (a, b) in out.iter().zip(feed) {
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() < 1e-5, "{x} {y}");
                energy += x * x;
            }
        }
    });
    assert!(energy > 1.0);
}

#[test]
fn the_feed_follows_the_latency_of_a_master_insert() {
    let mut s = base();
    s.edit.delay_compensation = true;
    let mut signal = vec![0.0f32; LEN];
    signal[2000] = 0.1;
    let o = add(&mut s, "obj", 2, signal, true, -1.0);
    s.track_mut(o).unwrap().mixer.binaural_distance = BinauralDistance::Far;
    let m = s.add_track(TrackKind::Master, ChannelFormat::Atmos714, Some("Master"));
    let mut maximizer = Insert::new("maximizer");
    maximizer.params.insert("threshold".into(), 0.0);
    maximizer.params.insert("ceiling".into(), 0.0);
    s.track_mut(m).unwrap().mixer.inserts[0] = Some(maximizer);
    let (mut main, mut feed) = (Vec::new(), Vec::new());
    let channel = ChannelFormat::Atmos714.speakers().iter().position(|sp| *sp == openpt_model::Speaker::L).unwrap();
    run(&s, true, |eng, out| {
        main.extend_from_slice(&out[channel]);
        feed.extend_from_slice(&eng.object_feeds[BinauralDistance::Far.index()][channel][..BLOCK]);
    });
    let peak = |v: &[f32]| v.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i).unwrap();
    assert!(peak(&main) > 2000, "the maximizer has lookahead: {}", peak(&main));
    assert_eq!(peak(&main), peak(&feed));
}

#[test]
fn an_object_and_the_bed_are_rendered_at_their_own_distances() {
    let mut s = base();
    s.atmos.binaural_distance = [BinauralDistance::Far; BINAURAL_CHANNELS];
    add(&mut s, "bed", 1, noise(1, 0.2), false, 1.0);
    let o = add(&mut s, "obj", 2, noise(2, 0.2), true, -1.0);
    s.track_mut(o).unwrap().mixer.binaural_distance = BinauralDistance::Near;
    let objects = split_modes(&s);
    assert_ne!(objects, 0);

    let mut together = Monitor::with_objects(&s.atmos, ChannelFormat::Atmos714, 48_000, objects);
    let mut near_settings = s.atmos;
    near_settings.binaural_distance = [BinauralDistance::Near; BINAURAL_CHANNELS];
    let mut far = Monitor::new(&s.atmos, ChannelFormat::Atmos714, 48_000);
    let mut near = Monitor::new(&near_settings, ChannelFormat::Atmos714, 48_000);

    let mut eng = MixEngine::new(48_000.0, BLOCK);
    eng.split_objects = true;
    let mut out = vec![vec![0.0f32; BLOCK]; 12];
    let mut compared = 0.0f32;
    for pos in (0..LEN - BLOCK).step_by(BLOCK) {
        eng.render(&s, pos as i64, BLOCK, &mut out);
        let feed = eng.object_feeds[BinauralDistance::Near.index()].clone();
        let mut bed: Vec<Vec<f32>> = out.iter().zip(&feed).map(|(a, b)| a.iter().zip(b).map(|(x, y)| x - y).collect()).collect();
        let mut object = feed;
        far.process(&mut bed, BLOCK);
        near.process(&mut object, BLOCK);
        together.process_objects(&mut out, &eng.object_feeds, BLOCK);
        for c in 0..2 {
            for i in 0..BLOCK {
                assert!((out[c][i] - bed[c][i] - object[c][i]).abs() < 2e-4, "pos {pos} ch {c} i {i}");
                compared += out[c][i] * out[c][i];
            }
        }
    }
    assert!(compared > 1.0);
}
