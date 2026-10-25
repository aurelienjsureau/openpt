use openpt_audio_io::AudioBuffer;
use openpt_model::{ChannelFormat, Clip, Insert, Session, SourceAudio, SourceId, TrackKind};
use openpt_time::Range;
use std::sync::Arc;

fn render(mix: f32) -> f32 {
    let mut s = Session::default();
    let frames = 48_000;
    let quiet: Vec<f32> = (0..frames).map(|i| 0.005 * ((i as f32) * 0.03).sin()).collect();
    s.pool.insert(SourceId(7), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![quiet] })));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    let id = s.new_clip_id();
    let tr = s.track_mut(t).expect("track");
    tr.playlist_mut().unwrap().clips.push(Clip::audio(id, "c", SourceId(7), 0, 0, frames as i64));
    let info = openpt_dsp::plugin_info("open_g").expect("registered");
    let mut ins = Insert::new("open_g");
    for p in info.params {
        ins.params.insert(p.id.to_string(), p.default);
    }
    ins.params.insert("mix".into(), mix);
    tr.mixer.inserts[0] = Some(ins);
    let out = openpt_mix::render_range(&s, Range::new(0, frames as i64), 512);
    out[0][24_000..].iter().fold(0.0, |m, v| m.max(v.abs()))
}

#[test]
fn dry_wet_brings_back_the_closed_signal() {
    let (wet, dry) = (render(100.0), render(0.0));
    assert!(dry > 10.0 * wet, "wet {wet} dry {dry}");
}

#[test]
fn master_bypass_automation_lets_the_signal_through() {
    use openpt_model::{AutoParam, AutomationLane, AutomationPoint, BYPASS_PARAM};
    let mut s = Session::default();
    let frames = 48_000;
    let quiet: Vec<f32> = (0..frames).map(|i| 0.005 * ((i as f32) * 0.03).sin()).collect();
    s.pool.insert(SourceId(7), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![quiet] })));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
    let id = s.new_clip_id();
    let tr = s.track_mut(t).expect("track");
    tr.playlist_mut().unwrap().clips.push(Clip::audio(id, "c", SourceId(7), 0, 0, frames as i64));
    let mut ins = Insert::new("open_g");
    ins.automated.push(BYPASS_PARAM.into());
    tr.mixer.inserts[0] = Some(ins);
    let mut lane = AutomationLane::new(AutoParam::Plugin { slot: 0, param: BYPASS_PARAM.into() });
    lane.points = vec![AutomationPoint { at: 0, value: 0.0 }, AutomationPoint { at: 23_999, value: 0.0 }, AutomationPoint { at: 24_000, value: 1.0 }];
    tr.automation.push(lane);
    let out = openpt_mix::render_range(&s, Range::new(0, frames as i64), 512);
    let peak = |a: usize, b: usize| out[0][a..b].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let (gated, through) = (peak(12_000, 23_000), peak(30_000, 47_000));
    assert!(through > 0.003 && gated < through / 10.0, "gated {gated} through {through}");
}
