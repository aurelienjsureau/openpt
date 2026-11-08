use crate::SoundApp;

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct SyncPrefs {
    pub clock_out: String,
    pub mtc_out: String,
    pub ltc_out: Option<u16>,
    pub chase: String,
    pub mtc_in: String,
    pub ltc_in: Option<u16>,
    pub online: bool,
}

pub const CHASE_SOURCES: [&str; 3] = ["", "MTC", "LTC"];

#[derive(Default)]
pub struct SyncState {
    #[cfg(not(target_arch = "wasm32"))]
    output: Option<((String, String), openpt_playback::PlayClock, openpt_playback::sync::SyncOutput)>,
    #[cfg(not(target_arch = "wasm32"))]
    mtc_in: Option<(String, openpt_playback::sync::MtcInput)>,
    #[cfg(not(target_arch = "wasm32"))]
    ltc_sent: Option<(Option<usize>, Option<usize>, openpt_playback::PlayClock)>,
    pub(crate) failed: Option<(String, f64)>,
    chasing: bool,
    last_relocate: f64,
    pub(crate) incoming: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
fn settings(app: &SoundApp) -> openpt_playback::sync::SyncSettings {
    let s = app.engine.session();
    openpt_playback::sync::SyncSettings {
        tempo: s.tempo.clone(),
        sample_rate: s.sample_rate,
        frame_rate: s.frame_rate,
        timecode_start: s.timecode_start,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(app: &mut SoundApp, ctx: &egui::Context) {
    use openpt_playback::sync::{MtcInput, SyncOutput};
    let now = ctx.input(|i| i.time);
    let prefs = app.ui.sync.clone();
    let Some(clock) = app.player.as_ref().map(openpt_playback::Player::clock) else { return };
    let retry = app.sync_state.failed.as_ref().is_none_or(|(_, at)| now - at > 5.0);

    let key = (prefs.clock_out.clone(), prefs.mtc_out.clone());
    let wants_output = !key.0.is_empty() || !key.1.is_empty();
    let current = app.sync_state.output.as_ref().is_some_and(|(k, c, _)| *k == key && c.same_as(&clock));
    if !current {
        app.sync_state.output = None;
        if wants_output && retry {
            let port = |n: &str| (!n.is_empty()).then(|| n.to_string());
            let (c, m) = (port(&key.0), port(&key.1));
            match SyncOutput::start(clock.clone(), settings(app), c.as_deref(), m.as_deref()) {
                Ok(out) => {
                    app.sync_state.output = Some((key, clock.clone(), out));
                    app.sync_state.failed = None;
                }
                Err(e) => app.sync_state.failed = Some((e, now)),
            }
        }
    }
    if let Some((_, _, out)) = &app.sync_state.output {
        out.update(settings(app));
    }

    let ltc_in = (prefs.chase == "LTC").then_some(prefs.ltc_in).flatten().map(usize::from);
    let ltc_out = prefs.ltc_out.map(usize::from);
    if ltc_in.is_some() {
        app.ensure_input();
    }
    let ltc_current = app.sync_state.ltc_sent.as_ref().is_some_and(|(o, i, c)| *o == ltc_out && *i == ltc_in && c.same_as(&clock));
    if !ltc_current {
        if let Some(p) = &app.player {
            p.set_ltc(ltc_out, ltc_in);
        }
        app.sync_state.ltc_sent = Some((ltc_out, ltc_in, clock.clone()));
    }

    if prefs.chase == "MTC" && !prefs.mtc_in.is_empty() {
        if app.sync_state.mtc_in.as_ref().is_none_or(|(n, _)| *n != prefs.mtc_in) && retry {
            app.sync_state.mtc_in = None;
            match MtcInput::open(&prefs.mtc_in) {
                Ok(m) => app.sync_state.mtc_in = Some((prefs.mtc_in.clone(), m)),
                Err(e) => app.sync_state.failed = Some((e, now)),
            }
        }
    } else {
        app.sync_state.mtc_in = None;
    }

    let reading = match prefs.chase.as_str() {
        "MTC" => app.sync_state.mtc_in.as_ref().and_then(|(_, m)| m.reading()),
        "LTC" => app.player.as_ref().and_then(openpt_playback::Player::ltc_reading),
        _ => None,
    };
    let now_us = openpt_playback::midi::monotonic_now_us();
    let sr = app.engine.session().sample_rate;
    let tc_start = app.engine.session().timecode_start;
    app.sync_state.incoming = reading.map(|r| openpt_time::Timecode::from_frames(r.frames, r.rate).format(r.rate));
    let fresh = reading.filter(|r| r.running && now_us.saturating_sub(r.at_us) < 250_000);
    if !prefs.online {
        app.sync_state.chasing = false;
        return;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(10));
    let Some(r) = fresh else {
        if app.sync_state.chasing && app.is_playing() {
            app.stop_play();
        }
        app.sync_state.chasing = false;
        return;
    };
    let latency = app.player.as_ref().map_or(0, openpt_playback::Player::output_latency_us);
    let target = |us: u64| (r.samples_at(us, sr) - tc_start as f64).round() as openpt_time::Samples;
    let spf = r.rate.samples_per_frame(sr);
    if !app.is_playing() {
        if app.engine.transport.recording {
            return;
        }
        let at = target(now_us + latency).max(0);
        app.start_play(at, None, None);
        app.sync_state.chasing = true;
        app.sync_state.last_relocate = now;
        return;
    }
    if !app.sync_state.chasing {
        return;
    }
    let (_, heard) = clock.now();
    let error = heard - target(now_us) as f64;
    if error.abs() > spf * 2.0 && now - app.sync_state.last_relocate > 0.5 && !app.engine.transport.recording {
        let at = target(now_us + latency).max(0);
        app.start_play(at, None, None);
        app.sync_state.last_relocate = now;
    }
}

#[cfg(target_arch = "wasm32")]
pub fn run(_: &mut SoundApp, _: &egui::Context) {}

pub fn tab(app: &mut SoundApp, ui: &mut egui::Ui) {
    let inputs: Vec<String> = openpt_playback::midi::list_input_devices().into_iter().map(|d| d.name).collect();
    let outputs = openpt_playback::surface::list_output_devices();
    let out_channels = app.player.as_ref().map_or(2, |p| usize::from(p.device_channels)).max(2);
    let in_channels = app.recorder.as_ref().map_or(8, |r| r.channels).max(1);
    let p = &mut app.ui.sync;
    let port = |ui: &mut egui::Ui, salt: &str, value: &mut String, list: &[String]| {
        let shown = if value.is_empty() { "none".to_string() } else { value.clone() };
        egui::ComboBox::from_id_salt(salt).width(220.0).selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(value, String::new(), "none");
            for n in list {
                ui.selectable_value(value, n.clone(), n);
            }
        });
    };
    let channel = |ui: &mut egui::Ui, salt: &str, value: &mut Option<u16>, count: usize| {
        let shown = value.map_or("none".to_string(), |c| format!("{}", c + 1));
        egui::ComboBox::from_id_salt(salt).width(80.0).selected_text(shown).show_ui(ui, |ui| {
            ui.selectable_value(value, None, "none");
            for c in 0..count.min(64) {
                ui.selectable_value(value, Some(c as u16), format!("{}", c + 1));
            }
        });
    };
    ui.strong(t!("Generate"));
    egui::Grid::new("sync_out").num_columns(2).spacing(egui::vec2(12.0, 8.0)).show(ui, |ui| {
        ui.label(t!("MIDI Beat Clock to"));
        port(ui, "sync_clock_out", &mut p.clock_out, &outputs);
        ui.end_row();
        ui.label(t!("MIDI Time Code to"));
        port(ui, "sync_mtc_out", &mut p.mtc_out, &outputs);
        ui.end_row();
        ui.label(t!("LTC on output channel"));
        channel(ui, "sync_ltc_out", &mut p.ltc_out, out_channels);
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.strong(t!("Chase"));
    egui::Grid::new("sync_in").num_columns(2).spacing(egui::vec2(12.0, 8.0)).show(ui, |ui| {
        ui.label(t!("Positional reference"));
        egui::ComboBox::from_id_salt("sync_chase").width(120.0).selected_text(if p.chase.is_empty() { "none" } else { p.chase.as_str() }).show_ui(
            ui,
            |ui| {
                for s in CHASE_SOURCES {
                    ui.selectable_value(&mut p.chase, s.to_string(), if s.is_empty() { "none" } else { s });
                }
            },
        );
        ui.end_row();
        match p.chase.as_str() {
            "MTC" => {
                ui.label(t!("MIDI Time Code from"));
                port(ui, "sync_mtc_in", &mut p.mtc_in, &inputs);
                ui.end_row();
            }
            "LTC" => {
                ui.label(t!("LTC on input channel"));
                channel(ui, "sync_ltc_in", &mut p.ltc_in, in_channels);
                ui.end_row();
            }
            _ => {}
        }
        ui.label("");
        ui.checkbox(&mut p.online, t!("Online: follow the incoming timecode"));
        ui.end_row();
    });
    let s = &app.sync_state;
    if !app.ui.sync.chase.is_empty() {
        ui.label(egui::RichText::new(s.incoming.as_ref().map_or_else(|| "No timecode received".to_string(), |tc| format!("Incoming: {tc}"))).small());
    }
    if let Some((e, _)) = &s.failed {
        ui.colored_label(egui::Color32::from_rgb(230, 120, 100), e);
    }
    ui.label(
        egui::RichText::new(t!(
            "Timecode follows the session frame rate and start time in Setup › Session. A port used here is not also a MIDI input for recording."
        ))
        .small(),
    );
}
