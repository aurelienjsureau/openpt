use crate::SoundApp;
use crate::theme::ThemeMode;
use egui::vec2;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Page {
    #[default]
    Display,
    Operation,
    Editing,
    Mixing,
}

const PAGES: [(Page, &str); 4] = [(Page::Display, "Display"), (Page::Operation, "Operation"), (Page::Editing, "Editing"), (Page::Mixing, "Mixing")];

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_preferences;
    if !open {
        return;
    }
    let page_id = egui::Id::new("prefs_page");
    let mut page = ctx.data(|d| d.get_temp::<Page>(page_id)).unwrap_or_default();
    let mut runs: Vec<(&'static str, Value)> = Vec::new();
    egui::Window::new("Preferences").open(&mut open).default_size(vec2(560.0, 360.0)).show(ctx, |ui| {
        ui.horizontal(|ui| {
            for (p, label) in PAGES {
                if ui.selectable_label(page == p, label).clicked() {
                    page = p;
                }
            }
        });
        ui.separator();
        let edit = app.engine.session().edit.clone();
        let mut toggle = |ui: &mut egui::Ui, id: &'static str, on: bool, label: &str| {
            let mut v = on;
            if crate::widgets::checkbox(ui, &mut v, label).changed() {
                runs.push((id, json!({"value": v})));
            }
        };
        match page {
            Page::Display => {
                egui::Grid::new("prefs_display").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                    ui.label("Appearance");
                    egui::ComboBox::from_id_salt("prefs_theme").selected_text(app.ui.theme.label()).show_ui(ui, |ui| {
                        for m in ThemeMode::ALL {
                            ui.selectable_value(&mut app.ui.theme, m, m.label());
                        }
                    });
                    ui.end_row();
                    ui.label("Interface scale");
                    ui.add(egui::Slider::new(&mut app.extra.ui_scale, 0.75..=1.5).step_by(0.05).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
                    ui.end_row();
                    ui.label("Track colour in the Edit window");
                    ui.add(egui::Slider::new(&mut app.ui.track_tint, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
                    ui.end_row();
                });
                crate::widgets::checkbox(ui, &mut app.ui.dashboard_at_startup, "Show the Dashboard at startup");
            }
            Page::Operation => {
                toggle(ui, "options.insertion_follows_playback", edit.insertion_follows_playback, "Insertion follows playback");
                toggle(ui, "options.link_timeline_edit", edit.link_timeline_edit, "Link timeline and edit selection");
                toggle(ui, "options.pre_post_roll", edit.pre_post_roll, "Pre/post-roll");
                let rate = f64::from(app.engine.session().sample_rate.hz()).max(1.0);
                egui::Grid::new("prefs_op").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                    for (id, label, samples) in [("options.pre_roll", "Pre-roll (s)", edit.pre_roll), ("options.post_roll", "Post-roll (s)", edit.post_roll)] {
                        ui.label(label);
                        let mut s = samples as f64 / rate;
                        if ui.add(egui::DragValue::new(&mut s).speed(0.05).range(0.0..=60.0).max_decimals(2)).changed() {
                            runs.push((id, json!({"length": {"seconds": s}})));
                        }
                        ui.end_row();
                    }
                    ui.label("Countoff (bars)");
                    let mut bars = edit.countoff_bars;
                    if ui.add(egui::DragValue::new(&mut bars).range(1..=16)).changed() {
                        runs.push(("setup.click_countoff", json!({"countoff_bars": bars})));
                    }
                    ui.end_row();
                });
            }
            Page::Editing => {
                toggle(ui, "options.tab_to_transient", edit.tab_to_transient, "Tab to transient");
                toggle(ui, "options.layered_editing", edit.layered_editing, "Layered editing");
                toggle(ui, "options.automation_follows_edit", edit.automation_follows_edit, "Automation follows edit");
                toggle(ui, "options.markers_follow_edit", edit.markers_follow_edit, "Markers follow edit");
                toggle(ui, "options.mirrored_midi", edit.mirrored_midi, "Mirrored MIDI editing");
            }
            Page::Mixing => {
                crate::widgets::checkbox(ui, &mut app.ui.plugin_native_editor, "Open plug-ins in their own editor (else generic controls)");
                toggle(ui, "options.delay_compensation", edit.delay_compensation, "Delay compensation");
                toggle(ui, "options.pre_fader_metering", edit.pre_fader_metering, "Pre-fader metering");
                ui.horizontal(|ui| {
                    ui.label("Solo mode");
                    let current = edit.solo_mode.split('+').next().unwrap_or("sip").to_string();
                    for (m, label) in [("sip", "Solo In Place"), ("afl", "AFL"), ("pfl", "PFL")] {
                        if ui.radio(current == m, label).clicked() && current != m {
                            runs.push(("options.solo_mode", json!({"mode": m})));
                        }
                    }
                });
            }
        }
    });
    for (id, params) in runs {
        if let Err(e) = app.run(id, params) {
            app.ui.status = e;
        }
    }
    ctx.data_mut(|d| d.insert_temp(page_id, page));
    app.ui.show_preferences = open;
}
