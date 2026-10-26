use crate::widgets::FitScreen;
use crate::SoundApp;
use crate::theme::Tokens;
use egui::vec2;
use serde_json::{Value, json};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OpsState {
    pub beat_sensitivity: f32,
    pub beat_separate: bool,
    pub beat_set_tempo: bool,
    pub beat_result: String,
    pub tempo_curve: String,
    pub tempo_start: f64,
    pub tempo_end: f64,
    pub time_op: String,
    pub time_seconds: f64,
    pub meter_num: u32,
    pub meter_den: u32,
    pub midi_op: String,
    pub quant_grid: String,
    pub quant_strength: f64,
    pub quant_swing: f64,
    pub transpose: i64,
    pub velocity_add: i64,
    pub rtp_velocity: i64,
    pub rtp_transpose: i64,
    pub rtp_duration: f64,
    pub rtp_delay: i64,
    pub identify_bars: u32,
    pub bd: BeatDetectiveState,
}

impl Default for OpsState {
    fn default() -> Self {
        OpsState {
            beat_sensitivity: 0.5,
            beat_separate: true,
            beat_set_tempo: false,
            beat_result: String::new(),
            tempo_curve: "linear".into(),
            tempo_start: 100.0,
            tempo_end: 120.0,
            time_op: "insert".into(),
            time_seconds: 4.0,
            meter_num: 4,
            meter_den: 4,
            midi_op: "quantize".into(),
            quant_grid: "1/16".into(),
            quant_strength: 100.0,
            quant_swing: 0.0,
            transpose: 12,
            velocity_add: 10,
            rtp_velocity: 0,
            rtp_transpose: 0,
            rtp_duration: 100.0,
            rtp_delay: 0,
            identify_bars: 1,
            bd: BeatDetectiveState::default(),
        }
    }
}

