#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod bounce;
pub mod control;
pub mod dashboard;
pub mod dialogs;
pub mod docking;
pub mod edit_window;
pub mod extra_windows;
pub mod fonts;
pub mod icons;
pub mod import_jobs;
pub mod menus;
pub mod midi_editor;
pub mod mix_window;
pub mod ops_windows;
pub mod palette;
pub mod panels;
pub mod preferences;
mod recent;
pub mod recovery;
pub mod sampler_window;
pub mod score_editor;
pub mod shortcuts;
pub mod spot;
pub mod theme;
pub mod toolbar;
pub mod video_track;
pub mod video_window;
pub mod warp_view;
pub mod waveform;
pub mod widgets;
pub mod windows;

use openpt_engine::{Engine, TransportRequest};
use openpt_model::TrackId;
use openpt_playback::record::Trouble;
use openpt_playback::{OutputTrouble, Player};
use openpt_time::{Range, Samples};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;

pub use control::ControlRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MainWindow {
    Edit,
    Mix,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct UiState {
    pub recent_sessions: Vec<String>,
    pub docking: docking::Workspace,
    #[serde(default = "docking::mix_default")]
    pub mix_docking: docking::Workspace,
    pub window: MainWindow,
    pub show_tracks_list: bool,
    pub show_clip_list: bool,
    pub show_transport: bool,
    pub show_memory_locations: bool,
    pub show_big_counter: bool,
    pub show_undo_history: bool,
    pub show_session_info: bool,
    pub show_audio_health: bool,
    pub show_about: bool,
    pub show_midi_editor: bool,
    pub show_universe: bool,
    pub narrow_mix: bool,
    pub mix_views: Vec<String>,
    pub edit_views: Vec<String>,
    pub track_tint: f32,
    pub plugin_windows: Vec<(TrackId, usize)>,
    #[serde(skip)]
    pub send_window: Option<(TrackId, usize)>,
    pub audiosuite: Option<String>,
    pub status: String,
    pub show_automation: bool,
    pub show_color_palette: bool,
    pub show_disk_usage: bool,
    pub show_system_usage: bool,
    pub show_task_manager: bool,
    pub show_metadata: bool,
    pub show_event_list: bool,
    pub show_midi_keyboard: bool,
    pub show_workspace: bool,
    pub show_configurations: bool,
    pub show_playback_engine: bool,
    pub audio: openpt_playback::AudioConfig,
    pub mix_separate: bool,
    pub show_io_setup: bool,
    pub show_shortcuts: bool,
    pub show_clip_effects: bool,
    pub show_beat_detective: bool,
    pub show_tempo_ops: bool,
    pub show_time_ops: bool,
    pub show_midi_ops: bool,
    pub show_rtp: bool,
    pub show_score: bool,
    pub show_search: bool,
    pub show_video: bool,
    pub show_video_universe: bool,
    pub video_burn_in: bool,
    pub workspace_dir: String,
    pub configurations: Vec<(String, Value)>,
    pub theme: theme::ThemeMode,
    pub window_size: [f32; 2],
    pub window_maximized: bool,
    pub ui_scale: f32,
    #[serde(skip)]
    pub show_dashboard: bool,
    pub dashboard_at_startup: bool,
    pub dashboard_tab_recent: bool,
    pub session_dir: String,
    pub show_preferences: bool,
    pub plugin_native_editor: bool,
    pub plugin_menu_org: String,
    pub dashboard_after_close: bool,
    pub autosave_enabled: bool,
    pub autosave_minutes: u32,
    pub undo_levels: usize,
    pub meter_peak_hold: String,
    pub meter_clip_hold: String,
    pub meter_decay_db_s: f32,
    pub note_names: String,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            recent_sessions: Vec::new(),
            docking: docking::Workspace::default(),
            mix_docking: docking::mix_default(),
            window: MainWindow::Edit,
            show_tracks_list: true,
            show_clip_list: true,
            show_transport: false,
            show_memory_locations: false,
            show_big_counter: false,
            show_undo_history: false,
            show_session_info: false,
            show_audio_health: false,
            show_about: false,
            show_midi_editor: false,
            show_universe: false,
            narrow_mix: false,
            mix_views: vec!["inserts_ae".into(), "sends_ae".into(), "io".into(), "color".into()],
            edit_views: vec!["inserts_ae".into(), "sends_ae".into(), "io".into(), "color".into()],
            track_tint: 0.0,
            plugin_windows: Vec::new(),
            send_window: None,
            audiosuite: None,
            status: String::new(),
            show_automation: false,
            show_color_palette: false,
            show_disk_usage: false,
            show_system_usage: false,
            show_task_manager: false,
            show_metadata: false,
            show_event_list: false,
            show_midi_keyboard: false,
            show_workspace: false,
            show_configurations: false,
            show_playback_engine: false,
            audio: openpt_playback::AudioConfig::default(),
            mix_separate: false,
            show_io_setup: false,
            show_shortcuts: false,
            show_clip_effects: false,
            show_beat_detective: false,
            show_tempo_ops: false,
            show_time_ops: false,
            show_midi_ops: false,
            show_rtp: false,
            show_score: false,
            show_search: false,
            show_video: false,
            show_video_universe: false,
            video_burn_in: false,
            workspace_dir: String::new(),
            configurations: Vec::new(),
            theme: theme::ThemeMode::Dark,
            window_size: [1600.0, 1000.0],
            window_maximized: false,
            ui_scale: 1.0,
            show_dashboard: false,
            dashboard_at_startup: true,
            dashboard_tab_recent: false,
            session_dir: String::new(),
            show_preferences: false,
            plugin_native_editor: true,
            plugin_menu_org: "manufacturer".into(),
            dashboard_after_close: false,
            autosave_enabled: true,
            autosave_minutes: 2,
            undo_levels: 200,
            meter_peak_hold: "3s".into(),
            meter_clip_hold: "infinite".into(),
            meter_decay_db_s: 26.0,
            note_names: "c3".into(),
        }
    }
}

#[derive(Default)]
pub struct Services {
    pub pick_open: Option<Box<dyn Fn(&str, &[&str]) -> Option<String>>>,
    pub pick_save: Option<Box<dyn Fn(&str, &str) -> Option<String>>>,
    pub pick_folder: Option<Box<dyn Fn(&str) -> Option<String>>>,
}

pub fn preset_folder_name(plugin: &str) -> String {
    plugin.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).collect()
}

pub fn fit_window_to_screen(ctx: &egui::Context) {
    let (inner, monitor) = ctx.input(|i| (i.viewport().inner_rect, i.viewport().monitor_size));
    if ctx.cumulative_pass_nr() == 0 && inner.zip(monitor).is_some_and(|(r, m)| (r.size() - m).max_elem() > 0.0) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
    }
}

#[derive(Debug, Clone)]
pub struct DragSource(pub Vec<u64>);

#[derive(Debug, Clone, Copy, Default)]
pub struct MeterDisplay {
    pub level: [f32; 2],
    pub hold: [f32; 2],
    pub hold_age: f32,
    pub clip: bool,
    pub clip_age: f32,
    pub gr: f32,
    pub plugin_failures: u32,
}

