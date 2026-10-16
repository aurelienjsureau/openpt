use crate::SoundApp;
use crate::theme::{ThemeMode, Tokens, bold};
use egui::{Ui, vec2};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum Page {
    #[default]
    Display,
    Operation,
    Editing,
    Mixing,
    Metering,
    Midi,
}

const PAGES: [(Page, &str); 6] = [
    (Page::Display, "Display"),
    (Page::Operation, "Operation"),
    (Page::Editing, "Editing"),
    (Page::Mixing, "Mixing"),
    (Page::Metering, "Metering"),
    (Page::Midi, "MIDI"),
];

#[derive(Debug, Clone, PartialEq)]
struct Opts {
    insertion_follows_playback: bool,
    link_timeline_edit: bool,
    pre_post_roll: bool,
    pre_roll_s: f64,
    post_roll_s: f64,
    countoff_bars: u32,
    tab_to_transient: bool,
    layered_editing: bool,
    automation_follows_edit: bool,
    markers_follow_edit: bool,
    mirrored_midi: bool,
    delay_compensation: bool,
    pre_fader_metering: bool,
    sends_default_inf: bool,
    solo_mode: String,
}

impl Opts {
    fn read(app: &SoundApp) -> Self {
        let s = app.engine.session();
        let e = &s.edit;
        let rate = f64::from(s.sample_rate.hz()).max(1.0);
        Opts {
            insertion_follows_playback: e.insertion_follows_playback,
            link_timeline_edit: e.link_timeline_edit,
            pre_post_roll: e.pre_post_roll,
            pre_roll_s: e.pre_roll as f64 / rate,
            post_roll_s: e.post_roll as f64 / rate,
            countoff_bars: e.countoff_bars,
            tab_to_transient: e.tab_to_transient,
            layered_editing: e.layered_editing,
            automation_follows_edit: e.automation_follows_edit,
            markers_follow_edit: e.markers_follow_edit,
            mirrored_midi: e.mirrored_midi,
            delay_compensation: e.delay_compensation,
            pre_fader_metering: e.pre_fader_metering,
            sends_default_inf: e.flag("pref.sends_default_inf"),
            solo_mode: e.solo_mode.split('+').next().unwrap_or("sip").to_string(),
        }
    }

    fn commands(&self, before: &Opts) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        let mut flag = |id: &'static str, now: bool, was: bool| {
            if now != was {
                out.push((id, json!({"value": now})));
            }
        };
        flag("options.insertion_follows_playback", self.insertion_follows_playback, before.insertion_follows_playback);
        flag("options.link_timeline_edit", self.link_timeline_edit, before.link_timeline_edit);
        flag("options.pre_post_roll", self.pre_post_roll, before.pre_post_roll);
        flag("options.tab_to_transient", self.tab_to_transient, before.tab_to_transient);
        flag("options.layered_editing", self.layered_editing, before.layered_editing);
        flag("options.automation_follows_edit", self.automation_follows_edit, before.automation_follows_edit);
        flag("options.markers_follow_edit", self.markers_follow_edit, before.markers_follow_edit);
        flag("options.mirrored_midi", self.mirrored_midi, before.mirrored_midi);
        flag("options.delay_compensation", self.delay_compensation, before.delay_compensation);
        flag("options.pre_fader_metering", self.pre_fader_metering, before.pre_fader_metering);
        flag("options.sends_default_inf", self.sends_default_inf, before.sends_default_inf);
        if (self.pre_roll_s - before.pre_roll_s).abs() > 1e-6 {
            out.push(("options.pre_roll", json!({"length": {"seconds": self.pre_roll_s}})));
        }
        if (self.post_roll_s - before.post_roll_s).abs() > 1e-6 {
            out.push(("options.post_roll", json!({"length": {"seconds": self.post_roll_s}})));
        }
        if self.countoff_bars != before.countoff_bars {
            out.push(("setup.click_countoff", json!({"countoff_bars": self.countoff_bars})));
        }
        if self.solo_mode != before.solo_mode {
            out.push(("options.solo_mode", json!({"mode": self.solo_mode})));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq)]