fn result_line(ui: &mut egui::Ui, r: &Result<Value, String>) {
    match r {
        Ok(v) => {
            let s = v.to_string();
            ui.label(egui::RichText::new(if s.len() > 160 { format!("{}…", &s[..160]) } else { s }).small().color(Tokens::current().counter_text));
        }
        Err(e) => {
            ui.label(egui::RichText::new(e).small().color(Tokens::current().rec));
        }
    }
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    beat_detective(app, ctx);
    tempo_ops(app, ctx);
    time_ops(app, ctx);
    midi_ops(app, ctx);
    rtp(app, ctx);
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct BeatDetectiveState {
    pub operation: String,
    pub start: String,
    pub end: String,
    pub contains: String,
    pub resolution: String,
    pub emphasis: String,
    pub sensitivity: f32,
    pub trigger_pad_ms: f64,
    pub strength: f64,
    pub exclude_within: f64,
    pub swing: f64,
    pub crossfade: bool,
    pub crossfade_ms: f64,
    pub groove_name: String,
    #[serde(skip)]
    pub triggers: Vec<(i64, String)>,
}

impl Default for BeatDetectiveState {
    fn default() -> Self {
        BeatDetectiveState {
            operation: "markers".into(),
            start: "1|1".into(),
            end: "2|1".into(),
            contains: "1/16".into(),
            resolution: "beats".into(),
            emphasis: "enhanced".into(),
            sensitivity: 0.5,
            trigger_pad_ms: 5.0,
            strength: 100.0,
            exclude_within: 0.0,
            swing: 0.0,
            crossfade: true,
            crossfade_ms: 5.0,
            groove_name: "Groove 1".into(),
            triggers: Vec::new(),
        }
    }
}

fn beat_detective(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_beat_detective;
    if !open {
        app.ops.bd.triggers.clear();
        return;
    }
    let t = Tokens::current();
    egui::Window::new("Beat Detective").open(&mut open).resizable(false).fit_screen(ctx).show(ctx, |ui| {
        let mut run: Option<&str> = None;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(190.0);
                section(ui, "Operation");
                let bd = &mut app.ops.bd;
                for (id, label) in [
                    ("markers", "Bar|Beat Marker Generation"),
                    ("groove", "Groove Template Extraction"),
                    ("separate", "Clip Separation"),
                    ("conform", "Clip Conform"),
                    ("smooth", "Edit Smoothing"),
                ] {
                    ui.radio_value(&mut bd.operation, id.to_string(), label);
                }
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_width(220.0);
                section(ui, "Selection");
                let bd = &mut app.ops.bd;
                egui::Grid::new("bd_sel").num_columns(2).spacing(vec2(8.0, 4.0)).show(ui, |ui| {
                    ui.label("Start Bar|Beat");
                    ui.add(egui::TextEdit::singleline(&mut bd.start).desired_width(70.0));
                    ui.end_row();
                    ui.label("End Bar|Beat");
                    ui.add(egui::TextEdit::singleline(&mut bd.end).desired_width(70.0));
                    ui.end_row();
                    ui.label("Contains");
                    egui::ComboBox::from_id_salt("bd_contains").width(70.0).selected_text(bd.contains.clone()).show_ui(ui, |ui| {
                        for v in ["1/4", "1/8", "1/8t", "1/16", "1/16t", "1/32"] {
                            ui.selectable_value(&mut bd.contains, v.to_string(), v);
                        }
                    });
                    ui.end_row();
                });
                if ui.button("Capture Selection").on_hover_text("Bars and beats of the Edit selection").clicked() {
                    let (s, e) = {
                        let s = app.engine.session();
                        let round = |at: i64| {
                            let tk = s.tempo.samples_to_ticks(at, s.sample_rate);
                            let tpb = s.tempo.meter_at_tick(tk).ticks_per_beat().max(1);
                            let bb = s.tempo.bar_beat_at_tick(((tk + tpb / 2) / tpb) * tpb);
                            format!("{}|{}", bb.bar, bb.beat)
                        };
                        (round(s.edit.selection.start), round(s.edit.selection.end))
                    };
                    app.ops.bd.start = s;
                    app.ops.bd.end = e;
                }
            });
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.set_width(230.0);
                let bd = &mut app.ops.bd;
                match bd.operation.as_str() {
                    "conform" => {
                        section(ui, "Conform");
                        ui.add(egui::Slider::new(&mut bd.strength, 0.0..=100.0).text("Strength %"));
                        ui.add(egui::Slider::new(&mut bd.exclude_within, 0.0..=100.0).text("Exclude Within %"));
                        ui.add(egui::Slider::new(&mut bd.swing, 0.0..=150.0).text("Swing %"));
                        ui.label(
                            egui::RichText::new("Moves the separated clips onto the grid; detection uses its last settings.")
                                .small()
                                .color(t.text_dim),
                        );
                    }
                    "smooth" => {
                        section(ui, "Smoothing");
                        ui.radio_value(&mut bd.crossfade, false, "Fill Gaps");
                        ui.radio_value(&mut bd.crossfade, true, "Fill And Crossfade");
                        ui.add_enabled(bd.crossfade, egui::Slider::new(&mut bd.crossfade_ms, 0.0..=50.0).text("Crossfade ms"));
                    }
                    _ => {
                        section(ui, "Detection");
                        egui::ComboBox::from_id_salt("bd_emph")
                            .width(150.0)
                            .selected_text(match bd.emphasis.as_str() {
                                "low" => "Low Emphasis",
                                "high" => "High Emphasis",
                                _ => "Enhanced Resolution",
                            })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut bd.emphasis, "enhanced".to_string(), "Enhanced Resolution");
                                ui.selectable_value(&mut bd.emphasis, "high".to_string(), "High Emphasis");
                                ui.selectable_value(&mut bd.emphasis, "low".to_string(), "Low Emphasis");
                            });
                        ui.horizontal(|ui| {
                            for (id, label) in [("bars", "Bars"), ("beats", "Beats"), ("sub_beats", "Sub-Beats")] {
                                ui.radio_value(&mut bd.resolution, id.to_string(), label);
                            }
                        });
                        ui.add(egui::Slider::new(&mut bd.sensitivity, 0.0..=1.0).text("Sensitivity").show_value(false));
                        ui.add(egui::Slider::new(&mut bd.trigger_pad_ms, 0.0..=50.0).text("Trigger Pad ms"));
                        if bd.operation == "groove" {
                            ui.horizontal(|ui| {
                                ui.label("Template name");
                                ui.add(egui::TextEdit::singleline(&mut bd.groove_name).desired_width(110.0));
                            });
                        }
                        if ui.button(egui::RichText::new("Analyze").strong()).clicked() {
                            run = Some("analyze");
                        }
                    }
                }
            });
        });
        ui.separator();
        let bd = &mut app.ops.bd;
        let what = match bd.operation.as_str() {
            "conform" => "Moves the clips that start on a trigger onto the session's grid.",
            "smooth" => "Fills the gaps conforming left between clips, with crossfades if asked.",
            "groove" => "Saves the timing of the triggers as a groove for Event Operations › Quantize.",
            "separate" => "Cuts the clips on the selected tracks at every trigger.",
            _ => "Adds tempo events so that the session's bars and beats follow the triggers.",
        };
        ui.label(egui::RichText::new(what).color(t.text_dim));
        ui.horizontal(|ui| {
            let label = match bd.operation.as_str() {
                "separate" => "Separate",
                "groove" => "Extract",
                "conform" => "Conform",
                "smooth" => "Smooth",
                _ => "Generate",
            };
            if ui.add(egui::Button::new(egui::RichText::new(label).strong()).min_size(vec2(90.0, 0.0))).clicked() {
                run = Some("apply");
            }
            ui.label(egui::RichText::new(&app.ops.beat_result).small().color(t.counter_text));
        });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Identify Beat: the selection is");
            ui.add(egui::DragValue::new(&mut app.ops.identify_bars).range(1..=64));
            ui.label("bar(s)");
            if ui.button("Set Tempo").clicked() {
                let r = app.run("event.identify_beat", json!({"bars": app.ops.identify_bars}));
                app.ops.beat_result = r.map_or_else(|e| e, |v| format!("Tempo {}", v["bpm"]));
            }
        });
        if let Some(what) = run {
            let bd = app.ops.bd.clone();
            let op = if what == "analyze" { "analyze".to_string() } else { bd.operation.clone() };
            let r = app.run(
                "event.beat_detective",
                json!({
                    "operation": op, "start_bar_beat": bd.start, "end_bar_beat": bd.end, "contains": bd.contains,
                    "resolution": bd.resolution, "emphasis": bd.emphasis, "sensitivity": bd.sensitivity,
                    "trigger_pad_ms": bd.trigger_pad_ms, "strength": bd.strength, "exclude_within": bd.exclude_within,
                    "swing": bd.swing, "crossfade": bd.crossfade, "crossfade_ms": bd.crossfade_ms, "name": bd.groove_name,
                }),
            );
            match r {
                Ok(v) => {
                    app.ops.bd.triggers = v["triggers"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|x| Some((x["at"].as_i64()?, x["bar_beat"].as_str()?.to_string()))).collect())
                        .unwrap_or_default();
                    app.ops.beat_result = match op.as_str() {
                        "analyze" => format!("{} triggers", v["count"]),
                        "markers" => format!("{} tempo events", v["done"]),
                        "groove" => format!("Groove \"{}\" extracted ({} steps)", app.ops.bd.groove_name, v["done"]),
                        "separate" => format!("{} separations", v["done"]),
                        "conform" => format!("{} clips moved", v["done"]),
                        _ => format!("{} crossfades", v["done"]),
                    };
                }
                Err(e) => app.ops.beat_result = e,
            }
        }
    });
    app.ui.show_beat_detective = open;
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title.to_uppercase()).font(crate::theme::bold(10.5)).color(Tokens::current().text_dim));
    ui.add_space(2.0);
}

