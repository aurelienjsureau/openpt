use crate::SoundApp;
use crate::widgets::FitScreen;
use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use openpt_engine::cmd::audio_midi::MidiNote;
use openpt_model::TrackId;
use openpt_time::Range;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Receiver;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Poly,
    Mono,
    Drums,
}

impl Mode {
    fn id(self) -> &'static str {
        match self {
            Mode::Poly => "poly",
            Mode::Mono => "mono",
            Mode::Drums => "drums",
        }
    }
}

enum Done {
    Pitched(openpt_transcribe::Analysis),
    Failed(String),
}

pub struct Converter {
    track: TrackId,
    range: Range,
    name: String,
    rate: u32,
    mono: Arc<Vec<f32>>,
    mode: Mode,
    onset: f32,
    frame: f32,
    min_ms: f32,
    low: u8,
    high: u8,
    bends: bool,
    sensitivity: f32,
    drum_note: u8,
    floor: u8,
    analysis: Option<Arc<openpt_transcribe::Analysis>>,
    job: Option<(Receiver<Done>, Arc<AtomicU32>, Arc<AtomicBool>)>,
    error: Option<String>,
    notes: Vec<MidiNote>,
    dropped: Vec<bool>,
    stale: bool,
    hits: Option<Vec<Hit>>,
    kept_edits: Vec<Hit>,
    retrigger: f32,
    view: (f64, f64),
    threshold: f32,
}

#[derive(Clone, Copy)]
struct Hit {
    at: f64,
    peak: f32,
    auto_note: u8,
    forced: Option<bool>,
}

