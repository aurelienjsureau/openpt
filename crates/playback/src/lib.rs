#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod devices;
pub mod midi;
pub mod record;
pub mod surface;

pub use devices::AudioConfig;
pub use midi::{LiveMidiEvent, MidiDeviceInfo, MidiInputRing, MidiManager};

use openpt_dsp::{Plugin, PluginEditor, PluginStateReader};
use openpt_mix::{MAX_CHANNELS, MixEngine, PreparedInstance, StripMeter, itu_stereo_fold, main_channels};
use openpt_model::{Session, TrackId};
use openpt_time::{Range, Samples};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError, channel, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};

const BLOCK: usize = 512;

const OLD_SESSIONS: usize = 16;

pub use openpt_mix::{mark_audio_thread, on_audio_thread};

#[derive(Debug, Clone, Default)]
pub struct MeterSnapshot {
    pub tracks: HashMap<TrackId, StripMeter>,
    pub main: StripMeter,
}

pub type PluginStates = Vec<(TrackId, usize, Vec<u8>)>;

pub type EditorEdit = (TrackId, usize, String, f32);

#[derive(Default)]
pub struct CaptureRing {
    data: Mutex<Vec<Vec<f32>>>,
}

impl CaptureRing {
    pub fn with_capacity(channels: usize, frames: usize) -> Arc<CaptureRing> {
        Arc::new(CaptureRing { data: Mutex::new((0..channels).map(|_| Vec::with_capacity(frames)).collect()) })
    }

    pub fn frames(&self) -> usize {
        self.data.lock().unwrap_or_else(PoisonError::into_inner).first().map_or(0, Vec::len)
    }

    fn push(&self, block: &[Vec<f32>], n: usize) {
        let mut d = self.data.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        d.resize_with(block.len(), Vec::new);
        for (out, ch) in d.iter_mut().zip(block) {
            out.extend_from_slice(&ch[..n.min(ch.len())]);
        }
    }

