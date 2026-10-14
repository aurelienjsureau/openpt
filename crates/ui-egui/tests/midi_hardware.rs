use openpt_ui_egui::{Services, SoundApp};
use serde_json::json;

#[test]
fn panic_all_notes_off_queues_silence_across_all_channels() {
    let engine = openpt_engine::Engine::default();
    let player = openpt_playback::Player::new(engine.session_arc());
    let ring = player.midi_input_ring();
    let mut app = SoundApp::new(engine, Some(player), Services::default());

    assert!(ring.is_empty());

    let res = app.run("event.all_notes_off", json!({}));
    assert!(res.is_ok());

    let ctx = egui::Context::default();
    app.logic(&ctx);

    let mut count = 0;
    while let Some(ev) = ring.pop() {
        assert!(ev.is_control_change());
        let cc = ev.cc_controller();
        assert!(cc == 120 || cc == 123, "Expected CC 120 or 123, got {cc}");
        count += 1;
    }
    assert_eq!(count, 32);
}

#[test]
fn setup_midi_input_devices_enables_and_syncs_flags() {
    let mut app = SoundApp::new(openpt_engine::Engine::default(), None, Services::default());

    let res = app.run("setup.midi_input_devices", json!({"enable": ["Master Keyboard", "Launchpad"]})).unwrap();
    let enabled = res["enabled"].as_array().unwrap();
    assert!(enabled.iter().any(|v| v == "Master Keyboard"));
    assert!(enabled.iter().any(|v| v == "Launchpad"));

    let res = app.run("setup.midi_input_devices", json!({"disable": ["Launchpad"]})).unwrap();
    let enabled = res["enabled"].as_array().unwrap();
    assert!(enabled.iter().any(|v| v == "Master Keyboard"));
    assert!(!enabled.iter().any(|v| v == "Launchpad"));
}

#[test]
fn midi_input_devices_dialog_syncs_with_manager() {
    let engine = openpt_engine::Engine::default();
    let player = openpt_playback::Player::new(engine.session_arc());
    let mut app = SoundApp::new(engine, Some(player), Services::default());

    assert!(app.dialogs.open_for_command(&app.engine, "setup.midi_input_devices"));
    assert_eq!(app.dialogs.open_name(), Some("midi_input_devices"));
}
