use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use openpt_ui_egui::dialogs::{Dialog, NewTrackRow};
use openpt_ui_egui::{Services, SoundApp};

#[test]
fn plus_adds_a_row_and_create_makes_every_row() {
    let mut app = SoundApp::new(openpt_engine::Engine::default(), None, Services::default());
    app.dialogs.open = Some(Dialog::NewTracks { rows: vec![NewTrackRow::default()] });
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(3);
    let before = h.state().engine.session().tracks.len();
    h.get_by_label("+").click();
    h.run_steps(3);
    if let Some(Dialog::NewTracks { rows }) = &mut h.state_mut().dialogs.open {
        assert_eq!(rows.len(), 2);
        rows[1].kind = "aux".into();
        rows[1].format = "Stereo".into();
        rows[1].name = "Verb".into();
        rows[1].count = 2;
    } else {
        panic!("dialog closed");
    }
    h.run_steps(2);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Create").click();
    h.run_steps(3);
    let s = h.state().engine.session();
    assert_eq!(s.tracks.len(), before + 3);
    let auxes = s.tracks.iter().filter(|t| t.kind == openpt_model::TrackKind::Aux).count();
    assert_eq!(auxes, 2);
}