impl Drop for Converter {
    fn drop(&mut self) {
        if let Some((_, _, cancel)) = &self.job {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

pub fn open(app: &mut SoundApp, track: Option<TrackId>) {
    let s = app.engine.session();
    let track = track.or_else(|| s.edit.selected_tracks.first().copied());
    let Some(t) = track.and_then(|t| s.track(t)).filter(|t| t.kind == openpt_model::TrackKind::Audio) else {
        app.ui.status = t!("Audio to MIDI: select audio on an audio track").into();
        return;
    };
    let sel = s.edit.selection;
    let range = if sel.is_empty() {
        let clips: Vec<_> = t.clips().iter().filter(|c| s.edit.selected_clips.contains(&c.id)).collect();
        match (clips.iter().map(|c| c.start).min(), clips.iter().map(|c| c.end()).max()) {
            (Some(a), Some(b)) => Range::new(a, b),
            _ => Range::new(t.clips().iter().map(|c| c.start).min().unwrap_or(0), t.clips().iter().map(|c| c.end()).max().unwrap_or(0)),
        }
    } else {
        openpt_engine::io::bounded_to_track(s, t.id, sel)
    };
    if range.is_empty() {
        app.ui.status = t!("Audio to MIDI: nothing to convert on this track").into();
        return;
    }
    let mono = Arc::new(openpt_engine::cmd::audio_midi::selected_mono(s, t.id, range));
    let d = openpt_transcribe::NoteOptions::default();
    let mut c = Converter {
        track: t.id,
        range,
        name: t.name.clone(),
        rate: s.sample_rate.hz(),
        mono,
        mode: if drum_note_for(&t.name) != 0 || ["drum", "room", "overhead", "oh "].iter().any(|k| t.name.to_lowercase().contains(k)) {
            Mode::Drums
        } else {
            Mode::Poly
        },
        onset: d.onset_threshold,
        frame: d.frame_threshold,
        min_ms: d.min_note_ms,
        low: d.min_pitch,
        high: d.max_pitch,
        bends: false,
        sensitivity: 55.0,
        drum_note: drum_note_for(&t.name),
        floor: 1,
        analysis: None,
        job: None,
        error: None,
        notes: Vec::new(),
        dropped: Vec::new(),
        stale: true,
        hits: None,
        kept_edits: Vec::new(),
        retrigger: 40.0,
        view: (0.0, 0.0),
        threshold: 0.0,
    };
    c.start();
    app.audio_to_midi = Some(c);
}

impl Converter {
    fn start(&mut self) {
        if self.mode == Mode::Drums || self.analysis.is_some() || self.job.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, k, mono, rate) = (Arc::clone(&progress), Arc::clone(&cancel), Arc::clone(&self.mono), self.rate);
        let spawned = std::thread::Builder::new().name("openpt-audio-to-midi".into()).spawn(move || {
            let r = openpt_transcribe::analyse(&mono, rate, &mut |f| {
                p.store((f * 1000.0) as u32, Ordering::Relaxed);
                !k.load(Ordering::Relaxed)
            });
            let _ = tx.send(match r {
                Ok(a) => Done::Pitched(a),
                Err(e) => Done::Failed(e),
            });
        });
        match spawned {
            Ok(_) => self.job = Some((rx, progress, cancel)),
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn poll(&mut self) {
        let Some((rx, _, _)) = &self.job else { return };
        match rx.try_recv() {
            Ok(Done::Pitched(a)) => {
                self.analysis = Some(Arc::new(a));
                self.job = None;
                self.stale = true;
            }
            Ok(Done::Failed(e)) => {
                self.error = Some(e);
                self.job = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.job = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }

    fn params(&self) -> Value {
        json!({
            "mode": self.mode.id(),
            "onset": self.onset,
            "frame": self.frame,
            "min_ms": self.min_ms,
            "low": self.low,
            "high": self.high,
            "bends": self.bends,
            "sensitivity": self.sensitivity / 100.0,
            "retrigger_ms": self.retrigger,
            "threshold_db": 20.0 * self.threshold.max(1e-6).log10(),
            "drum_note": self.drum_note,
        })
    }

    fn refresh(&mut self) {
        if !self.stale {
            return;
        }
        let notes = match self.mode {
            Mode::Drums => {
                self.find_hits();
                let fixed = self.drum_note;
                self.hits
                    .iter()
                    .flatten()
                    .filter(|h| h.forced.unwrap_or(h.peak >= self.threshold))
                    .map(|h| MidiNote {
                        start: h.at,
                        end: h.at + 0.1,
                        pitch: if fixed == 0 { h.auto_note } else { fixed },
                        velocity: velocity_of(h.peak),
                        bends: Vec::new(),
                    })
                    .collect()
            }
            _ => match &self.analysis {
                Some(a) => openpt_engine::cmd::audio_midi::notes_from(a, &openpt_engine::cmd::audio_midi::options(&self.params())),
                None => return,
            },
        };
        self.dropped = vec![false; notes.len()];
        self.notes = notes;
        self.stale = false;
    }

    fn peak_at(&self, at: f64) -> f32 {
        let r = f64::from(self.rate.max(1));
        let a = (at * r).max(0.0) as usize;
        let b = (a + (0.03 * r) as usize).min(self.mono.len());
        self.mono.get(a..b).unwrap_or(&[]).iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }

    fn find_hits(&mut self) {
        if self.hits.is_some() {
            return;
        }
        let found = openpt_engine::cmd::audio_midi::drt_hits(&self.mono, self.rate, self.sensitivity, self.retrigger);
        let mut hits: Vec<Hit> = found.iter().map(|h| Hit { at: h.at, peak: h.strength, auto_note: h.note, forced: None }).collect();
        for k in self.kept_edits.drain(..) {
            match hits.iter_mut().find(|h| (h.at - k.at).abs() < 0.02) {
                Some(h) => h.forced = k.forced,
                None if k.forced == Some(true) => hits.push(k),
                None => {}
            }
        }
        hits.sort_by(|a, b| a.at.total_cmp(&b.at));
        if self.threshold <= 0.0 {
            self.threshold = hits.iter().map(|h| h.peak).fold(0.0f32, f32::max) * 10f32.powf(-18.0 / 20.0);
        }
        self.hits = Some(hits);
    }

    fn redetect(&mut self) {
        self.kept_edits = self.hits.take().unwrap_or_default().into_iter().filter(|h| h.forced.is_some()).collect();
        self.stale = true;
    }

    fn add_hit(&mut self, at: f64) {
        let r = f64::from(self.rate.max(1));
        let from = ((at - 0.05) * r).max(0.0) as usize;
        let to = (((at + 0.3) * r) as usize).min(self.mono.len());
        let near = self.mono.get(from..to).map_or_else(Vec::new, |w| openpt_engine::cmd::audio_midi::drt_hits(w, self.rate, 100.0, 5.0));
        let found = near.iter().map(|h| (from as f64 / r + h.at, h.note)).min_by(|a, b| (a.0 - at).abs().total_cmp(&(b.0 - at).abs()));
        let (at, note) = match found {
            Some((t, n)) if (t - at).abs() < 0.03 => (t, n),
            Some((_, n)) => (at, n),
            None => (at, 38),
        };
        let peak = self.peak_at(at);
        if let Some(h) = &mut self.hits {
            h.push(Hit { at, peak, auto_note: note, forced: Some(true) });
            h.sort_by(|a, b| a.at.total_cmp(&b.at));
        }
        self.stale = true;
    }

    fn kept(&self) -> Vec<&MidiNote> {
        self.notes.iter().zip(&self.dropped).filter(|(n, d)| !**d && n.velocity >= self.floor).map(|(n, _)| n).collect()
    }
}

fn piano_roll(ui: &mut egui::Ui, c: &mut Converter) {
    let t = crate::theme::Tokens::current();
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(640.0), 260.0), Sense::click());
    let p = ui.painter_at(r);
    p.rect_filled(r, 3.0, t.counter_bg);
    let dur = c.range.len() as f64 / f64::from(c.rate.max(1));
    if c.notes.is_empty() {
        let text =
            if c.job.is_some() || (c.analysis.is_none() && c.mode != Mode::Drums) { "Analysing…" } else { "No notes found: lower the thresholds" };
        p.text(r.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), t.text_dim);
        return;
    }
    let lo = c.notes.iter().map(|n| n.pitch).min().unwrap_or(48).saturating_sub(2);
    let hi = c.notes.iter().map(|n| n.pitch).max().unwrap_or(72).saturating_add(2).max(lo + 12);
    let rows = f32::from(hi - lo + 1);
    let row_h = r.height() / rows;
    for k in lo..=hi {
        if matches!(k % 12, 1 | 3 | 6 | 8 | 10) {
            let y = r.max.y - f32::from(k - lo + 1) * row_h;
            p.rect_filled(Rect::from_min_size(pos2(r.min.x, y), vec2(r.width(), row_h)), 0.0, t.border.gamma_multiply(0.25));
        }
    }
    let x = |s: f64| r.min.x + (s / dur.max(1e-6)) as f32 * r.width();
    let rect_of = |n: &MidiNote| {
        let y = r.max.y - f32::from(n.pitch - lo + 1) * row_h;
        Rect::from_min_max(pos2(x(n.start), y + 0.5), pos2(x(n.end).max(x(n.start) + 2.0), y + row_h - 0.5))
    };
    let mut hovered = None;
    for (i, n) in c.notes.iter().enumerate() {
        let nr = rect_of(n);
        let off = c.dropped.get(i).copied().unwrap_or(false) || n.velocity < c.floor;
        let a = 0.35 + 0.65 * f32::from(n.velocity) / 127.0;
        let fill = if off { t.text_dim.gamma_multiply(0.25) } else { t.accent.gamma_multiply(a) };
        p.rect_filled(nr, 1.5, fill);
        p.rect_stroke(nr, 1.5, Stroke::new(1.0, if off { t.text_dim.gamma_multiply(0.4) } else { t.accent }), egui::StrokeKind::Inside);
        if resp.hover_pos().is_some_and(|h| nr.expand(1.0).contains(h)) {
            hovered = Some(i);
        }
    }
    if let Some(i) = hovered
        && let Some(n) = c.notes.get(i)
    {
        resp.clone().on_hover_text(format!("{} · velocity {} · {:.2}–{:.2} s · click: keep/drop", note_name(n.pitch), n.velocity, n.start, n.end));
        if resp.clicked()
            && let Some(d) = c.dropped.get_mut(i)
        {
            *d = !*d;
        }
    }
    p.text(pos2(r.min.x + 4.0, r.min.y + 2.0), egui::Align2::LEFT_TOP, note_name(hi), egui::FontId::proportional(10.0), t.text_dim);
    p.text(pos2(r.min.x + 4.0, r.max.y - 2.0), egui::Align2::LEFT_BOTTOM, note_name(lo), egui::FontId::proportional(10.0), t.text_dim);
}

fn velocity_of(peak: f32) -> u8 {
    let db = 20.0 * peak.max(1e-6).log10();
    ((db + 40.0) / 40.0 * 107.0 + 20.0).clamp(1.0, 127.0) as u8
}

fn drum_view(ui: &mut egui::Ui, c: &mut Converter) {
    let t = crate::theme::Tokens::current();
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width().max(640.0), 260.0), Sense::click_and_drag());
    let p = ui.painter_at(r);
    p.rect_filled(r, 3.0, t.counter_bg);
    let rate = f64::from(c.rate.max(1));
    let dur = (c.mono.len() as f64 / rate).max(1e-6);
    let min_len = 0.25f64.min(dur);
    let (mut v0, mut vlen) = if c.view.1 <= 0.0 { (0.0, dur) } else { c.view };
    if resp.hovered() {
        let (scroll, zoom, mods) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.modifiers));
        let m = resp.hover_pos().map_or(0.5, |m| f64::from(((m.x - r.min.x) / r.width()).clamp(0.0, 1.0)));
        let wheel = if mods.shift { 0.0 } else { scroll.y };
        let factor = if (zoom - 1.0).abs() > 1e-3 { 1.0 / f64::from(zoom) } else { (-f64::from(wheel) / 300.0).exp() };
        if (factor - 1.0).abs() > 1e-6 {
            let at = v0 + m * vlen;
            vlen = (vlen * factor).clamp(min_len, dur);
            v0 = at - m * vlen;
        }
        let pan = if mods.shift { scroll.y + scroll.x } else { scroll.x };
        if pan.abs() > 0.0 {
            v0 -= f64::from(pan) / f64::from(r.width()) * vlen;
        }
    }
    if resp.double_clicked() {
        (v0, vlen) = (0.0, dur);
    }
    v0 = v0.clamp(0.0, (dur - vlen).max(0.0));
    c.view = if vlen >= dur - 1e-9 { (0.0, 0.0) } else { (v0, vlen) };
    let cols = r.width().max(1.0) as usize;
    let a = (v0 * rate) as usize;
    let b = (((v0 + vlen) * rate) as usize).min(c.mono.len()).max(a + 1);
    let per = (b - a).div_ceil(cols).max(1);
    let column = c.mono.get(a..b).unwrap_or(&[]).chunks(per).map(|w| w.iter().fold(0.0f32, |m, v| m.max(v.abs())));
    let top = c.mono.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    let mid = r.center().y;
    let half = r.height() * 0.5 - 12.0;
    let y_of = |v: f32| half * (v / top).clamp(0.0, 1.0);
    let x_step = r.width() / ((b - a) as f32 / per as f32).max(1.0);
    for (i, v) in column.enumerate() {
        let x = r.min.x + (i as f32 + 0.5) * x_step;
        let h = y_of(v).max(0.5);
        p.line_segment([pos2(x, mid - h), pos2(x, mid + h)], Stroke::new(x_step.max(1.0), t.text_dim.gamma_multiply(0.55)));
    }
    let x_of = |s: f64| r.min.x + ((s - v0) / vlen) as f32 * r.width();
    let thr_y = y_of(c.threshold);
    let line = Color32::from_rgb(230, 180, 60);
    for y in [mid - thr_y, mid + thr_y] {
        p.line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(1.0, line));
    }
    let alt = ui.input(|i| i.modifiers.alt);
    let mut hovered = None;
    for (i, h) in c.hits.iter().flatten().enumerate() {
        if h.at < v0 || h.at > v0 + vlen {
            continue;
        }
        let x = x_of(h.at);
        let on = h.forced.unwrap_or(h.peak >= c.threshold);
        let col = match (on, h.forced) {
            (true, Some(true)) => Color32::from_rgb(120, 200, 255),
            (true, _) => t.accent,
            (false, Some(false)) => Color32::from_rgb(200, 90, 80).gamma_multiply(0.7),
            (false, _) => t.text_dim.gamma_multiply(0.35),
        };
        p.line_segment([pos2(x, r.min.y + 10.0), pos2(x, r.max.y - 2.0)], Stroke::new(if on { 1.5 } else { 1.0 }, col));
        p.add(egui::Shape::convex_polygon(
            vec![pos2(x - 4.0, r.min.y + 2.0), pos2(x + 4.0, r.min.y + 2.0), pos2(x, r.min.y + 9.0)],
            col,
            Stroke::NONE,
        ));
        if resp.hover_pos().is_some_and(|m| (m.x - x).abs() <= 3.0) {
            hovered = Some(i);
        }
    }
    if resp.dragged()
        && !alt
        && let Some(m) = resp.interact_pointer_pos()
    {
        c.threshold = ((m.y - mid).abs() / half).clamp(0.0, 1.0) * top;
        c.stale = true;
    }
    if resp.clicked()
        && let Some(m) = resp.interact_pointer_pos()
    {
        match (hovered, alt) {
            (Some(i), true) => {
                if let Some(v) = c.hits.as_mut()
                    && i < v.len()
                {
                    v.remove(i);
                }
                c.stale = true;
            }
            (Some(i), false) => {
                let thr = c.threshold;
                if let Some(h) = c.hits.as_mut().and_then(|v| v.get_mut(i)) {
                    let on = h.forced.unwrap_or(h.peak >= thr);
                    h.forced = if h.forced.is_some() { None } else { Some(!on) };
                }
                c.stale = true;
            }
            (None, true) => {
                let at = v0 + f64::from((m.x - r.min.x) / r.width()) * vlen;
                c.add_hit(at.clamp(0.0, dur));
            }
            (None, false) => {}
        }
    }
    let bar = Rect::from_min_max(pos2(r.min.x, r.max.y - 4.0), r.max);
    p.rect_filled(
        Rect::from_min_max(
            pos2(r.min.x + (v0 / dur) as f32 * r.width(), bar.min.y),
            pos2(r.min.x + ((v0 + vlen) / dur) as f32 * r.width(), bar.max.y),
        ),
        2.0,
        t.text_dim.gamma_multiply(0.6),
    );
    let tip = match hovered.and_then(|i| c.hits.as_ref().and_then(|v| v.get(i))) {
        Some(h) => {
            let pitch = if c.drum_note == 0 { h.auto_note } else { c.drum_note };
            format!(
                "{} · {:.1} dB · {:.3} s · click: keep/drop (click again: back to threshold) · Alt+click: delete",
                note_name(pitch),
                20.0 * h.peak.max(1e-6).log10(),
                h.at
            )
        }
        None => "Drag up/down: threshold · wheel: zoom · Shift+wheel: scroll · double-click: whole file · Alt+click: add a hit".into(),
    };
    let _ = resp.on_hover_text(tip);
    p.text(
        pos2(r.max.x - 4.0, r.min.y + 2.0),
        egui::Align2::RIGHT_TOP,
        format!("threshold {:.1} dB · {:.1}–{:.1} s", 20.0 * c.threshold.max(1e-6).log10(), v0, v0 + vlen),
        egui::FontId::proportional(10.0),
        line,
    );
}