#[derive(Debug, Clone)]
pub enum Gesture {
    Select {
        track: TrackId,
        anchor: Samples,
        tracks: Vec<TrackId>,
    },
    MoveClips {
        clips: Vec<openpt_model::ClipId>,
        grab_at: Samples,
        delta: Samples,
        from_track: TrackId,
        to_track: TrackId,
    },
    TrimStart {
        clip: openpt_model::ClipId,
        track: TrackId,
        to: Samples,
    },
    TrimEnd {
        clip: openpt_model::ClipId,
        track: TrackId,
        to: Samples,
    },
    Fade {
        clip: openpt_model::ClipId,
        track: TrackId,
        fade_in: bool,
        to: Samples,
    },
    FadeCurve {
        clip: openpt_model::ClipId,
        fade_in: bool,
        curvature: f32,
        anchor: (f32, f32),
        height: f32,
    },
    Scrub {
        last: Samples,
    },
    Fader {
        track: TrackId,
        start_db: f32,
    },
    ClipGain {
        clip: openpt_model::ClipId,
        track: TrackId,
        pos: f32,
        anchor: (f32, f32),
        fine: bool,
        fader_bottom: egui::Pos2,
    },
    AutoPoint {
        track: TrackId,
        param: String,
        at: Samples,
    },
    GainPoint {
        clip: openpt_model::ClipId,
        track: TrackId,
        at: Samples,
    },
    LineTrim {
        track: TrackId,
        param: Option<String>,
        clip: Option<openpt_model::ClipId>,
        start: Samples,
        end: Samples,
        anchor_y: f32,
        per_px: f32,
        applied: f32,
    },
    Pencil {
        track: TrackId,
        points: Vec<(Samples, f32)>,
    },
    MidiClip {
        track: TrackId,
        from: Samples,
        to: Samples,
    },
    ZoomBox {
        start: f32,
    },
}

pub struct SoundApp {
    pub engine: Engine,
    recent_seen: Option<String>,
    recent_changed: bool,
    pub(crate) pending_editor: Option<(TrackId, usize, u32)>,
    pub(crate) import_job: Option<import_jobs::ImportJob>,
    pub command_log: std::collections::VecDeque<String>,
    pub(crate) discard_take: bool,
    pub(crate) focus_clip_find: bool,
    recent_checked: Option<(f64, Vec<String>, Vec<bool>)>,
    pub player: Option<Player>,
    pub recorder: Option<openpt_playback::record::Recorder>,
    pub midi_recorder: Option<openpt_playback::midi::MidiRecorder>,
    pub midi_manager: Option<openpt_playback::midi::MidiManager>,
    record_start: Samples,
    punch: Option<Range>,
    recorder_failed: bool,
    lost_shown: u64,
    take_plan: Option<openpt_engine::io::Recording>,
    pub ui: UiState,
    pub audio_health_report: Option<Result<Value, String>>,
    pub services: Services,
    pub native_menu_bar: bool,
    pub autosave_dir: Option<std::path::PathBuf>,
    autosaver: Option<openpt_engine::autosave::Autosaver>,
    autosaved_rev: u64,
    pub leftovers: Vec<openpt_engine::autosave::Leftover>,
    panic_rev: u64,
    pub preset_dir: Option<std::path::PathBuf>,
    last_autosave: Option<std::time::Instant>,
    pub dialogs: dialogs::Dialogs,
    pub meters: HashMap<TrackId, MeterDisplay>,
    pub main_meter: MeterDisplay,
    meter_snapshot: openpt_playback::MeterSnapshot,
    pub gesture: Option<Gesture>,
    pub edit_layout: edit_window::EditLayout,
    pub midi: midi_editor::MidiEditorState,
    pub ops: ops_windows::OpsState,
    pub extra: extra_windows::ExtraState,
    pub palette: palette::PaletteState,
    pub(crate) focus_mix: bool,
    pub synthetic: Vec<egui::Event>,
    pub sample_drop_targets: Vec<(egui::Rect, TrackId)>,
    sample_reads: Vec<sampler_window::SampleRead>,
    synthetic_grace: u32,
    control_rx: Option<Receiver<ControlRequest>>,
    pending_shots: Vec<control::PendingShot>,
    last_rev: u64,
    fonts_ready: bool,
    fonts_installed: bool,
    theme_applied: Option<bool>,
    pub(crate) docking_context: Option<egui::Context>,
    sim: Option<(Samples, Option<Samples>, Option<Range>)>,
    record_hint: Option<(f64, &'static str)>,
    pub quit_requested: bool,
    pub arrange_request: bool,
    touched: std::collections::HashSet<TrackId>,
    last_write_at: Samples,
    pub frame_ms: f32,
    last_frame: Option<f64>,
    last_meter_poll: f64,
    pub video: video_track::VideoPool,
    pub bounce: Option<openpt_engine::io::Bounce>,
    #[cfg(test)]
    _palette_lock: theme::TestPaletteGuard,
}

fn automation_step_due(pos: Samples, last: Samples, step: Samples) -> bool {
    last == i64::MIN || pos.saturating_sub(last) >= step || last.saturating_sub(pos) >= step
}

impl SoundApp {
    pub fn new(engine: Engine, player: Option<Player>, services: Services) -> Self {
        SoundApp {
            recent_seen: engine.path.clone(),
            recent_changed: false,
            pending_editor: None,
            import_job: None,
            command_log: std::collections::VecDeque::new(),
            discard_take: false,
            focus_clip_find: false,
            recent_checked: None,
            midi_manager: player.as_ref().map(|p| {
                let mut mm = openpt_playback::midi::MidiManager::new(p.midi_input_ring());
                let flags = &engine.session().edit.flags;
                let enabled: Vec<String> = flags.iter().filter_map(|f| f.strip_prefix("midi.input.").map(str::to_string)).collect();
                mm.sync_enabled(&enabled);
                mm
            }),
            engine,
            player,
            recorder: None,
            midi_recorder: None,
            record_start: 0,
            punch: None,
            recorder_failed: false,
            lost_shown: 0,
            take_plan: None,
            ui: UiState::default(),
            audio_health_report: None,
            services,
            native_menu_bar: false,
            autosave_dir: None,
            autosaver: None,
            autosaved_rev: 0,
            leftovers: Vec::new(),
            panic_rev: 0,
            preset_dir: None,
            last_autosave: None,
            dialogs: dialogs::Dialogs::default(),
            meters: HashMap::new(),
            main_meter: MeterDisplay::default(),
            meter_snapshot: openpt_playback::MeterSnapshot::default(),
            gesture: None,
            edit_layout: edit_window::EditLayout::default(),
            midi: midi_editor::MidiEditorState::default(),
            ops: ops_windows::OpsState::default(),
            extra: extra_windows::ExtraState::default(),
            palette: palette::PaletteState::default(),
            focus_mix: false,
            synthetic: Vec::new(),
            sample_drop_targets: Vec::new(),
            sample_reads: Vec::new(),
            synthetic_grace: 0,
            control_rx: None,
            pending_shots: Vec::new(),
            last_rev: 0,
            fonts_ready: false,
            fonts_installed: false,
            theme_applied: None,
            docking_context: None,
            sim: None,
            record_hint: None,
            quit_requested: false,
            arrange_request: false,
            touched: std::collections::HashSet::new(),
            last_write_at: i64::MIN,
            frame_ms: 0.0,
            last_frame: None,
            last_meter_poll: f64::NEG_INFINITY,
            video: video_track::VideoPool::default(),
            bounce: None,
            #[cfg(test)]
            _palette_lock: theme::lock_test_palette(),
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        if self.command_log.len() >= 64 {
            self.command_log.pop_front();
        }
        self.command_log.push_back(id.to_string());
        if matches!(id, "session.new" | "session.open" | "session.close" | "session.revert") && bounce::refuses_session_switch(self) {
            return Err(self.ui.status.clone());
        }
        if let Some(r) = menus::run_ui_command(self, id, &params) {
            return r;
        }
        if id.starts_with("session.save") || id.starts_with("file.bounce") {
            self.capture_plugin_states();
        }
        let replacing = matches!(id, "session.new" | "session.open" | "session.close" | "session.revert");
        match self.engine.execute(id, &params) {
            Ok(v) => {
                if replacing || (id.starts_with("session.save") && !self.engine.is_dirty()) {
                    self.release_autosave(false);
                }
                if id == "app.quit" {
                    self.quit_requested = true;
                }
                if id == "session.close" && self.ui.dashboard_after_close {
                    self.ui.show_dashboard = true;
                }
                if id == "options.scrolling" {
                    self.edit_layout.follow_hold = false;
                }
                if id == "track.clear_clip_indicators" {
                    self.meters.values_mut().chain([&mut self.main_meter]).for_each(|m| m.clip = false);
                }
                Ok(v)
            }
            Err(e) => {
                self.ui.status = e.to_string();
                log::warn!("{id}: {e}");
                Err(e.to_string())
            }
        }
    }

    pub fn is_playing(&self) -> bool {
        self.engine.transport.playing
    }

    pub(crate) fn record_hint(&self) -> Option<&'static str> {
        let (at, why) = self.record_hint?;
        let fresh = self.last_frame.unwrap_or_default() - at < 4.0;
        (fresh && !self.engine.session().tracks.iter().any(|t| t.mixer.record_arm)).then_some(why)
    }

