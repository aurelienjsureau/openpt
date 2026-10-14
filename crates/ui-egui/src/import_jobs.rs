use crate::SoundApp;
use crate::theme::{Tokens, bold};
use egui::{Align2, vec2};
use openpt_engine::io::PreparedAudio;
use openpt_time::Samples;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

const WORKERS: usize = 4;

type Done = (usize, Result<PreparedAudio, String>);

pub struct ImportJob {
    rx: Receiver<Done>,
    cancel: Arc<AtomicBool>,
    names: Vec<String>,
    results: Vec<Option<Result<PreparedAudio, String>>>,
    at: Samples,
}

impl ImportJob {
    fn done(&self) -> usize {
        self.results.iter().filter(|r| r.is_some()).count()
    }
}

pub fn start(app: &mut SoundApp, paths: Vec<String>, at: Samples) -> bool {
    if paths.is_empty() {
        return true;
    }
    if app.import_job.is_some() {
        app.ui.status = "An import is already running".into();
        return false;
    }
    let rate = app.engine.session().sample_rate.hz();
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let names = paths.iter().map(|p| std::path::Path::new(p).file_name().map_or(p.clone(), |n| n.to_string_lossy().into_owned())).collect();
    let results = paths.iter().map(|_| None).collect();
    spawn(Arc::new(paths), rate, tx, cancel.clone());
    app.import_job = Some(ImportJob { rx, cancel, names, results, at });
    true
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn(paths: Arc<Vec<String>>, rate: u32, tx: Sender<Done>, cancel: Arc<AtomicBool>) {
    let next = Arc::new(AtomicUsize::new(0));
    for _ in 0..WORKERS.min(paths.len()) {
        let (paths, tx, cancel, next) = (paths.clone(), tx.clone(), cancel.clone(), next.clone());
        let worker = move || {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(p) = paths.get(i) else { break };
                let r = if cancel.load(Ordering::Relaxed) {
                    Err("cancelled".to_string())
                } else {
                    openpt_engine::io::prepare_import(p, rate).map_err(|e| e.to_string())
                };
                if tx.send((i, r)).is_err() {
                    break;
                }
            }
        };
        if let Err(e) = std::thread::Builder::new().name("audio-import".into()).spawn(worker) {
            log::warn!("import thread: {e}");
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn spawn(paths: Arc<Vec<String>>, rate: u32, tx: Sender<Done>, _cancel: Arc<AtomicBool>) {
    let _ = &AtomicUsize::new(0);
    for (i, p) in paths.iter().enumerate() {
        let _ = tx.send((i, openpt_engine::io::prepare_import(p, rate).map_err(|e| e.to_string())));
    }
}

fn poll(app: &mut SoundApp) -> Option<(usize, usize, String)> {
    let job = app.import_job.as_mut()?;
    while let Ok((i, r)) = job.rx.try_recv() {
        if let Some(slot) = job.results.get_mut(i) {
            *slot = Some(r);
        }
    }
    let (done, total) = (job.done(), job.results.len());
    let lost = done < total && matches!(job.rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected));
    if done == total || lost {
        finish(app);
        return None;
    }
    Some((done, total, job.names.iter().zip(&job.results).find(|(_, r)| r.is_none()).map(|(n, _)| n.clone()).unwrap_or_default()))
}

pub fn show(app: &mut SoundApp, ctx: &egui::Context) {
    let Some((done, total, current)) = poll(app) else { return };
    let mut cancel = false;
    egui::Window::new(egui::RichText::new("Importing Audio").font(bold(13.0)))
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, vec2(0.0, -80.0))
        .show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(format!("{done} of {total} files"));
            ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).show_percentage());
            ui.label(egui::RichText::new(current).small().color(Tokens::current().text_dim));
            ui.separator();
            cancel = ui.button("Cancel").clicked();
        });
    if cancel && let Some(job) = &app.import_job {
        job.cancel.store(true, Ordering::Relaxed);
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}

fn finish(app: &mut SoundApp) {
    let Some(job) = app.import_job.take() else { return };
    let cancelled = job.cancel.load(Ordering::Relaxed);
    let before = app.engine.session_arc();
    let mut placed = 0;
    let mut errors = Vec::new();
    for (name, r) in job.names.iter().zip(job.results) {
        match r {
            Some(Ok(p)) if !cancelled => match openpt_engine::io::place_prepared(&mut app.engine, p, None, job.at) {
                Ok(_) => placed += 1,
                Err(e) => errors.push(e.to_string()),
            },
            Some(Err(e)) if e != "cancelled" => errors.push(e),
            None => errors.push(format!("{name}: not imported")),
            _ => {}
        }
    }
    app.engine.push_undo("Import Audio", before);
    app.ui.status = match (cancelled, errors.first()) {
        (true, _) => "Import cancelled".to_string(),
        (false, Some(e)) if errors.len() == 1 => format!("Imported {placed} file(s); {e}"),
        (false, Some(e)) => format!("Imported {placed} file(s); {} failed, first: {e}", errors.len()),
        (false, None) => format!("Imported {placed} file(s)"),
    };
    for e in &errors {
        log::warn!("import: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_import_in_the_background_and_land_in_order_as_one_undo_step() {
        let dir = std::env::temp_dir().join(format!("openpt-import-job-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let mut paths = Vec::new();
        for (i, rate) in [44_100u32, 96_000, 48_000].iter().enumerate() {
            let buf = openpt_audio_io::AudioBuffer { sample_rate: *rate, channels: vec![vec![0.25; *rate as usize / 10]] };
            let bytes = openpt_audio_io::encode(&buf, &openpt_audio_io::EncodeOptions::default()).unwrap();
            let p = dir.join(format!("take{i}.wav"));
            std::fs::write(&p, bytes).unwrap();
            paths.push(p.to_string_lossy().into_owned());
        }
        paths.push(dir.join("missing.wav").to_string_lossy().into_owned());
        let mut app = SoundApp::new(openpt_engine::Engine::default(), None, crate::Services::default());
        let tracks = app.engine.session().tracks.len();
        assert!(start(&mut app, paths, 0));
        assert!(!start(&mut app, vec!["x.wav".into()], 0), "one import at a time");
        for _ in 0..500 {
            poll(&mut app);
            if app.import_job.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.import_job.is_none());
        let names: Vec<String> = app.engine.session().tracks.iter().skip(tracks).map(|t| t.name.clone()).collect();
        assert_eq!(names, ["take0", "take1", "take2"]);
        assert!(app.ui.status.contains("Imported 3 file(s)"), "{}", app.ui.status);
        app.run("edit.undo", serde_json::json!({})).unwrap();
        assert_eq!(app.engine.session().tracks.len(), tracks);
        let _ = std::fs::remove_dir_all(dir);
    }
}
