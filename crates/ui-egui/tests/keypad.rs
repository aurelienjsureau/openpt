use egui::Key;
use openpt_engine::TransportRequest;
use openpt_ui_egui::shortcuts::{self, Held, PadKey};
use openpt_ui_egui::{Services, SoundApp};
use serde_json::json;

fn app() -> SoundApp {
    SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default())
}

fn tap(app: &mut SoundApp, key: PadKey) {
    shortcuts::pad(app, key, true, Held::default());
    shortcuts::pad(app, key, false, Held::default());
}

fn marker_at(app: &mut SoundApp, at: i64) {
    app.engine.execute("markers.add", &json!({"at": at})).unwrap();
}

#[test]
fn transport_mode_digits_run_the_transport_and_period_number_period_recalls_a_marker() {
    let mut app = app();
    app.ui.numeric_keypad = "transport".into();
    tap(&mut app, PadKey::Digit(0));
    assert_eq!(app.engine.transport_requests.last(), Some(&TransportRequest::TogglePlay));
    let click = app.engine.session().edit.click;
    tap(&mut app, PadKey::Digit(7));
    assert_ne!(app.engine.session().edit.click, click, "7 toggles the click");
    marker_at(&mut app, 48_000);
    for k in [PadKey::Dot, PadKey::Digit(1), PadKey::Dot] {
        tap(&mut app, k);
    }
    assert_eq!(app.engine.session().edit.selection.start, 48_000);
}

#[test]
fn classic_mode_takes_the_number_first() {
    let mut app = app();
    app.ui.numeric_keypad = "classic".into();
    marker_at(&mut app, 96_000);
    tap(&mut app, PadKey::Digit(1));
    tap(&mut app, PadKey::Dot);
    assert_eq!(app.engine.session().edit.selection.start, 96_000);
}

#[test]
fn shuttle_mode_plays_while_held() {
    let mut app = app();
    app.ui.numeric_keypad = "shuttle".into();
    shortcuts::pad(&mut app, PadKey::Digit(9), true, Held::default());
    assert_eq!(app.engine.transport_requests.last(), Some(&TransportRequest::Shuttle(4000)));
    shortcuts::pad(&mut app, PadKey::Digit(9), false, Held::default());
    assert_eq!(app.engine.transport_requests.last(), Some(&TransportRequest::Shuttle(0)));
    shortcuts::pad(&mut app, PadKey::Digit(5), true, Held::default());
    shortcuts::pad(&mut app, PadKey::Digit(4), true, Held::default());
    assert_eq!(app.engine.transport_requests.last(), Some(&TransportRequest::Shuttle(-500)), "5+4 is ½× backwards");
}

#[test]
fn start_key_chords_run_the_mac_control_shortcuts() {
    let mut app = app();
    shortcuts::start_chord(&mut app, Key::S, Held { ctrl: true, ..Held::default() });
    assert!(app.ui.show_search, "Ctrl+Start+S opens Search");
    app.ui.show_search = false;
    shortcuts::start_chord(&mut app, Key::S, Held { shift: true, ..Held::default() });
    assert!(app.ui.show_search, "Start+Shift+S opens Search too");
    shortcuts::start_chord(&mut app, Key::Num2, Held { ctrl: true, ..Held::default() });
    assert_eq!(app.engine.session().edit.zoom.samples_per_px, 256.0, "Ctrl+Start+2 recalls zoom preset 2");
}
