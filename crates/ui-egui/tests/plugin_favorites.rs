use egui::{Event, Modifiers, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use openpt_ui_egui::{MainWindow, Services, SoundApp};
use serde_json::json;

fn harness() -> Harness<'static, SoundApp> {
    let mut engine = openpt_engine::Engine::default();
    engine.execute("track.new", &json!({"name": "Vox"})).unwrap();
    let mut app = SoundApp::new(engine, None, Services::default());
    app.ui.window = MainWindow::Mix;
    app.ui.plugin_menu_org = "category".into();
    let mut h = Harness::builder().with_size(vec2(1400.0, 1000.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn open(h: &mut Harness<SoundApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn ctrl_click(h: &mut Harness<SoundApp>, label: &str) {
    let at = h.get_by_label(label).rect().center();
    h.event(Event::ModifiersChanged(Modifiers::COMMAND));
    h.event(Event::PointerMoved(at));
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::COMMAND });
    h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::COMMAND });
    h.run_steps(2);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn a_ctrl_click_makes_a_plug_in_a_favourite_listed_on_top_of_the_insert_menu() {
    let plugin = openpt_dsp::plugins()
        .iter()
        .copied()
        .find(|p| !p.is_instrument && p.category == openpt_dsp::Category::Dynamics)
        .expect("a built-in dynamics plug-in");
    let mut h = harness();
    open(&mut h, "Insert A");
    assert!(h.query_by_label(plugin.name).is_none(), "plug-ins sit in their category before they are favourites");
    h.get_by_label_contains("Dynamics").click();
    h.run_steps(3);
    ctrl_click(&mut h, plugin.name);
    assert!(h.state().engine.session().tracks[0].mixer.inserts[0].is_none(), "a ctrl-click does not insert");
    assert_eq!(h.state().ui.plugin_favorites, [plugin.id.to_string()]);

    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    if h.query_by_label("Insert A").is_none() {
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
    }
    open(&mut h, "Insert A");
    ctrl_click(&mut h, plugin.name);
    assert!(h.state().ui.plugin_favorites.is_empty(), "a second ctrl-click removes the favourite");

    h.state_mut().ui.plugin_favorites = vec![plugin.id.to_string()];
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    if h.query_by_label("Insert A").is_none() {
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
    }
    open(&mut h, "Insert A");
    h.get_by_label(plugin.name).click();
    h.run_steps(3);
    let insert = h.state().engine.session().tracks[0].mixer.inserts[0].clone().expect("a plain click on a favourite inserts it");
    assert_eq!(insert.plugin, plugin.id);
}