    pub(crate) fn modal_open(&self) -> bool {
        self.dialogs.open.is_some() || self.ui.show_dashboard || self.ui.show_preferences || self.import_job.is_some()
    }

    pub fn position(&self) -> Samples {
        if self.engine.transport.playing { self.engine.transport.position } else { self.engine.session().edit.selection.start }
    }

    fn start_play(&mut self, from: Samples, end: Option<Samples>, looped: Option<Range>) {
        if let Some(p) = &self.player {
            p.update_session(self.engine.session_arc());
            p.play(from, end, looped);
        } else {
            self.sim = Some((from, end, looped));
        }
        self.engine.transport.playing = true;
        self.engine.transport.position = from;
        self.last_write_at = i64::MIN;
        self.edit_layout.follow_hold = false;
        self.edit_layout.last_scroll = self.engine.session().edit.zoom.scroll;
        self.edit_layout.last_follow_to = None;
    }

    fn stop_play(&mut self) {
        if let Some(p) = &self.player {
            p.set_speed(1.0);
            p.stop();
        }
        if self.engine.transport.recording {
            self.finish_recording();
        }
        self.sim = None;
        self.engine.transport.playing = false;
        self.engine.transport.recording = false;
        let stop_at = self.engine.transport.position;
        let (follow_insertion, after_playback) = {
            let s = self.engine.session();
            (s.edit.insertion_follows_playback, s.edit.scrolling == "after_playback")
        };
        if follow_insertion {
            let pos = self.engine.transport.position;
            self.engine.session_mut().edit.selection = Range::point(pos);
        }
        if after_playback {
            let at = stop_at.max(0);
            let (scroll, spp) = {
                let z = &self.engine.session().edit.zoom;
                (z.scroll, z.samples_per_px.max(0.01))
            };
            let width = f64::from(self.edit_layout.timeline[2] - self.edit_layout.timeline[0]).max(0.0);
            let end = scroll.saturating_add((width * spp) as Samples);
            if at < scroll || at >= end {
                let _ = self.engine.execute("view.scroll", &json!({ "to": at }));
            }
        }
        self.edit_layout.follow_hold = false;
    }

    fn start_recording(&mut self) {
        let armed = self.engine.session().tracks.iter().any(|t| t.mixer.record_arm);
        if !armed {
            let why = if self.engine.session().tracks.is_empty() {
                "Nothing to record on yet: add a track with Track › New…, then click its R button."
            } else {
                "No track is record-enabled: click the R button on the track to record."
            };
            self.ui.status = why.into();
            self.record_hint = Some((self.last_frame.unwrap_or_default(), why));
            return;
        }
        self.ensure_input();
        let takes = match self.arm_recorder() {
            Ok(takes) => takes,
            Err(err) => {
                self.ui.status = format!("Cannot record: {err}");
                return;
            }
        };
        self.lost_shown = 0;
        if let Some(p) = &self.player {
            p.set_recording(true);
        }
        let already_playing = self.is_playing();
        self.engine.transport.recording = true;
        if already_playing {
            self.record_start = self.position();
            self.punch = None;
            self.plan_recording(takes, None);
            return;
        }
        let e = self.engine.session().edit.clone();
        let sel = e.selection;
        if e.loop_record && !sel.is_empty() {
            self.record_start = sel.start;
            self.punch = None;
            self.start_play(sel.start, None, Some(sel));
            self.plan_recording(takes, Some(sel.len()));
        } else {
            let (pre, post) = if e.pre_post_roll { (e.pre_roll, e.post_roll) } else { (0, 0) };
            let from = (sel.start - pre).max(0);
            self.record_start = from;
            self.punch = (!sel.is_empty()).then_some(sel);
            let end = (!sel.is_empty()).then_some(sel.end + post);
            self.start_play(from, end, None);
            self.plan_recording(takes, None);
        }
    }

    fn plan_recording(&mut self, takes: Vec<openpt_engine::io::Take>, pass: Option<Samples>) {
        let rate = self.recorder.as_ref().map_or(0, |r| r.sample_rate);
        self.take_plan = Some(openpt_engine::io::Recording { start: self.record_start, pass, punch: self.punch, rate, takes });
        if let Some(root) = self.autosave_dir.clone() {
            self.queue_autosave(root);
        }
    }

    fn arm_recorder(&mut self) -> Result<Vec<openpt_engine::io::Take>, String> {
        let name = openpt_engine::io::sanitize_name(&self.engine.session().name);
        let unsaved_dir = self.autosave_dir.as_ref().map_or_else(std::env::temp_dir, |d| d.join(name));
        let midi_tracks = openpt_engine::io::plan_midi_takes(&self.engine);
        let plan = if let Some(r) = &mut self.recorder { openpt_engine::io::plan_takes(&self.engine, &unsaved_dir, r.channels) } else { Vec::new() };
        if plan.is_empty() && midi_tracks.is_empty() {
            let has_armed_audio = self.engine.session().tracks.iter().any(|t| t.mixer.record_arm && t.kind == openpt_model::TrackKind::Audio);
            if has_armed_audio && self.recorder.is_none() {
                return Err("no audio input device".into());
            }
            return Err("no track is record-enabled".into());
        }
        let takes: Vec<openpt_engine::io::Take> =
            plan.iter().map(|(track, inputs, path)| openpt_engine::io::Take { track: *track, path: path.clone(), channels: inputs.len() }).collect();
        if !plan.is_empty()
            && let Some(r) = &mut self.recorder
        {
            r.arm(plan)?;
        }
        if !midi_tracks.is_empty()
            && let Some(player) = &self.player
        {
            self.midi_recorder = Some(openpt_playback::midi::MidiRecorder::new(player.midi_record_ring(), midi_tracks, self.record_start));
        }
        Ok(takes)
    }

    pub fn capture_plugin_states(&mut self) {
        let Some(p) = &self.player else { return };
        for (t, slot, data) in p.capture_states() {
            let slot = if slot == openpt_mix::INSTRUMENT_SLOT { json!("instrument") } else { json!(slot) };
            let state = openpt_model::b64::encode(&data);
            if let Err(e) = self.engine.execute("mix.insert_state", &json!({"track": t.0, "slot": slot, "state": state})) {
                log::warn!("storing plugin state: {e}");
            }
        }
    }

