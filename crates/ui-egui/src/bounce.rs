use crate::SoundApp;
use crate::theme::{Tokens, bold};
use openpt_engine::io::{Bounce, Bounced};
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

type Rendered = (Bounce, openpt_engine::Result<Option<Bounced>>);

pub struct Job {
    path: std::path::PathBuf,
    range: openpt_time::Range,
    run: Run,
}

enum Run {
    Here(Box<Bounce>),
    Worker { progress: Arc<AtomicU32>, cancel: Arc<AtomicBool>, handle: Option<std::thread::JoinHandle<Rendered>> },
}

impl Job {
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn range(&self) -> openpt_time::Range {
        self.range
    }

    pub fn progress(&self) -> f32 {
        match &self.run {
            Run::Here(b) => b.progress(),
            Run::Worker { progress, .. } => f32::from_bits(progress.load(Ordering::Relaxed)),
        }
    }
}

fn lend_for_render(app: &SoundApp, session: &Arc<openpt_model::Session>) -> Option<Box<openpt_mix::MixEngine>> {
    let p = app.player.as_ref()?;
    p.update_session(Arc::clone(session));
    let mut eng = p.lend_engine()?;
    if (eng.sample_rate() - session.sample_rate.as_f64() as f32).abs() > 0.5 {
        p.return_engine(eng);
        return None;
    }
    eng.monitor_only = false;
    eng.recording = false;
    eng.metronome = false;
    eng.set_taps(Vec::new());
    Some(eng)
}

fn give_back(app: &SoundApp, mut eng: Box<openpt_mix::MixEngine>) {
    eng.set_taps(Vec::new());
    eng.reset();
    eng.metronome = true;
    if let Some(p) = &app.player {
        p.return_engine(eng);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_bounce(mut job: Bounce, progress: Arc<AtomicU32>, cancel: Arc<AtomicBool>) -> Option<std::thread::JoinHandle<Rendered>> {
    std::thread::Builder::new()
        .name("openpt-bounce".into())
        .spawn(move || {
            loop {
                let until = std::time::Instant::now() + std::time::Duration::from_millis(50);
                let r = job.step(|| !cancel.load(Ordering::Relaxed) && std::time::Instant::now() < until);
                progress.store(job.progress().to_bits(), Ordering::Relaxed);
                if !matches!(r, Ok(None)) || cancel.load(Ordering::Relaxed) {
                    return (job, r);
                }
            }
        })
        .ok()
}

pub fn start(app: &mut SoundApp, params: &Value) {
    if app.bounce.is_some() || app.live_commit.is_some() {
        app.ui.status = t!("A bounce is already running").into();
        return;
    }
    if app.is_playing() {
        app.stop_play();
    }
    app.capture_plugin_states();
    let mut job = match openpt_engine::cmd::bounce_job(&app.engine, params) {
        Ok(job) => job,
        Err(e) => {
            app.ui.status = e.to_string();
            return;
        }
    };
    let (path, range) = (job.path().to_path_buf(), job.range());
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(eng) = lend_for_render(app, &app.engine.session_arc()) {
        job.use_engine(eng);
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = spawn_bounce(job, Arc::clone(&progress), Arc::clone(&cancel));
        app.bounce = Some(Job { path, range, run: Run::Worker { progress, cancel, handle } });
        return;
    }
    app.bounce = Some(Job { path, range, run: Run::Here(Box::new(job)) });
}

fn bounced_status(path: &std::path::Path, r: openpt_engine::Result<Option<Bounced>>) -> String {
    match r {
        Ok(Some(done)) => {
            format!("Bounced {} (peak {:.1} dBFS, {:.1} LUFS)", path.file_name().unwrap_or_default().to_string_lossy(), done.peak_db, done.lufs)
        }
        Ok(None) => "Bounce cancelled".into(),
        Err(e) => e.to_string(),
    }
}

pub fn step(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(job) = &mut app.bounce else { return };
    let path = job.path.clone();
    let handle = match &mut job.run {
        Run::Here(b) => match b.step(slice()) {
            Ok(None) => {
                ctx.request_repaint();
                return;
            }
            r => {
                app.ui.status = bounced_status(&path, r);
                app.bounce = None;
                return;
            }
        },
        Run::Worker { handle, .. } => {
            if handle.as_ref().is_some_and(|h| !h.is_finished()) {
                ctx.request_repaint_after_secs(0.05);
                return;
            }
            handle.take()
        }
    };
    app.ui.status = finish_worker(app, handle).map_or_else(|| "Bounce failed".into(), |r| bounced_status(&path, r));
    app.bounce = None;
}

fn cancel_job(app: &mut SoundApp) {
    let Some(mut job) = app.bounce.take() else { return };
    if let Run::Worker { cancel, handle, .. } = &mut job.run {
        cancel.store(true, Ordering::Relaxed);
        let _ = finish_worker(app, handle.take());
    }
}

fn finish_worker(app: &SoundApp, handle: Option<std::thread::JoinHandle<Rendered>>) -> Option<openpt_engine::Result<Option<Bounced>>> {
    let (mut bounce, r) = handle?.join().ok()?;
    give_back(app, bounce.take_engine());
    Some(r)
}

pub fn refuses_session_switch(app: &mut SoundApp) -> bool {
    if app.bounce.is_some() {
        app.ui.status = t!("Finish or cancel the bounce first").into();
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
            cancel |= ui.button(t!("Cancel")).clicked();
            ui.label(egui::RichText::new(format!("{:.0}%", job.progress() * 100.0)).color(Tokens::current().text_dim));
        });
    });
    if cancel {
        cancel_job(app);
        app.ui.status = t!("Bounce cancelled").into();
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
        app.ui.status = t!("A bounce is already running").into();
        return;
    }
    if app.player.is_none() {
        app.ui.status = t!("A real-time bounce needs the audio engine").into();
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
            cancel |= ui.button(t!("Cancel")).clicked();
            ui.label(egui::RichText::new(format!("{:.0}%", done * 100.0)).color(Tokens::current().text_dim));
        });
    });
    if cancel {
        if let Some(p) = &app.player {
            p.set_capture(None);
        }
        app.rt_bounce = None;
        app.stop_play();
        app.ui.status = t!("Bounce cancelled").into();
    }
}