    pub fn take(&self) -> Vec<Vec<f32>> {
        std::mem::take(&mut *self.data.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }
}

enum Cmd {
    Session(Arc<Session>, Vec<PreparedInstance>),
    OldSessions(SyncSender<Arc<Session>>),
    Input(Arc<record::InputRing>),
    Capture(Option<Arc<CaptureRing>>),
    Lend { placeholder: Box<MixEngine>, reply: SyncSender<Box<MixEngine>> },
    Return(Box<MixEngine>),
    MidiInput(Arc<midi::MidiInputRing>),
    MidiRecord(Arc<midi::MidiRecordRing>),
    Recording(bool),
    Play {
        from: Samples,
        end: Option<Samples>,
        looped: Option<Range>,
    },
    Stop,
    Locate(Samples),
}

struct Shared {
    position: AtomicI64,
    playing: AtomicBool,
    speed: AtomicU32,
    meters: Mutex<MeterSnapshot>,
    late: Mutex<LateBlocks>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LateBlocks {
    pub blocks: u32,
    pub late: u32,
    pub close: u32,
    pub worst_us: u32,
    pub budget_us: u32,
    pub session_us: u32,
    pub slowest: Option<(TrackId, usize, u32, &'static str)>,
}

impl LateBlocks {
    fn add(&mut self, o: &LateBlocks) {
        self.blocks += o.blocks;
        self.late += o.late;
        self.close += o.close;
        if o.worst_us > self.worst_us {
            (self.worst_us, self.budget_us, self.session_us, self.slowest) = (o.worst_us, o.budget_us, o.session_us, o.slowest);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTrouble {
    Bug,
    Gone,
    Stalled,
}

#[derive(Default)]
struct Fault {
    failed: AtomicBool,
    gone: AtomicBool,
    beats: std::sync::atomic::AtomicU64,
    reported: AtomicBool,
}

impl Fault {
    fn trouble(&self, stalled: bool) -> Option<OutputTrouble> {
        let trouble = if self.failed.load(Ordering::Relaxed) {
            OutputTrouble::Bug
        } else if self.gone.load(Ordering::Relaxed) {
            OutputTrouble::Gone
        } else if stalled {
            OutputTrouble::Stalled
        } else {
            return None;
        };
        (!self.reported.swap(true, Ordering::Relaxed)).then_some(trouble)
    }
}

#[cfg(not(target_arch = "wasm32"))]
const STALL: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(not(target_arch = "wasm32"))]
fn stalled(seen: &mut Option<(u64, std::time::Instant)>, beats: u64, playing: bool) -> bool {
    match *seen {
        Some((b, since)) if playing && b == beats => since.elapsed() >= STALL,
        _ => {
            *seen = Some((beats, std::time::Instant::now()));
            false
        }
    }
}

fn guarded_block(fault: &Fault, shared: &Shared, out: &mut [f32], fill: impl FnOnce(&mut [f32])) {
    fault.beats.fetch_add(1, Ordering::Relaxed);
    if fault.failed.load(Ordering::Relaxed) || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fill(out))).is_err() {
        fault.failed.store(true, Ordering::Relaxed);
        shared.playing.store(false, Ordering::Relaxed);
        out.fill(0.0);
    }
}

struct AudioState {
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    session: Arc<Session>,
    mix: MixEngine,
    pos: Samples,
    playing: bool,
    end: Option<Samples>,
    looped: Option<Range>,
    pending: Vec<Vec<f32>>,
    fold_channels: usize,
    fold: [(f32, f32); MAX_CHANNELS],
    pending_len: usize,
    read: f64,
    device_rate: f64,
    input: Option<Arc<record::InputRing>>,
    capture: Option<Arc<CaptureRing>>,
    lent: bool,
    midi_input: Option<Arc<midi::MidiInputRing>>,
    midi_record: Option<Arc<midi::MidiRecordRing>>,
    recording: bool,
    stream_start_us: u64,
    rendered_samples: u64,
    drain_us: u32,
    garbage: Sender<Vec<Box<dyn Plugin>>>,
    old_sessions: Option<SyncSender<Arc<Session>>>,
    latency_probe: bool,
}

fn new_engine(sr: f32) -> MixEngine {
    let mut m = MixEngine::new(sr, BLOCK);
    m.set_external_instances(true);
    m.metronome = true;
    m
}

impl AudioState {
    fn new(rx: Receiver<Cmd>, shared: Arc<Shared>, session: Arc<Session>, device_rate: f64, garbage: Sender<Vec<Box<dyn Plugin>>>) -> Self {
        let sr = session.sample_rate.as_f64() as f32;
        let nch = openpt_mix::output_channels(&session);
        let fold_channels = main_channels(&session);
        AudioState {
            garbage,
            old_sessions: None,
            latency_probe: true,
            fold: fold_gains(&session),
            rx,
            shared,
            mix: new_engine(sr),
            session,
            pos: 0,
            playing: false,
            end: None,
            looped: None,
            pending: vec![vec![0.0; BLOCK]; nch],
            fold_channels,
            pending_len: 0,
            read: 0.0,
            device_rate,
            input: None,
            capture: None,
            lent: false,
            midi_input: None,
            midi_record: None,
            recording: false,
            stream_start_us: 0,
            rendered_samples: 0,
            drain_us: 0,
        }
    }

    fn drain(&mut self) {
        let mut changed = false;
        while let Ok(c) = self.rx.try_recv() {
            match c {
                Cmd::Input(r) => self.input = Some(r),
                Cmd::Capture(c) => self.capture = c,
                Cmd::Lend { placeholder, reply } if !self.lent => {
                    let eng = Box::new(std::mem::replace(&mut self.mix, *placeholder));
                    match reply.try_send(eng) {
                        Ok(()) => {
                            self.lent = true;
                            self.playing = false;
                        }
                        Err(TrySendError::Full(eng) | TrySendError::Disconnected(eng)) => self.mix = *eng,
                    }
                }
                Cmd::Lend { .. } => {}
                Cmd::Return(eng) => {
                    drop(std::mem::replace(&mut self.mix, *eng));
                    self.lent = false;
                    self.latency_probe = true;
                }
                Cmd::MidiInput(r) => self.midi_input = Some(r),
                Cmd::MidiRecord(r) => self.midi_record = Some(r),
                Cmd::Recording(r) => self.recording = r,
                Cmd::Session(s, instances) => {
                    changed = true;
                    self.latency_probe = true;
                    if (s.sample_rate.as_f64() - self.session.sample_rate.as_f64()).abs() > 0.5 {
                        let mut old = std::mem::replace(&mut self.mix, new_engine(s.sample_rate.as_f64() as f32));
                        let _ = self.garbage.send(old.drain_plugins());
                    }
                    self.mix.adopt(instances);
                    let nch = openpt_mix::output_channels(&s);
                    if self.pending.len() != nch {
                        self.pending = vec![vec![0.0; BLOCK]; nch];
                        self.pending_len = 0;
                        self.read = 0.0;
                    }
                    self.fold = fold_gains(&s);
                    self.fold_channels = main_channels(&s);
                    let old = std::mem::replace(&mut self.session, s);
                    if let Some(tx) = &self.old_sessions {
                        let _ = tx.try_send(old);
                    }
                }
                Cmd::OldSessions(tx) => self.old_sessions = Some(tx),
                Cmd::Play { from, end, looped } => {
                    self.pos = from;
                    self.end = end;
                    self.looped = looped.filter(|r| r.len() > 64);
                    self.playing = true;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.stream_start_us = 0;
                    self.rendered_samples = 0;
                    self.mix.reset();
                }
                Cmd::Stop => {
                    self.latency_probe = true;
                    self.playing = false;
                    self.end = None;
                    self.looped = None;
                    self.stream_start_us = 0;
                    self.rendered_samples = 0;
                    self.mix.reset();
                }
                Cmd::Locate(at) => {
                    self.pos = at;
                    self.pending_len = 0;
                    self.read = 0.0;
                    self.stream_start_us = 0;
                    self.rendered_samples = 0;
                    self.mix.reset();
                }
            }
        }
        if changed {
            self.mix.ensure_synced(&self.session);
        }
        self.send_garbage();
        self.shared.playing.store(self.playing, Ordering::Relaxed);
    }

    fn send_garbage(&mut self) {
        let g = self.mix.take_retired();
        if !g.is_empty() {
            let _ = self.garbage.send(g);
        }
    }

    fn render_next(&mut self, quantum: usize) {
        let mut n = quantum.clamp(1, BLOCK);
        if let Some(l) = self.looped
            && self.pos >= l.end
        {
            self.pos = l.start;
        }
        if let Some(l) = self.looped {
            n = n.min(usize::try_from((l.end - self.pos).max(1)).unwrap_or(BLOCK));
        } else if let Some(e) = self.end {
            if self.pos >= e {
                self.playing = false;
                self.end = None;
                self.shared.playing.store(false, Ordering::Relaxed);
                n = 0;
            } else {
                n = n.min(usize::try_from(e - self.pos).unwrap_or(BLOCK));
            }
        }
        if n == 0 {
            for c in &mut self.pending {
                c.iter_mut().for_each(|x| *x = 0.0);
            }
            self.pending_len = BLOCK;
            return;
        }
        if let Some(ring) = &self.input {
            let ch = ring.channels.load(Ordering::Relaxed).max(1);
            if self.mix.input.len() != ch {
                self.mix.input = vec![vec![0.0; BLOCK]; ch];
            }
            ring.pop_into(&mut self.mix.input, n, self.session.sample_rate.hz());
        }
        self.mix.midi_input.clear();
        if let Some(ring) = &self.midi_input {
            let sr = self.session.sample_rate.as_f64().max(1.0);
            if self.stream_start_us == 0 {
                self.stream_start_us = midi::monotonic_now_us();
                self.rendered_samples = 0;
            }
            let block_start_us = self.stream_start_us.saturating_add((self.rendered_samples as f64 * 1_000_000.0 / sr) as u64);
            self.rendered_samples = self.rendered_samples.saturating_add(n as u64);

            while let Some(ev) = ring.pop() {
                let sample_offset = if ev.timestamp_us > 0 && ev.timestamp_us >= block_start_us {
                    let elapsed_us = ev.timestamp_us.saturating_sub(block_start_us);
                    ((elapsed_us as f64 * sr / 1_000_000.0) as usize).min(n.saturating_sub(1))
                } else {
                    0
                };
                if self.mix.midi_input.len() < self.mix.midi_input.capacity() {
                    self.mix.midi_input.push((sample_offset, ev));
                }
                let is_panic_cc = ev.is_control_change() && (ev.cc_controller() == 120 || ev.cc_controller() == 123);
                if !is_panic_cc
                    && self.recording
                    && self.playing
                    && let Some(rec_ring) = &self.midi_record
                {
                    let at_sample = self.pos.saturating_add(sample_offset as i64);
                    for t in &self.session.tracks {
                        if t.kind.is_midi() && t.mixer.record_arm && !t.inactive && t.midi_from.is_none() && t.matches_midi_channel(ev.channel()) {
                            rec_ring.push(midi::RecordedMidiEvent { track_id: t.id.0, at_sample, event: ev });
                        }
                    }
                }
            }
        }
        self.mix.monitor_only = !self.playing;
        self.mix.recording = self.recording;
        self.mix.render(&self.session, self.pos, n, &mut self.pending);
        if self.playing
            && let Some(c) = &self.capture
        {
            c.push(&self.pending, n);
        }
        if self.recording
            && self.playing
            && let Some(rec_ring) = &self.midi_record
        {
            for &(t, off, m) in &self.mix.midi_routed {
                if self.session.track(t).is_some_and(|tr| tr.mixer.record_arm) {
                    let len = if matches!(m[0] & 0xF0, 0xC0 | 0xD0) { 2 } else { 3 };
                    let event = LiveMidiEvent { port_id: 0, timestamp_us: 0, len, data: m };
                    rec_ring.push(midi::RecordedMidiEvent { track_id: t.0, at_sample: self.pos.saturating_add(off as i64), event });
                }
            }
        }
        self.send_garbage();
        self.pending_len = n;
        if let Ok(mut m) = self.shared.meters.try_lock() {
            m.main.copy_from(&self.mix.main_meter);
            for (k, v) in &self.mix.meters {
                match m.tracks.get_mut(k) {
                    Some(d) => d.copy_from(v),
                    None => {
                        m.tracks.insert(*k, v.clone());
                    }
                }
            }
        }
        if self.playing {
            self.pos += n as i64;
            self.shared.position.store(self.pos, Ordering::Relaxed);
        }
    }

    fn fill(&mut self, out: &mut [f32], channels: usize) {
        let t0 = std::time::Instant::now();
        self.drain();
        self.drain_us = u32::try_from(t0.elapsed().as_micros()).unwrap_or(u32::MAX);
        if self.lent {
            out.iter_mut().for_each(|x| *x = 0.0);
            return;
        }
        let channels = channels.max(1);
        let audio_monitoring = self.input.is_some() && self.session.tracks.iter().any(|t| t.mixer.input_monitor || t.mixer.record_arm);
        let midi_monitoring = self.midi_input.is_some()
            && self.session.tracks.iter().any(|t| {
                t.kind.is_midi()
                    && (t.mixer.input_monitor
                        || t.mixer.record_arm
                        || (self.session.edit.flag("options.midi_thru") && self.session.edit.selected_tracks.contains(&t.id)))
            });
        let midi_has_events = self.midi_input.as_ref().is_some_and(|r| !r.is_empty());
        let monitoring = audio_monitoring || midi_monitoring || midi_has_events;
        if !self.playing && !monitoring {
            out.iter_mut().for_each(|x| *x = 0.0);
            if self.latency_probe {
                self.latency_probe = false;
                let n = 64.min(BLOCK);
                self.mix.render(&self.session, self.pos, n, &mut self.pending);
                self.pending_len = 0;
                self.read = 0.0;
                self.send_garbage();
                if let Ok(mut m) = self.shared.meters.try_lock() {
                    for (k, v) in &self.mix.meters {
                        match m.tracks.get_mut(k) {
                            Some(d) => d.copy_from(v),
                            None => {
                                m.tracks.insert(*k, v.clone());
                            }
                        }
                    }
                }
            }
            if let Ok(mut m) = self.shared.meters.try_lock() {
                m.main.clear_levels();
                for v in m.tracks.values_mut() {
                    v.clear_levels();
                }
            }
            return;
        }
        let speed = f64::from(self.shared.speed.load(Ordering::Relaxed)) / 1000.0;
        let step = self.session.sample_rate.as_f64() / self.device_rate.max(1.0) * speed.clamp(0.0625, 4.0);
        let callback_frames = out.len() / channels;
        let quantum = ((callback_frames as f64 * step).ceil() as usize).clamp(1, BLOCK);
        for frame in out.chunks_mut(channels) {
            while self.read >= self.pending_len as f64 {
                self.read -= self.pending_len as f64;
                if !self.playing && !monitoring {
                    break;
                }
                self.render_next(quantum);
                if self.pending_len == 0 {
                    break;
                }
            }
            let i0 = self.read.floor() as usize;
            let fr = (self.read - i0 as f64) as f32;
            let sounding = self.playing || monitoring;
            let nch = self.pending.len();
            let main_nch = self.fold_channels;
            let sample = |c: usize| {
                let ch = self.pending.get(c);
                let a = ch.and_then(|v| v.get(i0)).copied().unwrap_or(0.0);
                let b = ch.and_then(|v| v.get(i0 + 1)).copied().unwrap_or(a);
                a + (b - a) * fr
            };
            if main_nch > 2 && channels < main_nch {
                let (mut l, mut r) = (0.0f32, 0.0f32);
                if sounding {
                    for (c, (gl, gr)) in self.fold.iter().enumerate().take(main_nch) {
                        let v = sample(c);
                        l += v * gl;
                        r += v * gr;
                    }
                }
                for (c, o) in frame.iter_mut().enumerate() {
                    *o = match c {
                        0 => l.clamp(-1.0, 1.0),
                        1 => r.clamp(-1.0, 1.0),
                        _ => 0.0,
                    };
                }
            } else {
                for (c, o) in frame.iter_mut().enumerate() {
                    *o = if c < nch.max(1) && sounding { sample(c).clamp(-1.0, 1.0) } else { 0.0 };
                }
            }
            self.read += step;
        }
    }
}

struct Side {
    shadow: HashMap<(TrackId, usize), (String, usize, bool)>,
    rate: f32,
    editors: HashMap<(TrackId, usize), Box<dyn PluginEditor>>,
    states: BTreeMap<(TrackId, usize), Box<dyn PluginStateReader>>,
    refused: HashSet<(TrackId, usize)>,
    garbage: Receiver<Vec<Box<dyn Plugin>>>,
    old_sessions: Option<Receiver<Arc<Session>>>,
    ara_seen: HashMap<u64, u64>,
}

impl Side {
    fn new(rate: f32, garbage: Receiver<Vec<Box<dyn Plugin>>>) -> Side {
        Side {
            shadow: HashMap::new(),
            rate,
            editors: HashMap::new(),
            states: BTreeMap::new(),
            refused: HashSet::new(),
            garbage,
            old_sessions: None,
            ara_seen: HashMap::new(),
        }
    }

    fn collect_garbage(&mut self) {
        while let Ok(g) = self.garbage.try_recv() {
            for p in g {
                log::debug!("plugin instance released: {}", p.info().name);
                drop(p);
            }
        }
        if let Some(rx) = &self.old_sessions {
            rx.try_iter().for_each(drop);
        }
    }

    fn return_old_sessions(&mut self, tx: &Sender<Cmd>) {
        let (otx, orx) = sync_channel(OLD_SESSIONS);
        self.old_sessions = Some(orx);
        let _ = tx.send(Cmd::OldSessions(otx));
    }

    fn forget(&mut self, key: (TrackId, usize)) {
        self.shadow.remove(&key);
        self.states.remove(&key);
        self.refused.remove(&key);
        if let Some(mut e) = self.editors.remove(&key) {
            e.close();
        }
    }
}

pub struct Player {
    shared: Arc<Shared>,
    fault: Arc<Fault>,
    #[cfg(not(target_arch = "wasm32"))]
    watch: Mutex<Option<(u64, std::time::Instant)>>,
    tx: Sender<Cmd>,
    side: Mutex<Side>,
    #[cfg(not(target_arch = "wasm32"))]
    _stream: Option<cpal::Stream>,
    #[cfg(target_arch = "wasm32")]
    _stream: Option<cpal::Stream>,
    pub device_name: String,
    pub device_rate: u32,
    pub device_channels: u16,
    pub device_buffer: Option<u32>,
    pub silent: bool,
    pub midi_input: Arc<midi::MidiInputRing>,
    pub midi_record: Arc<midi::MidiRecordRing>,
}

impl Player {
    pub fn new(session: Arc<Session>) -> Player {
        Player::with_config(session, &AudioConfig::default())
    }

    pub fn with_config(session: Arc<Session>, cfg: &AudioConfig) -> Player {
        let p = Player::open(Arc::clone(&session), cfg);
        p.side().return_old_sessions(&p.tx);
        p.update_session(session);
        p
    }

    pub fn trouble(&self) -> Option<OutputTrouble> {
        #[cfg(not(target_arch = "wasm32"))]
        let stalled = {
            let playing = self.shared.playing.load(Ordering::Relaxed) && !self.silent;
            stalled(&mut self.watch.lock().unwrap_or_else(PoisonError::into_inner), self.fault.beats.load(Ordering::Relaxed), playing)
        };
        #[cfg(target_arch = "wasm32")]
        let stalled = false;
        self.fault.trouble(stalled)
    }

    fn side(&self) -> std::sync::MutexGuard<'_, Side> {
        self.side.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(session: Arc<Session>, cfg: &AudioConfig) -> Player {
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let want = session.sample_rate.hz();
        let want_ch: u16 = 64;
        let side_rate = session.sample_rate.as_f64() as f32;
        let new_side = |garbage: Receiver<Vec<Box<dyn Plugin>>>| Mutex::new(Side::new(side_rate, garbage));
        let midi_input = midi::MidiInputRing::new(midi::DEFAULT_RING_CAPACITY);
        let midi_record = midi::MidiRecordRing::new(midi::DEFAULT_RING_CAPACITY);
        match open_device(want, want_ch, cfg) {
            Ok((device, config, format, name)) => {
                let mut attempts = vec![config.clone()];
                if config.buffer_size != cpal::BufferSize::Default {
                    attempts.push(cpal::StreamConfig { buffer_size: cpal::BufferSize::Default, ..config });
                }
                for config in attempts {
                    let rate = config.sample_rate.0;
                    let channels = usize::from(config.channels);
                    let (tx, rx) = channel();
                    let (gtx, grx) = channel();
                    let state = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&session), f64::from(rate), gtx);
                    let _ = tx.send(Cmd::MidiInput(Arc::clone(&midi_input)));
                    let _ = tx.send(Cmd::MidiRecord(Arc::clone(&midi_record)));
                    let fault = Arc::new(Fault::default());
                    use cpal::traits::StreamTrait;
                    let built = match format {
                        cpal::SampleFormat::I32 => build_output::<i32>(&device, &config, state, channels, &fault, &shared),
                        cpal::SampleFormat::I24 => build_output::<cpal::I24>(&device, &config, state, channels, &fault, &shared),
                        cpal::SampleFormat::F64 => build_output::<f64>(&device, &config, state, channels, &fault, &shared),
                        cpal::SampleFormat::I16 => build_output::<i16>(&device, &config, state, channels, &fault, &shared),
                        _ => build_output::<f32>(&device, &config, state, channels, &fault, &shared),
                    };
                    match built {
                        Ok(stream) => {
                            if let Err(e) = stream.play() {
                                log::warn!("audio stream failed to start: {e}");
                            }
                            let buffer = match config.buffer_size {
                                cpal::BufferSize::Fixed(n) => Some(n),
                                cpal::BufferSize::Default => None,
                            };
                            log::info!(
                                "audio device: {name} @ {rate} Hz, {channels} ch, {format}, buffer {}",
                                buffer.map_or("default".to_string(), |n| n.to_string())
                            );
                            return Player {
                                shared,
                                fault,
                                #[cfg(not(target_arch = "wasm32"))]
                                watch: Mutex::new(None),
                                tx,
                                side: new_side(grx),
                                _stream: Some(stream),
                                device_name: name,
                                device_rate: rate,
                                device_channels: config.channels,
                                device_buffer: buffer,
                                silent: false,
                                midi_input,
                                midi_record,
                            };
                        }
                        Err(e) => log::warn!("cannot open audio stream ({:?}): {e}", config.buffer_size),
                    }
                }
                log::warn!("no usable audio stream; using a silent clock");
                let (tx, rx) = channel();
                let (gtx, grx) = channel();
                Player::silent_clock(shared, tx, rx, want, new_side(grx), gtx, midi_input, midi_record)
            }
            Err(e) => {
                log::warn!("no audio output: {e}; using a silent clock");
                let (tx, rx) = channel();
                let (gtx, grx) = channel();
                Player::silent_clock_with(shared, tx, rx, Arc::clone(&session), new_side(grx), gtx, midi_input, midi_record)
            }
        }
    }

