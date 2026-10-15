#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openpt_audio_io::AudioBuffer;
use openpt_engine::Engine;
use openpt_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};
use serde_json::json;
use std::sync::Arc;

const BEAT: i64 = 28_800;

fn session() -> (Engine, u64) {
    let mut e = Engine::default();
    let len = (17 * BEAT) as usize;
    let mut audio = vec![0.0f32; len];
    for k in 0..17 {
        let at = (k * BEAT) as usize;
        for i in 0..960 {
            if let Some(x) = audio.get_mut(at + i) {
                *x = 0.8 * (i as f32 * 0.13).sin() * (-(i as f32) / 200.0).exp();
            }
        }
    }
    let s = e.session_mut();
    s.pool.insert(SourceId(7), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![audio] })));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Drums"));
    let c = s.new_clip_id();
    s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(c, "drums", SourceId(7), 0, 0, len as i64));
    (e, t.0)
}

fn run(e: &mut Engine, t: u64, op: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut p = json!({"tracks": [t], "start": 0, "end": 16 * BEAT, "start_bar_beat": "1|1", "end_bar_beat": "5|1", "contains": "1/4", "resolution": "beats", "operation": op});
    if let (Some(m), Some(x)) = (p.as_object_mut(), extra.as_object()) {
        m.extend(x.clone());
    }
    e.execute("event.beat_detective", &p).unwrap()
}

#[test]
fn triggers_land_on_the_played_beats() {
    let (mut e, t) = session();
    let r = run(&mut e, t, "analyze", json!({}));
    let beats: Vec<i64> = r["beats"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    assert!(beats.len() >= 16, "{r}");
    for (k, b) in beats.iter().enumerate().take(16) {
        assert!((b - k as i64 * BEAT).abs() < 200, "beat {k} at {b}");
    }
    assert_eq!(r["triggers"][4]["bar_beat"], "2|1|000");
    assert!((r["bpm"].as_f64().unwrap() - 100.0).abs() < 0.1);
}

#[test]
fn bar_beat_markers_make_the_session_follow_the_playing() {
    let (mut e, t) = session();
    run(&mut e, t, "markers", json!({}));
    let s = e.session();
    let bar3 = s.tempo.samples_at_bar_beat(openpt_time::BarBeat { bar: 3, beat: 1, tick: 0 }, s.sample_rate);
    assert!((bar3 - 8 * BEAT).abs() < 300, "bar 3 at {bar3}");
}

#[test]
fn separated_clips_conform_to_the_session_beats() {
    let (mut e, t) = session();
    run(&mut e, t, "separate", json!({}));
    run(&mut e, t, "conform", json!({"strength": 100}));
    let s = e.session();
    let starts: Vec<i64> = s.track(openpt_model::TrackId(t)).unwrap().clips().iter().map(|c| c.start).collect();
    assert!(starts.iter().any(|st| (st - (4 * 24_000 - 240)).abs() < 300), "{starts:?}");
    run(&mut e, t, "smooth", json!({}));
    let clips = e.session().track(openpt_model::TrackId(t)).unwrap().clips().to_vec();
    for w in clips.windows(2) {
        assert!(w[1].start <= w[0].end(), "no gap left between {} and {}", w[0].end(), w[1].start);
    }
}

#[test]
fn a_swung_groove_is_extracted_and_quantizes_midi() {
    let mut e = Engine::default();
    let six = 6_000i64;
    let len = (16 * six + 4_800) as usize;
    let mut audio = vec![0.0f32; len];
    for k in 0..16 {
        let at = (k * six + if k % 2 == 1 { 1_500 } else { 0 }) as usize;
        for i in 0..400 {
            if let Some(x) = audio.get_mut(at + i) {
                *x = 0.8 * (i as f32 * 0.21).sin() * (-(i as f32) / 80.0).exp();
            }
        }
    }
    let s = e.session_mut();
    s.pool.insert(SourceId(8), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![audio] })));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Hats"));
    let c = s.new_clip_id();
    s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(c, "hats", SourceId(8), 0, 0, len as i64));
    let r = e
        .execute(
            "event.beat_detective",
            &json!({"tracks": [t.0], "start": 0, "end": 16 * six, "start_bar_beat": "1|1", "end_bar_beat": "2|1", "contains": "1/16", "resolution": "sub_beats", "operation": "groove", "name": "Swing"}),
        )
        .unwrap();
    assert_eq!(r["done"], 16, "{r}");
    let m = e.execute("track.new", &json!({"kind": "midi"})).unwrap()["tracks"][0].as_u64().unwrap();
    let clip = e.execute("midi.new_clip", &json!({"track": m, "start": 0, "end": 16 * six})).unwrap()["clip"].as_u64().unwrap();
    for k in 0..4 {
        e.execute("event.midi_note", &json!({"clip": clip, "pitch": 60, "start_ticks": k * 240, "length_ticks": 120})).unwrap();
    }
    e.execute("event.quantize", &json!({"clips": [clip], "groove": "Swing"})).unwrap();
    let notes = e.execute("midi.notes", &json!({"clip": clip})).unwrap();
    let starts: Vec<i64> = notes["notes"].as_array().unwrap().iter().map(|n| n["start"].as_i64().unwrap()).collect();
    assert!((starts[1] - 300).abs() <= 10 && (starts[3] - 780).abs() <= 10, "swung: {starts:?}");
    assert!(starts[0].abs() <= 10 && (starts[2] - 480).abs() <= 10, "straight: {starts:?}");
}

#[test]
fn triggers_follow_a_drummer_who_speeds_up() {
    let mut e = Engine::default();
    let mut at = 0.0f64;
    let mut beats = Vec::new();
    for k in 0..17 {
        beats.push(at as i64);
        let bpm = 100.0 + 12.0 * k as f64 / 16.0;
        at += 60.0 / bpm * 48_000.0;
    }
    let len = (*beats.last().unwrap() + 9_600) as usize;
    let mut audio = vec![0.0f32; len];
    for b in &beats {
        for i in 0..960 {
            if let Some(x) = audio.get_mut(*b as usize + i) {
                *x = 0.8 * (i as f32 * 0.13).sin() * (-(i as f32) / 200.0).exp();
            }
        }
    }
    let s = e.session_mut();
    s.pool.insert(SourceId(9), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![audio] })));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Drums"));
    let c = s.new_clip_id();
    s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(c, "drums", SourceId(9), 0, 0, len as i64));
    let end = beats[16];
    let r = e
        .execute(
            "event.beat_detective",
            &json!({"tracks": [t.0], "start": 0, "end": end, "start_bar_beat": "1|1", "end_bar_beat": "5|1", "contains": "1/4", "resolution": "beats", "operation": "analyze"}),
        )
        .unwrap();
    let found: Vec<i64> = r["beats"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap()).collect();
    assert!(found.len() >= 16, "{r}");
    for (k, f) in found.iter().enumerate().take(16) {
        assert!((f - beats[k]).abs() < 300, "beat {k}: found {f}, played {}", beats[k]);
    }
}