    fn apply_editor_edits(&mut self) {
        let Some(p) = &self.player else { return };
        for (t, slot, param, value) in p.idle() {
            if slot == openpt_mix::INSTRUMENT_SLOT {
                continue;
            }
            let key = format!("param:{}:{slot}:{param}", t.0);
            let _ = self.engine.execute_merged("mix.insert_param", &json!({"track": t.0, "slot": slot, "param": param, "value": value}), &key);
        }
    }

    fn autosave(&mut self) {
        let Some(dir) = self.autosave_dir.clone() else { return };
        let last = *self.last_autosave.get_or_insert_with(std::time::Instant::now);
        let every = u64::from(self.ui.autosave_minutes.clamp(1, 120)) * 60;
        if !self.ui.autosave_enabled || last.elapsed().as_secs() < every || !self.engine.is_dirty() || self.engine.transport.recording {
            return;
        }
        self.last_autosave = Some(std::time::Instant::now());
        self.capture_plugin_states();
        self.autosave_to(&dir);
    }

    fn autosave_changes(&mut self) {
        if let Some(e) = self.autosaver.as_ref().and_then(openpt_engine::autosave::Autosaver::failure) {
            self.ui.status = format!("Autosave failed: {e}");
        }
        if self.autosave_dir.is_none() || self.engine.revision == self.autosaved_rev {
            return;
        }
        self.autosaved_rev = self.engine.revision;
        if !self.engine.is_dirty() {
            self.release_autosave(false);
        } else if let Some(root) = self.autosave_dir.clone() {
            self.queue_autosave(root);
        }
    }

    fn queue_autosave(&mut self, root: std::path::PathBuf) {
        self.autosaved_rev = self.engine.revision;
        let w = self.autosaver.get_or_insert_with(|| openpt_engine::autosave::Autosaver::new(root));
        w.write(self.engine.session_arc(), self.engine.path.clone(), self.take_plan.clone());
    }

    fn autosave_to(&mut self, root: &std::path::Path) -> bool {
        self.queue_autosave(root.to_path_buf());
        let Some(w) = &self.autosaver else { return false };
        let written = w.flush();
        if let Err(e) = &written {
            self.ui.status = format!("Autosave failed: {e}");
        }
        written.is_ok()
    }

    fn write_autosave(&mut self) -> bool {
        self.autosave_dir.clone().is_some_and(|root| self.autosave_to(&root))
    }

    fn release_autosave(&mut self, keep: bool) {
        if let Some(w) = &self.autosaver {
            w.close(keep);
        }
    }

    pub fn close_autosave(&mut self) {
        let recording = self.engine.transport.recording;
        if recording {
            self.stop_play();
        }
        let keep = self.engine.is_dirty() && (recording || !self.quit_requested);
        if keep {
            self.capture_plugin_states();
            self.write_autosave();
        }
        self.release_autosave(keep);
        if let Some(w) = &self.autosaver {
            let _ = w.flush();
        }
    }