fn note_name(p: u8) -> String {
    const N: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", N[usize::from(p % 12)], i32::from(p / 12) - 2)
}

pub fn window(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(mut c) = app.audio_to_midi.take() else { return };
    c.poll();
    c.start();
    c.refresh();
    if c.job.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let mut open = true;
    let mut create = false;
    egui::Window::new(t!("Audio to MIDI"))
        .id(egui::Id::new("Audio to MIDI"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .fit_screen(ctx)
        .show(ctx, |ui| {
            ui.label(format!("{} · {:.2} s", c.name, c.range.len() as f64 / f64::from(c.rate.max(1))));
            ui.horizontal(|ui| {
                for (m, label, tip) in [
                    (Mode::Poly, "Polyphonic", "Chords and several voices: piano, guitar, pads"),
                    (Mode::Mono, "Melody", "One note at a time: vocals, bass, leads"),
                    (Mode::Drums, "Drums", "Hits sorted into kick, snare, hi-hat, toms and cymbals"),
                ] {
                    if ui.selectable_label(c.mode == m, label).on_hover_text(tip).clicked() && c.mode != m {
                        c.mode = m;
                        c.stale = true;
                    }
                }
            });
            ui.separator();
            let before = (c.onset, c.frame, c.min_ms, c.low, c.high, c.bends, c.sensitivity, c.drum_note);
            if c.mode == Mode::Drums {
                ui.horizontal(|ui| {
                    let mut db = 20.0 * c.threshold.max(1e-6).log10();
                    if ui.add(egui::Slider::new(&mut db, -60.0..=0.0).text("Threshold").suffix(" dB").max_decimals(1)).changed() {
                        c.threshold = 10f32.powf(db / 20.0);
                        c.stale = true;
                    }
                    let shown = DRUM_NOTES.iter().find(|d| d.0 == c.drum_note).map_or("Auto", |d| d.1);
                    egui::ComboBox::from_label("Hits as").selected_text(shown).show_ui(ui, |ui| {
                        for (n, label) in DRUM_NOTES {
                            ui.selectable_value(&mut c.drum_note, n, label);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    let s = ui.add(egui::Slider::new(&mut c.sensitivity, 0.0..=100.0).text("Sensitivity").suffix(" %").max_decimals(0));
                    let r =
                        ui.add(egui::Slider::new(&mut c.retrigger, 5.0..=300.0).logarithmic(true).text("Retrigger").suffix(" ms").max_decimals(0));
                    for resp in [s, r] {
                        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                            c.redetect();
                        }
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut c.onset, 0.05..=0.95).text("Note attack threshold"));
                    ui.add(egui::Slider::new(&mut c.frame, 0.05..=0.95).text("Sustain threshold"));
                });
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut c.min_ms, 20.0..=1000.0).logarithmic(true).text("Shortest note").suffix(" ms"));
                    ui.add(egui::Slider::new(&mut c.low, 21..=108).text("Lowest").custom_formatter(|v, _| note_name(v as u8)));
                    ui.add(egui::Slider::new(&mut c.high, 21..=108).text("Highest").custom_formatter(|v, _| note_name(v as u8)));
                });
                crate::widgets::checkbox(ui, &mut c.bends, "Pitch bend (melody and single notes)");
            }
            if before != (c.onset, c.frame, c.min_ms, c.low, c.high, c.bends, c.sensitivity, c.drum_note) {
                c.stale = true;
            }
            ui.add(egui::Slider::new(&mut c.floor, 1..=127).text("Drop notes quieter than (velocity)"));
            if let Some((_, progress, _)) = &c.job {
                ui.add(egui::ProgressBar::new(progress.load(Ordering::Relaxed) as f32 / 1000.0).text("Analysing the audio…"));
            }
            if let Some(e) = &c.error {
                ui.colored_label(Color32::from_rgb(230, 120, 100), e.as_str());
            }
            if c.mode == Mode::Drums {
                drum_view(ui, &mut c);
            } else {
                piano_roll(ui, &mut c);
            }
            let kept = c.kept().len();
            ui.horizontal(|ui| {
                if c.mode == Mode::Drums {
                    ui.label(format!(
                        "{kept} of {} hits kept · drag: threshold · click a hit: keep/drop · Alt+click: add or delete",
                        c.hits.as_ref().map_or(0, Vec::len)
                    ));
                } else {
                    ui.label(format!("{kept} of {} notes kept · click a note to drop it or bring it back", c.notes.len()));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(kept > 0, egui::Button::new(t!("Create MIDI Track"))).clicked() {
                        create = true;
                    }
                });
            });
        });
    if create {
        let notes: Vec<Value> = c
            .kept()
            .iter()
            .map(|n| json!([n.start, n.end, n.pitch, n.velocity, n.bends.iter().map(|(t, s)| json!([t, s])).collect::<Vec<_>>()]))
            .collect();
        let mut p = c.params();
        if let Some(o) = p.as_object_mut() {
            o.insert("track".into(), json!(c.track.0));
            o.insert("start".into(), json!(c.range.start));
            o.insert("end".into(), json!(c.range.end));
            o.insert("notes".into(), json!(notes));
        }
        match app.run("event.audio_to_midi", p) {
            Ok(v) => app.ui.status = format!("Audio to MIDI: {} notes on a new MIDI track", v["notes"]),
            Err(e) => app.ui.status = e,
        }
        return;
    }
    if open {
        app.audio_to_midi = Some(c);
    }
}