struct App {
    theme: ThemeMode,
    ui_scale: f32,
    track_tint: f32,
    plugin_menu_org: String,
    dashboard_at_startup: bool,
    dashboard_after_close: bool,
    autosave_enabled: bool,
    autosave_minutes: u32,
    undo_levels: usize,
    plugin_native_editor: bool,
    meter_peak_hold: String,
    meter_clip_hold: String,
    meter_decay_db_s: f32,
    note_names: String,
    vst3_folders: Vec<String>,
}

impl App {
    fn read(app: &SoundApp) -> Self {
        let u = &app.ui;
        App {
            theme: u.theme,
            ui_scale: app.extra.ui_scale,
            track_tint: u.track_tint,
            plugin_menu_org: u.plugin_menu_org.clone(),
            dashboard_at_startup: u.dashboard_at_startup,
            dashboard_after_close: u.dashboard_after_close,
            autosave_enabled: u.autosave_enabled,
            autosave_minutes: u.autosave_minutes,
            undo_levels: u.undo_levels,
            plugin_native_editor: u.plugin_native_editor,
            meter_peak_hold: u.meter_peak_hold.clone(),
            meter_clip_hold: u.meter_clip_hold.clone(),
            meter_decay_db_s: u.meter_decay_db_s,
            note_names: u.note_names.clone(),
            vst3_folders: u.vst3_folders.clone(),
        }
    }

    fn write(self, app: &mut SoundApp) {
        let u = &mut app.ui;
        u.theme = self.theme;
        u.track_tint = self.track_tint;
        u.plugin_menu_org = self.plugin_menu_org;
        u.dashboard_at_startup = self.dashboard_at_startup;
        u.dashboard_after_close = self.dashboard_after_close;
        u.autosave_enabled = self.autosave_enabled;
        u.autosave_minutes = self.autosave_minutes.clamp(1, 120);
        u.undo_levels = self.undo_levels.clamp(1, 10_000);
        u.plugin_native_editor = self.plugin_native_editor;
        u.meter_peak_hold = self.meter_peak_hold;
        u.meter_clip_hold = self.meter_clip_hold;
        u.meter_decay_db_s = self.meter_decay_db_s.clamp(1.0, 300.0);
        u.note_names = self.note_names;
        if u.vst3_folders != self.vst3_folders {
            u.vst3_folders = self.vst3_folders;
            openpt_vst3_host::set_search_dirs(u.vst3_folders.iter().map(std::path::PathBuf::from).collect());
        }
        app.extra.ui_scale = self.ui_scale.clamp(0.75, 1.5);
    }
}

#[derive(Debug, Clone)]
struct Draft {
    page: Page,
    search: String,
    focused: bool,
    app: App,
    opts: Opts,
    opts_before: Opts,
}

fn draft_id() -> egui::Id {
    egui::Id::new("prefs_draft")
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_preferences {
        ctx.data_mut(|d| d.remove::<Draft>(draft_id()));
        return;
    }
    let mut d = ctx.data(|d| d.get_temp::<Draft>(draft_id())).unwrap_or_else(|| {
        let opts = Opts::read(app);
        Draft { page: Page::Display, search: String::new(), focused: false, app: App::read(app), opts_before: opts.clone(), opts }
    });
    let (mut ok, mut cancel, mut open_window, mut add_folder) = (false, ctx.input(|i| i.key_pressed(egui::Key::Escape)), None, false);
    egui::Modal::new(egui::Id::new("preferences")).show(ctx, |ui| {
        let t = Tokens::current();
        ui.set_width(880.0);
        ui.label(egui::RichText::new("Preferences").font(bold(14.0)));
        ui.add_space(4.0);
        let search = ui.add(egui::TextEdit::singleline(&mut d.search).hint_text("🔍  Type to search preferences").desired_width(f32::INFINITY));
        if !d.focused {
            search.request_focus();
            d.focused = true;
        }
        ui.add_space(4.0);
        let q = d.search.trim().to_lowercase();
        if q.is_empty() {
            tabs(ui, &mut d.page);
        }
        egui::ScrollArea::vertical().max_height(520.0).min_scrolled_height(520.0).show(ui, |ui| {
            ui.set_min_height(520.0);
            let pages: Vec<Page> = if q.is_empty() { vec![d.page] } else { PAGES.iter().map(|p| p.0).collect() };
            for page in pages {
                page_body(ui, page, &q, &mut d.app, &mut d.opts, &mut open_window, &mut add_folder);
            }
        });
        ui.add_space(6.0);
        ui.allocate_ui_with_layout(vec2(ui.available_width(), 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(egui::Button::new(egui::RichText::new("OK").strong()).fill(t.accent).min_size(vec2(96.0, 22.0))).clicked() {
                ok = true;
            }
            if ui.add(egui::Button::new("Cancel").min_size(vec2(96.0, 22.0))).clicked() {
                cancel = true;
            }
        });
    });
    if add_folder
        && let Some(pick) = &app.services.pick_folder
        && let Some(dir) = pick(d.app.vst3_folders.last().map_or("", String::as_str))
        && !d.app.vst3_folders.contains(&dir)
    {
        d.app.vst3_folders.push(dir);
    }
    if let Some(cmd) = open_window {
        let _ = app.run(cmd, json!({"value": true}));
    }
    if ok {
        let cmds = d.opts.commands(&d.opts_before);
        d.app.clone().write(app);
        for (id, params) in cmds {
            if let Err(e) = app.run(id, params) {
                app.ui.status = e;
            }
        }
    }
    if ok || cancel {
        app.ui.show_preferences = false;
        ctx.data_mut(|m| m.remove::<Draft>(draft_id()));
    } else {
        ctx.data_mut(|m| m.insert_temp(draft_id(), d));
    }
}