    pub fn guarded(&mut self, f: impl FnOnce(&mut SoundApp)) {
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self))).is_ok() || self.panic_rev == self.engine.revision {
            return;
        }
        self.panic_rev = self.engine.revision;
        let saved = self.engine.is_dirty() && self.write_autosave();
        self.ui.status = if saved {
            "OpenPT hit an internal error (see the log). Your changes were autosaved: save them, then restart OpenPT.".into()
        } else {
            "OpenPT hit an internal error (see the log). Restart OpenPT to be safe.".into()
        };
    }

    pub fn recover(&mut self, left: &openpt_engine::autosave::Leftover) {
        if self.engine.transport.recording {
            self.ui.status = "Stop recording before recovering other work.".into();
            return;
        }
        self.close_autosave();
        match openpt_engine::autosave::recover(&mut self.engine, left) {
            Ok(missing) => {
                if self.write_autosave() {
                    openpt_engine::autosave::discard(left);
                }
                self.ui.status = match missing.len() {
                    0 => format!("Recovered “{}”. Save it to keep the changes.", left.name),
                    n => format!("Recovered “{}”, but {n} audio file(s) are missing. Save it to keep the changes.", left.name),
                };
            }
            Err(e) => self.ui.status = format!("Couldn't recover “{}”: {e}", left.name),
        }
    }

    pub fn latest_autosave(&self) -> Option<std::path::PathBuf> {
        let dir = self.autosave_dir.as_ref()?;
        let mut best: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
        for d in std::fs::read_dir(dir).ok()?.flatten() {
            for f in std::fs::read_dir(d.path()).into_iter().flatten().flatten() {
                let p = f.path();
                if p.extension().and_then(|x| x.to_str()) == Some("openpt")
                    && let Ok(m) = f.metadata().and_then(|m| m.modified())
                    && best.as_ref().is_none_or(|(t, _)| m > *t)
                {
                    best = Some((m, p));
                }
            }
        }
        best.map(|(_, p)| p)
    }

    fn meters_live(&self) -> bool {
        self.engine.transport.playing
            || (self.recorder.is_some() && self.engine.session().tracks.iter().any(|t| t.mixer.input_monitor || t.mixer.record_arm))
    }

    pub fn apply_audio_config(&mut self, cfg: openpt_playback::AudioConfig) -> Result<(), String> {
        if self.engine.transport.recording {
            return Err("stop recording before changing the audio device".into());
        }
        self.ui.audio = cfg;
        let at = self.position();
        if let Some(p) = &self.player {
            p.stop();
            p.close_editors();
        }
        self.recorder = None;
        self.recorder_failed = false;
        let player = openpt_playback::Player::with_config(self.engine.session_arc(), &self.ui.audio);
        player.locate(at);
        self.player = Some(player);
        self.sync_midi_devices();
        Ok(())
    }

    pub fn sync_midi_devices(&mut self) {
        if let Some(player) = &self.player {
            let flags = &self.engine.session().edit.flags;
            let enabled: Vec<String> = flags.iter().filter_map(|f| f.strip_prefix("midi.input.").map(str::to_string)).collect();
            if let Some(mm) = &mut self.midi_manager {
                mm.sync_enabled(&enabled);
            } else {
                let mut mm = openpt_playback::midi::MidiManager::new(player.midi_input_ring());
                mm.sync_enabled(&enabled);
                self.midi_manager = Some(mm);
            }
        }
    }

    fn ensure_input(&mut self) {
        if self.recorder.is_some() || self.recorder_failed || self.player.is_none() {
            return;
        }
        match openpt_playback::record::Recorder::open_with(&self.ui.audio) {
            Ok(r) => {
                if let Some(p) = &self.player {
                    p.set_input(std::sync::Arc::clone(&r.monitor));
                }
                self.recorder = Some(r);
            }
            Err(e) => {
                self.recorder_failed = true;
                self.ui.status = format!("No audio input: {e}");
            }
        }
    }

    fn watch_output(&mut self) {
        let Some(p) = &self.player else { return };
        let Some(trouble) = p.trouble() else { return };
        let name = p.device_name.clone();
        let (cause, then) = match trouble {
            OutputTrouble::Bug => (
                "the audio engine hit an internal error (see the log)".to_string(),
                "Use Reconnect audio device in Setup ▸ Playback Engine, or restart OpenPT.",
            ),
            OutputTrouble::Gone => {
                (format!("“{name}” was disconnected"), "Plug it back in, then use Reconnect audio device in Setup ▸ Playback Engine.")
            }
            OutputTrouble::Stalled => (format!("“{name}” stopped asking for audio"), "Use Reconnect audio device in Setup ▸ Playback Engine."),
        };
        let (recording, playing) = (self.engine.transport.recording, self.engine.transport.playing);
        if recording || playing {
            self.stop_play();
        }
        self.ui.status = if recording {
            format!("Recording stopped: {cause}. The take is kept up to there. {then}")
        } else if playing {
            format!("Playback stopped: {cause}. {then}")
        } else {
            format!("Audio output: {cause}. {then}")
        };
    }

    fn watch_input(&mut self) {
        if let Some(mr) = &mut self.midi_recorder {
            mr.poll(self.engine.session());
        }
        let recording = self.engine.transport.recording;
        let Some(r) = &mut self.recorder else { return };
        let Some(trouble) = r.trouble() else {
            let lost = r.lost_ms();
            if recording && lost > self.lost_shown {
                self.lost_shown = lost;
                self.ui.status = format!("The disk is too slow: this take has lost {lost} ms of input so far.");
            }
            return;
        };
        if recording {
            self.stop_play();
        }
        let (what, then) = match trouble {
            Trouble::Device(what) => {
                self.recorder = None;
                self.recorder_failed = true;
                (what, "Reconnect it, then use Reconnect audio device in Setup ▸ Playback Engine.")
            }
            Trouble::Disk(what) => (what, "Check the disk, then record again."),
        };
        self.ui.status = if recording {
            format!("Recording stopped: {what}. The take is kept up to there. {then}")
        } else {
            format!("Audio input: {what}. {then}")
        };
    }

    pub fn reconnect_audio(&mut self) {
        if self.is_playing() || self.engine.transport.recording {
            self.stop_play();
        }
        if let Some(p) = &self.player {
            p.close_editors();
        }
        self.recorder = None;
        self.recorder_failed = false;
        self.player = Some(Player::with_config(self.engine.session_arc(), &self.ui.audio));
        self.sync_midi_devices();
    }

    fn finish_recording(&mut self) {
        self.engine.transport.recording = false;
        if let Some(p) = &self.player {
            p.set_recording(false);
        }
        let pos = self.position();
        let plan = self.take_plan.take();
        let (audio_takes, problem) = self.recorder.as_mut().map(|r| r.finish()).unwrap_or_default();
        let midi_takes = self.midi_recorder.take().map(|mut r| r.finish(self.engine.session(), pos)).unwrap_or_default();
        if std::mem::take(&mut self.discard_take) {
            self.punch = None;
            self.ui.status = "Take discarded".into();
            return;
        }
        let tracks = audio_takes.len().max(1);
        let plan = plan.unwrap_or_else(|| openpt_engine::io::Recording {
            start: self.record_start,
            pass: None,
            punch: self.punch,
            rate: 0,
            takes: Vec::new(),
        });
        let punch = self.punch;
        self.punch = None;
        self.autosaved_rev = 0;
        let mut audio_ids = Vec::new();
        if !audio_takes.is_empty() {
            audio_ids = openpt_engine::io::place_recording(&mut self.engine, &plan, audio_takes);
        }
        let mut midi_ids = Vec::new();
        if !midi_takes.is_empty() {
            let merge = self.engine.session().edit.flag("options.midi_merge");
            midi_ids = openpt_engine::io::place_midi_takes(&mut self.engine, midi_takes, punch, merge, plan.pass);
        }
        let total_clips = audio_ids.len() + midi_ids.len();
        self.ui.status = if plan.pass.is_some() {
            format!("Loop-recorded {} take(s)", audio_ids.len() / tracks)
        } else {
            format!("Recorded {} clip(s)", total_clips)
        };
        if let Some(p) = problem {
            log::warn!("recording: {p}");
            self.ui.status = format!("{} ({p})", self.ui.status);
        }
    }

    fn write_automation(&mut self, ctx: &egui::Context) {
        use openpt_model::AutomationMode as M;
        const PASS: &str = "automation_pass";
        let playing = self.is_playing();
        let may_write =
            playing && self.engine.session().tracks.iter().any(|t| matches!(t.mixer.automation_mode, M::Write | M::Touch | M::Latch | M::TouchLatch));
        self.engine.hold_merge(may_write.then_some(PASS));
        if !playing && self.touched.is_empty() {
            self.last_write_at = i64::MIN;
            return;
        }
        let pos = self.engine.transport.position;
        let step = self.engine.session().sample_rate.samples(0.02);
        if playing && !automation_step_due(pos, self.last_write_at, step) {
            return;
        }
        let from = std::mem::replace(&mut self.last_write_at, if playing { pos } else { i64::MIN });
        let dragging = ctx.input(|i| i.pointer.any_down());
        let fader_track = match &self.gesture {
            Some(Gesture::Fader { track, .. }) => Some(*track),
            _ => None,
        };
        let writes: Vec<(TrackId, f32)> = self
            .engine
            .session()
            .tracks
            .iter()
            .filter_map(|t| {
                let m = t.mixer.automation_mode;
                let touched_now = dragging && fader_track == Some(t.id);
                let write = match m {
                    M::Write => true,
                    M::Touch => touched_now,
                    M::Latch | M::TouchLatch => touched_now || self.touched.contains(&t.id),
                    _ => false,
                };
                write.then_some((t.id, t.mixer.volume_db))
            })
            .collect();
        for (t, v) in writes {
            self.touched.insert(t);
            let mut point = serde_json::json!({"track": t.0, "param": "volume", "at": pos, "value": v});
            if from != i64::MIN {
                point["from"] = from.into();
            }
            let _ = self.engine.execute_merged("automation.set_point", &point, PASS);
        }
        if !playing {
            self.touched.clear();
            self.engine.end_merge();
        }
    }

    pub fn play_from_selection(&mut self) {
        let s = self.engine.session();
        let sel = s.edit.selection;
        let preroll = if s.edit.pre_post_roll { s.edit.pre_roll } else { 0 };
        let postroll = if s.edit.pre_post_roll { s.edit.post_roll } else { 0 };
        if s.edit.loop_playback && !sel.is_empty() {
            self.start_play(sel.start, None, Some(sel));
        } else if !sel.is_empty() {
            self.start_play((sel.start - preroll).max(0), Some(sel.end + postroll), None);
        } else {
            self.start_play((sel.start - preroll).max(0), None, None);
        }
    }

    fn handle_transport(&mut self, dt: f32) {
        let reqs: Vec<TransportRequest> = std::mem::take(&mut self.engine.transport_requests);
        for r in reqs {
            match r {
                TransportRequest::ResetSession => {
                    if let Some(p) = &self.player {
                        p.stop();
                        p.set_recording(false);
                        p.set_speed(1.0);
                        p.locate(self.engine.transport.position);
                    }
                    self.sim = None;
                    self.recorder = None;
                    self.recorder_failed = false;
                    self.punch = None;
                    self.gesture = None;
                    self.touched.clear();
                    self.last_write_at = i64::MIN;
                    self.edit_layout.follow_hold = false;
                    self.meters.clear();
                    self.main_meter = MeterDisplay::default();
                }
                TransportRequest::Play => {
                    if !self.is_playing() {
                        self.play_from_selection();
                    }
                }
                TransportRequest::Stop | TransportRequest::Pause => self.stop_play(),
                TransportRequest::TogglePlay => {
                    if self.is_playing() {
                        self.stop_play();
                    } else {
                        self.play_from_selection();
                    }
                }
                TransportRequest::Record => {
                    if self.engine.transport.recording {
                        self.stop_play();
                    } else {
                        self.start_recording();
                    }
                }
                TransportRequest::Locate(at) => {
                    if self.is_playing() {
                        if let Some(p) = &self.player {
                            p.locate(at);
                        }
                        if let Some(sim) = &mut self.sim {
                            sim.0 = at;
                        }
                        self.last_write_at = i64::MIN;
                    }
                    self.engine.transport.position = at;
                }
                TransportRequest::PlaySelection => {
                    let sel = self.engine.session().edit.selection;
                    self.start_play(sel.start, Some(sel.end.max(sel.start + 1)), None);
                }
                TransportRequest::HalfSpeed => {
                    if let Some(p) = &self.player {
                        p.set_speed(0.5);
                    }
                    self.play_from_selection();
                }
                TransportRequest::Scrub(at) => self.engine.transport.position = at,
                TransportRequest::AllNotesOff => {
                    if let Some(p) = &self.player {
                        p.panic_all_notes_off();
                    }
                }
            }
        }
        if let Some(p) = &self.player {
            let playing = p.is_playing();
            let ended = self.engine.transport.playing && !playing;
            if ended {
                p.set_speed(1.0);
            }
            if self.engine.transport.playing {
                self.engine.transport.position = p.position();
            }
            let now = self.last_frame.unwrap_or_default();
            let meter_interval = if self.ui.window == MainWindow::Mix { 1.0 / 60.0 } else { 1.0 / 20.0 };
            if now - self.last_meter_poll >= meter_interval {
                p.copy_meters(&mut self.meter_snapshot);
                self.last_meter_poll = now;
            }
            let snap = &self.meter_snapshot;
            let b = Ballistics::from_prefs(&self.ui);
            for (id, m) in &snap.tracks {
                let d = self.meters.entry(*id).or_default();
                feed_meter(d, m.peak, m.gain_reduction_db, dt, &b);
                if m.plugin_failures.count != d.plugin_failures {
                    d.plugin_failures = m.plugin_failures.count;
                    if let Some(msg) = failed_plugin_status(self.engine.session(), *id, m.plugin_failures.last) {
                        self.ui.status = msg;
                    }
                }
            }
            let main = snap.main.peak;
            feed_meter(&mut self.main_meter, main, 0.0, dt, &b);
            if ended {
                self.stop_play();
            }
        } else if let Some((pos, end, looped)) = &mut self.sim {
            let sr = self.engine.session().sample_rate.as_f64();
            *pos += (f64::from(dt) * sr) as i64;
            if let Some(l) = looped
                && *pos >= l.end
            {
                *pos = l.start;
            }
            let stop = end.is_some_and(|e| *pos >= e);
            self.engine.transport.position = *pos;
            if stop {
                self.stop_play();
            }
        }
        if !self.meters_live() {
            for d in self.meters.values_mut() {
                feed_meter(d, [0.0, 0.0], 0.0, dt, &Ballistics::from_prefs(&self.ui));
            }
            feed_meter(&mut self.main_meter, [0.0, 0.0], 0.0, dt, &Ballistics::from_prefs(&self.ui));
        }
    }

    pub fn logic(&mut self, ctx: &egui::Context) {
        self.engine.history_limit = self.ui.undo_levels.clamp(1, 10_000);
        openpt_midi::set_middle_c(match self.ui.note_names.as_str() {
            "c4" => openpt_midi::MiddleC::C4,
            "number" => openpt_midi::MiddleC::Number,
            _ => openpt_midi::MiddleC::C3,
        });
        if ctx.current_pass_index() > 0 {
            return;
        }
        if !self.fonts_ready {
            if self.fonts_installed {
                self.fonts_ready = true;
            } else {
                fonts::install(ctx);
                self.fonts_installed = true;
                ctx.request_repaint();
            }
        }
        let light = theme::wants_light(ctx, self.ui.theme);
        if self.theme_applied != Some(light) || theme::is_light() != light {
            theme::apply(ctx, light);
            self.theme_applied = Some(light);
        }
        let now = ctx.input(|i| i.time);
        let dt = self.last_frame.replace(now).map_or(1.0 / 60.0, |t| (now - t) as f32).clamp(0.0, 0.25);
        self.frame_ms = self.frame_ms * 0.9 + dt * 1000.0 * 0.1;
        control::drain(self, ctx);
        self.note_recent_session();
        self.apply_editor_edits();
        bounce::step(self, ctx);
        if !ctx.input(|i| i.pointer.any_down()) {
            if matches!(self.gesture, Some(Gesture::Fader { .. })) {
                self.gesture = None;
            }
            if !ctx.egui_wants_keyboard_input() && self.touched.is_empty() {
                self.engine.end_merge();
            }
        }
        self.watch_output();
        self.watch_input();
        self.handle_transport(dt);
        if self.engine.revision != self.last_rev {
            self.last_rev = self.engine.revision;
            if let Some(p) = &self.player {
                p.update_session(self.engine.session_arc());
            }
        }
        self.autosave_changes();
        self.write_automation(ctx);
        self.autosave();
        if self.recorder.is_none()
            && !self.recorder_failed
            && self.engine.session().tracks.iter().any(|t| t.mixer.input_monitor || t.mixer.record_arm)
        {
            self.ensure_input();
        }
        let editors = self.player.as_ref().is_some_and(Player::any_editor_open);
        ctx.request_repaint_after_secs(if editors { 0.03 } else { 1.0 } + ctx.input(|i| i.predicted_dt));
        control::collect_screenshots(self, ctx);
    }

    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        if !self.synthetic.is_empty() {
            self.synthetic_grace = 6;
        }
        if self.synthetic_grace > 0 {
            self.synthetic_grace -= 1;
            raw.events.retain(|e| {
                !matches!(e, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. } | egui::Event::PointerGone | egui::Event::MouseMoved(_))
            });
        }
        let Some(first) = self.synthetic.first().cloned() else { return };
        let take = match first {
            egui::Event::Key { pressed: true, .. } => {
                if matches!(self.synthetic.get(1), Some(egui::Event::Key { pressed: false, .. })) { 2 } else { 1 }
            }
            _ => 1,
        };
        let n = take.min(self.synthetic.len());
        let evs: Vec<egui::Event> = self.synthetic.drain(..n).collect();
        for e in &evs {
            if let egui::Event::PointerButton { pos, .. } | egui::Event::PointerMoved(pos) = e {
                raw.events.push(egui::Event::PointerMoved(*pos));
            }
        }
        raw.events.extend(evs);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.set_style(ctx.global_style());
        if !self.fonts_ready {
            ctx.request_repaint();
            return;
        }
        if self.arrange_request {
            self.arrange_request = false;
            ctx.memory_mut(|m| m.reset_areas());
        }
        shortcuts::handle(self, &ctx);
        self.sample_drop_targets.clear();
        if !self.native_menu_bar {
            menus::menu_bar(self, ui);
        }
        match if self.ui.mix_separate { MainWindow::Edit } else { self.ui.window } {
            MainWindow::Edit => edit_window::show(self, ui),
            MainWindow::Mix => mix_window::show(self, ui),
        }
        if self.ui.mix_separate {
            mix_window::show_separate(self, &ctx);
        }
        panels::floating(self, &ctx);
        windows::show(self, &ctx);
        video_window::show(self, &ctx);
        ops_windows::show(self, &ctx);
        extra_windows::show(self, &ctx);
        recovery::show(self, &ctx);
        score_editor::show(self, &ctx);
        palette::show(self, &ctx);
        mix_window::pending_editor(self, &ctx);
        edit_window::io_mini_fader(self, &ctx);
        import_jobs::show(self, &ctx);
        dashboard::show(self, &ctx);
        preferences::show(self, &ctx);
        dialogs::show(self, &ctx);
        if self.meters_live() {
            let hz = if self.ui.window == MainWindow::Mix { 60 } else { 30 };
            ctx.request_repaint_after(std::time::Duration::from_millis(1000 / hz));
        } else if self.meters.values().any(|m| m.level[0] > 0.0001) {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        } else if !self.pending_shots.is_empty() || !self.synthetic.is_empty() {
            ctx.request_repaint();
        }
    }

    pub fn inspect(&self, ctx: &egui::Context) -> Value {
        let size = ctx.content_rect().size();
        json!({
            "ui": self.ui,
            "window_size": [size.x, size.y],
            "playing": self.is_playing(),
            "position": self.position(),
            "dialog": self.dialogs.open_name(),
            "frame_ms": self.frame_ms,
            "audio_device": self.player.as_ref().map(|p| p.device_name.clone()),
            "status": self.ui.status,
        })
    }
}