pub(crate) fn tempo_ops_apply(curve: &str, start_bpm: f64, end_bpm: f64) -> (&'static str, Value) {
    let start = if start_bpm.is_finite() { start_bpm.max(1.0) } else { 1.0 };
    let end = if end_bpm.is_finite() { end_bpm.max(1.0) } else { 1.0 };
    match curve {
        "constant" => ("event.tempo_constant", json!({"bpm": start_bpm})),
        "parabolic" => ("event.tempo_parabolic", json!({"start_bpm": start_bpm, "end_bpm": end_bpm})),
        "s-curve" => ("event.tempo_s_curve", json!({"start_bpm": start_bpm, "end_bpm": end_bpm})),
        "scale" => ("event.tempo_scale", json!({"factor": end / start})),
        "stretch" => ("event.tempo_stretch", json!({"factor": start / end})),
        _ => ("event.tempo_linear", json!({"start_bpm": start_bpm, "end_bpm": end_bpm})),
    }
}

fn tempo_ops(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_tempo_ops;
    if !open {
        return;
    }
    egui::Window::new("Tempo Operations").open(&mut open).default_size(vec2(360.0, 200.0)).fit_screen(ctx).show(ctx, |ui| {
        egui::ComboBox::from_label("Curve").selected_text(app.ops.tempo_curve.clone()).show_ui(ui, |ui| {
            for c in ["constant", "linear", "parabolic", "s-curve", "scale", "stretch"] {
                ui.selectable_value(&mut app.ops.tempo_curve, c.to_string(), c);
            }
        });
        ui.add(egui::DragValue::new(&mut app.ops.tempo_start).range(5.0..=999.0).prefix("start bpm "));
        ui.add(egui::DragValue::new(&mut app.ops.tempo_end).range(5.0..=999.0).prefix("end bpm "));
        ui.label(
            egui::RichText::new(
                "Applies across the edit selection (constant: at the insertion point; scale: factor = end / start; stretch: factor = start / end).",
            )
            .small(),
        );
        if ui.button(egui::RichText::new("Apply").strong()).clicked() {
            let (id, p) = tempo_ops_apply(&app.ops.tempo_curve, app.ops.tempo_start, app.ops.tempo_end);
            let r = app.run(id, p);
            result_line(ui, &r);
        }
    });
    app.ui.show_tempo_ops = open;
}