    fn silent_clock(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        rate: u32,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
        midi_input: Arc<midi::MidiInputRing>,
        midi_record: Arc<midi::MidiRecordRing>,
    ) -> Player {
        let s = Arc::new(Session::new("Untitled", openpt_time::SampleRate::new(rate).unwrap_or_default()));
        Player::silent_clock_with(shared, tx, rx, s, side, gtx, midi_input, midi_record)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        gtx: Sender<Vec<Box<dyn Plugin>>>,
        midi_input: Arc<midi::MidiInputRing>,
        midi_record: Arc<midi::MidiRecordRing>,
    ) -> Player {
        let rate = session.sample_rate.hz();
        let _ = tx.send(Cmd::MidiInput(Arc::clone(&midi_input)));
        let _ = tx.send(Cmd::MidiRecord(Arc::clone(&midi_record)));
        let mut state = AudioState::new(rx, Arc::clone(&shared), session, f64::from(rate), gtx);
        let fault = Arc::new(Fault::default());
        let failed = Arc::clone(&fault);
        let spawn = std::thread::Builder::new().name("openpt-clock".into()).spawn(move || {
            let mut buf = vec![0.0f32; 4096];
            let mut last = std::time::Instant::now();
            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));
                let now = std::time::Instant::now();
                let frames = ((now - last).as_secs_f64() * state.device_rate) as usize;
                last = now;
                let n = (frames * 2).min(buf.len());
                if let Some(b) = buf.get_mut(..n) {
                    let shared = Arc::clone(&state.shared);
                    guarded_block(&failed, &shared, b, |b| state.fill(b, 2));
                }
                if Arc::strong_count(&state.shared) <= 1 {
                    break;
                }
            }
        });
        if let Err(e) = spawn {
            log::warn!("clock thread failed: {e}");
        }
        Player {
            shared,
            fault,
            watch: Mutex::new(None),
            tx,
            side,
            _stream: None,
            device_name: "No audio device (silent)".into(),
            device_rate: rate,
            device_channels: 0,
            device_buffer: None,
            silent: true,
            midi_input,
            midi_record,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn silent_clock_with(
        shared: Arc<Shared>,
        tx: Sender<Cmd>,
        _rx: Receiver<Cmd>,
        session: Arc<Session>,
        side: Mutex<Side>,
        _gtx: Sender<Vec<Box<dyn Plugin>>>,
        midi_input: Arc<midi::MidiInputRing>,
        midi_record: Arc<midi::MidiRecordRing>,
    ) -> Player {
        let _ = tx.send(Cmd::MidiInput(Arc::clone(&midi_input)));
        let _ = tx.send(Cmd::MidiRecord(Arc::clone(&midi_record)));
        Player {
            shared,
            fault: Arc::default(),
            tx,
            side,
            _stream: None,
            device_name: "No audio device".into(),
            device_rate: session.sample_rate.hz(),
            device_channels: 0,
            device_buffer: None,
            silent: true,
            midi_input,
            midi_record,
        }
    }

    pub fn update_session(&self, s: Arc<Session>) {
        let started = std::time::Instant::now();
        let instances = self.side().prepare(&s);
        let took = started.elapsed();
        if took.as_millis() >= 50 {
            log::warn!("slow session update: {} ms ({} new plugin instances)", took.as_millis(), instances.len());
        }
        let _ = self.tx.send(Cmd::Session(s, instances));
    }

    pub fn take_late_blocks(&self) -> Option<LateBlocks> {
        let mut l = self.shared.late.lock().ok()?;
        let got = std::mem::take(&mut *l);
        (got.late + got.close > 0).then_some(got)
    }

    pub fn capture_states(&self) -> PluginStates {
        self.side().capture_states()
    }

    pub fn capture_state(&self, track: TrackId, slot: usize) -> Option<Vec<u8>> {
        self.side().capture_state((track, slot))
    }

    pub fn idle(&self) -> Vec<EditorEdit> {
        let mut side = self.side();
        side.collect_garbage();
        let mut out = Vec::new();
        for (&(t, slot), e) in side.editors.iter_mut() {
            for (id, v) in e.idle() {
                out.push((t, slot, id, v));
            }
        }
        out
    }

    pub fn set_editor_header(&self, track: TrackId, slot: usize, header: &openpt_dsp::EditorHeader) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.set_header(header);
        }
    }

    pub fn header_actions(&self) -> Vec<(TrackId, usize, openpt_dsp::HeaderAction)> {
        let mut side = self.side();
        let mut out = Vec::new();
        for (&(t, slot), e) in side.editors.iter_mut() {
            out.extend(e.header_actions().into_iter().map(|a| (t, slot, a)));
        }
        out
    }

    pub fn plugin_state(&self, track: TrackId, slot: usize) -> Option<Vec<u8>> {
        self.side().states.get_mut(&(track, slot)).and_then(|r| r.save_state())
    }

    pub fn load_plugin_state(&self, track: TrackId, slot: usize, data: &[u8]) -> bool {
        self.side().editors.get_mut(&(track, slot)).is_some_and(|e| e.load_state(data))
    }

    pub fn editor_value_text(&self, track: TrackId, slot: usize, id: &str, value: f32) -> Option<String> {
        self.side().editors.get_mut(&(track, slot))?.value_text(id, value)
    }

    pub fn plugin_output_buses(&self, track: TrackId, slot: usize) -> Vec<(String, u32)> {
        self.side().editors.get_mut(&(track, slot)).map(|e| e.output_buses()).unwrap_or_default()
    }
    pub fn editor_params(&self, track: TrackId, slot: usize, ids: &[String]) -> Vec<(String, f32)> {
        let mut side = self.side();
        let Some(e) = side.editors.get_mut(&(track, slot)) else { return Vec::new() };
        ids.iter().filter_map(|id| Some((id.clone(), e.param_value(id)?))).collect()
    }

    pub fn has_editor(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.contains_key(&(track, slot))
    }

    pub fn open_editor(&self, track: TrackId, slot: usize) -> Result<(), String> {
        self.side().open_editor((track, slot))
    }

    pub fn close_editor(&self, track: TrackId, slot: usize) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.close();
        }
    }

    pub fn close_editors(&self) {
        for e in self.side().editors.values_mut() {
            e.close();
        }
    }

    pub fn editor_open(&self, track: TrackId, slot: usize) -> bool {
        self.side().editors.get(&(track, slot)).is_some_and(|e| e.is_open())
    }

    pub fn any_editor_open(&self) -> bool {
        self.side().editors.values().any(|e| e.is_open())
    }

    pub fn editor_set_param(&self, track: TrackId, slot: usize, id: &str, value: f32) {
        if let Some(e) = self.side().editors.get_mut(&(track, slot)) {
            e.set_param(id, value);
        }
    }

    pub fn play(&self, from: Samples, end: Option<Samples>, looped: Option<Range>) {
        self.shared.position.store(from, Ordering::Relaxed);
        self.shared.playing.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Play { from, end, looped });
    }

    pub fn set_capture(&self, ring: Option<Arc<CaptureRing>>) {
        let _ = self.tx.send(Cmd::Capture(ring));
    }

    pub fn lend_engine(&self) -> Option<Box<MixEngine>> {
        self.shared.playing.store(false, Ordering::Relaxed);
        let (reply, rx) = sync_channel(1);
        let placeholder = Box::new(new_engine(48_000.0));
        self.tx.send(Cmd::Lend { placeholder, reply }).ok()?;
        rx.recv_timeout(std::time::Duration::from_secs(2)).ok()
    }

    pub fn return_engine(&self, eng: Box<MixEngine>) {
        let _ = self.tx.send(Cmd::Return(eng));
    }

    pub fn set_input(&self, ring: Arc<record::InputRing>) {
        let _ = self.tx.send(Cmd::Input(ring));
    }

    pub fn set_midi_input(&self, ring: Arc<midi::MidiInputRing>) {
        let _ = self.tx.send(Cmd::MidiInput(ring));
    }

    pub fn midi_input_ring(&self) -> Arc<midi::MidiInputRing> {
        Arc::clone(&self.midi_input)
    }

    pub fn panic_all_notes_off(&self) {
        for ch in 0..16u8 {
            self.midi_input.push(openpt_midi::LiveMidiEvent::all_sound_off(ch));
            self.midi_input.push(openpt_midi::LiveMidiEvent::all_notes_off(ch));
        }
    }

    pub fn midi_record_ring(&self) -> Arc<midi::MidiRecordRing> {
        Arc::clone(&self.midi_record)
    }

    pub fn set_midi_record(&self, ring: Arc<midi::MidiRecordRing>) {
        let _ = self.tx.send(Cmd::MidiRecord(ring));
    }

    pub fn set_recording(&self, on: bool) {
        let _ = self.tx.send(Cmd::Recording(on));
    }

    pub fn stop(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Stop);
    }

    pub fn locate(&self, at: Samples) {
        self.shared.position.store(at, Ordering::Relaxed);
        let _ = self.tx.send(Cmd::Locate(at));
    }

    pub fn set_speed(&self, speed: f32) {
        let v = if speed.is_finite() { (speed.clamp(0.0625, 4.0) * 1000.0) as u32 } else { 1000 };
        self.shared.speed.store(v, Ordering::Relaxed);
    }

    pub fn speed(&self) -> f32 {
        self.shared.speed.load(Ordering::Relaxed) as f32 / 1000.0
    }

    pub fn position(&self) -> Samples {
        self.shared.position.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn meters(&self) -> MeterSnapshot {
        self.shared.meters.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    pub fn copy_meters(&self, out: &mut MeterSnapshot) {
        let source = self.shared.meters.lock().unwrap_or_else(PoisonError::into_inner);
        copy_meter_snapshot(&source, out);
    }
}

fn copy_meter_snapshot(source: &MeterSnapshot, out: &mut MeterSnapshot) {
    out.tracks.retain(|id, _| source.tracks.contains_key(id));
    for (id, meter) in &source.tracks {
        out.tracks.entry(*id).or_default().copy_from(meter);
    }
    out.main.copy_from(&source.main);
}

impl Side {
    fn capture_states(&mut self) -> PluginStates {
        self.states.iter_mut().filter(|(k, _)| !self.refused.contains(*k)).filter_map(|(&(t, slot), r)| Some((t, slot, r.save_state()?))).collect()
    }

    fn capture_state(&mut self, key: (TrackId, usize)) -> Option<Vec<u8>> {
        if self.refused.contains(&key) {
            return None;
        }
        self.states.get_mut(&key)?.save_state()
    }

    fn open_editor(&mut self, key: (TrackId, usize)) -> Result<(), String> {
        let e = self.editors.get_mut(&key).ok_or_else(|| "this plugin has no editor (or it is not loaded)".to_string())?;
        e.open()?;
        self.refused.remove(&key);
        Ok(())
    }

    fn prepare(&mut self, s: &Session) -> Vec<PreparedInstance> {
        self.collect_garbage();
        let sr = s.sample_rate.as_f64() as f32;
        if (sr - self.rate).abs() > 0.5 {
            let keys: Vec<(TrackId, usize)> = self.shadow.keys().copied().collect();
            for k in keys {
                self.forget(k);
            }
            self.rate = sr;
        }
        let specs = openpt_mix::instance_specs(s);
        let stale: Vec<(TrackId, usize)> = self.shadow.keys().filter(|k| !specs.iter().any(|x| (x.track, x.slot) == **k)).copied().collect();
        for k in stale {
            self.forget(k);
        }
        let mut out = Vec::new();
        for spec in specs {
            let key = (spec.track, spec.slot);
            if self.shadow.get(&key).is_some_and(|(id, ch, _)| *id == spec.id && *ch == spec.ch) {
                continue;
            }
            self.forget(key);
            let ara = if spec.id.starts_with("vst3:") { openpt_mix::ara_track(s, spec.track) } else { None };
            let made = openpt_mix::create_instance_on(&spec.id, spec.state.as_deref(), sr, BLOCK, spec.ch, ara.as_ref());
            log::debug!("plugin instance for {:?} slot {}: {} ({} ch, created: {})", spec.track, spec.slot, spec.id, spec.ch, made.is_some());
            self.shadow.insert(key, (spec.id.clone(), spec.ch, made.is_some()));
            if let Some(openpt_mix::NewInstance { mut plugin, refused_state }) = made {
                if let Some(e) = plugin.editor() {
                    self.editors.insert(key, e);
                }
                if let Some(r) = plugin.state_reader() {
                    self.states.insert(key, r);
                }
                if refused_state {
                    self.refused.insert(key);
                }
                let state_loaded = spec.state.is_some() && !refused_state;
                out.push(PreparedInstance { track: spec.track, slot: spec.slot, id: spec.id, ch: spec.ch, plugin, state_loaded });
            }
        }
        openpt_mix::ara_follow(s, &mut self.ara_seen);
        out
    }
}

fn fold_gains(s: &Session) -> [(f32, f32); MAX_CHANNELS] {
    let mut g = [(0.0, 0.0); MAX_CHANNELS];
    let f = s.main_format();
    let sp = f.speakers();
    if sp.len() == f.channels() {
        for (o, x) in g.iter_mut().zip(sp.iter()) {
            *o = itu_stereo_fold(*x);
        }
    } else {
        g[0] = (std::f32::consts::FRAC_1_SQRT_2, std::f32::consts::FRAC_1_SQRT_2);
    }
    g
}

fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut state: AudioState,
    channels: usize,
    fault: &Arc<Fault>,
    shared: &Arc<Shared>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let frame = channels.max(1);
    let mut scratch = vec![0.0f32; 8192 * frame];
    let chunk = scratch.len();
    let (failed, guard, gone) = (Arc::clone(fault), Arc::clone(shared), Arc::clone(fault));
    let rate = f64::from(config.sample_rate.0.max(1));
    let mut late = LateBlocks::default();
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            mark_audio_thread();
            let t0 = std::time::Instant::now();
            for out in data.chunks_mut(chunk) {
                let Some(s) = scratch.get_mut(..out.len()) else { return };
                guarded_block(&failed, &guard, s, |d| state.fill(d, channels));
                for (o, x) in out.iter_mut().zip(s.iter()) {
                    *o = T::from_sample(*x);
                }
            }
            let took = u32::try_from(t0.elapsed().as_micros()).unwrap_or(u32::MAX);
            let budget = ((data.len() / frame) as f64 * 1_000_000.0 / rate) as u32;
            let slowest = state.mix.take_slowest_insert();
            let mut this = LateBlocks { blocks: 1, ..LateBlocks::default() };
            if took > budget / 5 * 4 {
                this = LateBlocks {
                    blocks: 1,
                    late: u32::from(took > budget),
                    close: u32::from(took <= budget),
                    worst_us: took,
                    budget_us: budget,
                    session_us: state.drain_us,
                    slowest,
                };
            }
            late.add(&this);
            if let Ok(mut l) = guard.late.try_lock() {
                l.add(&late);
                late = LateBlocks::default();
            }
        },
        move |e: cpal::StreamError| {
            mark_audio_thread();
            if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                gone.gone.store(true, Ordering::Relaxed);
            }
            log::warn!("audio stream error: {e}");
        },
        None,
    )
}

