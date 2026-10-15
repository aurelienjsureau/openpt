#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod autosave;
pub mod beat_detective;
pub mod catalog;
pub mod catalog_more;
pub mod clip_group_file;
pub mod cmd;
pub mod demo;
pub mod edit;
pub mod file_access;
pub mod inspect;
pub mod io;
pub mod score;

use openpt_model::{Clip, Session, TrackId, ZoomState};
use openpt_time::Samples;
use serde_json::{Value, json};
use std::cell::Cell;
use std::sync::Arc;

pub use cmd::{CommandInfo, CommandSpec, command_specs, find_command};
pub use openpt_model as model;

pub fn shutdown_plugin_hosts() {
    openpt_vst3_host::shutdown();
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    #[error("`{0}` is not available: {1}")]
    Disabled(String, String),
    #[error("bad parameters for `{0}`: {1}")]
    BadParams(String, String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Io(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("internal error in `{0}`: {1}")]
    Internal(String, String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub enum TransportRequest {
    ResetSession,
    Play,
    Stop,
    TogglePlay,
    Record,
    Locate(Samples),
    PlaySelection,
    Pause,
    HalfSpeed,
    Scrub(Samples),
    AllNotesOff,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize)]
pub struct TransportStatus {
    pub playing: bool,
    pub recording: bool,
    pub position: Samples,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clipboard {
    pub length: Samples,
    pub tracks: Vec<Vec<Clip>>,
    pub automation: Vec<Vec<openpt_model::AutomationLane>>,
    pub clip_gain: Vec<Vec<(Samples, f32)>>,
    pub clip_effects: Vec<(String, f64)>,
    pub markers: Vec<openpt_model::MemoryLocation>,
    pub grid_offset: Samples,
    pub automation_only: bool,
}

impl Clipboard {
    pub fn of_range(s: &Session, range: openpt_time::Range) -> Clipboard {
        let line = s.grid_line_at_or_before(range.start);
        Clipboard { length: range.len(), grid_offset: range.start.saturating_sub(line), ..Clipboard::default() }
    }
}

#[derive(Debug, Clone)]
struct HistoryEntry {
    label: String,
    doc: Arc<Session>,
}

pub struct Engine {
    doc: Arc<Session>,
    undo: Vec<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    pub history_limit: usize,
    pub clipboard: Clipboard,
    pub transport_requests: Vec<TransportRequest>,
    pub transport: TransportStatus,
    pub journal: Vec<(String, Value)>,
    pub path: Option<String>,
    pub revision: u64,
    dirty: bool,
    saved: Arc<Session>,
    dirty_at: Cell<(u64, bool)>,
    merge_key: Option<String>,
    merge_hold: Option<String>,
    pub messages: Vec<String>,
    pub last_selection: Option<(openpt_time::Range, Vec<TrackId>)>,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new(Session::default())
    }
}

impl Engine {
    pub fn new(session: Session) -> Self {
        install_panic_hook();
        let doc = Arc::new(session);
        Engine {
            saved: Arc::clone(&doc),
            doc,
            undo: Vec::new(),
            redo: Vec::new(),
            history_limit: 200,
            clipboard: Clipboard::default(),
            transport_requests: Vec::new(),
            transport: TransportStatus::default(),
            journal: Vec::new(),
            path: None,
            revision: 1,
            dirty: false,
            dirty_at: Cell::new((0, false)),
            merge_key: None,
            merge_hold: None,
            messages: Vec::new(),
            last_selection: None,
        }
    }

    pub fn session(&self) -> &Session {
        &self.doc
    }

    pub fn session_arc(&self) -> Arc<Session> {
        Arc::clone(&self.doc)
    }

    pub fn session_mut(&mut self) -> &mut Session {
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        Arc::make_mut(&mut self.doc)
    }

    pub fn zoom_mut(&mut self) -> &mut ZoomState {
        self.dirty = true;
        &mut Arc::make_mut(&mut self.doc).edit.zoom
    }

    pub fn replace_session(&mut self, s: Session) {
        self.transport = TransportStatus { position: s.edit.selection.start.max(0), ..TransportStatus::default() };
        self.transport_requests.clear();
        self.transport_requests.push(TransportRequest::ResetSession);
        self.doc = Arc::new(s);
        self.saved = Arc::clone(&self.doc);
        self.undo.clear();
        self.redo.clear();
        self.last_selection = None;
        self.revision = self.revision.wrapping_add(1);
        self.dirty = false;
    }

    pub fn is_dirty(&self) -> bool {
        if !self.dirty {
            return false;
        }
        let (rev, dirty) = self.dirty_at.get();
        if rev == self.revision {
            return dirty;
        }
        let dirty = !same_document(&self.saved, &self.doc);
        self.dirty_at.set((self.revision, dirty));
        dirty
    }
    pub fn mark_clean(&mut self) {
        self.dirty = false;
        self.saved = Arc::clone(&self.doc);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|h| h.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|h| h.label.as_str())
    }
    pub fn undo_history(&self) -> Vec<String> {
        self.undo.iter().map(|h| h.label.clone()).collect()
    }

    pub fn undo(&mut self) -> bool {
        let Some(h) = self.undo.pop() else { return false };
        let cur = std::mem::replace(&mut self.doc, h.doc);
        self.keep_edit_mode(&cur);
        self.redo.push(HistoryEntry { label: h.label, doc: cur });
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(h) = self.redo.pop() else { return false };
        let cur = std::mem::replace(&mut self.doc, h.doc);
        self.keep_edit_mode(&cur);
        self.undo.push(HistoryEntry { label: h.label, doc: cur });
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        true
    }

    fn keep_edit_mode(&mut self, from: &Session) {
        type Listen = (bool, bool, bool, bool, bool, bool, openpt_model::TrackHeight, String);
        let listen = |t: &openpt_model::Track| -> Listen {
            let m = &t.mixer;
            (m.solo, m.mute, m.record_arm, m.input_monitor, m.solo_safe, m.record_safe, t.height, t.view.clone())
        };
        let was: std::collections::HashMap<openpt_model::TrackId, Listen> = from.tracks.iter().map(|t| (t.id, listen(t))).collect();
        if self.doc.tracks.iter().any(|t| was.get(&t.id).is_some_and(|w| *w != listen(t))) {
            for t in &mut Arc::make_mut(&mut self.doc).tracks {
                if let Some((solo, mute, rec, input, solo_safe, rec_safe, height, view)) = was.get(&t.id).cloned() {
                    let m = &mut t.mixer;
                    (m.solo, m.mute, m.record_arm, m.input_monitor, m.solo_safe, m.record_safe) = (solo, mute, rec, input, solo_safe, rec_safe);
                    t.height = height;
                    t.view = view;
                }
            }
        }
        use edit::{RELATIVE_GRID, SHUFFLE_LOCK, SNAP_TO_GRID};
        let flags = [SNAP_TO_GRID, SHUFFLE_LOCK, RELATIVE_GRID];
        let ed = &self.doc.edit;
        if ed.edit_mode != from.edit.edit_mode || flags.iter().any(|f| ed.flag(f) != from.edit.flag(f)) {
            let ed = &mut Arc::make_mut(&mut self.doc).edit;
            ed.edit_mode = from.edit.edit_mode;
            for f in flags {
                ed.set_flag(f, from.edit.flag(f));
            }
        }
    }

    pub fn push_undo(&mut self, label: &str, before: Arc<Session>) {
        if Arc::ptr_eq(&before, &self.doc) {
            return;
        }
        self.undo.push(HistoryEntry { label: label.to_string(), doc: before });
        self.redo.clear();
        if self.undo.len() > self.history_limit {
            let excess = self.undo.len() - self.history_limit;
            self.undo.drain(..excess);
        }
    }

    pub fn blocked_while_recording(&self, id: &str) -> bool {
        self.transport.recording && matches!(id, "session.new" | "session.open" | "session.close" | "session.revert" | "app.quit")
    }

    pub fn execute_each(&mut self, id: &str, params: &[Value]) -> Vec<Result<Value>> {
        let before = Arc::clone(&self.doc);
        let undo_len = self.undo.len();
        let out: Vec<Result<Value>> = params.iter().map(|p| self.execute(id, p)).collect();
        if self.undo.len() > undo_len {
            let label = self.undo.get(undo_len).map(|h| h.label.clone()).unwrap_or_default();
            self.undo.truncate(undo_len);
            self.push_undo(&label, before);
        }
        out
    }

    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        if self.blocked_while_recording(id) {
            return Err(EngineError::Disabled(id.to_string(), "stop recording first".into()));
        }
        if let Err(reason) = (spec.enabled)(self) {
            let names_targets = ["track", "tracks", "clip", "clips", "sources", "start", "end", "at"].iter().any(|k| params.get(k).is_some());
            if !names_targets {
                return Err(EngineError::Disabled(id.to_string(), reason));
            }
        }
        if spec.view {
            return self.execute_view(spec, params);
        }
        let before = Arc::clone(&self.doc);
        let rev = self.revision;
        let dirty = self.dirty;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, params)));
        let result = match result {
            Ok(r) => r,
            Err(p) => {
                self.doc = before;
                self.revision = rev.wrapping_add(1);
                self.dirty = dirty;
                return Err(EngineError::Internal(id.to_string(), panic_message(&*p)));
            }
        };
        if !Arc::ptr_eq(&before, &self.doc) {
            self.merge_key = None;
        }
        match result {
            Ok(v) => {
                cmd::remember_selection(self, &before);
                if spec.undoable && !Arc::ptr_eq(&before, &self.doc) {
                    self.push_undo(spec.label, before);
                }
                if spec.journal {
                    self.journal.push((id.to_string(), params.clone()));
                }
                Ok(v)
            }
            Err(e) => {
                if !Arc::ptr_eq(&before, &self.doc) {
                    self.doc = before;
                    self.revision = self.revision.wrapping_add(1);
                    self.dirty = dirty;
                }
                Err(e)
            }
        }
    }

    fn execute_view(&mut self, spec: &CommandSpec, params: &Value) -> Result<Value> {
        let zoom = self.doc.edit.zoom;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (spec.run)(self, params)))
            .unwrap_or_else(|p| Err(EngineError::Internal(spec.id.to_string(), panic_message(&*p))));
        if result.is_err() && self.doc.edit.zoom != zoom {
            Arc::make_mut(&mut self.doc).edit.zoom = zoom;
        }
        result
    }

    pub fn execute_merged(&mut self, id: &str, params: &Value, key: &str) -> Result<Value> {
        let key = self.merge_hold.clone().unwrap_or_else(|| key.to_string());
        let continuing = self.merge_key.as_ref() == Some(&key);
        let before_len = self.undo.len();
        let r = self.execute(id, params)?;
        if continuing && self.undo.len() == before_len + 1 && before_len > 0 {
            self.undo.pop();
        }
        self.merge_key = Some(key);
        Ok(r)
    }

    pub fn hold_merge(&mut self, key: Option<&str>) {
        if self.merge_hold.as_deref() != key {
            self.merge_hold = key.map(str::to_string);
        }
    }

    pub fn end_merge(&mut self) {
        self.merge_key = None;
    }

    pub fn run(&mut self, id: &str) -> Result<Value> {
        self.execute(id, &json!({}))
    }

    pub fn message(&mut self, m: impl Into<String>) {
        let m = m.into();
        log::info!("{m}");
        self.messages.push(m);
        if self.messages.len() > 100 {
            self.messages.remove(0);
        }
    }

    pub fn target_tracks(&self) -> Vec<TrackId> {
        let s = self.session();
        if s.edit.selected_tracks.is_empty() {
            Vec::new()
        } else {
            s.tracks.iter().filter(|t| s.edit.selected_tracks.contains(&t.id)).map(|t| t.id).collect()
        }
    }
}