struct Ballistics {
    decay_db_s: f32,
    hold: f32,
    clip: f32,
}

impl Ballistics {
    fn from_prefs(ui: &UiState) -> Self {
        let secs = |s: &str| match s {
            "infinite" => f32::INFINITY,
            "none" => 0.0,
            _ => 3.0,
        };
        Ballistics { decay_db_s: ui.meter_decay_db_s.clamp(1.0, 300.0), hold: secs(&ui.meter_peak_hold), clip: secs(&ui.meter_clip_hold) }
    }
}

fn feed_meter(d: &mut MeterDisplay, peak: [f32; 2], gr: f32, dt: f32, b: &Ballistics) {
    let fall = 10f32.powf(-b.decay_db_s * dt / 20.0);
    for c in 0..2 {
        let v = peak.get(c).copied().unwrap_or(0.0);
        let lvl = d.level.get(c).copied().unwrap_or(0.0);
        let nv = if v >= lvl { v } else { (lvl * fall).max(v) };
        if let Some(x) = d.level.get_mut(c) {
            *x = if nv < 1e-5 { 0.0 } else { nv };
        }
        if v >= d.hold.get(c).copied().unwrap_or(0.0) {
            if let Some(h) = d.hold.get_mut(c) {
                *h = v;
            }
            d.hold_age = 0.0;
        }
        if v >= 1.0 && b.clip > 0.0 {
            d.clip = true;
            d.clip_age = 0.0;
        }
    }
    d.hold_age += dt;
    if d.hold_age > b.hold {
        d.hold = d.level;
    }
    d.clip_age += dt;
    if d.clip_age > b.clip {
        d.clip = false;
    }
    d.gr = if gr > d.gr { gr } else { d.gr * fall };
}

