use egui::{Event, Modifiers, PointerButton, pos2, vec2};
use egui_kittest::Harness;
use openpt_ui_egui::{Services, SoundApp};
use serde_json::json;

#[test]
fn a_dragged_track_scrolls_the_tracks_at_the_edge_and_lands_where_dropped() {
    let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
    app.run("edit.select", json!({"tracks": []})).unwrap();
    app.run("track.new", json!({"count": 12})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    let kick = h.state().engine.session().track_by_name("Kick").unwrap().id;
    let row = h.state().edit_layout.rows.iter().find(|r| r.0 == kick.0).map(|r| r.1).unwrap();
    let [_, _, _, bottom] = h.state().edit_layout.timeline;
    let grab = pos2(row[0] + 40.0, row[1] + 8.0);
    let press = |h: &mut Harness<'_, SoundApp>, at, pressed| {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    };
    h.event(Event::PointerMoved(grab));
    h.run_steps(1);
    press(&mut h, grab, true);
    for k in 1..=20 {
        h.event(Event::PointerMoved(pos2(grab.x, grab.y + (bottom - 2.0 - grab.y) * k as f32 / 20.0)));
        h.run_steps(1);
    }
    h.run_steps(20);
    assert!(h.state().edit_layout.scroll_y > 0.0, "no scroll while holding at the edge");
    press(&mut h, pos2(grab.x, bottom - 2.0), false);
    h.run_steps(2);
    let s = h.state().engine.session();
    assert!(s.track_index(kick).unwrap() > 6, "Kick moved down past the first screenful: {}", s.track_index(kick).unwrap());
}