fn tabs(ui: &mut Ui, page: &mut Page) {
    let t = Tokens::current();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (p, label) in PAGES {
            let on = *page == p;
            let text = egui::RichText::new(label).font(bold(12.5)).color(if on { t.text } else { t.text_dim });
            let fill = if on { t.group_bg } else { t.group_header };
            if ui
                .add(egui::Button::new(text).fill(fill).corner_radius(egui::CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 }).min_size(vec2(96.0, 24.0)))
                .clicked()
            {
                *page = p;
            }
        }
    });
}

fn hit(q: &str, label: &str) -> bool {
    q.is_empty() || label.to_lowercase().contains(q)
}

fn group(ui: &mut Ui, q: &str, title: &str, labels: &[&str], body: impl FnOnce(&mut Ui)) {
    if !(q.is_empty() || hit(q, title) || labels.iter().any(|l| hit(q, l))) {
        return;
    }
    let t = Tokens::current();
    egui::Frame::new().fill(t.group_bg).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(2).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::hover());
        ui.painter().rect_filled(r, egui::CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, t.group_header);
        ui.painter().text(egui::pos2(r.min.x + 8.0, r.center().y), egui::Align2::LEFT_CENTER, title, bold(12.5), t.text);
        egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 10, top: 6, bottom: 10 }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
    });
    ui.add_space(6.0);
}

fn check(ui: &mut Ui, q: &str, on: &mut bool, label: &str) {
    if hit(q, label) {
        crate::widgets::checkbox(ui, on, label);
    }
}

fn number<N: egui::emath::Numeric>(ui: &mut Ui, v: &mut N, range: std::ops::RangeInclusive<N>, speed: f64, suffix: &str) {
    crate::widgets::value_box(ui, |ui| ui.add(egui::DragValue::new(v).range(range).speed(speed).min_decimals(0).max_decimals(2)));
    if !suffix.is_empty() {
        ui.label(suffix);
    }
}

fn row(ui: &mut Ui, q: &str, label: &str, body: impl FnOnce(&mut Ui)) {
    if hit(q, label) {
        ui.horizontal(|ui| {
            ui.label(format!("{label}:"));
            body(ui);
        });
    }
}

fn combo(ui: &mut Ui, id: &str, value: &mut String, options: &[(&str, &str)]) {
    let shown = options.iter().find(|o| o.0 == value.as_str()).map_or(value.as_str(), |o| o.1).to_string();
    egui::ComboBox::from_id_salt(id).width(200.0).selected_text(shown).show_ui(ui, |ui| {
        for (id, label) in options {
            ui.selectable_value(value, (*id).to_string(), *label);
        }
    });
}

fn radios(ui: &mut Ui, value: &mut String, options: &[(&str, &str)]) {
    for (id, label) in options {
        if ui.radio(value == id, *label).clicked() {
            *value = (*id).to_string();
        }
    }
}