fn time_ops(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_time_ops;
    if !open {
        return;
    }
    egui::Window::new("Time Operations").open(&mut open).default_size(vec2(360.0, 200.0)).fit_screen(ctx).show(ctx, |ui| {
        egui::ComboBox::from_label("Operation").selected_text(app.ops.time_op.clone()).show_ui(ui, |ui| {
            for c in ["insert", "cut", "change meter", "move song start"] {
                ui.selectable_value(&mut app.ops.time_op, c.to_string(), c);
            }
        });
        match app.ops.time_op.as_str() {
            "change meter" => {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut app.ops.meter_num).range(1..=64));
                    ui.label("/");
                    egui::ComboBox::from_id_salt("den").selected_text(app.ops.meter_den.to_string()).show_ui(ui, |ui| {
                        for d in [2u32, 4, 8, 16] {
                            ui.selectable_value(&mut app.ops.meter_den, d, d.to_string());
                        }
                    });
                });
            }
            "insert" => {
                ui.add(egui::DragValue::new(&mut app.ops.time_seconds).range(0.0..=3600.0).suffix(" s"));
            }
            _ => {
                ui.label(egui::RichText::new("Uses the edit selection.").small());
            }
        }
        if ui.button(egui::RichText::new("Apply").strong()).clicked() {
            let at = app.engine.session().edit.selection.start;
            let r = match app.ops.time_op.as_str() {
                "insert" => app.run("event.insert_time", json!({"start": at, "length": {"seconds": app.ops.time_seconds}})),
                "cut" => app.run("event.cut_time", json!({})),
                "change meter" => app.run("event.meter", json!({"numerator": app.ops.meter_num, "denominator": app.ops.meter_den, "at": at})),
                _ => app.run("event.move_song_start", json!({"to": at})),
            };
            result_line(ui, &r);
        }
    });
    app.ui.show_time_ops = open;
}

fn midi_ops(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_midi_ops;
    if !open {
        return;
    }
    egui::Window::new("Event Operations").open(&mut open).default_size(vec2(360.0, 220.0)).fit_screen(ctx).show(ctx, |ui| {
        egui::ComboBox::from_label("Operation").selected_text(app.ops.midi_op.clone()).show_ui(ui, |ui| {
            for c in ["quantize", "transpose", "change velocity", "flatten performance", "restore performance"] {
                ui.selectable_value(&mut app.ops.midi_op, c.to_string(), c);
            }
        });
        match app.ops.midi_op.as_str() {
            "quantize" => {
                let grooves: Vec<String> = app
                    .engine
                    .session()
                    .edit
                    .flags
                    .iter()
                    .filter_map(|f| f.strip_prefix("groove.").and_then(|r| r.split_once('=')).map(|(n, _)| format!("Groove: {n}")))
                    .collect();
                egui::ComboBox::from_label("Grid").selected_text(app.ops.quant_grid.clone()).show_ui(ui, |ui| {
                    for g in ["1/4", "1/8", "1/8t", "1/16", "1/16t", "1/32"] {
                        ui.selectable_value(&mut app.ops.quant_grid, g.to_string(), g);
                    }
                    if !grooves.is_empty() {
                        ui.separator();
                    }
                    for g in grooves {
                        ui.selectable_value(&mut app.ops.quant_grid, g.clone(), g);
                    }
                });
                ui.add(egui::Slider::new(&mut app.ops.quant_strength, 0.0..=100.0).text("Strength %"));
                ui.add(egui::Slider::new(&mut app.ops.quant_swing, 0.0..=100.0).text("Swing %"));
            }
            "transpose" => {
                ui.add(egui::Slider::new(&mut app.ops.transpose, -48..=48).text("Semitones"));
            }
            "change velocity" => {
                ui.add(egui::Slider::new(&mut app.ops.velocity_add, -127..=127).text("Add"));
            }
            _ => {}
        }
        ui.label(egui::RichText::new("Applies to the selected MIDI clips (or those under the selection).").small());
        if ui.button(egui::RichText::new("Apply").strong()).clicked() {
            let r = match app.ops.midi_op.as_str() {
                "quantize" => {
                    let mut p = json!({"grid": app.ops.quant_grid, "strength": app.ops.quant_strength, "swing": app.ops.quant_swing});
                    if let Some(name) = app.ops.quant_grid.strip_prefix("Groove: ") {
                        p = json!({"groove": name, "strength": app.ops.quant_strength});
                    }
                    app.run("event.quantize", p)
                }
                "transpose" => app.run("event.transpose", json!({"semitones": app.ops.transpose})),
                "change velocity" => app.run("event.change_velocity", json!({"add": app.ops.velocity_add})),
                "flatten performance" => app.run("event.flatten_performance", json!({})),
                _ => app.run("event.restore_performance", json!({})),
            };
            result_line(ui, &r);
        }
    });
    app.ui.show_midi_ops = open;
}