const DRUM_NOTES: [(u8, &str); 9] = [
    (0, "Auto (sort by sound)"),
    (36, "Kick (C1)"),
    (38, "Snare (D1)"),
    (42, "Closed Hi-Hat (F#1)"),
    (46, "Open Hi-Hat (A#1)"),
    (50, "High Tom (D2)"),
    (47, "Mid Tom (B1)"),
    (45, "Low Tom (A1)"),
    (49, "Crash (C#2)"),
];

fn drum_note_for(name: &str) -> u8 {
    let n = name.to_lowercase();
    let has = |k: &[&str]| k.iter().any(|x| n.contains(x));
    if has(&["kick", "kik", "bd"]) {
        36
    } else if has(&["snare", "sn", "caisse"]) {
        38
    } else if has(&["hat", "hh"]) {
        42
    } else if has(&["floor", "ftom", "tom3", "tom 3"]) {
        45
    } else if has(&["tom2", "tom 2"]) {
        47
    } else if has(&["tom"]) {
        50
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn converter(mono: Vec<f32>) -> Converter {
        Converter {
            track: TrackId(1),
            range: Range::new(0, mono.len() as i64),
            name: "Snare".into(),
            rate: 48_000,
            mono: Arc::new(mono),
            mode: Mode::Drums,
            onset: 0.5,
            frame: 0.3,
            min_ms: 60.0,
            low: 21,
            high: 108,
            bends: false,
            sensitivity: 55.0,
            drum_note: 38,
            floor: 1,
            analysis: None,
            job: None,
            error: None,
            notes: Vec::new(),
            dropped: Vec::new(),
            stale: true,
            hits: None,
            kept_edits: Vec::new(),
            retrigger: 40.0,
            view: (0.0, 0.0),
            threshold: 0.0,
        }
    }

    fn hits_at(times: &[(f64, f32)]) -> Vec<f32> {
        let mut x = vec![0.0f32; 96_000];
        let mut seed = 7u32;
        for &(t, g) in times {
            let at = (t * 48_000.0) as usize;
            for i in 0..6_000 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                let n = (seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                if let Some(v) = x.get_mut(at + i) {
                    *v += g * n * (-(i as f32) / 900.0).exp();
                }
            }
        }
        x
    }

    #[test]
    fn the_threshold_line_picks_hits_and_clicks_override_it() {
        let mut c = converter(hits_at(&[(0.2, 0.8), (0.7, 0.05), (1.2, 0.8), (1.6, 0.8)]));
        c.refresh();
        assert_eq!(c.hits.as_ref().map(Vec::len), Some(4));
        c.threshold = 0.3;
        c.stale = true;
        c.refresh();
        assert_eq!(c.kept().len(), 3, "the quiet one sits under the line");
        c.threshold = 0.01;
        c.stale = true;
        c.refresh();
        assert_eq!(c.kept().len(), 4);
        c.threshold = 2.0;
        c.stale = true;
        c.refresh();
        assert!(c.kept().is_empty());
        if let Some(h) = c.hits.as_mut().and_then(|v| v.first_mut()) {
            h.forced = Some(true);
        }
        c.stale = true;
        c.refresh();
        assert_eq!(c.kept().len(), 1);
        assert_eq!(c.kept()[0].pitch, 38);
    }

    #[test]
    fn alt_click_adds_a_hit_on_the_nearest_attack() {
        let mut c = converter(hits_at(&[(0.2, 0.8), (1.0, 0.03)]));
        c.refresh();
        c.hits = Some(c.hits.take().unwrap_or_default().into_iter().filter(|h| h.at < 0.5).collect());
        c.add_hit(1.012);
        c.refresh();
        let added = c.hits.as_ref().and_then(|v| v.iter().find(|h| h.at > 0.5).copied()).map(|h| h.at);
        assert!(added.is_some_and(|t| (t - 1.0).abs() < 0.01), "{added:?}");
        assert_eq!(c.kept().len(), 2);
    }
}