fn same_document(saved: &Session, doc: &Session) -> bool {
    let mut doc = doc.clone();
    let (from, to) = (&saved.edit, &mut doc.edit);
    to.selection = from.selection;
    to.timeline_selection = from.timeline_selection;
    to.selected_tracks.clone_from(&from.selected_tracks);
    to.selected_clips.clone_from(&from.selected_clips);
    to.playhead = from.playhead;
    to.zoom = from.zoom;
    *saved == doc
}

fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture());
            prev(info);
        }));
    });
}

fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    #[test]
    fn execute_each_is_one_undo_step() {
        let mut e = crate::demo::demo_engine();
        let ids: Vec<u64> = e.session().tracks.iter().take(3).map(|t| t.id.0).collect();
        let params: Vec<Value> = ids.iter().map(|t| json!({"track": t, "output": "Bus 9"})).collect();
        let undo_before = e.undo.len();
        assert!(e.execute_each("track.output", &params).iter().all(Result::is_ok));
        assert_eq!(e.undo.len(), undo_before + 1);
        e.execute("edit.undo", &json!({})).unwrap();
        let bus9 = e.session().busses.iter().find(|b| b.name == "Bus 9").map(|b| b.id);
        assert!(bus9.is_none() || e.session().tracks.iter().all(|t| t.mixer.output != openpt_model::Route::Bus(bus9.unwrap())));
    }

    #[test]
    fn merged_gesture_is_one_undo_step() {
        let mut e = Engine::default();
        let t = e.session_mut().add_track(openpt_model::TrackKind::Audio, openpt_model::ChannelFormat::Mono, None);
        for db in [-1.0, -2.0, -3.0, -4.0] {
            e.execute_merged("mix.volume", &json!({"track": t.0, "db": db}), "fader").unwrap();
        }
        e.end_merge();
        assert_eq!(e.undo_history().len(), 1);
        e.undo();
        assert_eq!(e.session().track(t).unwrap().mixer.volume_db, 0.0);
    }

    #[test]
    fn a_held_merge_joins_other_gestures_into_its_step() {
        let mut e = Engine::default();
        let t = e.session_mut().add_track(openpt_model::TrackKind::Audio, openpt_model::ChannelFormat::Mono, None);
        e.hold_merge(Some("pass"));
        for db in [-1.0, -2.0, -3.0] {
            e.execute_merged("mix.volume", &json!({"track": t.0, "db": db}), "fader").unwrap();
            e.execute_merged("mix.pan", &json!({"track": t.0, "pan": db / 4.0}), "pan").unwrap();
        }
        assert_eq!(e.undo_history().len(), 1);
        e.hold_merge(None);
        e.execute_merged("mix.volume", &json!({"track": t.0, "db": -6.0}), "fader").unwrap();
        assert_eq!(e.undo_history().len(), 2);
    }

    #[test]
    fn only_a_change_to_the_document_ends_a_gesture() {
        let mut e = Engine::default();
        let t = e.session_mut().add_track(openpt_model::TrackKind::Audio, openpt_model::ChannelFormat::Mono, None);
        e.execute_merged("mix.volume", &json!({"track": t.0, "db": -1.0}), "fader").unwrap();
        e.execute("transport.stop", &json!({})).unwrap();
        e.execute_merged("mix.volume", &json!({"track": t.0, "db": -2.0}), "fader").unwrap();
        assert_eq!(e.undo_history().len(), 1);
        e.execute("mix.pan", &json!({"tracks": [t.0], "pan": 0.5})).unwrap();
        e.execute_merged("mix.volume", &json!({"track": t.0, "db": -3.0}), "fader").unwrap();
        assert_eq!(e.undo_history().len(), 3);
    }
}