fn failed_plugin_status(s: &openpt_model::Session, track: TrackId, slot: usize) -> Option<String> {
    let t = s.track(track)?;
    let ins = if slot == openpt_mix::INSTRUMENT_SLOT { t.instrument.as_ref() } else { t.mixer.inserts.get(slot)?.as_ref() }?;
    let name = crate::mix_window::plugin_info(&ins.plugin).map_or(ins.plugin.as_str(), |p| p.name);
    Some(format!("{name} on {} put out invalid audio, so it is bypassed until playback stops or starts again.", t.name))
}

#[cfg(test)]
mod tests {
    use super::{Services, SoundApp, automation_step_due};
    use serde_json::json;

    #[test]
    fn new_session_stops_playback_and_the_old_clock() {
        for (command, params) in [
            ("session.new", json!({"name": "New project", "sample_rate": 96_000})),
            ("session.new", json!({"template": "demo"})),
            ("session.close", json!({})),
        ] {
            let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
            app.run("transport.play", json!({"from": 48_000})).unwrap();
            app.handle_transport(0.25);
            assert!(app.is_playing());
            assert!(app.position() > 48_000);
            app.run(command, params).unwrap();
            let insertion = app.engine.session().edit.selection.start;
            app.handle_transport(0.25);
            assert!(!app.is_playing(), "{command}");
            assert_eq!(app.position(), insertion, "{command}");
            assert_eq!(app.engine.transport.position, insertion, "{command}");
            app.handle_transport(0.25);
            assert_eq!(app.engine.transport.position, insertion, "the old clock must stay stopped");
            app.run("transport.play", json!({})).unwrap();
            app.handle_transport(0.25);
            assert!(app.is_playing());
            assert_eq!(app.position(), insertion + i64::from(app.engine.session().sample_rate.hz()) / 4);
        }
    }