type Taps = Vec<(openpt_model::TrackId, Vec<Vec<f32>>, usize)>;

struct CommitRender {
    eng: Box<openpt_mix::MixEngine>,
    session: Arc<openpt_model::Session>,
    taps: Taps,
    scratch: Vec<Vec<f32>>,
    pos: i64,
    stop: i64,
}

impl CommitRender {
    fn more(&mut self) -> bool {
        if self.pos >= self.stop {
            return false;
        }
        let n = usize::try_from(self.stop - self.pos).unwrap_or(0).min(self.eng.max_block());
        self.eng.render(&self.session, self.pos, n, &mut self.scratch);
        for (t, chans, lag) in &mut self.taps {
            let Some((b, l)) = self.eng.tapped(&self.session, *t) else { continue };
            *lag = l;
            chans.resize_with(b.len(), Vec::new);
            for (out, src) in chans.iter_mut().zip(b) {
                out.extend_from_slice(src.get(..n).unwrap_or(src));
                out.resize(out.len().max(usize::try_from(self.pos).unwrap_or(0) + n), 0.0);
            }
        }
        self.pos += n as i64;
        true
    }
}

enum CommitRun {
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Here(Box<CommitRender>),
    Worker {
        progress: Arc<AtomicU32>,
        cancel: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<CommitRender>>,
    },
}

pub struct LiveCommit {
    run: CommitRun,
    end: i64,
    stop: i64,
    count: usize,
    cmd: String,
    then: Vec<openpt_model::TrackId>,
    freeze: bool,
}

