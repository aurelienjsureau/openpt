use crate::widgets::FitScreen;
use crate::SoundApp;
use openpt_model::{TrackId, TrackKind};
use serde_json::json;

pub const CONTROLLER_ROWS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Controller {
    pub kind: String,
    pub input: String,
    pub output: String,
    pub channels: u8,
}

impl Default for Controller {
    fn default() -> Self {
        Controller { kind: "none".into(), input: String::new(), output: String::new(), channels: 8 }
    }
}

impl Controller {
    fn active(&self) -> bool {
        self.kind == "HUI" && !self.input.is_empty() && !self.output.is_empty()
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct Live {
    cfg: Controller,
    port: openpt_playback::surface::HuiPort,
    bank: usize,
    last_ping: f64,
    touched: [bool; 8],
    sent: std::collections::HashMap<(u8, &'static str), String>,
}

#[derive(Default)]
pub struct Surfaces {
    #[cfg(not(target_arch = "wasm32"))]
    live: Vec<Option<Live>>,
    failed: Vec<Option<(Controller, f64, String)>>,
}

fn strips(app: &SoundApp) -> Vec<TrackId> {
    app.engine.session().tracks.iter().filter(|t| t.kind != TrackKind::Video && !t.hidden).map(|t| t.id).collect()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(app: &mut SoundApp, ctx: &egui::Context) {
    use openpt_playback::surface::{self as hui, BankButton, StripButton, SurfaceEvent, TransportButton};
    let now = ctx.input(|i| i.time);
    let cfgs = app.ui.peripherals.clone();
    if app.surfaces.live.len() < CONTROLLER_ROWS {
        app.surfaces.live.resize_with(CONTROLLER_ROWS, || None);
        app.surfaces.failed.resize(CONTROLLER_ROWS, None);
    }
    for row in 0..CONTROLLER_ROWS {
        let cfg = cfgs.get(row).cloned().unwrap_or_default();
        let live = app.surfaces.live.get_mut(row).and_then(Option::as_mut);
        if (live.is_some_and(|l| l.cfg != cfg) || !cfg.active())
            && let Some(slot) = app.surfaces.live.get_mut(row)
        {
            *slot = None;
        }
        let connected = app.surfaces.live.get(row).is_some_and(Option::is_some);
        let retry = app.surfaces.failed.get(row).cloned().flatten().is_none_or(|(c, at, _)| c != cfg || now - at > 5.0);
        if cfg.active() && !connected && retry {
            app.sync_midi_devices();
            match hui::HuiPort::open(&cfg.input, &cfg.output) {
                Ok(port) => {
                    if let Some(slot) = app.surfaces.live.get_mut(row) {
                        *slot = Some(Live { cfg: cfg.clone(), port, bank: 0, last_ping: f64::MIN, touched: [false; 8], sent: Default::default() });
                    }
                    if let Some(f) = app.surfaces.failed.get_mut(row) {
                        *f = None;
                    }
                }
                Err(e) => {
                    if let Some(f) = app.surfaces.failed.get_mut(row) {
                        *f = Some((cfg.clone(), now, e));
                    }
                }
            }
        }
    }
    let tracks = strips(app);
    let mut any = false;
    for row in 0..CONTROLLER_ROWS {
        let Some(mut live) = app.surfaces.live.get_mut(row).and_then(Option::take) else { continue };
        any = true;
        let offset: usize = cfgs.iter().take(row).filter(|c| c.active()).map(|c| usize::from(c.channels.max(8) / 8 * 8)).sum();
        let track_at = |live: &Live, n: u8| tracks.get(live.bank + offset + usize::from(n)).copied();
        for ev in live.port.poll() {
            match ev {
                SurfaceEvent::Fader(n, pos) => {
                    if let Some(t) = track_at(&live, n) {
                        let db = openpt_model::fader_pos_to_db(pos).clamp(-144.0, 12.0);
                        let _ = app.engine.execute_merged("mix.volume", &json!({"track": t.0, "db": db}), &format!("hui_fader:{}", t.0));
                    }
                }
                SurfaceEvent::Touch(n, down) => {
                    if let Some(x) = live.touched.get_mut(usize::from(n)) {
                        *x = down;
                    }
                    if !down {
                        app.engine.end_merge();
                    }
                }
                SurfaceEvent::Strip(n, b) => {
                    let Some(t) = track_at(&live, n) else { continue };
                    let _ = match b {
                        StripButton::Select => app.run("edit.select", json!({"tracks": [t.0]})),
                        StripButton::Mute => app.run("mix.mute", json!({"tracks": [t.0]})),
                        StripButton::Solo => app.run("mix.solo", json!({"tracks": [t.0]})),
                        StripButton::RecReady => app.run("mix.record_arm", json!({"tracks": [t.0]})),
                        _ => Ok(json!({})),
                    };
                }
                SurfaceEvent::Pot(n, steps) => {
                    if let Some(t) = track_at(&live, n)
                        && let Some(p) = app.engine.session().track(t).and_then(|tr| tr.mixer.pan.first().copied())
                    {
                        let pan = (p + f32::from(steps) * 0.02).clamp(-1.0, 1.0);
                        let _ = app.engine.execute_merged("mix.pan", &json!({"tracks": [t.0], "pan": pan, "index": 0}), &format!("hui_pan:{}", t.0));
                    }
                }
                SurfaceEvent::Transport(b) => {
                    let cmd = match b {
                        TransportButton::Play => "transport.play",
                        TransportButton::Stop => "transport.stop",
                        TransportButton::Record => "transport.record",
                        TransportButton::Rewind => "transport.rewind",
                        TransportButton::FastForward => "transport.fast_forward",
                        TransportButton::ReturnToZero => "transport.rtz",
                        TransportButton::End => "transport.go_to_end",
                        TransportButton::Loop => "options.loop_playback",
                    };
                    let _ = app.run(cmd, json!({}));
                }
                SurfaceEvent::Bank(b) => {
                    let width = usize::from(live.cfg.channels.max(8));
                    let last = tracks.len().saturating_sub(1);
                    live.bank = match b {
                        BankButton::ChannelLeft => live.bank.saturating_sub(1),
                        BankButton::BankLeft => live.bank.saturating_sub(width),
                        BankButton::ChannelRight => (live.bank + 1).min(last),
                        BankButton::BankRight => (live.bank + width).min(last),
                    };
                }
                SurfaceEvent::Alive => {}
            }
        }
        if now - live.last_ping >= 1.0 {
            live.port.send(&hui::ping());
            live.last_ping = now;
        }
        let s = app.engine.session();
        let sel = s.edit.selected_tracks.clone();
        let mut out: Vec<Vec<u8>> = Vec::new();
        for n in 0..8u8 {
            let tr = track_at(&live, n).and_then(|t| s.track(t));
            let mut set = |key: &'static str, value: String, msgs: Vec<Vec<u8>>| {
                if live.sent.get(&(n, key)) != Some(&value) {
                    live.sent.insert((n, key), value);
                    out.extend(msgs);
                }
            };
            let name = tr.map_or(String::new(), |t| t.name.clone());
            set("name", name.clone(), vec![hui::strip_name(n, &name)]);
            if !live.touched.get(usize::from(n)).copied().unwrap_or(false) {
                let pos = tr.map_or(0.0, |t| openpt_model::fader_db_to_pos(t.mixer.volume_db));
                set("fader", format!("{pos:.4}"), hui::fader(n, pos).iter().map(|m| m.to_vec()).collect());
            }
            for (key, b, on) in [
                ("mute", StripButton::Mute, tr.is_some_and(|t| t.mixer.mute)),
                ("solo", StripButton::Solo, tr.is_some_and(|t| t.mixer.solo)),
                ("rec", StripButton::RecReady, tr.is_some_and(|t| t.mixer.record_arm)),
                ("select", StripButton::Select, tr.is_some_and(|t| sel.contains(&t.id))),
            ] {
                set(key, on.to_string(), hui::strip_led(n, b, on).iter().map(|m| m.to_vec()).collect());
            }
        }
        let (playing, recording) = (app.engine.transport.playing, app.engine.transport.recording);
        for (key, b, on) in
            [("play", TransportButton::Play, playing), ("stop", TransportButton::Stop, !playing), ("record", TransportButton::Record, recording)]
        {
            if live.sent.get(&(0xFF, key)) != Some(&on.to_string()) {
                live.sent.insert((0xFF, key), on.to_string());
                out.extend(hui::transport_led(b, on).iter().map(|m| m.to_vec()));
            }
        }
        for m in out {
            live.port.send(&m);
        }
        if let Some(slot) = app.surfaces.live.get_mut(row) {
            *slot = Some(live);
        }
    }
    if any {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
}

#[cfg(target_arch = "wasm32")]
pub fn run(_: &mut SoundApp, _: &egui::Context) {}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    if !app.ui.show_peripherals {
        return;
    }
    if app.ui.peripherals.len() < CONTROLLER_ROWS {
        app.ui.peripherals.resize(CONTROLLER_ROWS, Controller::default());
    }
    let tab_key = egui::Id::new("peripherals_tab");
    let mut tab: usize = ctx.data(|d| d.get_temp(tab_key)).unwrap_or(2);
    let inputs: Vec<String> = openpt_playback::midi::list_input_devices().into_iter().map(|d| d.name).collect();
    let outputs = openpt_playback::surface::list_output_devices();
    let mut open = true;
    let mut close = false;
    egui::Window::new("Peripherals").open(&mut open).collapsible(false).resizable(false).fit_screen(ctx).show(ctx, |ui| {
        ui.horizontal(|ui| {
            for (i, name) in ["Synchronization", "Machine Control", "MIDI Controllers", "Ethernet Controllers", "Mic Preamps"].iter().enumerate() {
                if ui.selectable_label(tab == i, *name).clicked() {
                    tab = i;
                }
            }
        });
        ui.separator();
        if tab != 2 {
            ui.label("Nothing to set up here yet.");
        } else {
            egui::Grid::new("midi_controllers").num_columns(5).spacing(egui::vec2(12.0, 8.0)).show(ui, |ui| {
                for h in ["#", "Type", "Receive From", "Send To", "# Ch's"] {
                    ui.strong(h);
                }
                ui.end_row();
                for (row, c) in app.ui.peripherals.iter_mut().enumerate().take(CONTROLLER_ROWS) {
                    ui.label(format!("#{}", row + 1));
                    egui::ComboBox::from_id_salt(("pc_type", row)).width(90.0).selected_text(c.kind.as_str()).show_ui(ui, |ui| {
                        for k in ["none", "HUI", crate::midi_learn::KIND] {
                            ui.selectable_value(&mut c.kind, k.to_string(), k);
                        }
                    });
                    let port = |ui: &mut egui::Ui, salt: &str, value: &mut String, list: &[String]| {
                        let shown = if value.is_empty() { "none".to_string() } else { value.clone() };
                        egui::ComboBox::from_id_salt((salt, row)).width(200.0).selected_text(shown).show_ui(ui, |ui| {
                            ui.selectable_value(value, String::new(), "none");
                            for n in list {
                                ui.selectable_value(value, n.clone(), n);
                            }
                        });
                    };
                    port(ui, "pc_in", &mut c.input, &inputs);
                    port(ui, "pc_out", &mut c.output, &outputs);
                    egui::ComboBox::from_id_salt(("pc_ch", row)).width(50.0).selected_text(c.channels.to_string()).show_ui(ui, |ui| {
                        for n in [8u8, 16, 24, 32] {
                            ui.selectable_value(&mut c.channels, n, n.to_string());
                        }
                    });
                    ui.end_row();
                }
            });
            for (row, f) in app.surfaces.failed.iter().enumerate() {
                if let Some((_, _, e)) = f {
                    ui.colored_label(egui::Color32::from_rgb(230, 120, 100), format!("#{}: {e}", row + 1));
                }
            }
            ui.label(egui::RichText::new("A port used by a controller is not also a MIDI input for recording.").small());
            ui.separator();
            crate::midi_learn::window_rows(app, ui);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("OK").clicked() {
                close = true;
            }
        });
    });
    ctx.data_mut(|d| d.insert_temp(tab_key, tab));
    if !open || close {
        app.ui.show_peripherals = false;
    }
}
