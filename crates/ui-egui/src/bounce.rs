use crate::SoundApp;
use crate::theme::{Tokens, bold};
use serde_json::Value;

pub fn start(app: &mut SoundApp, params: &Value) {
    if app.bounce.is_some() {
        app.ui.status = "A bounce is already running".into();
        return;
    }
    app.capture_plugin_states();
    match openpt_engine::cmd::bounce_job(&app.engine, params) {
        Ok(job) => app.bounce = Some(job),
        Err(e) => app.ui.status = e.to_string(),
    }
}

pub fn step(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(job) = &mut app.bounce else { return };
    app.ui.status = match job.step(slice()) {
        Ok(None) => {
            ctx.request_repaint();
            return;
        }
        Ok(Some(done)) => {
            format!("Bounced {} (peak {:.1} dBFS, {:.1} LUFS)", job.path().file_name().unwrap_or_default().to_string_lossy(), done.peak_db, done.lufs)
        }
        Err(e) => e.to_string(),
    };
    app.bounce = None;
}

pub fn refuses_session_switch(app: &mut SoundApp) -> bool {
    if app.bounce.is_some() {
        app.ui.status = "Finish or cancel the bounce first".into();
    }
    app.bounce.is_some()
}

#[cfg(not(target_arch = "wasm32"))]
fn slice() -> impl FnMut() -> bool {
    let until = std::time::Instant::now() + std::time::Duration::from_millis(30);
    move || std::time::Instant::now() < until
}

#[cfg(target_arch = "wasm32")]
fn slice() -> impl FnMut() -> bool {
    let mut blocks = 32;
    move || {
        blocks -= 1;
        blocks > 0
    }
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(job) = &app.bounce else { return };
    let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Modal::new(egui::Id::new("bounce_progress")).show(ctx, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.set_width(320.0);
        let name = job.path().file_name().unwrap_or_default().to_string_lossy();
        ui.label(egui::RichText::new(format!("Bouncing {name}")).font(bold(13.0)));
        ui.add(egui::ProgressBar::new(job.progress()));
        ui.horizontal(|ui| {
            cancel |= ui.button("Cancel").clicked();
            ui.label(egui::RichText::new(format!("{:.0}%", job.progress() * 100.0)).color(Tokens::current().text_dim));
        });
    });
    if cancel {
        app.bounce = None;
        app.ui.status = "Bounce cancelled".into();
    }
}

pub struct RealtimeBounce {
    ring: std::sync::Arc<openpt_playback::CaptureRing>,
    params: Value,
    range: openpt_time::Range,
    path: String,
}

pub fn start_realtime(app: &mut SoundApp, params: &Value) {
    if app.bounce.is_some() || app.rt_bounce.is_some() {
        app.ui.status = "A bounce is already running".into();
        return;
    }
    if app.player.is_none() {
        app.ui.status = "A real-time bounce needs the audio engine".into();
        return;
    }
    let s = app.engine.session();
    let sel = s.edit.selection;
    let range = if sel.is_empty() { openpt_time::Range::new(0, s.content_end().max(1)) } else { sel };
    let frames = usize::try_from(range.len()).unwrap_or(0) + 8192;
    let ring = openpt_playback::CaptureRing::with_capacity(2, frames);
    if app.is_playing() {
        app.stop_play();
    }
    if let Some(p) = &app.player {
        p.set_capture(Some(std::sync::Arc::clone(&ring)));
    }
    let path = params.get("path").and_then(Value::as_str).unwrap_or_default().to_string();
    app.rt_bounce = Some(RealtimeBounce { ring, params: params.clone(), range, path });
    app.start_play(range.start, Some(range.end), None);
}

pub fn step_realtime(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(rt) = &app.rt_bounce else { return };
    if app.is_playing() {
        ctx.request_repaint_after_secs(0.1);
        return;
    }
    if let Some(p) = &app.player {
        p.set_capture(None);
    }
    let complete = rt.ring.frames() as i64 >= rt.range.len();
    let channels = rt.ring.take();
    let name = std::path::Path::new(&rt.path).file_name().unwrap_or_default().to_string_lossy().into_owned();
    app.ui.status = if !complete {
        "Real-time bounce stopped before the end: nothing written".into()
    } else {
        let len = usize::try_from(rt.range.len()).unwrap_or(0);
        let channels = channels.into_iter().map(|mut c| {
            c.truncate(len);
            c
        });
        match openpt_engine::cmd::write_captured_bounce(&rt.params, channels.collect(), app.engine.session().sample_rate.hz()) {
            Ok(v) => format!("Bounced {name} in real time (peak {:.1} dBFS)", v["peak_db"].as_f64().unwrap_or(0.0)),
            Err(e) => e.to_string(),
        }
    };
    app.rt_bounce = None;
}

pub fn show_realtime(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(rt) = &app.rt_bounce else { return };
    let done = (rt.ring.frames() as f64 / rt.range.len().max(1) as f64).min(1.0) as f32;
    let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    let name = std::path::Path::new(&rt.path).file_name().unwrap_or_default().to_string_lossy().into_owned();
    egui::Modal::new(egui::Id::new("rt_bounce_progress")).show(ctx, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.set_width(320.0);
        ui.label(egui::RichText::new(format!("Bouncing {name} in real time")).font(bold(13.0)));
        ui.add(egui::ProgressBar::new(done));
        ui.horizontal(|ui| {
            cancel |= ui.button("Cancel").clicked();
            ui.label(egui::RichText::new(format!("{:.0}%", done * 100.0)).color(Tokens::current().text_dim));
        });
    });
    if cancel {
        if let Some(p) = &app.player {
            p.set_capture(None);
        }
        app.rt_bounce = None;
        app.stop_play();
        app.ui.status = "Bounce cancelled".into();
    }
}