fn open_device(want_rate: u32, want_ch: u16, cfg: &AudioConfig) -> Result<(cpal::Device, cpal::StreamConfig, cpal::SampleFormat, String), String> {
    use cpal::traits::DeviceTrait;
    let host = devices::host(cfg);
    let device = devices::device(host, cfg.output.as_deref(), true).ok_or_else(|| "no output device".to_string())?;
    #[allow(deprecated)]
    let name = device.name().unwrap_or_else(|_| "Audio device".into());
    let want_ch = want_ch.max(2);
    let rates: Vec<u32> = cfg.sample_rate.into_iter().chain(Some(want_rate)).collect();
    let mut chosen: Option<(cpal::StreamConfig, cpal::SupportedBufferSize, cpal::SampleFormat)> = None;
    let better = |c: &cpal::SupportedStreamConfigRange, x: &(cpal::StreamConfig, cpal::SupportedBufferSize, cpal::SampleFormat)| {
        let (rc, rx) = (devices::format_rank(c.sample_format()).unwrap_or(usize::MAX), devices::format_rank(x.2).unwrap_or(usize::MAX));
        rc < rx || (rc == rx && c.channels() > x.0.channels)
    };
    if let Ok(configs) = device.supported_output_configs() {
        let configs: Vec<cpal::SupportedStreamConfigRange> = configs.collect();
        for rate in rates {
            for c in &configs {
                if devices::format_rank(c.sample_format()).is_some()
                    && c.min_sample_rate().0 <= rate
                    && c.max_sample_rate().0 >= rate
                    && c.channels() >= 2
                    && c.channels() <= want_ch
                    && chosen.as_ref().is_none_or(|x| better(c, x))
                {
                    chosen = Some((c.with_sample_rate(cpal::SampleRate(rate)).config(), *c.buffer_size(), c.sample_format()));
                }
            }
            if chosen.is_some() {
                break;
            }
        }
    }
    let (config, format) = match chosen {
        Some((mut c, buffers, format)) => {
            c.buffer_size = devices::buffer_size(cfg.buffer_size, &buffers);
            (c, format)
        }
        None => {
            let d = device.default_output_config().map_err(|e| e.to_string())?;
            if devices::format_rank(d.sample_format()).is_none() {
                return Err(format!("unsupported device sample format {:?}", d.sample_format()));
            }
            let buffers = *d.buffer_size();
            let format = d.sample_format();
            let mut c = d.config();
            c.buffer_size = devices::buffer_size(cfg.buffer_size, &buffers);
            (c, format)
        }
    };
    Ok((device, config, format, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reusable_meter_snapshot_updates_and_drops_stale_tracks() {
        let keep = TrackId(1);
        let stale = TrackId(2);
        let mut source = MeterSnapshot::default();
        source.tracks.insert(keep, StripMeter { peak: [0.5, 0.25], peaks: vec![0.5, 0.25], gain_reduction_db: 3.0, ..StripMeter::default() });
        source.main.peaks = vec![0.75, 0.5];
        let mut out = MeterSnapshot::default();
        out.tracks.insert(stale, StripMeter::default());
        copy_meter_snapshot(&source, &mut out);
        assert_eq!(out.tracks.len(), 1);
        assert_eq!(out.tracks[&keep], source.tracks[&keep]);
        assert_eq!(out.main, source.main);
    }

    #[test]
    fn a_block_that_panics_goes_silent_and_is_reported_once() {
        let shared = Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(true),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        };
        let fault = Fault::default();
        let mut out = [0.5f32; 8];
        guarded_block(&fault, &shared, &mut out, |o| o.fill(0.25));
        assert_eq!((out, fault.trouble(false)), ([0.25; 8], None));
        guarded_block(&fault, &shared, &mut out, |_| std::panic::panic_any("a bug in the mix"));
        guarded_block(&fault, &shared, &mut out, |o| o.fill(0.25));
        assert_eq!(out, [0.0; 8]);
        assert!(!shared.playing.load(Ordering::Relaxed));
        assert_eq!((fault.trouble(false), fault.trouble(false)), (Some(OutputTrouble::Bug), None));
        assert_eq!(fault.beats.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn a_lost_or_stalled_device_is_reported_once() {
        let ago = |seen: &mut Option<(u64, std::time::Instant)>| {
            if let Some(s) = seen {
                s.1 = std::time::Instant::now().checked_sub(STALL).unwrap();
            }
        };
        let mut seen = None;
        assert!(!stalled(&mut seen, 5, true));
        ago(&mut seen);
        assert!(stalled(&mut seen, 5, true));
        assert!(!stalled(&mut seen, 6, true), "it asked again");
        ago(&mut seen);
        assert!(!stalled(&mut seen, 6, false), "stopped, so nothing to ask for");
        assert_eq!(Fault::default().trouble(true), Some(OutputTrouble::Stalled));
        let fault = Fault::default();
        fault.gone.store(true, Ordering::Relaxed);
        assert_eq!((fault.trouble(false), fault.trouble(true)), (Some(OutputTrouble::Gone), None));
    }

    #[test]
    fn audio_state_plays_and_stops_at_end() {
        let s = Arc::new(openpt_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(shared.position.load(Ordering::Relaxed) >= 1000);
        assert!(!shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn short_device_callbacks_render_short_mix_blocks() {
        let s = Arc::new(openpt_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        let mut out = vec![0.0f32; 2 * 64];
        st.fill(&mut out, 2);
        assert_eq!(shared.position.load(Ordering::Relaxed), 64);
    }

    #[test]
    fn surround_main_maps_to_device_channels_or_folds_down() {
        use openpt_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};
        let mut s = openpt_model::Session::default();
        s.outputs[0].format = ChannelFormat::Surround51;
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Surround51, None);
        let chans: Vec<Vec<f32>> = (0..6).map(|c| vec![0.1 * (c + 1) as f32; 48_000]).collect();
        s.pool.insert(SourceId(5), Arc::new(SourceAudio::new(openpt_audio_io::AudioBuffer { sample_rate: 48_000, channels: chans })));
        let id = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "x", SourceId(5), 0, 0, 48_000));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::clone(&s), 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
        let mut out = vec![0.0f32; 8 * 256];
        st.fill(&mut out, 8);
        let f = &out[8 * 100..8 * 101];
        for c in 0..6 {
            assert!((f[c] - 0.1 * (c + 1) as f32).abs() < 1e-5, "{f:?}");
        }
        assert!(f[6] == 0.0 && f[7] == 0.0);
        let mut out = vec![0.0f32; 2 * 256];
        st.fill(&mut out, 2);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let l = (0.1 + h * 0.3 + h * 0.5f32).min(1.0);
        assert!((out[200] - l).abs() < 1e-4, "{} vs {l}", out[200]);
        assert_eq!(shared.meters.lock().unwrap().main.peaks.len(), 6);
    }

    #[test]
    fn instances_arrive_with_the_session_and_leave_through_the_return_channel() {
        use openpt_model::{ChannelFormat, Insert, TrackKind};
        let mut s = openpt_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("vst3:test.fake"));
        let s = Arc::new(s);
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(openpt_model::Session::default()), 48_000.0, gtx);
        let plugin = openpt_dsp::create("gain").unwrap();
        tx.send(Cmd::Session(Arc::clone(&s), vec![PreparedInstance { track: t, slot: 1, id: "vst3:test.fake".into(), ch: 2, plugin, state_loaded: false }])).unwrap();
        let mut out = vec![0.0f32; 256];
        st.fill(&mut out, 2);
        assert!(grx.try_recv().is_err(), "nothing retired yet");
        tx.send(Cmd::Session(Arc::new(openpt_model::Session::default()), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
        tx.send(Cmd::Session(
            Arc::clone(&s),
            vec![PreparedInstance { track: t, slot: 1, id: "vst3:test.fake".into(), ch: 2, plugin: openpt_dsp::create("gain").unwrap(), state_loaded: false }],
        ))
        .unwrap();
        st.fill(&mut out, 2);
        let mut s44 = (*s).clone();
        s44.sample_rate = openpt_time::SampleRate::new(44_100).unwrap();
        tx.send(Cmd::Session(Arc::new(s44), Vec::new())).unwrap();
        st.fill(&mut out, 2);
        assert_eq!(grx.try_recv().unwrap().len(), 1);
    }

    #[test]
    fn latencies_are_reported_while_stopped() {
        use openpt_model::{ChannelFormat, Insert, TrackKind};
        let mut s = openpt_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("maximizer"));
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let (gtx, _grx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(openpt_model::Session::default()), 48_000.0, gtx);
        tx.send(Cmd::Session(Arc::new(s), Vec::new())).unwrap();
        let mut out = vec![0.0f32; 256];
        st.fill(&mut out, 2);
        st.fill(&mut out, 2);
        let m = shared.meters.lock().unwrap();
        let latency = m.tracks.get(&t).map_or(0, |m| m.latency);
        assert!(latency > 0, "stopped, the Delay Compensation view still sees the maximizer's lookahead");
        assert!(out.iter().all(|x| *x == 0.0), "the probe block is not heard");
    }

    #[test]
    fn plugin_states_are_read_without_the_audio_thread() {
        let mut side = Side::new(48_000.0, channel().1);
        let (a, b) = (TrackId(1), TrackId(2));
        side.states.insert((b, 0), Box::new(|| Some(b"b0".to_vec())));
        side.states.insert((a, openpt_mix::INSTRUMENT_SLOT), Box::new(|| Some(b"synth".to_vec())));
        side.states.insert((a, 3), Box::new(|| Some(b"a3".to_vec())));
        side.states.insert((a, 4), Box::new(|| None));
        let want = vec![(a, 3, b"a3".to_vec()), (a, openpt_mix::INSTRUMENT_SLOT, b"synth".to_vec()), (b, 0, b"b0".to_vec())];
        assert_eq!(side.capture_states(), want);
        side.forget((a, 3));
        assert_eq!(side.capture_states(), want[1..]);
    }

    struct Window;

    impl PluginEditor for Window {
        fn open(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn close(&mut self) {}
        fn is_open(&self) -> bool {
            false
        }
        fn idle(&mut self) -> Vec<(String, f32)> {
            Vec::new()
        }
    }

    #[test]
    fn a_refused_state_is_kept_until_the_editor_opens() {
        let mut side = Side::new(48_000.0, channel().1);
        let k = (TrackId(1), 0);
        side.states.insert(k, Box::new(|| Some(b"defaults".to_vec())));
        side.editors.insert(k, Box::new(Window));
        side.refused.insert(k);
        assert!(side.capture_states().is_empty());
        side.open_editor(k).unwrap();
        assert_eq!(side.capture_states(), vec![(TrackId(1), 0, b"defaults".to_vec())]);
    }

    #[test]
    fn side_creates_each_missing_instance_once() {
        use openpt_model::{ChannelFormat, Insert, TrackKind};
        let mut side = Side::new(48_000.0, channel().1);
        let mut s = openpt_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("vst3:does.not.exist"));
        s.track_mut(t).unwrap().mixer.inserts[1] = Some(Insert::new("gain"));
        assert!(side.prepare(&s).is_empty(), "the plugin is not installed; built-ins stay with the engine");
        assert_eq!(side.shadow.get(&(t, 0)), Some(&("vst3:does.not.exist".to_string(), 1, false)));
        assert!(side.prepare(&s).is_empty());
        assert_eq!(side.shadow.len(), 1, "failures are remembered, not retried every update");
        s.track_mut(t).unwrap().mixer.inserts[0] = None;
        side.prepare(&s);
        assert!(side.shadow.is_empty());
    }

    #[test]
    fn replaced_sessions_are_freed_on_the_ui_thread() {
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let (gtx, grx) = channel();
        let mut st = AudioState::new(rx, shared, Arc::new(openpt_model::Session::default()), 48_000.0, gtx);
        let mut side = Side::new(48_000.0, grx);
        side.return_old_sessions(&tx);
        let mut out = vec![0.0f32; 256];
        let mut watched = Vec::new();
        for _ in 0..3 * OLD_SESSIONS {
            let s = Arc::new(openpt_model::Session::default());
            watched.push(Arc::downgrade(&s));
            side.collect_garbage();
            tx.send(Cmd::Session(s, Vec::new())).unwrap();
            st.fill(&mut out, 2);
            let replaced = watched.len().checked_sub(2).and_then(|i| watched.get(i));
            assert!(replaced.is_none_or(|w| w.strong_count() == 1), "freed on the audio thread");
        }
        side.collect_garbage();
        assert_eq!(watched.iter().filter(|w| w.strong_count() > 0).count(), 1, "only the live snapshot is left");
    }

    #[test]
    fn looping_wraps() {
        let s = Arc::new(openpt_model::Session::default());
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), s, 48_000.0, channel().0);
        tx.send(Cmd::Play { from: 0, end: None, looped: Some(Range::new(0, 1000)) }).unwrap();
        let mut out = vec![0.0f32; 20_000];
        st.fill(&mut out, 2);
        let p = shared.position.load(Ordering::Relaxed);
        assert!(p <= 1000, "position {p} should stay inside the loop");
        assert!(shared.playing.load(Ordering::Relaxed));
    }

    #[test]
    fn locate_while_playing_drops_the_old_position_audio() {
        use openpt_model::{ChannelFormat, Clip, Insert, SourceAudio, SourceId, TrackKind};
        for (plugin, params) in [("compressor", &[("lookahead", 5.0), ("ratio", 1.0)][..]), ("room_reverb", &[][..])] {
            let mut s = openpt_model::Session::default();
            s.edit.click = false;
            let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
            s.pool.insert(
                SourceId(5),
                Arc::new(SourceAudio::new(openpt_audio_io::AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.5; 4800]] })),
            );
            let id = s.new_clip_id();
            let tr = s.track_mut(t).unwrap();
            tr.playlist_mut().unwrap().clips.push(Clip::audio(id, "x", SourceId(5), 0, 0, 4800));
            let mut ins = Insert::new(plugin);
            for (k, v) in params {
                ins.params.insert((*k).into(), *v);
            }
            tr.mixer.inserts[0] = Some(ins);
            let shared = Arc::new(Shared {
                position: AtomicI64::new(0),
                playing: AtomicBool::new(false),
                speed: AtomicU32::new(1000),
                meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
            });
            let (tx, rx) = channel();
            let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);
            tx.send(Cmd::Play { from: 0, end: None, looped: None }).unwrap();
            let mut out = vec![0.0f32; 2 * 512];
            let mut before = 0.0f32;
            for _ in 0..9 {
                st.fill(&mut out, 2);
                before = out.iter().fold(before, |m, x| m.max(x.abs()));
            }
            assert!(before > 0.1, "{plugin}: the clip plays before the locate");
            tx.send(Cmd::Locate(1_000_000)).unwrap();
            for _ in 0..8 {
                st.fill(&mut out, 2);
                assert!(out.iter().all(|x| x.abs() < 1e-6), "{plugin}: audio from before the locate");
            }
        }
    }

    #[test]
    fn armed_tracks_are_heard_and_metered_after_playback_stops() {
        use openpt_model::{ChannelFormat, TrackKind};
        let mut s = openpt_model::Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(t).unwrap().mixer.record_arm = true;
        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);
        let ring = record::InputRing::new(1, 48_000);
        tx.send(Cmd::Input(Arc::clone(&ring))).unwrap();
        tx.send(Cmd::Play { from: 0, end: Some(1000), looped: None }).unwrap();
        let mut out = vec![0.0f32; 4096];
        st.fill(&mut out, 2);
        assert!(!shared.playing.load(Ordering::Relaxed));
        ring.push(&[0.5; 4096]);
        st.fill(&mut out, 2);
        assert!(out.iter().any(|x| x.abs() > 0.1), "the input is heard");
        assert!(shared.meters.lock().unwrap().tracks.get(&t).is_some_and(|m| m.peak[0] > 0.1), "and metered");
    }

    #[test]
    fn armed_instrument_tracks_audition_live_midi_when_stopped() {
        use crate::midi::MidiInputRing;
        use openpt_midi::LiveMidiEvent;
        use openpt_model::{ChannelFormat, Insert, TrackKind};

        let mut s = openpt_model::Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        let mut ins = Insert::new("subtractive_synth");
        ins.params.insert("level".into(), -6.0);
        s.track_mut(t).unwrap().instrument = Some(ins);
        s.track_mut(t).unwrap().mixer.record_arm = true;

        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(false),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);

        let midi_ring = Arc::new(MidiInputRing::new(1024));
        tx.send(Cmd::MidiInput(Arc::clone(&midi_ring))).unwrap();

        let mut out = vec![0.0f32; 2 * 512];
        st.fill(&mut out, 2);
        assert!(out.iter().all(|x| x.abs() < 1e-6), "silent before note on");

        assert!(midi_ring.push(LiveMidiEvent { port_id: 0, timestamp_us: 100, len: 3, data: [0x90, 60, 100] }));

        st.fill(&mut out, 2);
        assert!(out.iter().any(|x| x.abs() > 1e-4), "armed instrument must produce audio on live note on");
    }

    #[test]
    fn recording_captures_live_midi_events_into_record_ring() {
        use crate::midi::{MidiInputRing, MidiRecordRing};
        use openpt_midi::LiveMidiEvent;
        use openpt_model::{ChannelFormat, TrackKind};

        let mut s = openpt_model::Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().mixer.record_arm = true;

        let shared = Arc::new(Shared {
            position: AtomicI64::new(0),
            playing: AtomicBool::new(true),
            speed: AtomicU32::new(1000),
            meters: Mutex::new(MeterSnapshot::default()),
            late: Mutex::default(),
        });
        let (tx, rx) = channel();
        let mut st = AudioState::new(rx, Arc::clone(&shared), Arc::new(s), 48_000.0, channel().0);

        let midi_in = Arc::new(MidiInputRing::new(64));
        let midi_rec = Arc::new(MidiRecordRing::new(64));
        tx.send(Cmd::MidiInput(Arc::clone(&midi_in))).unwrap();
        tx.send(Cmd::MidiRecord(Arc::clone(&midi_rec))).unwrap();
        tx.send(Cmd::Recording(true)).unwrap();
        tx.send(Cmd::Play { from: 1000, end: None, looped: None }).unwrap();

        let mut out = vec![0.0f32; 2 * 512];
        st.fill(&mut out, 2);

        assert!(midi_in.push(LiveMidiEvent { port_id: 0, timestamp_us: 100, len: 3, data: [0x90, 60, 100] }));
        st.fill(&mut out, 2);

        assert_eq!(midi_rec.len(), 1);
        let rec = midi_rec.pop().unwrap();
        assert_eq!(rec.track_id, t.0);
        assert_eq!(rec.event.note_pitch(), 60);
        assert_eq!(rec.event.note_velocity(), 100);
    }
}
