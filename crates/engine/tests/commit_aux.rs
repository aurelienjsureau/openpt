use openpt_audio_io::{AudioBuffer, FileFormat};
use openpt_engine::Engine;
use openpt_model::{ChannelFormat, Clip, Route, Session, TrackId, TrackKind};
use openpt_time::Range;
use serde_json::json;

fn audio_track(s: &mut Session, name: &str, level: f32) -> TrackId {
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some(name));
    let buf = AudioBuffer { sample_rate: 48_000, channels: vec![vec![level; 48_000]] };
    let src = openpt_engine::io::add_source(s, name, buf, None, FileFormat::Wav);
    let id = s.new_clip_id();
    openpt_engine::edit::place_clip(s, t, Clip::audio(id, name, src, 0, 0, 48_000));
    t
}

fn peak(audio: &[Vec<f32>]) -> f32 {
    audio.iter().flatten().fold(0.0, |m, x| m.max(x.abs()))
}

#[test]
fn an_aux_commits_what_its_bus_carries() {
    let mut s = Session::default();
    let src = audio_track(&mut s, "Snare", 0.25);
    audio_track(&mut s, "Other", 0.5);
    let bus = s.add_bus("Verb", ChannelFormat::Stereo);
    let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Verb"));
    s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
    let mut e = Engine::new(s);
    e.execute("mix.send", &json!({"track": src.0, "slot": 0, "bus": "Verb", "pre_fader": true})).unwrap();

    let range = Range::new(0, 48_000);
    let mono = openpt_mix::render_track_pre_fader(e.session(), src, range);
    assert_eq!(mono.len(), 1);
    assert!((peak(&mono) - 0.25).abs() < 1e-5, "a mono track commits at unity: {}", peak(&mono));
    let heard = peak(&openpt_mix::render_track_pre_fader(e.session(), aux, range));
    assert!(heard > 0.1, "the send reaches the committed aux: {heard}");
    e.session_mut().track_mut(src).unwrap().mixer.output = Route::None;
    let send_only = peak(&openpt_mix::render_track_pre_fader(e.session(), aux, range));
    assert!((heard - send_only).abs() < 1e-4, "the source's own output stays out: {heard} vs {send_only}");

    e.execute("track.commit", &json!({"tracks": [src.0]})).unwrap();
    assert!(e.session().track(src).unwrap().inactive);
    let after = peak(&openpt_mix::render_track_pre_fader(e.session(), aux, range));
    assert!((heard - after).abs() < 1e-4, "a source committed first still feeds the aux: {after}");

    let r = e.execute("track.commit", &json!({"tracks": [aux.0]})).unwrap();
    assert_eq!(r["tracks"].as_array().map(Vec::len), Some(1), "{r}");
    assert!(e.session().track(aux).unwrap().inactive);
}

#[test]
fn a_frozen_track_plays_its_render_without_its_plugins() {
    let mut s = Session::default();
    let t = audio_track(&mut s, "Gtr", 0.25);
    s.track_mut(t).unwrap().mixer.inserts[0] = Some(openpt_model::Insert::new("maximizer"));
    let mut e = Engine::new(s);
    let range = Range::new(0, 48_000);
    let live = openpt_mix::render_range(e.session(), range, 512);
    let r = e.execute("track.freeze", &json!({"tracks": [t.0]})).unwrap();
    assert_eq!(r["frozen"], true, "{r}");
    let tr = e.session().track(t).unwrap();
    assert!(tr.is_frozen() && tr.freeze.is_some());
    let frozen = openpt_mix::render_range(e.session(), range, 512);
    let diff = live.iter().flatten().zip(frozen.iter().flatten()).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(diff < 1e-3, "the frozen render sounds as the live track: {diff}");

    e.execute("track.freeze", &json!({"tracks": [t.0]})).unwrap();
    assert!(!e.session().track(t).unwrap().is_frozen(), "a second Freeze unfreezes");
    assert!(e.undo());
    assert!(e.session().track(t).unwrap().is_frozen());
}

#[test]
fn a_committed_track_keeps_its_pan() {
    let mut s = Session::default();
    let t = audio_track(&mut s, "Tom", 0.25);
    s.track_mut(t).unwrap().mixer.pan = vec![-0.6];
    let mut e = Engine::new(s);
    let r = e.execute("track.commit", &json!({"tracks": [t.0]})).unwrap();
    let nt = TrackId(r["tracks"][0].as_u64().unwrap());
    assert_eq!(e.session().track(nt).unwrap().mixer.pan, vec![-0.6]);
}
