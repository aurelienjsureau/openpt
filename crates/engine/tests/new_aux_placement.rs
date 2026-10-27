use openpt_engine::demo::demo_engine;
use openpt_model::{ChannelFormat, TrackKind};
use serde_json::json;

#[test]
fn a_new_aux_from_an_output_lands_right_under_a_track_below_the_master() {
    let mut e = demo_engine();
    let s = e.session_mut();
    s.add_track(TrackKind::Master, ChannelFormat::Stereo, Some("Sum"));
    let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Late"));
    let i = s.track_index(t).unwrap();
    let m = s.tracks.remove(s.tracks.iter().position(|x| x.kind == TrackKind::Master).unwrap());
    s.tracks.insert(i - 1, m);
    let i = e.session().track_index(t).unwrap();
    let r = e.execute("mix.route_to_new_aux", &json!({"tracks": [t.0]})).unwrap();
    let aux = e.session().tracks.iter().position(|x| x.id.0 == r["aux"].as_u64().unwrap()).unwrap();
    assert_eq!(e.session().track_index(t), Some(i));
    assert_eq!(aux, i + 1);
}
