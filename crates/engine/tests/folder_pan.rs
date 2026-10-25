use openpt_engine::demo::demo_engine;
use openpt_time::Range;
use serde_json::json;

fn balance(e: &openpt_engine::Engine) -> (f32, f32) {
    let out = openpt_mix::render_range(e.session(), Range::new(443_077, 443_077 + 96_000), 512);
    let rms = |c: &[f32]| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt();
    (rms(&out[0]), rms(&out[1]))
}

#[test]
fn folder_members_read_their_pan_automation() {
    let mut e = demo_engine();
    let solo = e.session().tracks.iter().filter(|t| t.name != "Kick").map(|t| t.id.0).collect::<Vec<_>>();
    e.execute("mix.mute", &json!({"tracks": solo, "mute": true})).unwrap();
    e.execute("automation.set_point", &json!({"track": "Kick", "param": "pan", "at": 0, "value": -1.0})).unwrap();
    let outside = balance(&e);
    e.execute("track.move_to_new_folder", &json!({"tracks": ["Kick"], "name": "Drums"})).unwrap();
    let inside = balance(&e);
    assert!(outside.0 > 10.0 * outside.1, "hard left outside: {outside:?}");
    assert!(inside.0 > 10.0 * inside.1, "hard left inside: {inside:?}");
}

#[test]
fn stereo_folder_members_read_both_pans() {
    let mut e = demo_engine();
    let s = e.session();
    let st = s.tracks.iter().find(|t| t.mixer.pan.len() == 2 && !t.clips().is_empty()).expect("a stereo track in the demo");
    let (name, start) = (st.name.clone(), st.clips()[0].start);
    let others = s.tracks.iter().filter(|t| t.name != name).map(|t| t.id.0).collect::<Vec<_>>();
    e.execute("mix.mute", &json!({"tracks": others, "mute": true})).unwrap();
    e.execute("automation.set_point", &json!({"track": name, "param": "pan", "at": 0, "value": -1.0})).unwrap();
    e.execute("automation.set_point", &json!({"track": name, "param": "pan2", "at": 0, "value": -1.0})).unwrap();
    let rms = |e: &openpt_engine::Engine| {
        let out = openpt_mix::render_range(e.session(), Range::new(start, start + 96_000), 512);
        let r = |c: &[f32]| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt();
        (r(&out[0]), r(&out[1]))
    };
    let outside = rms(&e);
    e.execute("track.move_to_new_folder", &json!({"tracks": [name], "name": "Grp"})).unwrap();
    let inside = rms(&e);
    assert!(outside.0 > 10.0 * outside.1 && inside.0 > 10.0 * inside.1, "{outside:?} {inside:?}");
}
