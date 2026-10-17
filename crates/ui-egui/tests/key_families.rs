use egui::os::OperatingSystem;
use egui::{Key, Modifiers};
use openpt_ui_egui::{Services, SoundApp, shortcuts};
use serde_json::json;

const MAC_CMD: Modifiers = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };
const PC_CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
const OSES: [(OperatingSystem, Modifiers); 3] =
    [(OperatingSystem::Mac, MAC_CMD), (OperatingSystem::Windows, PC_CTRL), (OperatingSystem::Nix, PC_CTRL)];

fn press(ctx: &egui::Context, app: &mut SoundApp, key: Key, modifiers: Modifiers) {
    let events = [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }).to_vec();
    ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| shortcuts::handle(app, ui.ctx())).textures_delta.clear();
}

fn rig(os: OperatingSystem) -> (egui::Context, SoundApp) {
    let ctx = egui::Context::default();
    ctx.set_os(os);
    (ctx, SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default()))
}

#[test]
fn cmd_alt_brackets_zoom_waveforms_and_cmd_alt_ctrl_digits_rate_clips() {
    for (os, cmd) in OSES {
        let (ctx, mut app) = rig(os);
        let cmd_alt = Modifiers { alt: true, ..cmd };
        press(&ctx, &mut app, Key::CloseBracket, cmd_alt);
        press(&ctx, &mut app, Key::CloseBracket, cmd_alt);
        press(&ctx, &mut app, Key::OpenBracket, cmd_alt);
        assert_eq!(app.engine.session().edit.zoom.waveform_zoom, 2.0, "{os:?}");
        let kick = app.engine.session().track_by_name("Kick").unwrap().clips()[0].id;
        app.engine.execute("edit.select", &json!({"clips": [kick.0]})).unwrap();
        let rate = |app: &mut SoundApp, key: Key| {
            if os == OperatingSystem::Mac {
                press(&ctx, app, key, Modifiers { ctrl: true, ..cmd_alt });
            } else {
                shortcuts::start_chord(app, key, shortcuts::Held { ctrl: true, alt: true, shift: false });
            }
        };
        rate(&mut app, Key::Num4);
        assert_eq!(app.engine.session().find_clip(kick).unwrap().1.rating, 4, "{os:?}");
        rate(&mut app, Key::Num9);
        assert_eq!(app.engine.session().find_clip(kick).unwrap().1.rating, 4, "{os:?}: 9 is outside 0-5");
        press(&ctx, &mut app, Key::Num2, cmd_alt);
        assert_eq!(app.engine.session().edit.keyboard_focus, "clips", "{os:?}: Cmd+Alt+2 is Clip List focus");
    }
}

#[test]
fn cmd_ctrl_digits_recall_zoom_presets_on_macos() {
    let (ctx, mut app) = rig(OperatingSystem::Mac);
    press(&ctx, &mut app, Key::Num2, Modifiers { ctrl: true, ..MAC_CMD });
    assert_eq!(app.engine.session().edit.zoom.samples_per_px, 256.0);
    press(&ctx, &mut app, Key::Num6, Modifiers { ctrl: true, ..MAC_CMD });
    assert_eq!(app.engine.session().edit.zoom.samples_per_px, 256.0, "6 is outside 1-5");
}

#[test]
fn commands_focus_digits_recall_zoom_presets_and_n_toggles_insertion_follows_playback() {
    for (os, _) in OSES {
        let (ctx, mut app) = rig(os);
        app.engine.execute("options.keyboard_focus", &json!({"focus": "commands"})).unwrap();
        let follows = app.engine.session().edit.insertion_follows_playback;
        press(&ctx, &mut app, Key::Num5, Modifiers::NONE);
        press(&ctx, &mut app, Key::N, Modifiers::NONE);
        let edit = &app.engine.session().edit;
        assert_eq!((edit.zoom.samples_per_px, edit.insertion_follows_playback), (16384.0, !follows), "{os:?}");
    }
}