fn rtp(app: &mut SoundApp, ctx: &egui::Context) {
    let mut open = app.ui.show_rtp;
    if !open {
        return;
    }
    egui::Window::new("MIDI Real-Time Properties").open(&mut open).default_size(vec2(340.0, 200.0)).fit_screen(ctx).show(ctx, |ui| {
        ui.label("Non-destructive, applied at playback to the selected MIDI tracks.");
        let mut changed = false;
        changed |= ui.add(egui::Slider::new(&mut app.ops.rtp_velocity, -127..=127).text("Velocity +")).changed();
        changed |= ui.add(egui::Slider::new(&mut app.ops.rtp_transpose, -48..=48).text("Transpose")).changed();
        changed |= ui.add(egui::Slider::new(&mut app.ops.rtp_duration, 1.0..=400.0).text("Duration %")).changed();
        changed |= ui.add(egui::Slider::new(&mut app.ops.rtp_delay, -960..=960).text("Delay (ticks)")).changed();
        if changed {
            let p = json!({"velocity": app.ops.rtp_velocity, "transpose": app.ops.rtp_transpose, "duration": app.ops.rtp_duration, "delay": app.ops.rtp_delay});
            let _ = app.engine.execute_merged("event.midi_rtp", &p, "rtp");
        }
        ui.horizontal(|ui| {
            if ui.button("Write to Notes").clicked() {
                let _ = app.run("track.write_midi_rtp", json!({}));
            }
            if ui.button("Clear").clicked() {
                let _ = app.run("event.midi_rtp", json!({"clear": true}));
                app.ops.rtp_velocity = 0;
                app.ops.rtp_transpose = 0;
                app.ops.rtp_duration = 100.0;
                app.ops.rtp_delay = 0;
            }
        });
    });
    app.ui.show_rtp = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretch_uses_start_over_end_and_scale_uses_end_over_start() {
        let (id, p) = tempo_ops_apply("stretch", 120.0, 60.0);
        assert_eq!(id, "event.tempo_stretch");
        assert!((p["factor"].as_f64().unwrap() - 2.0).abs() < 1e-9, "{p}");
        let (_, p) = tempo_ops_apply("stretch", 120.0, 120.0);
        assert!((p["factor"].as_f64().unwrap() - 1.0).abs() < 1e-9, "{p}");
        let (id, p) = tempo_ops_apply("scale", 120.0, 60.0);
        assert_eq!(id, "event.tempo_scale");
        assert!((p["factor"].as_f64().unwrap() - 0.5).abs() < 1e-9, "{p}");
        let (_, p) = tempo_ops_apply("stretch", f64::NAN, f64::INFINITY);
        assert!((p["factor"].as_f64().unwrap() - 1.0).abs() < 1e-9, "{p}");
        let (id, p) = tempo_ops_apply("linear", 100.0, 140.0);
        assert_eq!(id, "event.tempo_linear");
        assert_eq!(p["end_bpm"], 140.0);
    }
}
