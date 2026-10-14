use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use openpt_ui_egui::{Services, SoundApp};
use serde_json::json;

#[test]
fn peripherals_shows_the_midi_controllers_page() {
    let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
    app.run("window.peripherals", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    for label in ["MIDI Controllers", "Receive From", "Send To", "# Ch's", "#1", "#4"] {
        assert!(h.query_by_label(label).is_some(), "{label}");
    }
    assert_eq!(h.state().ui.peripherals.len(), 4);
    h.get_by_label("OK").click();
    h.run_steps(2);
    assert!(!h.state().ui.show_peripherals);
}
