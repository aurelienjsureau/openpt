use egui::{pos2, vec2};
use egui_kittest::Harness;
use openpt_ui_egui::{Services, SoundApp};

#[test]
fn the_pan_and_volume_faders_are_only_as_big_as_their_contents() {
    let app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
    let pad = app.engine.session().track_by_name("Pad").unwrap().id;
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    for (which, max) in [(1usize, vec2(300.0, 90.0)), (0usize, vec2(110.0, 230.0))] {
        h.ctx.data_mut(|d| d.insert_temp(egui::Id::new("io_mini_fader"), (pad.0, which, pos2(600.0, 400.0))));
        h.run_steps(3);
        let r = h.ctx.memory(|m| m.area_rect(egui::Id::new("io_mini_fader").with("area"))).expect("open");
        assert!(r.width() <= max.x && r.height() <= max.y, "fader {which}: {r:?}");
        h.ctx.data_mut(|d| d.remove::<(u64, usize, egui::Pos2)>(egui::Id::new("io_mini_fader")));
        h.run_steps(2);
    }
}