fn page_body(ui: &mut Ui, page: Page, q: &str, a: &mut App, o: &mut Opts, open_window: &mut Option<&'static str>, add_folder: &mut bool) {
    ui.columns(2, |cols| {
        let [left, right] = cols else { return };
        match page {
            Page::Display => {
                group(left, q, "Basics", &["Organize Plugin Menus By", "Interface Scale", "Track Color Tint"], |ui| {
                    row(ui, q, "Organize Plugin Menus By", |ui| {
                        combo(
                            ui,
                            "pref_plugin_org",
                            &mut a.plugin_menu_org,
                            &[
                                ("flat", "Flat List"),
                                ("category", "Category"),
                                ("manufacturer", "Manufacturer"),
                                ("category_manufacturer", "Category and Manufacturer"),
                            ],
                        )
                    });
                    row(ui, q, "Interface Scale", |ui| {
                        ui.add(egui::Slider::new(&mut a.ui_scale, 0.75..=1.5).step_by(0.05).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
                    });
                    row(ui, q, "Track Color Tint", |ui| {
                        ui.add(egui::Slider::new(&mut a.track_tint, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
                    });
                });
                group(right, q, "Warnings & Dialogs", &["Show Dashboard when OpenPT starts", "Show Dashboard after closing a session"], |ui| {
                    check(ui, q, &mut a.dashboard_at_startup, "Show Dashboard when OpenPT starts");
                    check(ui, q, &mut a.dashboard_after_close, "Show Dashboard after closing a session");
                });
            }
            Page::Operation => {
                group(
                    left,
                    q,
                    "Transport",
                    &["Insertion Follows Playback", "Link Timeline and Edit Selection", "Pre/Post-Roll", "Pre-Roll", "Post-Roll", "Countoff"],
                    |ui| {
                        check(ui, q, &mut o.insertion_follows_playback, "Insertion Follows Playback");
                        check(ui, q, &mut o.link_timeline_edit, "Link Timeline and Edit Selection");
                        check(ui, q, &mut o.pre_post_roll, "Pre/Post-Roll");
                        row(ui, q, "Pre-Roll", |ui| number(ui, &mut o.pre_roll_s, 0.0..=60.0, 0.05, "seconds"));
                        row(ui, q, "Post-Roll", |ui| number(ui, &mut o.post_roll_s, 0.0..=60.0, 0.05, "seconds"));
                        row(ui, q, "Countoff", |ui| number(ui, &mut o.countoff_bars, 1..=16, 0.1, "bars"));
                    },
                );
                group(left, q, "Auto Backup", &["Enable Session File Auto Backup", "Backup every"], |ui| {
                    check(ui, q, &mut a.autosave_enabled, "Enable Session File Auto Backup");
                    row(ui, q, "Backup every", |ui| number(ui, &mut a.autosave_minutes, 1..=120, 0.1, "minutes"));
                });
                group(right, q, "Audio", &["Playback Engine", "I/O Setup", "audio interface", "sound card", "buffer"], |ui| {
                    ui.label(egui::RichText::new("Audio interface, sample rate and buffer size.").color(Tokens::current().text_dim));
                    ui.horizontal(|ui| {
                        if ui.button("Playback Engine…").clicked() {
                            *open_window = Some("window.playback_engine");
                        }
                        if ui.button("I/O Setup…").clicked() {
                            *open_window = Some("window.io_setup");
                        }
                    });
                });
            }
            Page::Editing => {
                group(
                    left,
                    q,
                    "Clips",
                    &["Tab to Transient", "Layered Editing", "Automation Follows Edit", "Markers Follow Edit", "Mirrored MIDI Editing"],
                    |ui| {
                        check(ui, q, &mut o.tab_to_transient, "Tab to Transient");
                        check(ui, q, &mut o.layered_editing, "Layered Editing");
                        check(ui, q, &mut o.automation_follows_edit, "Automation Follows Edit");
                        check(ui, q, &mut o.markers_follow_edit, "Markers Follow Edit");
                        check(ui, q, &mut o.mirrored_midi, "Mirrored MIDI Editing");
                    },
                );
                group(right, q, "Undo", &["Levels of Undo"], |ui| {
                    row(ui, q, "Levels of Undo", |ui| number(ui, &mut a.undo_levels, 1..=10_000, 1.0, ""));
                });
            }
            Page::Mixing => {
                group(left, q, "Setup", &["Open Plug-ins in Their Own Editor", "Solo Mode", "Send Levels Default to -INF"], |ui| {
                    check(ui, q, &mut a.plugin_native_editor, "Open Plug-ins in Their Own Editor");
                    check(ui, q, &mut o.sends_default_inf, "Send Levels Default to -INF");
                    if hit(q, "Solo Mode") {
                        ui.label("Solo Mode:");
                        ui.indent("solo", |ui| radios(ui, &mut o.solo_mode, &[("sip", "Solo In Place"), ("afl", "AFL"), ("pfl", "PFL")]));
                    }
                });
                group(right, q, "Delay Compensation", &["Delay Compensation", "latency"], |ui| {
                    check(ui, q, &mut o.delay_compensation, "Delay Compensation");
                });
                group(right, q, "VST3 Plug-in Folders", &["VST3 Plug-in Folders", "plugin folder"], |ui| {
                    ui.label(egui::RichText::new("Searched after the standard VST3 folders.").small().color(Tokens::current().text_dim));
                    let mut remove = None;
                    for (i, dir) in a.vst3_folders.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.small_button("✕").on_hover_text("Remove").clicked() {
                                remove = Some(i);
                            }
                            ui.label(dir);
                        });
                    }
                    if let Some(i) = remove {
                        a.vst3_folders.remove(i);
                    }
                    if ui.button("Add Folder…").clicked() {
                        *add_folder = true;
                    }
                });
            }
            Page::Metering => {
                group(left, q, "Peak / Clip", &["Peak Hold", "Clip Indication", "Decay"], |ui| {
                    ui.columns(2, |c| {
                        if let [l, r] = c {
                            if hit(q, "Peak Hold") {
                                l.label("Peak Hold:");
                                radios(l, &mut a.meter_peak_hold, &[("3s", "3 Seconds"), ("infinite", "Infinite"), ("none", "None")]);
                            }
                            if hit(q, "Clip Indication") {
                                r.label("Clip Indication:");
                                radios(r, &mut a.meter_clip_hold, &[("3s", "3 Seconds"), ("infinite", "Infinite"), ("none", "None")]);
                            }
                        }
                    });
                    row(ui, q, "Decay", |ui| number(ui, &mut a.meter_decay_db_s, 1.0..=300.0, 0.5, "dB/s"));
                });
                group(right, q, "Track Meters", &["Pre-Fader Metering"], |ui| {
                    check(ui, q, &mut o.pre_fader_metering, "Pre-Fader Metering");
                });
            }
            Page::Midi => {
                group(left, q, "Note Display", &["Middle C", "Note Display"], |ui| {
                    radios(
                        ui,
                        &mut a.note_names,
                        &[
                            ("c3", "Standard Pitch (Middle C = \"C3\")"),
                            ("c4", "Alternate Pitch (Middle C = \"C4\")"),
                            ("number", "MIDI Note Number (Middle C = \"60\")"),
                        ],
                    );
                });
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_applies_only_what_changed() {
        let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, crate::Services::default());
        let before = Opts::read(&app);
        let mut now = before.clone();
        assert!(now.commands(&before).is_empty());
        now.tab_to_transient = !before.tab_to_transient;
        now.solo_mode = "afl".into();
        now.pre_roll_s = 1.5;
        for (id, p) in now.commands(&before) {
            app.run(id, p).unwrap();
        }
        let after = Opts::read(&app);
        assert_eq!(after.tab_to_transient, now.tab_to_transient);
        assert_eq!(after.solo_mode, "afl");
        assert!((after.pre_roll_s - 1.5).abs() < 1e-3);
        let mut a = App::read(&app);
        a.undo_levels = 0;
        a.plugin_menu_org = "category".into();
        a.write(&mut app);
        assert_eq!(app.ui.undo_levels, 1, "clamped");
        assert_eq!(app.ui.plugin_menu_org, "category");
    }

    #[test]
    fn search_matches_labels_case_insensitively() {
        assert!(hit("", "Anything"));
        assert!(hit("undo", "Levels of Undo"));
        assert!(!hit("undo", "Pre-Roll"));
    }
}
