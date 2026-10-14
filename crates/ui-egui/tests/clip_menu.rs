use egui::{Event, Modifiers, PointerButton, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use openpt_ui_egui::{Services, SoundApp};

fn step(h: &mut Harness<'_, SoundApp>) {
    let mut raw = std::mem::take(h.input_mut());
    h.state_mut().raw_input_hook(&mut raw);
    *h.input_mut() = raw;
    h.step();
}

fn press(h: &mut Harness<'_, SoundApp>, x: f32, y: f32, button: PointerButton) {
    let pos = pos2(x, y);
    let s = &mut h.state_mut().synthetic;
    s.push(Event::PointerMoved(pos));
    s.push(Event::PointerButton { pos, button, pressed: true, modifiers: Modifiers::NONE });
    s.push(Event::PointerButton { pos, button, pressed: false, modifiers: Modifiers::NONE });
}

fn kick_clip_count(h: &Harness<'_, SoundApp>) -> usize {
    let s = h.state().engine.session();
    s.tracks.iter().find(|t| t.name == "Kick").map_or(0, |t| t.clips().len())
}

#[test]
fn clip_menu_stays_open_while_moving_onto_delete() {
    let engine = openpt_engine::demo::demo_engine();
    let app = SoundApp::new(engine, None, Services::default());
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_pixels_per_point(2.0).build_ui_state(
        |ui, app: &mut SoundApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for _ in 0..6 {
        step(&mut h);
    }
    let before = kick_clip_count(&h);
    assert!(before > 0, "the demo session should have a recorded Kick clip");

    let (x, y) = {
        let app = h.state();
        let kick = app.engine.session().tracks.iter().find(|t| t.name == "Kick").map(|t| t.id.0).unwrap();
        let row = app.edit_layout.rows.iter().find(|r| r.0 == kick).map(|r| r.1).unwrap();
        let tl = app.edit_layout.timeline;
        (tl[0] + (tl[2] - tl[0]) * 0.8, (row[1] + row[3]) * 0.5)
    };
    press(&mut h, x, y, PointerButton::Secondary);
    for _ in 0..10 {
        step(&mut h);
    }
    let delete = h.get_by_label("Delete").rect().center();
    for (x, y) in [(x + 20.0, y + 30.0), (delete.x, delete.y)] {
        h.state_mut().synthetic.push(Event::PointerMoved(pos2(x, y)));
        for _ in 0..10 {
            step(&mut h);
        }
    }
    press(&mut h, delete.x, delete.y, PointerButton::Primary);
    for _ in 0..15 {
        step(&mut h);
    }

    assert_eq!(kick_clip_count(&h), before - 1, "clicking Delete in the clip menu should remove the clip");
}
