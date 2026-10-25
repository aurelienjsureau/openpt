use openpt_engine::demo::demo_engine;
use openpt_model::{ChannelFormat, Group, GroupId, TrackKind};
use openpt_time::Range;
use serde_json::json;

#[test]
fn a_vca_on_a_routing_folder_solos_and_mutes_its_contents() {
    check(false);
}

#[test]
fn a_vca_on_a_routing_folder_solos_what_plays_into_it() {
    check(true);
}

fn check(outside: bool) {
    let mut e = demo_engine();
    let kick = e.session().tracks.iter().find(|t| t.name == "Kick").unwrap();
    let start = kick.clips()[0].start;
    e.execute("track.move_to_new_folder", &json!({"tracks": ["Kick"], "name": "Drums"})).unwrap();
    let s = e.session_mut();
    let folder = s.tracks.iter().find(|t| t.is_folder()).unwrap().id;
    if outside {
        s.tracks.iter_mut().filter(|t| t.folder == Some(folder)).for_each(|t| t.folder = None);
    }
    let g = GroupId(s.alloc());
    s.groups.push(Group { id: g, name: "Drums".into(), letter: 'a', members: vec![folder], edit: true, mix: true, active: true, color: [0; 3], attributes: Vec::new() });
    let vca = s.add_track(TrackKind::Vca, ChannelFormat::Mono, Some("VCA Drums"));
    s.track_mut(vca).unwrap().mixer.vca_group = Some(g);
    let rms = |e: &openpt_engine::Engine| {
        let out = openpt_mix::render_range(e.session(), Range::new(start, start + 48_000), 512);
        (out[0].iter().map(|x| x * x).sum::<f32>() / out[0].len() as f32).sqrt()
    };
    let all = rms(&e);
    e.execute("mix.solo", &json!({"tracks": [vca.0], "value": true})).unwrap();
    let soloed = rms(&e);
    assert!(soloed > 1e-4 && soloed < all, "kick alone in solo: {soloed} (all {all})");
    e.execute("mix.solo", &json!({"tracks": [vca.0], "value": false})).unwrap();
    e.execute("mix.mute", &json!({"tracks": [vca.0], "value": true})).unwrap();
    let muted = rms(&e);
    assert!(muted < all - 1e-4, "kick muted by the VCA: {muted} (all {all})");
}
