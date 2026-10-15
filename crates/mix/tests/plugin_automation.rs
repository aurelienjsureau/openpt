#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openpt_audio_io::AudioBuffer;
use openpt_mix::render_range;
use openpt_model::{AutoParam, AutomationLane, AutomationPoint, ChannelFormat, Clip, Insert, Session, SourceAudio, SourceId, TrackKind};
use openpt_time::Range;
use std::sync::Arc;

#[test]
fn an_untouched_parameter_follows_its_lane() {
    let mut s = Session::default();
    s.pool.insert(SourceId(1), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48000, channels: vec![vec![0.5; 4800]] })));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("T"));
    let c = s.new_clip_id();
    let tr = s.track_mut(t).unwrap();
    tr.playlist_mut().unwrap().clips.push(Clip::audio(c, "c", SourceId(1), 0, 0, 4800));
    tr.mixer.inserts[0] = Some(Insert::new("gain"));
    let mut lane = AutomationLane::new(AutoParam::Plugin { slot: 0, param: "gain".into() });
    lane.points = vec![AutomationPoint { at: 0, value: -12.0 }, AutomationPoint { at: 4800, value: -12.0 }];
    tr.automation.push(lane);
    let out = render_range(&s, Range::new(0, 4800), 480);
    let late = out[0][4000];
    let plain = {
        s.track_mut(t).unwrap().automation.clear();
        render_range(&s, Range::new(0, 4800), 480)[0][4000]
    };
    assert!((late / plain - 10f32.powf(-12.0 / 20.0)).abs() < 0.02, "automated {late} vs plain {plain}");
}