    #[test]
    fn the_status_names_a_plugin_the_engine_took_out() {
        use openpt_model::{ChannelFormat, Insert, TrackKind};
        let mut s = openpt_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some("Vox"));
        s.track_mut(t).unwrap().mixer.inserts[3] = Some(Insert::new("gain"));
        let msg = super::failed_plugin_status(&s, t, 3).unwrap();
        assert!(msg.starts_with("Gain on Vox put out invalid audio"), "{msg}");
        assert_eq!(super::failed_plugin_status(&s, t, 0), None);
        assert_eq!(super::failed_plugin_status(&s, t, openpt_mix::INSTRUMENT_SLOT), None);
    }

    #[test]
    fn new_session_cancels_play_queued_before_the_next_frame() {
        let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
        app.run("transport.play", json!({})).unwrap();
        app.run("session.new", json!({})).unwrap();
        app.handle_transport(0.25);
        assert!(!app.is_playing());
        assert_eq!(app.engine.transport.position, 0);
    }

    #[test]
    fn record_with_nothing_armed_says_what_to_do() {
        let mut app = SoundApp::new(openpt_engine::Engine::default(), None, Services::default());
        app.run("transport.record", json!({})).unwrap();
        app.handle_transport(0.25);
        assert!(!app.is_playing() && !app.engine.transport.recording);
        assert!(app.record_hint().is_some_and(|why| why.contains("Track › New")), "{:?}", app.record_hint());

        app.run("track.new", json!({"count": 1, "format": "mono"})).unwrap();
        app.run("transport.record", json!({})).unwrap();
        app.handle_transport(0.25);
        assert!(app.record_hint().is_some_and(|why| why.contains("R button")), "{:?}", app.record_hint());
        assert_eq!(Some(app.ui.status.as_str()), app.record_hint());
        app.last_frame = Some(5.0);
        assert_eq!(app.record_hint(), None, "the hint shows for a few seconds");

        app.last_frame = None;
        app.run("transport.record", json!({})).unwrap();
        app.handle_transport(0.25);
        app.run("mix.record_arm", json!({})).unwrap();
        assert_eq!(app.record_hint(), None, "arming a track answers the hint");
    }

    #[test]
    fn a_hidden_window_still_autosaves() {
        let dir = std::env::temp_dir().join(format!("openpt-hidden-autosave-{}", std::process::id()));
        let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
        app.autosave_dir = Some(dir.clone());
        app.run("track.new", json!({"count": 1, "format": "mono"})).unwrap();
        app.last_autosave = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(121));
        let hidden = egui::ViewportInfo { minimized: Some(true), ..Default::default() };
        let raw = egui::RawInput { viewports: [(egui::ViewportId::ROOT, hidden)].into_iter().collect(), ..Default::default() };
        let _ = egui::Context::default().run_logic(&raw, |ctx| app.logic(ctx));
        assert!(app.latest_autosave().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_reset_does_not_apply_old_stop_edits_to_the_new_session() {
        let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
        app.run("transport.play", json!({"from": 48_000})).unwrap();
        app.handle_transport(0.25);
        let mut session = openpt_model::Session::default();
        session.edit.selection = openpt_time::Range::new(12_000, 24_000);
        session.edit.insertion_follows_playback = true;
        session.edit.scrolling = "after_playback".into();
        let selection = session.edit.selection;
        app.engine.replace_session(session);
        app.handle_transport(0.25);
        assert!(!app.is_playing());
        assert_eq!(app.engine.session().edit.selection, selection);
        assert_eq!(app.position(), selection.start);
        assert_eq!(app.engine.session().edit.zoom.scroll, 0);
        assert!(!app.engine.is_dirty());
    }

    #[test]
    fn clear_all_clip_indicators_clears_every_meter() {
        let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
        let kick = app.engine.session().track_by_name("Kick").unwrap().id;
        app.meters.entry(kick).or_default().clip = true;
        app.main_meter.clip = true;
        app.run("track.clear_clip_indicators", json!({})).unwrap();
        assert!(!app.meters[&kick].clip && !app.main_meter.clip);
    }

    #[test]
    fn automation_first_pass_after_start_is_due_without_overflow() {
        assert!(automation_step_due(0, i64::MIN, 960));
        assert!(automation_step_due(48_000, i64::MIN, 960));
    }

    #[test]
    fn automation_write_throttles_to_step() {
        assert!(!automation_step_due(1_000, 900, 960));
        assert!(automation_step_due(2_000, 900, 960));
    }

    #[test]
    fn automation_write_fires_after_loop_wrap() {
        assert!(automation_step_due(0, 1_000_000, 960));
    }

    #[test]
    fn a_write_pass_replaces_the_automation_it_plays_over() {
        let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
        let fader = app.engine.session().track_by_name("Kick").unwrap().mixer.volume_db;
        let q = i64::from(app.engine.session().sample_rate.hz()) / 4;
        for at in (0..12 * q).step_by(usize::try_from(q / 25).unwrap()) {
            app.run("automation.set_point", json!({"track": "Kick", "at": at, "value": -30.0})).unwrap();
        }
        app.run("mix.automation_mode", json!({"tracks": ["Kick"], "mode": "write"})).unwrap();
        app.run("transport.locate", json!({"at": 0})).unwrap();
        app.run("transport.play", json!({})).unwrap();
        let ctx = egui::Context::default();
        let pass = |app: &mut SoundApp| {
            app.handle_transport(0.25);
            app.write_automation(&ctx);
        };
        pass(&mut app);
        pass(&mut app);
        app.run("transport.locate", json!({"at": 4 * q})).unwrap();
        pass(&mut app);
        app.run("edit.select", json!({"tracks": ["Kick"], "start": 7 * q, "end": 11 * q})).unwrap();
        app.run("transport.play_selection", json!({})).unwrap();
        pass(&mut app);
        app.handle_transport(0.25);
        app.run("transport.stop", json!({})).unwrap();
        app.handle_transport(0.0);
        app.write_automation(&ctx);
        let lane = app.engine.session().track_by_name("Kick").unwrap().lane(&openpt_model::AutoParam::Volume).unwrap();
        let values = |from, to| lane.points.iter().filter(|p| (from..=to).contains(&p.at)).map(|p| p.value).collect::<Vec<_>>();
        assert_eq!(values(q, 2 * q), [fader; 2]);
        assert_eq!(lane.value_at(3 * q, 0.0), -30.0);
        assert_eq!(lane.value_at(6 * q, 0.0), -30.0);
        assert_eq!(values(8 * q, 9 * q), [fader; 2]);
        assert_eq!(lane.value_at(10 * q, 0.0), -30.0);
    }

    #[test]
    fn a_pass_and_the_fader_ride_driving_it_are_one_undo_step() {
        let mut app = SoundApp::new(openpt_engine::demo::demo_engine(), None, Services::default());
        let kick = app.engine.session().track_by_name("Kick").unwrap().id;
        app.run("mix.automation_mode", json!({"tracks": ["Kick"], "mode": "write"})).unwrap();
        app.run("transport.play", json!({})).unwrap();
        let steps = app.engine.undo_history().len();
        let ctx = egui::Context::default();
        for frame in 0..60 {
            app.handle_transport(1.0 / 60.0);
            app.write_automation(&ctx);
            let db = -10.0 + frame as f32 / 10.0;
            app.engine.execute_merged("mix.volume", &json!({"track": kick.0, "db": db}), &format!("fader:{}", kick.0)).unwrap();
        }
        app.run("transport.stop", json!({})).unwrap();
        app.handle_transport(0.0);
        app.write_automation(&ctx);
        assert_eq!(app.engine.undo_history().len(), steps + 1);
    }

    #[test]
    fn preferences_saved_before_the_window_was_remembered_keep_full_scale() {
        let ui: super::UiState = serde_json::from_value(json!({"show_transport": true})).unwrap();
        assert_eq!((ui.window_size, ui.window_maximized, ui.ui_scale), ([1600.0, 1000.0], false, 1.0));
    }

    #[test]
    fn a_window_bigger_than_its_screen_opens_maximized() {
        let maximizes = |w: f32, frames: usize| {
            let ctx = egui::Context::default();
            let info = egui::ViewportInfo {
                inner_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, 1000.0))),
                monitor_size: Some(egui::vec2(1728.0, 1117.0)),
                ..Default::default()
            };
            let mut maximized = false;
            for _ in 0..frames {
                let raw = egui::RawInput { viewports: [(egui::ViewportId::ROOT, info.clone())].into_iter().collect(), ..Default::default() };
                let mut out = ctx.run_ui(raw, |ui| super::fit_window_to_screen(ui.ctx()));
                out.textures_delta.clear();
                maximized = out.viewport_output[&egui::ViewportId::ROOT].commands.contains(&egui::ViewportCommand::Maximized(true));
            }
            maximized
        };
        assert!(maximizes(2400.0, 1));
        assert!(!maximizes(1600.0, 1));
        assert!(!maximizes(2400.0, 2));
    }

    #[test]
    fn ordinary_play_after_stop_is_full_speed() {
        use serde_json::json;
        let engine = openpt_engine::Engine::default();
        let player = openpt_playback::Player::new(engine.session_arc());
        let mut app = super::SoundApp::new(engine, Some(player), super::Services::default());
        app.engine.execute("edit.select", &json!({"start": 0, "end": 48000})).unwrap();
        app.engine.execute("options.loop_playback", &json!({"value": true})).unwrap();
        app.engine.execute("options.pre_post_roll", &json!({"value": false})).unwrap();
        let speed = |app: &super::SoundApp| app.player.as_ref().map(|p| p.speed()).unwrap_or(0.0);
        assert!((speed(&app) - 1.0).abs() < 1e-3);

        app.engine.execute("transport.half_speed", &json!({})).unwrap();
        app.handle_transport(1.0 / 60.0);
        assert!(app.is_playing());
        assert!((speed(&app) - 0.5).abs() < 1e-3, "half-speed playback");

        app.engine.execute("transport.stop", &json!({})).unwrap();
        app.handle_transport(1.0 / 60.0);
        assert!(!app.is_playing());
        assert!((speed(&app) - 1.0).abs() < 1e-3, "stop restores full speed");

        app.engine.execute("transport.play", &json!({})).unwrap();
        app.handle_transport(1.0 / 60.0);
        assert!(app.is_playing());
        assert!((speed(&app) - 1.0).abs() < 1e-3, "ordinary play stays at full speed");

        app.engine.execute("transport.stop", &json!({})).unwrap();
        app.handle_transport(1.0 / 60.0);
        app.engine.execute("options.loop_playback", &json!({"value": false})).unwrap();
        app.engine.execute("transport.half_speed", &json!({})).unwrap();
        app.handle_transport(1.0 / 60.0);
        assert!((speed(&app) - 0.5).abs() < 1e-3);
        app.player.as_ref().unwrap().stop();
        app.handle_transport(1.0 / 60.0);
        assert!(!app.is_playing());
        assert!((speed(&app) - 1.0).abs() < 1e-3, "a half-speed pass that ends on its own restores full speed");
    }
}