#[cfg(test)]
mod dirty_tests {
    use super::*;

    #[test]
    fn undo_leaves_solo_mute_and_track_views_alone() {
        let mut e = crate::demo::demo_engine();
        e.execute("mix.pan", &json!({"tracks": ["Kick"], "pan": 0.5})).unwrap();
        e.execute("mix.mute", &json!({"tracks": ["Kick"]})).unwrap();
        e.execute("mix.solo", &json!({"tracks": ["Snare"]})).unwrap();
        e.execute("track.height", &json!({"tracks": ["Kick"], "height": "large"})).unwrap();
        assert!(e.undo());
        let s = e.session();
        let (kick, snare) = (s.track_by_name("Kick").unwrap(), s.track_by_name("Snare").unwrap());
        assert!(kick.mixer.pan.first().is_some_and(|p| p.abs() < 1e-6), "the pan is undone");
        assert!(kick.mixer.mute && snare.mixer.solo, "listening stays");
        assert_eq!(kick.height, openpt_model::TrackHeight::Large);
    }

    #[test]
    fn selecting_locating_and_zooming_leave_the_session_clean() {
        let mut e = crate::demo::demo_engine();
        for (id, p) in [
            ("edit.select", json!({"tracks": ["Kick"], "start": 0, "end": 48_000})),
            ("edit.select_none", json!({})),
            ("transport.locate", json!({"at": 96_000})),
            ("view.zoom_in", json!({})),
            ("view.scroll", json!({"to": 48_000})),
        ] {
            e.execute(id, &p).unwrap();
            assert!(!e.is_dirty(), "{id}");
        }
        e.execute("mix.pan", &json!({"tracks": ["Kick"], "pan": 0.5})).unwrap();
        assert!(e.is_dirty());
        e.undo();
        assert!(!e.is_dirty());
    }
}