impl LiveCommit {
    fn progress(&self) -> f32 {
        match &self.run {
            CommitRun::Here(r) => (r.pos as f64 / self.stop.max(1) as f64).min(1.0) as f32,
            CommitRun::Worker { progress, .. } => f32::from_bits(progress.load(Ordering::Relaxed)),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_commit(mut r: CommitRender, progress: Arc<AtomicU32>, cancel: Arc<AtomicBool>) -> Option<std::thread::JoinHandle<CommitRender>> {
    std::thread::Builder::new()
        .name("openpt-commit".into())
        .spawn(move || {
            while !cancel.load(Ordering::Relaxed) && r.more() {
                progress.store(((r.pos as f64 / r.stop.max(1) as f64).min(1.0) as f32).to_bits(), Ordering::Relaxed);
            }
            r
        })
        .ok()
}

pub fn commits_live(s: &openpt_model::Session, t: openpt_model::TrackId) -> bool {
    let multi_out = |x: &openpt_model::Track| x.instrument.as_ref().is_some_and(|i| i.multi_out && openpt_mix::is_third_party(&i.plugin));
    s.track(t).is_some_and(multi_out) || openpt_mix::feeding_tracks(s, t).iter().filter_map(|x| s.track(*x)).any(multi_out)
}

pub fn renders_live(s: &openpt_model::Session, t: openpt_model::TrackId) -> bool {
    let active = |x: openpt_model::TrackId| s.track(x).is_some_and(|x| !x.inactive);
    active(t) && openpt_mix::feeding_tracks(s, t).into_iter().all(active)
}

pub fn start_live_commit(app: &mut SoundApp, tracks: Vec<openpt_model::TrackId>, cmd: &str, then: Vec<openpt_model::TrackId>) {
    let freeze = cmd == "track.freeze";
    if app.bounce.is_some() || app.rt_bounce.is_some() || app.live_commit.is_some() {
        app.ui.status = t!("A bounce is already running").into();
        return;
    }
    let session = app.engine.session_arc();
    let end = tracks.iter().filter_map(|t| openpt_engine::io::commit_range(&session, *t)).map(|r| r.end).max().unwrap_or(0);
    if end <= 0 {
        return;
    }
    if app.is_playing() {
        app.stop_play();
    }
    let Some(mut eng) = lend_for_render(app, &session) else {
        app.ui.status = t!("Commit: the audio engine did not answer").into();
        return;
    };
    eng.reset();
    eng.set_taps(tracks.clone());
    let mut unsoloed = (*session).clone();
    unsoloed.tracks.iter_mut().for_each(|t| t.mixer.solo = false);
    let taps = tracks.iter().map(|t| (*t, Vec::new(), 0)).collect();
    let scratch = vec![vec![0.0f32; eng.max_block()]; openpt_mix::main_channels(&session)];
    let stop = end + session.sample_rate.samples(1.0);
    let render = CommitRender { eng, session: Arc::new(unsoloed), taps, scratch, pos: 0, stop };
    let count = tracks.len();
    #[cfg(not(target_arch = "wasm32"))]
    let run = {
        let progress = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let handle = spawn_commit(render, Arc::clone(&progress), Arc::clone(&cancel));
        CommitRun::Worker { progress, cancel, handle }
    };
    #[cfg(target_arch = "wasm32")]
    let run = CommitRun::Here(Box::new(render));
    app.live_commit = Some(LiveCommit { run, end, stop, count, cmd: cmd.to_string(), then, freeze });
}

fn take_render(lc: &mut LiveCommit) -> Option<CommitRender> {
    match std::mem::replace(&mut lc.run, CommitRun::Worker { progress: Arc::default(), cancel: Arc::default(), handle: None }) {
        CommitRun::Here(r) => Some(*r),
        CommitRun::Worker { cancel, handle, .. } => {
            cancel.store(true, Ordering::Relaxed);
            handle?.join().ok()
        }
    }
}

pub fn step_live_commit(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(lc) = &mut app.live_commit else { return };
    match &mut lc.run {
        CommitRun::Here(r) => {
            let until = std::time::Instant::now() + std::time::Duration::from_millis(30);
            while std::time::Instant::now() < until && r.more() {}
            if r.pos < r.stop {
                ctx.request_repaint();
                return;
            }
        }
        CommitRun::Worker { handle, .. } => {
            if handle.as_ref().is_some_and(|h| !h.is_finished()) {
                ctx.request_repaint_after_secs(0.05);
                return;
            }
        }
    }
    let Some(mut lc) = app.live_commit.take() else { return };
    let Some(CommitRender { eng, taps, .. }) = take_render(&mut lc) else {
        app.ui.status = t!("Commit failed").into();
        return;
    };
    give_back(app, eng);
    let LiveCommit { end, cmd, then, freeze, .. } = lc;
    let len = usize::try_from(end).unwrap_or(0);
    let captured = taps
        .into_iter()
        .map(|(t, chans, lag)| (t, chans.into_iter().map(|c| c.get(lag..).map_or_else(Vec::new, |c| c[..len.min(c.len())].to_vec())).collect()))
        .collect();
    let mut n = if freeze {
        openpt_engine::io::freeze_captured(&mut app.engine, captured)
    } else {
        openpt_engine::io::commit_captured(&mut app.engine, captured).len()
    };
    if !then.is_empty() {
        let ids: Vec<u64> = then.iter().map(|t| t.0).collect();
        match app.engine.execute(&cmd, &serde_json::json!({"tracks": ids, "value": true})) {
            Ok(v) => n += v["tracks"].as_array().map_or(0, Vec::len),
            Err(e) => {
                app.ui.status = e.to_string();
                return;
            }
        }
    }
    app.ui.status = format!("{} {n} track{}", if freeze { "Froze" } else { "Committed" }, if n == 1 { "" } else { "s" });
}

pub fn show_live_commit(app: &mut SoundApp, ctx: &egui::Context) {
    let Some(lc) = &app.live_commit else { return };
    let done = lc.progress();
    let n = lc.count;
    let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    egui::Modal::new(egui::Id::new("live_commit_progress")).show(ctx, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
        ui.set_width(320.0);
        let what = if lc.freeze { "Freezing" } else { "Committing" };
        ui.label(egui::RichText::new(format!("{what} {n} track{}", if n == 1 { "" } else { "s" })).font(bold(13.0)));
        ui.add(egui::ProgressBar::new(done));
        ui.horizontal(|ui| {
            cancel |= ui.button(t!("Cancel")).clicked();
            ui.label(egui::RichText::new(format!("{:.0}%", done * 100.0)).color(Tokens::current().text_dim));
        });
    });
    if cancel && let Some(mut lc) = app.live_commit.take() {
        if let Some(r) = take_render(&mut lc) {
            give_back(app, r.eng);
        }
        app.ui.status = t!("Commit cancelled").into();
    }
}

#[cfg(test)]
mod tests {
    use openpt_model::{ChannelFormat, Insert, Route, Session, TrackKind};

    #[test]
    fn a_multi_output_instrument_commits_live_with_the_auxes_it_feeds() {
        let mut s = Session::default();
        let snare = s.add_bus("Snare", ChannelFormat::Stereo);
        let kick = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Kick"));
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Snare"));
        let other = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Gtr"));
        let mut inst = Insert::new("vst3:0123456789ABCDEF0123456789ABCDEF");
        inst.multi_out = true;
        inst.outputs = vec![snare];
        s.track_mut(kick).unwrap().instrument = Some(inst);
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(snare);
        assert!(super::commits_live(&s, kick), "the instrument's own track (its main output)");
        assert!(super::commits_live(&s, aux));
        assert!(!super::commits_live(&s, other));
        s.track_mut(kick).unwrap().instrument.as_mut().unwrap().multi_out = false;
        assert!(!super::commits_live(&s, kick), "a stereo instrument commits offline as before");
    }
}
