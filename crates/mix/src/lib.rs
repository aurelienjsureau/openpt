#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod multi_mono;

use openpt_dsp::pan::{PanLaw, SpeakerPos, SurroundParams, gains};
use openpt_dsp::{Plugin, SampleSource, Transport, db_to_gain};
use openpt_midi::{CtrlKind, LiveMidiEvent};
use openpt_model::{
    AutoParam, BusId, ChannelFormat, Clip, ClipContent, MidiFrom, Route, Session, SourceAudio, Speaker, SurroundPan, Track, TrackId, TrackKind,
};
use openpt_time::{Range, Samples, TICKS_PER_QUARTER};
use std::collections::HashMap;
use std::ops::Bound;
use std::sync::Arc;

pub const MAIN_CHANNELS: usize = 2;

pub const MAX_CHANNELS: usize = 16;

pub fn main_channels(s: &Session) -> usize {
    s.main_format().channels().clamp(MAIN_CHANNELS, MAX_CHANNELS)
}

pub const INSTRUMENT_SLOT: usize = usize::MAX;

pub fn is_third_party(id: &str) -> bool {
    let id = id.strip_prefix(openpt_model::MULTI_OUT_PREFIX).unwrap_or(id);
    let id = id.strip_prefix(openpt_model::MULTI_MONO_PREFIX).unwrap_or(id);
    id.starts_with(openpt_vst3_host::ID_PREFIX) || id.starts_with(openpt_au_host::ID_PREFIX)
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceSpec {
    pub track: TrackId,
    pub slot: usize,
    pub id: String,
    pub ch: usize,
    pub state: Option<Vec<u8>>,
}

pub struct PreparedInstance {
    pub track: TrackId,
    pub slot: usize,
    pub id: String,
    pub ch: usize,
    pub plugin: Box<dyn Plugin>,
}

pub fn instance_specs(s: &Session) -> Vec<InstanceSpec> {
    let main_ch = main_channels(s);
    let mut out = Vec::new();
    for t in &s.tracks {
        let ch = strip_channels(t, main_ch);
        for (i, ins) in t.mixer.inserts.iter().enumerate() {
            if let Some(ins) = ins
                && is_third_party(&ins.plugin)
            {
                out.push(InstanceSpec { track: t.id, slot: i, id: ins.instance_id(), ch, state: ins.state_bytes() });
            }
        }
        if let Some(ins) = &t.instrument
            && is_third_party(&ins.plugin)
        {
            out.push(InstanceSpec { track: t.id, slot: INSTRUMENT_SLOT, id: ins.instance_id(), ch: ch.max(2), state: ins.state_bytes() });
        }
    }
    out
}

pub struct NewInstance {
    pub plugin: Box<dyn Plugin>,
    pub refused_state: bool,
}

pub fn create_instance(id: &str, state: Option<&[u8]>, sample_rate: f32, max_block: usize, ch: usize) -> Option<NewInstance> {
    create_instance_on(id, state, sample_rate, max_block, ch, None)
}

pub fn create_instance_on(
    id: &str,
    state: Option<&[u8]>,
    sample_rate: f32,
    max_block: usize,
    ch: usize,
    ara: Option<&openpt_dsp::AraTrack>,
) -> Option<NewInstance> {
    let multi_out = id.starts_with(openpt_model::MULTI_OUT_PREFIX);
    let id = id.strip_prefix(openpt_model::MULTI_OUT_PREFIX).unwrap_or(id);
    let (mut plugin, refused_state) = match id.strip_prefix(openpt_model::MULTI_MONO_PREFIX) {
        Some(inner) => {
            let (mm, refused) = multi_mono::MultiMono::new(inner, state)?;
            (Box::new(mm) as Box<dyn Plugin>, refused)
        }
        None => {
            let mut p = create_plugin(id)?;
            let refused = state.is_some_and(|st| !p.load_state(st));
            (p, refused)
        }
    };
    if refused_state {
        log::warn!("{id}: the plugin did not accept its stored state");
    }
    if let Some(t) = ara {
        let bound = plugin.ara_bind(t);
        log::info!("{id}: ARA {} ({} clips)", if bound { "bound" } else { "not bound" }, t.clips.len());
    }
    plugin.set_multi_output(multi_out);
    plugin.prepare(sample_rate, max_block, ch);
    Some(NewInstance { plugin, refused_state })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StripMeter {
    pub peak: [f32; 2],
    pub peaks: Vec<f32>,
    pub gain_reduction_db: f32,
    pub plugin_failures: PluginFailures,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PluginFailures {
    pub count: u32,
    pub last: usize,
}

impl PluginFailures {
    fn record(&mut self, slot: usize) {
        self.count = self.count.wrapping_add(1);
        self.last = slot;
    }
}

impl StripMeter {
    pub fn copy_from(&mut self, o: &StripMeter) {
        self.peak = o.peak;
        self.peaks.clone_from(&o.peaks);
        self.gain_reduction_db = o.gain_reduction_db;
        self.plugin_failures = o.plugin_failures;
    }

    fn measure(&mut self, bufs: &[Vec<f32>], frames: usize, gr: f32) {
        if self.peaks.len() != bufs.len() {
            self.peaks.resize(bufs.len(), 0.0);
        }
        for (p, b) in self.peaks.iter_mut().zip(bufs.iter()) {
            *p = peak(b.get(..frames).unwrap_or(&[]));
        }
        let last = self.peaks.len().saturating_sub(1);
        self.peak = [self.peaks.first().copied().unwrap_or(0.0), self.peaks.get(1.min(last)).copied().unwrap_or(0.0)];
        self.gain_reduction_db = gr;
    }
}

struct BusBuf {
    fmt: ChannelFormat,
    buf: Vec<Vec<f32>>,
}

struct PluginSlot {
    id: String,
    ch: usize,
    plugin: Box<dyn Plugin>,
    applied: Vec<f32>,
    failed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldNotes {
    mask: [u64; 32],
}

impl HeldNotes {
    pub const fn new() -> Self {
        Self { mask: [0u64; 32] }
    }

    #[inline]
    fn bit_pos(channel: u8, pitch: u8) -> (usize, u64) {
        let ch = (channel & 0x0F) as usize;
        let p = (pitch & 0x7F) as usize;
        let idx = ch * 128 + p;
        (idx / 64, 1u64 << (idx % 64))
    }

    #[inline]
    pub fn contains(&self, channel: u8, pitch: u8) -> bool {
        if channel >= 16 || pitch >= 128 {
            return false;
        }
        let (word, bit) = Self::bit_pos(channel, pitch);
        (self.mask[word] & bit) != 0
    }

    #[inline]
    pub fn insert(&mut self, channel: u8, pitch: u8) {
        if channel < 16 && pitch < 128 {
            let (word, bit) = Self::bit_pos(channel, pitch);
            self.mask[word] |= bit;
        }
    }

    #[inline]
    pub fn remove(&mut self, channel: u8, pitch: u8) {
        if channel < 16 && pitch < 128 {
            let (word, bit) = Self::bit_pos(channel, pitch);
            self.mask[word] &= !bit;
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.mask = [0u64; 32];
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.mask.iter().all(|&w| w == 0)
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.mask.iter().map(|w| w.count_ones() as usize).sum()
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (u8, u8)> + '_ {
        (0..16u8).flat_map(move |ch| (0..128u8).filter_map(move |p| if self.contains(ch, p) { Some((ch, p)) } else { None }))
    }
}

struct Strip {
    buf: Vec<Vec<f32>>,
    heat: openpt_dsp::heat::Heat,
    pre: Vec<Vec<f32>>,
    scratch: Vec<f32>,
    plugins: Vec<Option<PluginSlot>>,
    instrument: Option<PluginSlot>,
    sample: Option<Arc<SourceAudio>>,
    held: HeldNotes,
    chase: bool,
    muted: bool,
    gr: f32,
    clip_fx: HashMap<u64, ClipFx>,
    clipbuf: Vec<Vec<f32>>,
    latency: usize,
    align: DelayLine,
    align_pre: DelayLine,
    out_extra: DelayLine,
    outbuf: Vec<Vec<f32>>,
    failures: PluginFailures,
    key_wait: DelayLine,
    key_wait_len: usize,
    key_lags: Vec<usize>,
    key_lag_lines: Vec<(DelayLine, Vec<Vec<f32>>)>,
    key_feed: Vec<(BusId, usize, DelayLine, Vec<Vec<f32>>)>,
    extras: Vec<Vec<Vec<f32>>>,
    extra_align: Vec<DelayLine>,
    midi_out: Vec<(usize, [u8; 3])>,
}

const MAX_LATENCY: usize = 1 << 20;

#[derive(Default)]
struct DelayLine {
    bufs: Vec<Vec<f32>>,
    idx: usize,
    len: usize,
}

impl DelayLine {
    fn set(&mut self, len: usize, ch: usize) {
        let len = len.min(MAX_LATENCY);
        if len != self.len || self.bufs.len() != ch {
            self.len = len;
            self.idx = 0;
            self.bufs = vec![vec![0.0; len]; ch];
        }
    }
    #[cfg(test)]
    #[inline]
    fn len(&self) -> usize {
        self.len
    }
    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        if self.len == 0 {
            return;
        }
        let start = self.idx;
        for (c, ch) in io.iter_mut().enumerate() {
            let Some(ring) = self.bufs.get_mut(c) else { continue };
            let mut i = start;
            for x in ch.iter_mut().take(frames) {
                if let Some(r) = ring.get_mut(i) {
                    std::mem::swap(r, x);
                }
                i += 1;
                if i >= self.len {
                    i = 0;
                }
            }
        }
        self.idx = (start + frames) % self.len.max(1);
    }
    fn clear(&mut self) {
        self.bufs.iter_mut().for_each(|b| b.fill(0.0));
    }
}

struct ClipFx {
    eq: Option<Box<dyn Plugin>>,
    comp: Option<Box<dyn Plugin>>,
    gain_db: f32,
    sr: f32,
    ch: usize,
}

impl Strip {
    fn new() -> Self {
        Strip {
            buf: Vec::new(),
            pre: Vec::new(),
            scratch: Vec::new(),
            plugins: Vec::new(),
            instrument: None,
            sample: None,
            held: HeldNotes::new(),
            chase: true,
            muted: false,
            gr: 0.0,
            clip_fx: HashMap::new(),
            clipbuf: Vec::new(),
            latency: 0,
            align: DelayLine::default(),
            align_pre: DelayLine::default(),
            out_extra: DelayLine::default(),
            outbuf: Vec::new(),
            failures: PluginFailures::default(),
            key_wait: DelayLine::default(),
            key_wait_len: 0,
            key_lags: Vec::new(),
            key_lag_lines: Vec::new(),
            key_feed: Vec::new(),
            extras: Vec::new(),
            midi_out: Vec::with_capacity(512),
            extra_align: Vec::new(),
            heat: openpt_dsp::heat::Heat::default(),
        }
    }
}

pub struct MixEngine {
    sample_rate: f32,
    max_block: usize,
    strips: HashMap<TrackId, Strip>,
    busses: HashMap<BusId, BusBuf>,
    main: Vec<Vec<f32>>,
    main_fmt: ChannelFormat,
    pub meters: HashMap<TrackId, StripMeter>,
    pub main_meter: StripMeter,
    pub pan_law: PanLaw,
    last_end: Samples,
    synced: (usize, usize, usize),
    order: Vec<usize>,
    pub latency: usize,
    track_latency: usize,
    aux_fed: Vec<BusId>,
    aux_latency: usize,
    pub input: Vec<Vec<f32>>,
    pub midi_input: Vec<(usize, LiveMidiEvent)>,
    pub midi_routed: Vec<(TrackId, usize, [u8; 3])>,
    pub monitor_only: bool,
    pub recording: bool,
    pub metronome: bool,
    external: bool,
    adopt: HashMap<(TrackId, usize), PreparedInstance>,
    retired: Vec<Box<dyn Plugin>>,
    work: Vec<(usize, Strip)>,
    keys: HashMap<BusId, BusBuf>,
    key_latency: HashMap<BusId, usize>,
}

struct SlotCtx<'a> {
    sr: f32,
    mb: usize,
    external: bool,
    adopt: &'a mut HashMap<(TrackId, usize), PreparedInstance>,
    retired: &'a mut Vec<Box<dyn Plugin>>,
}

impl SlotCtx<'_> {
    fn retire(&mut self, slot: Option<PluginSlot>) {
        if let Some(s) = slot {
            self.retired.push(s.plugin);
        }
    }

    fn waiting(&self, t: TrackId, idx: usize) -> bool {
        self.external && self.adopt.contains_key(&(t, idx))
    }

    fn make(&mut self, t: TrackId, idx: usize, ins: &openpt_model::Insert, ch: usize) -> Option<PluginSlot> {
        let third = is_third_party(&ins.plugin);
        let plugin = if third && self.external {
            match self.adopt.remove(&(t, idx)) {
                Some(p) if ins.is_instance(&p.id) && p.ch == ch => Some(p.plugin),
                Some(p) => {
                    self.retired.push(p.plugin);
                    None
                }
                None => None,
            }
        } else {
            let state = if third { ins.state_bytes() } else { None };
            create_instance(&ins.instance_id(), state.as_deref(), self.sr, self.mb, ch).map(|n| n.plugin)
        }?;
        Some(PluginSlot { id: ins.instance_id(), ch, plugin, applied: Vec::new(), failed: false })
    }
}

impl MixEngine {
    pub fn new(sample_rate: f32, max_block: usize) -> Self {
        let max_block = max_block.clamp(16, 16_384);
        MixEngine {
            sample_rate: if sample_rate.is_finite() && sample_rate > 0.0 { sample_rate } else { 48_000.0 },
            max_block,
            strips: HashMap::new(),
            busses: HashMap::new(),
            main: vec![vec![0.0; max_block]; MAIN_CHANNELS],
            main_fmt: ChannelFormat::Stereo,
            meters: HashMap::new(),
            main_meter: StripMeter::default(),
            pan_law: PanLaw::Minus2_5,
            last_end: 0,
            synced: (0, 0, 0),
            order: Vec::new(),
            latency: 0,
            track_latency: 0,
            aux_fed: Vec::new(),
            aux_latency: 0,
            input: Vec::new(),
            midi_input: Vec::with_capacity(256),
            midi_routed: Vec::with_capacity(2048),
            monitor_only: false,
            recording: false,
            metronome: false,
            external: false,
            adopt: HashMap::new(),
            retired: Vec::new(),
            work: Vec::new(),
            keys: HashMap::new(),
            key_latency: HashMap::new(),
        }
    }

    pub fn set_external_instances(&mut self, on: bool) {
        self.external = on;
    }

    pub fn adopt(&mut self, instances: Vec<PreparedInstance>) {
        for p in instances {
            if let Some(old) = self.adopt.insert((p.track, p.slot), p) {
                self.retired.push(old.plugin);
            }
        }
        self.synced = (usize::MAX, usize::MAX, usize::MAX);
    }

    pub fn take_retired(&mut self) -> Vec<Box<dyn Plugin>> {
        std::mem::take(&mut self.retired)
    }

    pub fn drain_plugins(&mut self) -> Vec<Box<dyn Plugin>> {
        let mut out = std::mem::take(&mut self.retired);
        for st in self.strips.values_mut() {
            out.extend(st.plugins.iter_mut().filter_map(Option::take).map(|s| s.plugin));
            out.extend(st.instrument.take().map(|s| s.plugin));
        }
        out.extend(self.adopt.drain().map(|(_, p)| p.plugin));
        self.synced = (usize::MAX, usize::MAX, usize::MAX);
        out
    }

    pub fn max_block(&self) -> usize {
        self.max_block
    }

    pub fn main_format(&self) -> ChannelFormat {
        self.main_fmt
    }

    pub fn main_channels(&self) -> usize {
        self.main.len()
    }

    pub fn reset(&mut self) {
        for s in self.strips.values_mut() {
            for p in s.plugins.iter_mut().flatten() {
                p.plugin.reset();
                p.failed = false;
            }
            if let Some(i) = &mut s.instrument {
                i.plugin.all_notes_off();
                i.plugin.reset();
                i.failed = false;
            }
            s.held.clear();
            s.chase = true;
            s.align.clear();
            s.align_pre.clear();
            s.out_extra.clear();
        }
        self.meters.clear();
        self.main_meter = StripMeter::default();
        self.midi_input.clear();
        self.midi_routed.clear();
    }

    pub fn sync(&mut self, s: &Session) {
        let mb = self.max_block;
        let main_ch = main_channels(s);
        self.main_fmt = s.main_format();
        if self.main.len() != main_ch {
            self.main = vec![vec![0.0; mb]; main_ch];
        }
        let mut cx = SlotCtx { sr: self.sample_rate, mb, external: self.external, adopt: &mut self.adopt, retired: &mut self.retired };
        let gone: Vec<TrackId> = self.strips.keys().filter(|id| s.track(**id).is_none()).copied().collect();
        for id in gone {
            if let Some(mut st) = self.strips.remove(&id) {
                for p in st.plugins.drain(..) {
                    cx.retire(p);
                }
                cx.retire(st.instrument.take());
            }
        }
        for t in &s.tracks {
            let ch = strip_channels(t, main_ch);
            let strip = self.strips.entry(t.id).or_insert_with(Strip::new);
            if strip.buf.len() != ch || strip.buf.first().map_or(0, Vec::len) != mb {
                strip.buf = vec![vec![0.0; mb]; ch];
            }
            while strip.plugins.len() > t.mixer.inserts.len() {
                let p = strip.plugins.pop().flatten();
                cx.retire(p);
            }
            strip.plugins.resize_with(t.mixer.inserts.len(), || None);
            for (idx, (slot, ins)) in strip.plugins.iter_mut().zip(t.mixer.inserts.iter()).enumerate() {
                match ins {
                    Some(i) if slot.as_ref().is_none_or(|p| !i.is_instance(&p.id) || p.ch != ch) || cx.waiting(t.id, idx) => {
                        let old = slot.take();
                        cx.retire(old);
                        *slot = cx.make(t.id, idx, i, ch);
                    }
                    None => {
                        let old = slot.take();
                        cx.retire(old);
                    }
                    _ => {}
                }
            }
            let want = t.instrument.as_ref();
            let ich = ch.max(2);
            if strip.instrument.as_ref().map(|p| p.ch) != want.map(|_| ich)
                || (want.is_some() && !matches!((&strip.instrument, want), (Some(p), Some(i)) if i.is_instance(&p.id)))
                || (want.is_some() && cx.waiting(t.id, INSTRUMENT_SLOT))
            {
                let old = strip.instrument.take();
                cx.retire(old);
                strip.instrument = want.and_then(|i| cx.make(t.id, INSTRUMENT_SLOT, i, ich));
                strip.sample = None;
            }
            sync_sample(strip, want, s);
        }
        let leftovers: Vec<(TrackId, usize)> = cx.adopt.keys().copied().collect();
        for k in leftovers {
            if let Some(p) = cx.adopt.remove(&k) {
                cx.retired.push(p.plugin);
            }
        }
        if !self.external {
            self.retired.clear();
        }
        for b in &s.busses {
            let n = b.format.channels().clamp(1, MAX_CHANNELS);
            let bus = self.busses.entry(b.id).or_insert_with(|| BusBuf { fmt: b.format, buf: Vec::new() });
            bus.fmt = b.format;
            if bus.buf.len() != n {
                bus.buf = vec![vec![0.0; mb]; n];
            }
        }
        self.busses.retain(|id, _| s.bus(*id).is_some());
    }

    pub fn ensure_synced(&mut self, s: &Session) {
        let key = (structure_fingerprint(s), s.tracks.len(), s.busses.len());
        if key != self.synced {
            self.sync(s);
            self.order = processing_order(s);
            self.synced = key;
        }
    }

    pub fn render(&mut self, s: &Session, pos: Samples, frames: usize, out: &mut [Vec<f32>]) {
        let frames = frames.min(self.max_block);
        if pos != self.last_end {
            for st in self.strips.values_mut() {
                if let Some(i) = &mut st.instrument {
                    i.plugin.all_notes_off();
                }
                st.held.clear();
                st.chase = true;
            }
        }
        self.last_end = pos.saturating_add(frames as i64);
        self.ensure_synced(s);
        for b in self.busses.values_mut() {
            for c in b.buf.iter_mut() {
                c.iter_mut().take(frames).for_each(|x| *x = 0.0);
            }
        }
        for c in &mut self.main {
            c.iter_mut().take(frames).for_each(|x| *x = 0.0);
        }
        let any_solo = s.tracks.iter().any(|t| t.mixer.solo && !t.inactive);
        let order = std::mem::take(&mut self.order);
        let live = |t: &Track| !(t.kind == TrackKind::Master || t.inactive || t.kind == TrackKind::Vca);
        let is_aux = reads_bus;
        let mut work = std::mem::take(&mut self.work);
        work.clear();
        for &i in &order {
            let Some(t) = s.tracks.get(i).filter(|t| live(t) && !is_aux(t)) else { continue };
            if let Some(st) = self.strips.remove(&t.id) {
                work.push((i, st));
            }
        }
        for (_, st) in work.iter_mut() {
            if st.scratch.len() < self.max_block {
                st.scratch.resize(self.max_block, 0.0);
            }
        }
        self.midi_routed.clear();
        for t in s.tracks.iter().filter(|t| t.kind.is_midi() && !t.inactive) {
            let Some(from) = t.midi_from else { continue };
            let listening =
                t.mixer.record_arm || t.mixer.input_monitor || (s.edit.flag("options.midi_thru") && s.edit.selected_tracks.contains(&t.id));
            let Some(src) = s.track(from.track()).filter(|_| listening) else { continue };
            let routed = &mut self.midi_routed;
            let mut push = |off: usize, m: [u8; 3]| {
                if routed.len() < routed.capacity() {
                    routed.push((t.id, off.min(frames.saturating_sub(1)), m));
                }
            };
            match from {
                MidiFrom::Track(_) if !self.monitor_only => track_events(s, src, pos, frames, &mut push),
                MidiFrom::Track(_) => {}
                MidiFrom::Plugins(_) => {
                    let st = work
                        .iter()
                        .find(|(i, _)| s.tracks.get(*i).is_some_and(|x| x.id == src.id))
                        .map(|(_, st)| st)
                        .or_else(|| self.strips.get(&src.id));
                    for &(off, m) in st.map_or(&[][..], |st| st.midi_out.as_slice()) {
                        push(off, m);
                    }
                }
            }
        }
        let n_free = order_keyed(s, &mut work);
        let busses = std::mem::take(&mut self.busses);
        let live_in = LiveInput {
            input: &self.input,
            midi_input: &self.midi_input,
            routed: &self.midi_routed,
            monitor_only: self.monitor_only,
            recording: self.recording,
        };
        run_strips(work.get_mut(..n_free).unwrap_or(&mut []), |(i, st)| {
            if let Some(t) = s.tracks.get(*i) {
                process_strip(s, t, st, &busses, None, &live_in, pos, frames, any_solo);
            }
        });
        self.busses = busses;
        self.build_keys(s, &mut work, n_free, pos, frames);
        let pdc = s.edit.delay_compensation;
        for (i, st) in work.iter_mut().skip(n_free) {
            if let Some(t) = s.tracks.get(*i) {
                plan_keys(t, st, pos, &self.key_latency);
            }
        }
        if pdc {
            for t in s.tracks.iter().filter(|t| is_aux(t) && !t.inactive) {
                if let Some(st) = self.strips.get_mut(&t.id) {
                    note_insert_latency(t, st, pos);
                }
            }
        }
        let mut lmax_m = 0usize;
        if pdc {
            for t in s.tracks.iter().filter(|t| t.kind == TrackKind::Master && !t.inactive) {
                if let Some(st) = self.strips.get_mut(&t.id) {
                    note_insert_latency(t, st, pos);
                    lmax_m = lmax_m.saturating_add(st.latency);
                }
            }
        }
        let lmax_t = if pdc { work.iter().map(|(_, st)| st.latency).max().unwrap_or(0) } else { 0 };
        let lmax_a =
            if pdc { self.strips.iter().filter(|(id, _)| s.track(**id).is_some_and(is_aux)).map(|(_, st)| st.latency).max().unwrap_or(0) } else { 0 };
        self.track_latency = lmax_t;
        self.latency = lmax_t + lmax_a + lmax_m;
        self.aux_latency = lmax_a;
        self.aux_fed.clear();
        if pdc {
            for t in s.tracks.iter().filter(|t| is_aux(t) && live(t)) {
                if let Route::Bus(b) = &t.mixer.output
                    && !self.aux_fed.contains(b)
                {
                    self.aux_fed.push(*b);
                }
            }
        }
        let align = |t: &Track, st: &mut Strip, aux_fed: &[BusId]| {
            let ch = st.buf.len();
            let is_monitored = t.mixer.record_arm || t.mixer.input_monitor;
            let align_delay = if pdc && !is_monitored { lmax_t.saturating_sub(st.latency) } else { 0 };
            st.align.set(align_delay, ch);
            st.align.process(&mut st.buf, frames);
            if t.mixer.sends.iter().flatten().any(|x| x.pre_fader) {
                st.align_pre.set(align_delay, ch);
                st.align_pre.process(&mut st.pre, frames);
            }
            let waits = match &t.mixer.output {
                Route::Main => !is_monitored,
                Route::Bus(b) => !is_monitored && aux_fed.contains(b),
                _ => false,
            };
            st.out_extra.set(if waits { lmax_a } else { 0 }, ch);
        };
        let extra_delay =
            |t: &Track, st: &Strip| if pdc && !(t.mixer.record_arm || t.mixer.input_monitor) { lmax_t.saturating_sub(st.latency) } else { 0 };
        for (i, st) in work.iter_mut().take(n_free) {
            if let Some(t) = s.tracks.get(*i) {
                align(t, st, &self.aux_fed);
                self.route_strip(t, st, pos, frames);
                let d = extra_delay(t, st);
                self.route_extras(t, st, d, frames);
            }
        }
        for (i, st) in work.iter_mut().skip(n_free) {
            let Some(t) = s.tracks.get(*i) else { continue };
            let live_in = LiveInput {
                input: &self.input,
                midi_input: &self.midi_input,
                routed: &self.midi_routed,
                monitor_only: self.monitor_only,
                recording: self.recording,
            };
            process_strip(s, t, st, &self.busses, Some(&self.keys), &live_in, pos, frames, any_solo);
            let law = self.pan_law;
            feed_keys(t, st, &mut self.keys, &self.key_latency, pos, frames, law);
            align(t, st, &self.aux_fed);
            self.route_strip(t, st, pos, frames);
            let d = extra_delay(t, st);
            self.route_extras(t, st, d, frames);
        }
        for (i, st) in work.drain(..) {
            if let Some(t) = s.tracks.get(i) {
                self.strips.insert(t.id, st);
            }
        }
        self.work = work;
        let is_aux = reads_bus;
        for t in order.iter().filter_map(|&i| s.tracks.get(i)).filter(|t| live(t)).filter(|t| is_aux(t)) {
            let Some(mut st) = self.strips.remove(&t.id) else { continue };
            if st.scratch.len() < self.max_block {
                st.scratch.resize(self.max_block, 0.0);
            }
            let live_in = LiveInput {
                input: &self.input,
                midi_input: &self.midi_input,
                routed: &self.midi_routed,
                monitor_only: self.monitor_only,
                recording: self.recording,
            };
            process_strip(s, t, &mut st, &self.busses, None, &live_in, pos, frames, any_solo);
            let ch = st.buf.len();
            st.align.set(if pdc { lmax_a.saturating_sub(st.latency) } else { 0 }, ch);
            st.align.process(&mut st.buf, frames);
            st.out_extra.set(0, ch);
            self.route_strip(t, &mut st, pos, frames);
            self.strips.insert(t.id, st);
        }
        self.order = order;
        let transport = transport_at(s, pos, !self.monitor_only, self.recording);
        for t in s.tracks.iter().filter(|t| t.kind == TrackKind::Master && !t.inactive) {
            if let Some(strip) = self.strips.get_mut(&t.id) {
                for (dst, src) in strip.buf.iter_mut().zip(self.main.iter()) {
                    if let (Some(d), Some(sr)) = (dst.get_mut(..frames), src.get(..frames)) {
                        d.copy_from_slice(sr);
                    }
                }
                for (i, ins) in t.mixer.inserts.iter().enumerate() {
                    if let (Some(ins), Some(Some(slot))) = (ins, strip.plugins.get_mut(i))
                        && ins.active
                        && !ins.bypass
                        && !slot.failed
                    {
                        apply_params(slot, ins, t, i, pos);
                        slot.plugin.set_transport(&transport);
                        slot.plugin.process(&mut strip.buf, frames);
                        if contain_failure(slot, &mut strip.buf, frames) {
                            strip.failures.record(i);
                        }
                    }
                }
                let v0 = volume_db_at(t, pos);
                let v1 = volume_db_at(t, pos + frames as i64);
                for (ch, src) in strip.buf.iter().enumerate().take(self.main.len()) {
                    if let Some(m) = self.main.get_mut(ch) {
                        for i in 0..frames {
                            let g = lerp_gain(v0, v1, i, frames);
                            if let (Some(d), Some(x)) = (m.get_mut(i), src.get(i)) {
                                *d = x * g;
                            }
                        }
                    }
                }
                let m = self.meters.entry(t.id).or_default();
                m.measure(&self.main, frames, 0.0);
                m.plugin_failures = strip.failures;
            }
        }
        if self.metronome && !self.monitor_only && s.edit.click && (!s.edit.flag("click.only_during_record") || self.recording) {
            click::render(s, pos, frames, &mut self.main);
        }
        for (c, o) in out.iter_mut().enumerate() {
            let src = self.main.get(c);
            for i in 0..frames {
                let v = src.and_then(|m| m.get(i)).copied().unwrap_or(0.0);
                if let Some(d) = o.get_mut(i) {
                    *d = if v.is_finite() { v } else { 0.0 };
                }
            }
        }
        self.main_meter.measure(&self.main, frames, 0.0);
    }

    fn route_extras(&mut self, t: &Track, st: &mut Strip, delay: usize, frames: usize) {
        let Some(ins) = t.instrument.as_ref().filter(|i| i.multi_out) else { return };
        st.extra_align.resize_with(st.extras.len(), DelayLine::default);
        for (k, (out, line)) in st.extras.iter_mut().zip(st.extra_align.iter_mut()).enumerate() {
            let Some(b) = ins.outputs.get(k) else { break };
            let Some(dst) = self.busses.get_mut(b) else { continue };
            line.set(delay, out.len());
            line.process(out, frames);
            let (ns, nd) = (out.len(), dst.buf.len());
            let m = if ns == nd {
                identity_matrix(ns)
            } else if ns == 1 {
                let mut m = [[0.0; MAX_CHANNELS]; MAX_CHANNELS];
                for v in m[0].iter_mut().take(nd.min(MAX_CHANNELS)) {
                    *v = 1.0;
                }
                m
            } else {
                build_matrix(ChannelFormat::for_channels(ns), ns, dst.fmt, nd, &PanSpec { stereo: [-1.0, 1.0], surround: None }, PanLaw::Minus3)
            };
            apply_matrix(out, frames, &mut dst.buf, &m, 1.0);
        }
    }

    fn build_keys(&mut self, s: &Session, work: &mut [(usize, Strip)], n_free: usize, pos: Samples, frames: usize) {
        self.key_latency.clear();
        let mut wanted: Vec<BusId> = work.iter().skip(n_free).filter_map(|(i, _)| s.tracks.get(*i)).flat_map(key_busses).collect();
        wanted.sort_unstable_by_key(|b| b.0);
        wanted.dedup();
        self.keys.retain(|b, _| wanted.contains(b));
        for b in &wanted {
            let lk = work.iter().filter(|(i, _)| s.tracks.get(*i).is_some_and(|t| feeds_bus(t, *b))).map(|(_, st)| st.latency).max().unwrap_or(0);
            self.key_latency.insert(*b, lk);
            let (fmt, ch) = self.busses.get(b).map_or((ChannelFormat::Stereo, 2), |x| (x.fmt, x.buf.len()));
            let mb = self.max_block;
            let e = self.keys.entry(*b).or_insert_with(|| BusBuf { fmt, buf: vec![vec![0.0; mb]; ch] });
            if e.fmt != fmt || e.buf.len() != ch {
                *e = BusBuf { fmt, buf: vec![vec![0.0; mb]; ch] };
            }
            for c in &mut e.buf {
                c.iter_mut().take(frames).for_each(|x| *x = 0.0);
            }
        }
        if wanted.is_empty() {
            return;
        }
        let law = self.pan_law;
        for (i, st) in work.iter_mut().take(n_free) {
            if let Some(t) = s.tracks.get(*i) {
                feed_keys(t, st, &mut self.keys, &self.key_latency, pos, frames, law);
            }
        }
    }
    fn bus_latency(&self, bus: BusId) -> usize {
        self.track_latency + if self.aux_fed.contains(&bus) { self.aux_latency } else { 0 }
    }

    fn copy_bus(&self, bus: BusId, frames: usize, out: &mut [Vec<f32>]) {
        for o in out.iter_mut() {
            for x in o.iter_mut().take(frames) {
                *x = 0.0;
            }
        }
        let Some(src) = self.busses.get(&bus) else { return };
        for (o, c) in out.iter_mut().zip(src.buf.iter()) {
            for i in 0..frames {
                let v = c.get(i).copied().unwrap_or(0.0);
                if let Some(d) = o.get_mut(i) {
                    *d = if v.is_finite() { v } else { 0.0 };
                }
            }
        }
    }

    fn route_strip(&mut self, t: &Track, strip: &mut Strip, pos: Samples, frames: usize) {
        let m = self.meters.entry(t.id).or_default();
        m.measure(&strip.buf, frames, strip.gr);
        m.plugin_failures = strip.failures;
        let src_fmt = strip_format(t, strip.buf.len());
        let law = self.pan_law;
        for (i, snd) in t.mixer.sends.iter().enumerate() {
            let Some(snd) = snd else { continue };
            if snd.mute || strip.muted {
                continue;
            }
            let lvl = send_level(t, i, snd.level_db, pos);
            let gain = db_to_gain(lvl);
            let src = if snd.pre_fader { &strip.pre } else { &strip.buf };
            let Route::Bus(b) = &snd.target else { continue };
            let Some(dst) = self.busses.get_mut(b) else { continue };
            if gain <= 0.0 {
                continue;
            }
            let stereo = match (src.len(), snd.follow_main_pan) {
                (1, true) => [pan_at(t, 0, pos), 0.0],
                (1, false) => [snd.pan, 0.0],
                (_, true) => [pan_at(t, 0, pos), pan_at(t, 1, pos)],
                _ => [-1.0, 1.0],
            };
            let surround = snd.surround.as_ref().or(if snd.follow_main_pan { t.mixer.surround.as_ref() } else { None });
            let m = build_matrix(src_fmt, src.len(), dst.fmt, dst.buf.len(), &PanSpec { stereo, surround }, law);
            apply_matrix(src, frames, &mut dst.buf, &m, gain);
        }
        let src: &[Vec<f32>] = match &t.mixer.output {
            Route::Main | Route::Bus(_) if strip.out_extra.len > 0 => {
                if strip.outbuf.len() != strip.buf.len() {
                    strip.outbuf = vec![Vec::new(); strip.buf.len()];
                }
                for (d, src) in strip.outbuf.iter_mut().zip(strip.buf.iter()) {
                    d.clear();
                    d.extend_from_slice(src.get(..frames).unwrap_or(&[]));
                }
                strip.out_extra.process(&mut strip.outbuf, frames);
                &strip.outbuf
            }
            _ => &strip.buf,
        };
        let (dst, dst_fmt) = match &t.mixer.output {
            Route::Main => (&mut self.main, self.main_fmt),
            Route::Bus(b) => match self.busses.get_mut(b) {
                Some(d) => (&mut d.buf, d.fmt),
                None => return,
            },
            _ => return,
        };
        let mut start = 0;
        while start < frames {
            let (at, end) = pan_segment(t, pos, start, frames);
            let spec = PanSpec { stereo: [pan_at(t, 0, at), pan_at(t, 1, at)], surround: t.mixer.surround.as_ref() };
            let m = build_matrix(src_fmt, src.len(), dst_fmt, dst.len(), &spec, law);
            apply_matrix_span(src, start..end, dst, &m, 1.0);
            start = end;
        }
    }
}

struct LiveInput<'a> {
    input: &'a [Vec<f32>],
    midi_input: &'a [(usize, LiveMidiEvent)],
    routed: &'a [(TrackId, usize, [u8; 3])],
    monitor_only: bool,
    recording: bool,
}

fn process_strip(
    s: &Session,
    t: &Track,
    strip: &mut Strip,
    busses: &HashMap<BusId, BusBuf>,
    keys: Option<&HashMap<BusId, BusBuf>>,
    live: &LiveInput<'_>,
    pos: Samples,
    frames: usize,
    any_solo: bool,
) {
    strip.midi_out.clear();
    for c in strip.buf.iter_mut() {
        c.iter_mut().take(frames).for_each(|x| *x = 0.0);
    }
    let transport = transport_at(s, pos, !live.monitor_only, live.recording);
    match t.kind {
        TrackKind::Audio
            if matches!(t.mixer.input, Route::Bus(_)) && (t.mixer.input_monitor || (t.mixer.record_arm && (live.monitor_only || live.recording))) =>
        {
            bus_input(t, strip, busses, frames);
        }
        TrackKind::Audio if !live.input.is_empty() && (t.mixer.input_monitor || (t.mixer.record_arm && (live.monitor_only || live.recording))) => {
            let first = match &t.mixer.input {
                Route::Hardware(h) => s.input_first_channel(h),
                _ => 0,
            };
            let n_in = live.input.len().max(1);
            for (k, dst) in strip.buf.iter_mut().enumerate() {
                if let Some(src) = live.input.get((first + k) % n_in) {
                    for (d, x) in dst.iter_mut().zip(src.iter()).take(frames) {
                        *d = *x;
                    }
                }
            }
        }
        TrackKind::Audio if live.monitor_only => {}
        TrackKind::Audio => {
            let block = Range { start: pos, end: pos.saturating_add(frames as i64) };
            for clip in t.clips() {
                if clip.muted || !clip.is_audio() || !clip.range().overlaps(&block) {
                    continue;
                }
                match clip_fx_params(s, clip.id.0) {
                    None => render_clip(s, clip, pos, frames, &mut strip.buf, &mut strip.scratch),
                    Some(params) => {
                        let nch = strip.buf.len();
                        if strip.clipbuf.len() != nch || strip.clipbuf.first().map_or(0, Vec::len) < frames {
                            strip.clipbuf = vec![vec![0.0; frames.max(strip.scratch.len())]; nch];
                        }
                        for c in strip.clipbuf.iter_mut() {
                            c.iter_mut().take(frames).for_each(|x| *x = 0.0);
                        }
                        render_clip(s, clip, pos, frames, &mut strip.clipbuf, &mut strip.scratch);
                        let fx = strip.clip_fx.entry(clip.id.0).or_insert_with(|| ClipFx::new(s.sample_rate.as_f64() as f32, nch));
                        fx.configure(params);
                        fx.process(&mut strip.clipbuf, frames);
                        for (d, c) in strip.buf.iter_mut().zip(strip.clipbuf.iter()) {
                            for (x, y) in d.iter_mut().zip(c.iter()).take(frames) {
                                *x += *y;
                            }
                        }
                    }
                }
            }
        }
        TrackKind::Aux | TrackKind::Folder => {
            bus_input(t, strip, busses, frames);
        }
        TrackKind::Instrument | TrackKind::Midi => {
            let is_selected = s.edit.selected_tracks.contains(&t.id);
            let midi_target = t.mixer.record_arm || t.mixer.input_monitor || (s.edit.flag("options.midi_thru") && is_selected);

            if live.monitor_only && !midi_target {
                if let Some(inst) = strip.instrument.as_mut().filter(|i| !i.failed)
                    && live.midi_input.iter().any(|(_, ev)| ev.is_control_change() && (ev.cc_controller() == 120 || ev.cc_controller() == 123))
                {
                    inst.plugin.all_notes_off();
                    for (ch, p) in strip.held.iter() {
                        inst.plugin.note_off_chan(0, ch, p);
                    }
                    strip.held.clear();
                    if strip.buf.len() >= 2 {
                        inst.plugin.process(&mut strip.buf, frames);
                    }
                }
            } else if let Some(inst) = strip.instrument.as_mut().filter(|i| !i.failed) {
                if let Some(ins) = &t.instrument {
                    apply_params(inst, ins, t, INSTRUMENT_SLOT, pos);
                }
                inst.plugin.set_transport(&transport);
                if std::mem::take(&mut strip.chase) {
                    chase_controllers(s, t, pos, inst);
                }
                if !live.monitor_only {
                    schedule_notes(s, t, pos, frames, inst, &mut strip.held);
                }
                if midi_target {
                    if t.midi_from.is_some() {
                        for &(_, off, m) in live.routed.iter().filter(|r| r.0 == t.id) {
                            if t.matches_midi_channel(m[0] & 0x0F) {
                                deliver(inst, &mut strip.held, off, m);
                            }
                        }
                    }
                    for &(offset, ev) in live.midi_input.iter().filter(|_| t.midi_from.is_none()) {
                        if !t.matches_midi_channel(ev.channel()) {
                            continue;
                        }
                        let off = offset.min(frames.saturating_sub(1));
                        if ev.is_note_on() {
                            let pitch = ev.note_pitch();
                            inst.plugin.note_on_chan(off, ev.channel(), pitch, ev.note_velocity());
                            strip.held.insert(ev.channel(), pitch);
                        } else if ev.is_note_off() {
                            let pitch = ev.note_pitch();
                            inst.plugin.note_off_chan(off, ev.channel(), pitch);
                            strip.held.remove(ev.channel(), pitch);
                        } else {
                            if ev.is_control_change() && (ev.cc_controller() == 120 || ev.cc_controller() == 123) {
                                inst.plugin.all_notes_off();
                                for (ch, p) in strip.held.iter() {
                                    inst.plugin.note_off_chan(off, ch, p);
                                }
                                strip.held.clear();
                            }
                            inst.plugin.midi(off, ev.data);
                        }
                    }
                }
                if strip.buf.len() >= 2 {
                    inst.plugin.process(&mut strip.buf, frames);
                } else if let Some(c0) = strip.buf.first_mut() {
                    let mut st = [std::mem::take(c0), std::mem::take(&mut strip.scratch)];
                    inst.plugin.process(&mut st, frames);
                    let [a, b] = st;
                    *c0 = a;
                    strip.scratch = b;
                    for (x, y) in c0.iter_mut().zip(strip.scratch.iter()).take(frames) {
                        *x = 0.5 * (*x + *y);
                    }
                }
                let n_extra = inst.plugin.extra_outputs();
                strip.extras.resize_with(n_extra, Vec::new);
                for (b, out) in strip.extras.iter_mut().enumerate() {
                    let ch = inst.plugin.extra_output_channels(b);
                    out.resize_with(ch, Vec::new);
                    for (c, dst) in out.iter_mut().enumerate() {
                        dst.clear();
                        if let Some(src) = inst.plugin.extra_output(b, c) {
                            dst.extend_from_slice(src.get(..frames).unwrap_or(src));
                        }
                        dst.resize(frames, 0.0);
                    }
                }
                inst.plugin.take_midi_out(&mut strip.midi_out);
                if contain_failure(inst, &mut strip.buf, frames) {
                    strip.failures.record(INSTRUMENT_SLOT);
                }
            }
        }
        _ => {}
    }
    let trim = db_to_gain(t.mixer.trim_db) * if t.mixer.phase_invert { -1.0 } else { 1.0 };
    if (trim - 1.0).abs() > f32::EPSILON {
        for c in strip.buf.iter_mut() {
            c.iter_mut().take(frames).for_each(|x| *x *= trim);
        }
    }
    let heat = s.heat.active
        && !s.heat.bypass
        && !t.mixer.heat_bypass
        && matches!(t.kind, TrackKind::Audio | TrackKind::Instrument | TrackKind::Aux | TrackKind::Master);
    let sr = s.sample_rate.as_f64() as f32;
    if heat && t.mixer.heat_pre {
        strip.heat.process(&mut strip.buf, frames, s.heat.drive, s.heat.tone, sr);
    }
    let mut gr = 0.0f32;
    let mut latency = 0usize;
    let mut waited = false;
    let key_src = keys.unwrap_or(busses);
    for (i, ins) in t.mixer.inserts.iter().enumerate() {
        let (Some(ins), Some(Some(slot))) = (ins, strip.plugins.get_mut(i)) else { continue };
        if !ins.active || ins.bypass || slot.failed {
            continue;
        }
        apply_params(slot, ins, t, i, pos);
        slot.plugin.set_transport(&transport);
        let keyed = ins.keyed_bus();
        if keyed.is_some() && !waited && keys.is_some() {
            waited = true;
            let ch = strip.buf.len();
            strip.key_wait.set(strip.key_wait_len, ch);
            strip.key_wait.process(&mut strip.buf, frames);
            latency = latency.saturating_add(strip.key_wait_len);
        }
        let mut key = keyed.and_then(|b| key_src.get(&b)).map(|k| k.buf.as_slice());
        let lag = if keys.is_some() { strip.key_lags.get(i).copied().unwrap_or(0) } else { 0 };
        if lag > 0
            && let Some(k) = key
        {
            if strip.key_lag_lines.len() <= i {
                strip.key_lag_lines.resize_with(i + 1, Default::default);
            }
            if let Some((line, buf)) = strip.key_lag_lines.get_mut(i) {
                buf.resize_with(k.len(), Vec::new);
                for (d, src) in buf.iter_mut().zip(k) {
                    d.clear();
                    d.extend_from_slice(src.get(..frames).unwrap_or(src));
                }
                line.set(lag, k.len());
                line.process(buf, frames);
                key = Some(buf.as_slice());
            }
        }
        match key {
            Some(k) if slot.plugin.accepts_key() => slot.plugin.process_keyed(&mut strip.buf, k, frames),
            _ => slot.plugin.process(&mut strip.buf, frames),
        }
        slot.plugin.take_midi_out(&mut strip.midi_out);
        if contain_failure(slot, &mut strip.buf, frames) {
            strip.failures.record(i);
            continue;
        }
        if ins.key_listen
            && let Some(k) = key.filter(|k| !k.is_empty())
        {
            for (c, d) in strip.buf.iter_mut().enumerate() {
                let src = k.get(c % k.len()).map(Vec::as_slice).unwrap_or(&[]);
                for (n, x) in d.iter_mut().take(frames).enumerate() {
                    *x = src.get(n).copied().filter(|v| v.is_finite()).unwrap_or(0.0);
                }
            }
        }
        latency = latency.saturating_add(slot.plugin.latency());
        if gr == 0.0 {
            gr = slot.plugin.gain_reduction_db();
        }
    }
    if heat && !t.mixer.heat_pre {
        strip.heat.process(&mut strip.buf, frames, s.heat.drive, s.heat.tone, sr);
    }
    strip.gr = gr;
    strip.latency = latency.min(MAX_LATENCY);
    let auto_mute = t.mixer.automation_mode.reads() && t.lane(&AutoParam::Mute).is_some_and(|l| l.value_at(pos, 0.0) >= 0.5);
    let soloed_out = any_solo && !t.mixer.solo && !t.mixer.solo_safe && !receives_solo(s, t);
    strip.muted = t.mixer.mute || auto_mute || soloed_out;
    if t.mixer.sends.iter().flatten().any(|x| x.pre_fader) {
        strip.pre.resize_with(strip.buf.len(), Vec::new);
        for (d, src) in strip.pre.iter_mut().zip(strip.buf.iter()) {
            d.clear();
            d.extend_from_slice(src.get(..frames).unwrap_or(&[]));
        }
    }
    let last = pos.saturating_add(frames.saturating_sub(1) as i64);
    let mut v0 = volume_db_at(t, pos);
    let mut v1 = volume_db_at(t, last);
    if let Some(l) = t
        .automation
        .iter()
        .find(|l| matches!(&l.param, AutoParam::Plugin { slot: u8::MAX, param } if param == "trim"))
        .filter(|l| !l.points.is_empty())
    {
        v0 += l.value_at(pos, 0.0);
        v1 += l.value_at(last, 0.0);
    }
    if let Some(vid) = t.mixer.vca
        && let Some(vca) = s.tracks.iter().find(|x| x.id.0 == vid && x.kind == TrackKind::Vca)
    {
        v0 += volume_db_at(vca, pos);
        v1 += volume_db_at(vca, last);
        if vca.mixer.mute {
            strip.muted = true;
        }
    }
    let (g0, g1) = if strip.muted { (-144.0, -144.0) } else { (v0.min(12.0), v1.min(12.0)) };
    for c in strip.buf.iter_mut() {
        for i in 0..frames {
            if let Some(x) = c.get_mut(i) {
                *x *= lerp_gain(g0, g1, i, frames.saturating_sub(1));
            }
        }
    }
}

fn contain_failure(slot: &mut PluginSlot, buf: &mut [Vec<f32>], frames: usize) -> bool {
    if buf.iter().all(|c| c.get(..frames).unwrap_or(c).iter().fold(true, |ok, x| ok & x.is_finite())) {
        return false;
    }
    for c in buf.iter_mut() {
        c.iter_mut().take(frames).for_each(|x| *x = 0.0);
    }
    slot.failed = true;
    true
}

fn clip_fx_params(s: &Session, clip: u64) -> Option<impl Iterator<Item = (&str, f32)> + Clone> {
    if s.edit.values.is_empty() || Key::new(format_args!("clip_fx.bypass.{clip}")).is_none_or(|k| s.edit.flag(k.as_str())) {
        return None;
    }
    let pre = Key::new(format_args!("clip_fx.{clip}."))?;
    let mut params = s
        .edit
        .values
        .range::<str, _>((Bound::Included(pre.as_str()), Bound::Unbounded))
        .take_while(move |(k, _)| k.starts_with(pre.as_str()))
        .map(move |(k, v)| (k.get(pre.len..).unwrap_or(""), *v as f32))
        .peekable();
    params.peek()?;
    Some(params)
}

#[derive(Clone, Copy)]
struct Key {
    buf: [u8; 64],
    len: usize,
}

impl Key {
    fn new(args: std::fmt::Arguments<'_>) -> Option<Key> {
        let mut k = Key { buf: [0; 64], len: 0 };
        std::fmt::Write::write_fmt(&mut k, args).ok()?;
        Some(k)
    }

    fn as_str(&self) -> &str {
        self.buf.get(..self.len).and_then(|b| std::str::from_utf8(b).ok()).unwrap_or("")
    }
}

impl std::fmt::Write for Key {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let end = self.len.checked_add(s.len()).ok_or(std::fmt::Error)?;
        self.buf.get_mut(self.len..end).ok_or(std::fmt::Error)?.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

impl ClipFx {
    fn new(sr: f32, ch: usize) -> ClipFx {
        ClipFx { eq: None, comp: None, gain_db: 0.0, sr, ch }
    }
    fn make(&self, id: &str) -> Option<Box<dyn Plugin>> {
        openpt_dsp::create(id).map(|mut p| {
            p.prepare(self.sr, 16_384, self.ch.max(1));
            p
        })
    }
    fn configure<'a>(&mut self, params: impl Iterator<Item = (&'a str, f32)> + Clone) {
        self.gain_db = 0.0;
        let has_eq = params.clone().any(|(k, _)| k.starts_with("eq."));
        let has_comp = params.clone().any(|(k, _)| k.starts_with("comp."));
        if has_eq && self.eq.is_none() {
            self.eq = self.make("eq_7band");
        } else if !has_eq {
            self.eq = None;
        }
        if has_comp && self.comp.is_none() {
            self.comp = self.make("compressor");
        } else if !has_comp {
            self.comp = None;
        }
        for (k, v) in params {
            if let Some(p) = k.strip_prefix("eq.") {
                if let Some(eq) = &mut self.eq
                    && eq.param(p) != Some(v)
                {
                    eq.set_param(p, v);
                }
            } else if let Some(p) = k.strip_prefix("comp.") {
                if let Some(c) = &mut self.comp
                    && c.param(p) != Some(v)
                {
                    c.set_param(p, v);
                }
            } else if k == "gain" {
                self.gain_db = v;
            }
        }
    }
    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        if let Some(eq) = &mut self.eq {
            eq.process(io, frames);
        }
        if let Some(c) = &mut self.comp {
            c.process(io, frames);
        }
        if self.gain_db.abs() > 1e-6 {
            let g = db_to_gain(self.gain_db);
            for ch in io.iter_mut() {
                ch.iter_mut().take(frames).for_each(|x| *x *= g);
            }
        }
    }
}

fn structure_fingerprint(s: &Session) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for t in &s.tracks {
        t.id.hash(&mut h);
        t.kind.hash(&mut h);
        t.format.hash(&mut h);
        t.inactive.hash(&mut h);
        t.mixer.inserts.len().hash(&mut h);
        t.mixer.input.hash(&mut h);
        t.mixer.output.hash(&mut h);
        for i in &t.mixer.inserts {
            i.as_ref().map(|x| (x.plugin.as_str(), x.multi_mono, x.multi_out, &x.outputs)).hash(&mut h);
        }
        for snd in t.mixer.sends.iter().flatten() {
            snd.target.hash(&mut h);
        }
        t.instrument.as_ref().map(|x| (x.plugin.as_str(), x.multi_out, &x.outputs)).hash(&mut h);
        let sample = t.instrument.as_ref().and_then(|x| x.sample);
        sample.hash(&mut h);
        sample.and_then(|id| s.pool.get(id)).map(|a| Arc::as_ptr(a) as usize).hash(&mut h);
    }
    for b in &s.busses {
        b.id.hash(&mut h);
        b.format.hash(&mut h);
    }
    s.main_format().hash(&mut h);
    h.finish() as usize
}

std::thread_local! {
    static AUDIO_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn mark_audio_thread() {
    AUDIO_THREAD.set(true);
}

pub fn on_audio_thread() -> bool {
    AUDIO_THREAD.get()
}

#[cfg(not(target_arch = "wasm32"))]
fn as_audio_thread(audio: bool, f: impl FnOnce()) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            AUDIO_THREAD.set(self.0);
        }
    }
    let _restore = Restore(AUDIO_THREAD.replace(audio));
    f();
}

#[cfg(not(target_arch = "wasm32"))]
fn run_strips<T: Send>(work: &mut [T], f: impl Fn(&mut T) + Sync + Send) {
    use rayon::prelude::*;
    if work.len() >= 4 {
        let audio = on_audio_thread();
        work.par_iter_mut().for_each(|w| as_audio_thread(audio, || f(w)));
    } else {
        work.iter_mut().for_each(f);
    }
}

#[cfg(target_arch = "wasm32")]
fn run_strips<T: Send>(work: &mut [T], f: impl Fn(&mut T) + Sync + Send) {
    work.iter_mut().for_each(f);
}

fn strip_channels(t: &Track, main_ch: usize) -> usize {
    match t.kind {
        TrackKind::Master => main_ch,
        TrackKind::Instrument => t.channels().clamp(2, MAX_CHANNELS),
        _ => t.channels().clamp(1, MAX_CHANNELS),
    }
}

fn strip_format(t: &Track, n: usize) -> ChannelFormat {
    if t.format.channels().min(MAX_CHANNELS) == n { t.format } else { ChannelFormat::for_channels(n) }
}

fn plan_keys(t: &Track, strip: &mut Strip, pos: Samples, key_latency: &HashMap<BusId, usize>) {
    let mut acc = 0usize;
    let mut keyed: Vec<(usize, usize, usize)> = Vec::new();
    for (i, ins) in t.mixer.inserts.iter().enumerate() {
        let (Some(ins), Some(Some(slot))) = (ins, strip.plugins.get_mut(i)) else { continue };
        if !ins.active || ins.bypass || slot.failed {
            continue;
        }
        apply_params(slot, ins, t, i, pos);
        if let Some(b) = ins.keyed_bus() {
            keyed.push((i, acc, key_latency.get(&b).copied().unwrap_or(0)));
        }
        acc = acc.saturating_add(slot.plugin.latency());
    }
    let wait = keyed.iter().map(|(_, at, lk)| lk.saturating_sub(*at)).max().unwrap_or(0).min(MAX_LATENCY);
    strip.key_wait_len = wait;
    strip.key_lags.clear();
    strip.key_lags.resize(t.mixer.inserts.len(), 0);
    for (i, at, lk) in keyed {
        if let Some(l) = strip.key_lags.get_mut(i) {
            *l = (at + wait).saturating_sub(lk).min(MAX_LATENCY);
        }
    }
    strip.latency = acc.saturating_add(wait).min(MAX_LATENCY);
}

fn feed_keys(
    t: &Track,
    st: &mut Strip,
    keys: &mut HashMap<BusId, BusBuf>,
    key_latency: &HashMap<BusId, usize>,
    pos: Samples,
    frames: usize,
    law: PanLaw,
) {
    if st.muted || keys.is_empty() {
        return;
    }
    let src_fmt = strip_format(t, st.buf.len());
    let mono = st.buf.len() == 1;
    let mut feeds: Vec<(BusId, usize, bool, f32, [f32; 2], Option<&openpt_model::SurroundPan>)> = Vec::new();
    if let Route::Bus(b) = &t.mixer.output
        && keys.contains_key(b)
    {
        feeds.push((*b, 0, false, 1.0, [pan_at(t, 0, pos), pan_at(t, 1, pos)], t.mixer.surround.as_ref()));
    }
    for (i, snd) in t.mixer.sends.iter().enumerate() {
        let Some(snd) = snd else { continue };
        let Route::Bus(b) = &snd.target else { continue };
        if snd.mute || !keys.contains_key(b) {
            continue;
        }
        let gain = db_to_gain(send_level(t, i, snd.level_db, pos));
        let stereo = match (mono, snd.follow_main_pan) {
            (true, true) => [pan_at(t, 0, pos), 0.0],
            (true, false) => [snd.pan, 0.0],
            (false, true) => [pan_at(t, 0, pos), pan_at(t, 1, pos)],
            _ => [-1.0, 1.0],
        };
        let surround = snd.surround.as_ref().or(if snd.follow_main_pan { t.mixer.surround.as_ref() } else { None });
        feeds.push((*b, i + 1, snd.pre_fader, gain, stereo, surround));
    }
    for (b, which, pre, gain, stereo, surround) in feeds {
        if gain <= 0.0 {
            continue;
        }
        let d = key_latency.get(&b).copied().unwrap_or(0).saturating_sub(st.latency);
        let k = match st.key_feed.iter().position(|f| f.0 == b && f.1 == which) {
            Some(k) => k,
            None => {
                st.key_feed.push((b, which, DelayLine::default(), Vec::new()));
                st.key_feed.len() - 1
            }
        };
        let src = if pre { &st.pre } else { &st.buf };
        let Some((_, _, line, buf)) = st.key_feed.get_mut(k) else { continue };
        buf.resize_with(src.len(), Vec::new);
        for (dst, s) in buf.iter_mut().zip(src.iter()) {
            dst.clear();
            dst.extend_from_slice(s.get(..frames).unwrap_or(s));
        }
        line.set(d, buf.len());
        line.process(buf, frames);
        let Some(dst) = keys.get_mut(&b) else { continue };
        let m = build_matrix(src_fmt, buf.len(), dst.fmt, dst.buf.len(), &PanSpec { stereo, surround }, law);
        apply_matrix(buf, frames, &mut dst.buf, &m, gain);
    }
}

fn key_busses(t: &Track) -> impl Iterator<Item = BusId> + '_ {
    t.mixer.inserts.iter().flatten().filter_map(openpt_model::Insert::keyed_bus)
}

fn feeds_bus(t: &Track, b: BusId) -> bool {
    let r = Route::Bus(b);
    t.mixer.output == r || t.mixer.sends.iter().flatten().any(|sn| sn.target == r)
}

fn order_keyed<T>(s: &Session, work: &mut [(usize, T)]) -> usize {
    let keyed = |i: usize| s.tracks.get(i).is_some_and(|t| key_busses(t).next().is_some());
    work.sort_by_key(|(i, _)| keyed(*i));
    let n_free = work.iter().take_while(|(i, _)| !keyed(*i)).count();
    let n = work.len();
    for k in n_free..n {
        let waits_on_later = |at: usize| {
            let Some(t) = work.get(at).and_then(|(i, _)| s.tracks.get(*i)) else { return false };
            key_busses(t).any(|b| {
                work.iter()
                    .skip(k)
                    .filter(|(j, _)| s.tracks.get(*j).is_some_and(|u| u.id != t.id))
                    .any(|(j, _)| s.tracks.get(*j).is_some_and(|u| feeds_bus(u, b)))
            })
        };
        if let Some(ready) = (k..n).find(|&at| !waits_on_later(at))
            && let Some(w) = work.get_mut(k..=ready)
        {
            w.rotate_right(1);
        }
    }
    n_free
}

fn reads_bus(t: &Track) -> bool {
    matches!(t.kind, TrackKind::Aux | TrackKind::Folder) || (t.kind == TrackKind::Audio && matches!(t.mixer.input, Route::Bus(_)))
}

fn bus_input(t: &Track, strip: &mut Strip, busses: &HashMap<BusId, BusBuf>, frames: usize) {
    let Route::Bus(b) = &t.mixer.input else { return };
    let Some(src) = busses.get(b) else { return };
    let (ns, nd) = (src.buf.len(), strip.buf.len());
    let dst_fmt = strip_format(t, nd);
    let m = if src.fmt == dst_fmt && ns == nd {
        identity_matrix(ns)
    } else if ns == 1 && nd == 2 {
        let mut m = [[0.0; MAX_CHANNELS]; MAX_CHANNELS];
        m[0][0] = 1.0;
        m[0][1] = 1.0;
        m
    } else {
        let stereo = if ns == 1 { [0.0, 0.0] } else { [-1.0, 1.0] };
        build_matrix(src.fmt, ns, dst_fmt, nd, &PanSpec { stereo, surround: None }, PanLaw::Minus3)
    };
    apply_matrix(&src.buf, frames, &mut strip.buf, &m, 1.0);
}

fn processing_order(s: &Session) -> Vec<usize> {
    let mut order: Vec<usize> = (0..s.tracks.len()).filter(|&i| s.tracks.get(i).is_some_and(|t| !reads_bus(t))).collect();
    let mut auxes: Vec<usize> = (0..s.tracks.len()).filter(|&i| s.tracks.get(i).is_some_and(reads_bus)).collect();
    let depth = |i: usize| -> usize {
        let mut d = 0;
        let mut cur = vec![i];
        for _ in 0..8 {
            let keys: Vec<Route> = cur.iter().filter_map(|&k| s.tracks.get(k)).flat_map(key_busses).map(Route::Bus).collect();
            let inputs: Vec<&Route> = cur.iter().filter_map(|&k| s.tracks.get(k)).map(|t| &t.mixer.input).chain(keys.iter()).collect();
            let feeders: Vec<usize> = auxes_feeding(s, &inputs);
            if feeders.is_empty() {
                break;
            }
            d += 1;
            cur = feeders;
        }
        d
    };
    auxes.sort_by_key(|&i| depth(i));
    order.extend(auxes);
    order
}

fn auxes_feeding(s: &Session, inputs: &[&Route]) -> Vec<usize> {
    s.tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| reads_bus(t))
        .filter(|(_, t)| {
            inputs.iter().any(|r| matches!(r, Route::Bus(_)) && (&&t.mixer.output == r || t.mixer.sends.iter().flatten().any(|sn| &&sn.target == r)))
        })
        .map(|(i, _)| i)
        .collect()
}

fn receives_solo(s: &Session, t: &Track) -> bool {
    let Route::Bus(b) = &t.mixer.input else { return false };
    s.tracks
        .iter()
        .any(|o| o.mixer.solo && (o.mixer.output == Route::Bus(*b) || o.mixer.sends.iter().flatten().any(|sn| sn.target == Route::Bus(*b))))
}

mod click {
    use super::*;

    const LEN_S: f64 = 0.03;

    pub fn render(s: &Session, pos: Samples, frames: usize, main: &mut [Vec<f32>]) {
        let sr = s.sample_rate;
        let rate = sr.as_f64();
        let len = (LEN_S * rate) as i64;
        let end = pos.saturating_add(frames as i64);
        let gain = f64::from(db_to_gain(s.edit.value("click.volume_db", -6.0).clamp(-60.0, 12.0) as f32));
        let accent = (note_hz(s.edit.value("click.accent_note", 84.0)), s.edit.value("click.accent_velocity", 127.0));
        let normal = (note_hz(s.edit.value("click.normal_note", 72.0)), s.edit.value("click.normal_velocity", 100.0));
        let t0 = s.tempo.samples_to_ticks(pos.saturating_sub(len), sr).max(0).saturating_sub(1);
        let mut tick = t0.saturating_sub(s.tempo.bar_beat_at_tick(t0).tick);
        for _ in 0..4096 {
            let at = s.tempo.tick_to_samples(tick, sr);
            if at >= end {
                break;
            }
            if at.saturating_add(len) > pos {
                let (hz, vel) = if s.tempo.bar_beat_at_tick(tick).beat == 1 { accent } else { normal };
                let amp = gain * (vel.clamp(1.0, 127.0) / 127.0);
                blip(main, at - pos, frames, hz, amp as f32, rate);
            }
            tick = tick.saturating_add(s.tempo.meter_at_tick(tick).ticks_per_beat());
        }
    }

    fn blip(main: &mut [Vec<f32>], rel: i64, frames: usize, hz: f64, amp: f32, rate: f64) {
        let len = (LEN_S * rate) as usize;
        let w = std::f64::consts::TAU * hz / rate;
        let first = usize::try_from(-rel).unwrap_or(0);
        for i in first..len {
            let Ok(at) = usize::try_from(rel + i as i64) else { continue };
            if at >= frames {
                break;
            }
            let env = (-6.0 * i as f64 / len as f64).exp() as f32;
            let v = ((w * i as f64).sin() as f32) * env * amp;
            for ch in main.iter_mut() {
                if let Some(d) = ch.get_mut(at) {
                    *d += v;
                }
            }
        }
    }

    fn note_hz(note: f64) -> f64 {
        openpt_dsp::midi_to_hz(note.clamp(0.0, 127.0))
    }
}

fn render_clip(s: &Session, clip: &Clip, pos: Samples, frames: usize, buf: &mut [Vec<f32>], _scratch: &mut [f32]) {
    let ClipContent::Audio { source, offset } = clip.content else { return };
    let block = Range { start: pos, end: pos.saturating_add(frames as i64) };
    let Some(isect) = clip.range().intersect(&block) else { return };
    let Some(audio) = s.pool.get(source) else { return };
    let nsrc = audio.buffer.channels.len();
    if nsrc == 0 {
        return;
    }
    let stretch = if clip.stretch.is_finite() && clip.stretch > 0.0 { clip.stretch } else { 1.0 };
    let i0 = usize::try_from(isect.start - pos).unwrap_or(0);
    let i1 = usize::try_from(isect.end - pos).unwrap_or(0).min(frames);
    const STEP: usize = 32;
    for (ch, dst) in buf.iter_mut().enumerate() {
        let src = audio.buffer.channels.get(ch % nsrc).map(Vec::as_slice).unwrap_or(&[]);
        let mut i = i0;
        while i < i1 {
            let seg_end = (i + STEP).min(i1);
            let rel0 = pos.saturating_add(i as i64).saturating_sub(clip.start);
            let rel1 = pos.saturating_add(seg_end.saturating_sub(1) as i64).saturating_sub(clip.start);
            let g0 = clip.gain_at(rel0);
            let g1 = clip.gain_at(rel1.min(clip.length));
            let n = (seg_end - i).saturating_sub(1).max(1) as f32;
            for k in i..seg_end {
                let rel = pos.saturating_add(k as i64).saturating_sub(clip.start);
                let v = if (stretch - 1.0).abs() < 1e-9 {
                    usize::try_from(offset.saturating_add(rel)).ok().and_then(|x| src.get(x)).copied().unwrap_or(0.0)
                } else {
                    let f = offset as f64 + rel as f64 / stretch;
                    let x0 = f.floor();
                    let fr = (f - x0) as f32;
                    let a = if x0 >= 0.0 { src.get(x0 as usize).copied().unwrap_or(0.0) } else { 0.0 };
                    let b = if x0 + 1.0 >= 0.0 { src.get((x0 as usize).saturating_add(1)).copied().unwrap_or(0.0) } else { 0.0 };
                    a + (b - a) * fr
                };
                let t = (k - i) as f32 / n;
                if let Some(d) = dst.get_mut(k) {
                    *d += v * (g0 + (g1 - g0) * t);
                }
            }
            i = seg_end;
        }
    }
}

fn midi_offset(s: &Session, t: &Track) -> i64 {
    const PREFIX: &str = "midi.offset.";
    s.edit
        .values
        .range::<str, _>((std::ops::Bound::Included(PREFIX), std::ops::Bound::Unbounded))
        .take_while(|(k, _)| k.starts_with(PREFIX))
        .find(|(k, _)| k.get(PREFIX.len()..).and_then(|id| id.parse::<u64>().ok()) == Some(t.id.0))
        .map(|(_, v)| *v)
        .filter(|v| v.is_finite())
        .map_or(0, |v| v as i64)
}

fn schedule_notes(s: &Session, t: &Track, pos: Samples, frames: usize, inst: &mut PluginSlot, held: &mut HeldNotes) {
    track_events(s, t, pos, frames, |off, m| deliver(inst, held, off, m));
}

fn deliver(inst: &mut PluginSlot, held: &mut HeldNotes, off: usize, m: [u8; 3]) {
    let ch = m[0] & 0x0F;
    match m[0] & 0xF0 {
        0x90 if m[2] > 0 => {
            inst.plugin.note_on_chan(off, ch, m[1], m[2]);
            held.insert(ch, m[1]);
        }
        0x80 | 0x90 => {
            inst.plugin.note_off_chan(off, ch, m[1]);
            held.remove(ch, m[1]);
        }
        _ => inst.plugin.midi(off, m),
    }
}

fn track_events(s: &Session, t: &Track, pos: Samples, frames: usize, mut emit: impl FnMut(usize, [u8; 3])) {
    let sr = s.sample_rate;
    let offset = midi_offset(s, t);
    let pos = pos.saturating_sub(offset);
    let end = pos.saturating_add(frames as i64);
    for clip in t.clips() {
        if clip.muted {
            continue;
        }
        let ClipContent::Midi { sequence } = &clip.content else { continue };
        if !clip.range().overlaps(&Range { start: pos, end }) {
            continue;
        }
        let base = s.tempo.samples_to_ticks(clip.start, sr);
        let rtp = |name: &str, d: f64| Key::new(format_args!("rtp.{}.{name}", t.id.0)).map_or(d, |k| s.edit.value(k.as_str(), d));
        let (dv, dp, dur, delay) = if s.edit.values.is_empty() {
            (0i64, 0i64, 1.0f64, 0i64)
        } else {
            (
                rtp("velocity", 0.0).clamp(-127.0, 127.0) as i64,
                rtp("transpose", 0.0).clamp(-127.0, 127.0) as i64,
                rtp("duration", 100.0).clamp(1.0, 1000.0) / 100.0,
                rtp("delay", 0.0).clamp(-1e7, 1e7) as i64,
            )
        };
        for n0 in &sequence.notes {
            let mut n = *n0;
            n.velocity = (i64::from(n.velocity) + dv).clamp(1, 127) as u8;
            n.pitch = (i64::from(n.pitch) + dp).clamp(0, 127) as u8;
            n.length = ((n.length as f64 * dur) as i64).max(1);
            n.start = n.start.saturating_add(delay).max(0);
            let on = s.tempo.tick_to_samples(base.saturating_add(n.start), sr);
            let off = s.tempo.tick_to_samples(base.saturating_add(n.start).saturating_add(n.length), sr).min(clip.end());
            if on >= clip.end() {
                continue;
            }
            if on >= pos && on < end {
                emit(usize::try_from(on - pos).unwrap_or(0), [0x90 | (n.channel & 0x0F), n.pitch, n.velocity]);
            }
            if off >= pos && off < end {
                emit(usize::try_from(off - pos).unwrap_or(0), [0x80 | (n.channel & 0x0F), n.pitch, 0]);
            }
        }
        let near = s.tempo.samples_to_ticks(pos, sr).saturating_sub(base).saturating_sub(1)
            ..=s.tempo.samples_to_ticks(end, sr).saturating_sub(base).saturating_add(1);
        for c in sequence.ctrls.iter().filter(|c| c.tick >= 0) {
            let tick = c.tick.saturating_add(delay).max(0);
            if !near.contains(&tick) {
                continue;
            }
            let at = s.tempo.tick_to_samples(base.saturating_add(tick), sr);
            if at >= pos && at < end && at < clip.end() {
                emit(usize::try_from(at - pos).unwrap_or(0), ctrl_message(c.kind, c.value, dp));
            }
        }
    }
}

fn ctrl_message(kind: CtrlKind, value: i32, transpose: i64) -> [u8; 3] {
    let v7 = value.clamp(0, 127) as u8;
    match kind {
        CtrlKind::Cc(n) => [0xB0, n & 0x7F, v7],
        CtrlKind::PitchBend => {
            let v = value.saturating_add(8192).clamp(0, 16383);
            [0xE0, (v & 0x7F) as u8, (v >> 7) as u8]
        }
        CtrlKind::ChannelPressure => [0xD0, v7, 0],
        CtrlKind::PolyPressure(k) => [0xA0, (i64::from(k) + transpose).clamp(0, 127) as u8, v7],
        CtrlKind::Program => [0xC0, v7, 0],
    }
}

fn chase_controllers(s: &Session, t: &Track, pos: Samples, inst: &mut PluginSlot) {
    inst.plugin.midi(0, [0xB0, 121, 0]);
    let pos = pos.saturating_sub(midi_offset(s, t));
    const BEND: usize = 128;
    const PRESSURE: usize = 129;
    const PROGRAM: usize = 130;
    let mut last = [(i64::MIN, 0i32); 131];
    let sr = s.sample_rate;
    for clip in t.clips() {
        let ClipContent::Midi { sequence } = &clip.content else { continue };
        if clip.muted || clip.start >= pos {
            continue;
        }
        let base = s.tempo.samples_to_ticks(clip.start, sr);
        let until = pos.min(clip.end());
        for c in sequence.ctrls.iter().filter(|c| c.tick >= 0) {
            let slot = match c.kind {
                CtrlKind::Cc(n) => usize::from(n & 0x7F),
                CtrlKind::PitchBend => BEND,
                CtrlKind::ChannelPressure => PRESSURE,
                CtrlKind::Program => PROGRAM,
                CtrlKind::PolyPressure(_) => continue,
            };
            let at = s.tempo.tick_to_samples(base.saturating_add(c.tick), sr);
            if at < until
                && let Some(l) = last.get_mut(slot)
                && at >= l.0
            {
                *l = (at, c.value);
            }
        }
    }
    for (slot, &(at, value)) in last.iter().enumerate() {
        let kind = match slot {
            BEND => CtrlKind::PitchBend,
            PRESSURE => CtrlKind::ChannelPressure,
            PROGRAM => CtrlKind::Program,
            n => CtrlKind::Cc(u8::try_from(n).unwrap_or(0)),
        };
        if at != i64::MIN {
            inst.plugin.midi(0, ctrl_message(kind, value, 0));
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AraRequest {
    Start,
    Stop,
    Locate(f64),
    Cycle(f64, f64),
    EnableCycle(bool),
}

pub fn ara_select_clips(clips: &[openpt_model::ClipId]) {
    #[cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]
    openpt_vst3_host::ara_select_clips(&clips.iter().map(|c| c.0).collect::<Vec<_>>());
    #[cfg(not(all(not(target_arch = "wasm32"), target_arch = "x86_64")))]
    let _ = clips;
}

pub fn ara_bindings() -> Vec<(TrackId, String)> {
    #[cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]
    {
        openpt_vst3_host::ara_bindings().into_iter().map(|(t, n)| (TrackId(t), n)).collect()
    }
    #[cfg(not(all(not(target_arch = "wasm32"), target_arch = "x86_64")))]
    {
        Vec::new()
    }
}

pub fn ara_requests() -> Vec<AraRequest> {
    #[cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]
    {
        use openpt_vst3_host::PlaybackRequest as P;
        openpt_vst3_host::ara_playback_requests()
            .into_iter()
            .map(|r| match r {
                P::Start => AraRequest::Start,
                P::Stop => AraRequest::Stop,
                P::Locate(t) => AraRequest::Locate(t),
                P::Cycle(a, d) => AraRequest::Cycle(a, d),
                P::EnableCycle(on) => AraRequest::EnableCycle(on),
            })
            .collect()
    }
    #[cfg(not(all(not(target_arch = "wasm32"), target_arch = "x86_64")))]
    {
        Vec::new()
    }
}

pub fn ara_follow(s: &Session, seen: &mut std::collections::HashMap<u64, u64>) {
    #[cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]
    for t in openpt_vst3_host::ara_bound_tracks() {
        let Some(at) = ara_track(s, TrackId(t)) else { continue };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for c in &at.clips {
            std::hash::Hash::hash(&(c.clip, c.source, c.offset, c.length, c.start), &mut h);
        }
        let fp = std::hash::Hasher::finish(&h);
        if seen.insert(t, fp) != Some(fp) {
            openpt_vst3_host::ara_sync_track(&at);
        }
    }
    #[cfg(all(not(target_arch = "wasm32"), target_arch = "x86_64"))]
    openpt_vst3_host::ara_finish_restores();
    #[cfg(not(all(not(target_arch = "wasm32"), target_arch = "x86_64")))]
    let _ = (s, seen);
}

pub fn ara_track(s: &Session, t: TrackId) -> Option<openpt_dsp::AraTrack> {
    let tr = s.track(t).filter(|tr| tr.kind.has_playlist() && !tr.kind.is_midi())?;
    let clips = tr
        .clips()
        .iter()
        .filter_map(|c| {
            let src = c.source()?;
            let audio = s.pool.get(src)?;
            Some(openpt_dsp::AraClip {
                clip: c.id.0,
                name: c.name.clone(),
                source: src.0,
                source_name: s.source(src).map_or_else(|| c.name.clone(), |x| x.name.clone()),
                audio: Arc::new(PoolSample(Arc::clone(audio))),
                offset: c.source_offset(),
                length: c.length,
                start: c.start,
            })
        })
        .collect();
    Some(openpt_dsp::AraTrack { track: t.0, name: tr.name.clone(), sample_rate: s.sample_rate.as_f64(), clips })
}

struct PoolSample(Arc<SourceAudio>);

impl SampleSource for PoolSample {
    fn sample_rate(&self) -> f32 {
        self.0.buffer.sample_rate as f32
    }
    fn num_channels(&self) -> usize {
        self.0.buffer.num_channels()
    }
    fn channel(&self, ch: usize) -> Option<&[f32]> {
        self.0.channel(ch)
    }
}

fn sync_sample(strip: &mut Strip, want: Option<&openpt_model::Insert>, s: &Session) {
    let audio = want.and_then(|i| i.sample).and_then(|id| s.pool.get(id));
    let same = match (strip.sample.as_ref(), audio) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    };
    if same {
        return;
    }
    if let Some(inst) = strip.instrument.as_mut() {
        let shared = audio.map(|a| Arc::new(PoolSample(Arc::clone(a))) as Arc<dyn SampleSource>);
        inst.plugin.set_sample(shared);
    }
    strip.sample = audio.cloned();
}

fn note_insert_latency(t: &Track, strip: &mut Strip, pos: Samples) {
    let mut latency = 0usize;
    for (i, ins) in t.mixer.inserts.iter().enumerate() {
        let (Some(ins), Some(Some(slot))) = (ins, strip.plugins.get_mut(i)) else { continue };
        if !ins.active || ins.bypass {
            continue;
        }
        apply_params(slot, ins, t, i, pos);
        latency = latency.saturating_add(slot.plugin.latency());
    }
    strip.latency = latency.min(MAX_LATENCY);
}

fn apply_params(slot: &mut PluginSlot, ins: &openpt_model::Insert, t: &Track, i: usize, pos: Samples) {
    let Ok(slot_u8) = u8::try_from(i) else {
        return apply_values(slot, ins, |_, v| v);
    };
    let automated = t.mixer.automation_mode.reads()
        && t.automation.iter().any(|l| matches!(&l.param, AutoParam::Plugin { slot, .. } if *slot == slot_u8) && !l.points.is_empty());
    apply_values(slot, ins, |k, val| {
        if automated
            && let Some(l) = t.automation.iter().find(|l| matches!(&l.param, AutoParam::Plugin { slot, param } if *slot == slot_u8 && param == k))
            && !l.points.is_empty()
        {
            return l.value_at(pos, val);
        }
        val
    });
    if !automated {
        return;
    }
    for l in &t.automation {
        let AutoParam::Plugin { slot: s, param } = &l.param else { continue };
        if *s != slot_u8 || l.points.is_empty() || ins.params.contains_key(param) {
            continue;
        }
        let Some(pi) = slot.plugin.info().param(param) else { continue };
        let v = l.value_at(pos, pi.default);
        if slot.plugin.param(param).is_none_or(|cur| cur.to_bits() != v.to_bits()) {
            slot.plugin.set_param(param, v);
        }
    }
}

fn apply_values(slot: &mut PluginSlot, ins: &openpt_model::Insert, value: impl Fn(&str, f32) -> f32) {
    if slot.applied.len() != ins.params.len() {
        slot.applied = vec![f32::NAN; ins.params.len()];
    }
    for ((k, v), last) in ins.params.iter().zip(slot.applied.iter_mut()) {
        let val = value(k, *v);
        if val.to_bits() != last.to_bits() {
            slot.plugin.set_param(k, val);
            *last = val;
        }
    }
}

fn transport_at(s: &Session, pos: Samples, playing: bool, recording: bool) -> Transport {
    let sr = s.sample_rate;
    let ticks = Some(s.tempo.samples_to_ticks_f(pos, sr)).filter(|t| t.is_finite()).unwrap_or(0.0);
    let tick = ticks.floor() as i64;
    let meter = s.tempo.meter_at_tick(tick);
    let bar = s.tempo.bar_beat_at_tick(tick).bar;
    let q = TICKS_PER_QUARTER as f64;
    Transport {
        playing,
        recording,
        tempo: s.tempo.tempo_at_tick(tick),
        numerator: meter.numerator,
        denominator: meter.denominator,
        samples: pos,
        seconds: pos as f64 / sr.as_f64().max(1.0),
        quarters: ticks / q,
        bar_start: s.tempo.bar_start_tick(bar) as f64 / q,
        bar: bar - 1,
    }
}

pub fn volume_db_at(t: &Track, pos: Samples) -> f32 {
    if t.mixer.automation_mode.reads()
        && t.mixer.automation_mode != openpt_model::AutomationMode::Write
        && let Some(l) = t.lane(&AutoParam::Volume)
        && !l.points.is_empty()
    {
        return l.value_at(pos, t.mixer.volume_db);
    }
    t.mixer.volume_db
}

pub fn pan_at(t: &Track, idx: usize, pos: Samples) -> f32 {
    let base = t.mixer.pan.get(idx).copied().unwrap_or(if idx == 0 { 0.0 } else { 1.0 });
    if t.mixer.automation_mode.reads()
        && let Some(l) = t.lane(&AutoParam::Pan(u8::try_from(idx).unwrap_or(0)))
        && !l.points.is_empty()
    {
        return l.value_at(pos, base);
    }
    base
}

const PAN_STEP: Samples = 64;

fn pan_segment(t: &Track, pos: Samples, start: usize, frames: usize) -> (Samples, usize) {
    let automated = t.mixer.automation_mode.reads() && t.automation.iter().any(|l| matches!(l.param, AutoParam::Pan(_)) && !l.points.is_empty());
    if !automated {
        return (pos, frames);
    }
    let here = pos.saturating_add(start as i64);
    let at = here.div_euclid(PAN_STEP).saturating_mul(PAN_STEP);
    let left = usize::try_from(at.saturating_add(PAN_STEP).saturating_sub(here)).unwrap_or(1).max(1);
    (at, start.saturating_add(left).min(frames))
}

fn send_level(t: &Track, i: usize, base: f32, pos: Samples) -> f32 {
    if t.mixer.automation_mode.reads()
        && let Some(l) = t.lane(&AutoParam::SendLevel(u8::try_from(i).unwrap_or(0)))
        && !l.points.is_empty()
    {
        return l.value_at(pos, base);
    }
    base
}

fn lerp_gain(db0: f32, db1: f32, i: usize, n: usize) -> f32 {
    let g0 = db_to_gain(db0);
    let g1 = db_to_gain(db1);
    if n == 0 { g0 } else { g0 + (g1 - g0) * (i as f32 / n as f32) }
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| if v.is_finite() { m.max(v.abs()) } else { m })
}

type Matrix = [[f32; MAX_CHANNELS]; MAX_CHANNELS];

struct PanSpec<'a> {
    stereo: [f32; 2],
    surround: Option<&'a SurroundPan>,
}

fn identity_matrix(n: usize) -> Matrix {
    let mut m = [[0.0; MAX_CHANNELS]; MAX_CHANNELS];
    for (i, row) in m.iter_mut().enumerate().take(n) {
        if let Some(g) = row.get_mut(i) {
            *g = 1.0;
        }
    }
    m
}

pub fn itu_stereo_fold(sp: Speaker) -> (f32, f32) {
    use Speaker::*;
    let h = std::f32::consts::FRAC_1_SQRT_2;
    match sp {
        L | Lc | Lw => (1.0, 0.0),
        R | Rc | Rw => (0.0, 1.0),
        C => (h, h),
        Lfe => (0.0, 0.0),
        Cs => (0.5, 0.5),
        Ls | Lss | Lrs | Ltf | Ltm | Ltr => (h, 0.0),
        Rs | Rss | Rrs | Rtf | Rtm | Rtr => (0.0, h),
    }
}

fn layout_of(fmt: ChannelFormat, out: &mut [SpeakerPos; MAX_CHANNELS]) -> usize {
    let sp = fmt.speakers();
    for (o, s) in out.iter_mut().zip(sp.iter()) {
        *o = SpeakerPos { az: s.azimuth(), el: s.elevation(), lfe: s.is_lfe() };
    }
    sp.len().min(MAX_CHANNELS)
}

fn surround_params(sp: &SurroundPan, dx: f32) -> SurroundParams {
    SurroundParams { x: (sp.x + dx).clamp(-1.0, 1.0), y: sp.y, z: sp.z, divergence: sp.divergence, center: sp.center / 100.0 }
}

fn build_matrix(src_fmt: ChannelFormat, ns: usize, dst_fmt: ChannelFormat, nd: usize, pan: &PanSpec<'_>, law: PanLaw) -> Matrix {
    let mut m = [[0.0f32; MAX_CHANNELS]; MAX_CHANNELS];
    let (ns, nd) = (ns.min(MAX_CHANNELS), nd.min(MAX_CHANNELS));
    if ns == 0 || nd == 0 {
        return m;
    }
    let set = |m: &mut Matrix, s: usize, d: usize, g: f32| {
        if let Some(x) = m.get_mut(s).and_then(|r| r.get_mut(d)) {
            *x += g;
        }
    };
    if nd == 1 {
        let st = build_matrix(src_fmt, ns, ChannelFormat::Stereo, 2, pan, law);
        for (row, srow) in m.iter_mut().zip(st.iter()).take(ns) {
            row[0] = 0.5 * (srow[0] + srow[1]);
        }
        return m;
    }
    let src_sp = src_fmt.speakers();
    let src_amb = src_fmt.is_ambisonic() && ns >= 4;
    if nd == 2 && !dst_fmt.is_ambisonic() {
        match ns {
            1 => {
                let (gl, gr) = gains(pan.stereo[0], law);
                m[0][0] = gl;
                m[0][1] = gr;
            }
            2 if !src_amb => {
                let (a_l, a_r) = gains(pan.stereo[0], PanLaw::Zero);
                let (b_l, b_r) = gains(pan.stereo[1], PanLaw::Zero);
                m[0] = [0.0; MAX_CHANNELS];
                m[0][0] = a_l;
                m[0][1] = a_r;
                m[1][0] = b_l;
                m[1][1] = b_r;
            }
            _ if src_amb => {
                for (d, az) in [(0usize, -30.0f32), (1, 30.0)] {
                    let g = openpt_dsp::pan::ambisonic_decode(az, 0.0, 2);
                    for (c, gc) in g.iter().enumerate() {
                        set(&mut m, c, d, *gc);
                    }
                }
            }
            _ if src_sp.len() == ns => {
                for (s, sp) in src_sp.iter().enumerate() {
                    let (l, r) = itu_stereo_fold(*sp);
                    set(&mut m, s, 0, l);
                    set(&mut m, s, 1, r);
                }
            }
            _ => {
                let h = std::f32::consts::FRAC_1_SQRT_2;
                m[0][0] = 1.0;
                m[1][1] = 1.0;
                for row in m.iter_mut().take(ns).skip(2) {
                    row[0] = h;
                    row[1] = h;
                }
            }
        }
        return m;
    }
    if ns > 2 && src_fmt == dst_fmt && ns == nd {
        return identity_matrix(ns);
    }
    let mut layout = [SpeakerPos::default(); MAX_CHANNELS];
    let nl = layout_of(dst_fmt, &mut layout);
    let layout = layout.get(..nl).unwrap_or(&[]);
    let dst_sp = dst_fmt.speakers();
    let dst_amb = dst_fmt.is_ambisonic();
    let find = |sp: Speaker| dst_sp.iter().position(|x| *x == sp).filter(|i| *i < nd);
    let mut g = [0.0f32; MAX_CHANNELS];
    if dst_amb {
        if src_amb || (src_fmt.is_ambisonic() && ns > 2) {
            return identity_matrix(ns.min(nd));
        }
        let mut enc = |m: &mut Matrix, s: usize, az: f32, el: f32, spread: f32| {
            openpt_dsp::pan::ambisonic_encode(az, el, spread, &mut g);
            for (d, gd) in g.iter().enumerate().take(nd) {
                set(m, s, d, *gd);
            }
        };
        match (ns, pan.surround) {
            (1 | 2, Some(sp)) => {
                for s in 0..ns {
                    let dx = if ns == 2 { if s == 0 { -1.0 } else { 1.0 } } else { 0.0 };
                    let p = surround_params(sp, dx);
                    enc(&mut m, s, openpt_dsp::pan::puck_azimuth(&[], p.x, p.y), p.z * 90.0, p.divergence);
                }
            }
            (1, None) => enc(&mut m, 0, pan.stereo[0].clamp(-1.0, 1.0) * 30.0, 0.0, 0.0),
            (2, None) => {
                enc(&mut m, 0, -30.0, 0.0, 0.0);
                enc(&mut m, 1, 30.0, 0.0, 0.0);
            }
            _ => {
                for (s, sp) in src_sp.iter().enumerate().take(ns) {
                    if !sp.is_lfe() {
                        enc(&mut m, s, sp.azimuth(), sp.elevation(), 0.0);
                    }
                }
            }
        }
        return m;
    }
    if dst_sp.len() != nd {
        return identity_matrix(ns.min(nd));
    }
    let (li, ri) = (find(Speaker::L), find(Speaker::R));
    match (ns, pan.surround) {
        (1 | 2, Some(sp)) if !src_amb => {
            for s in 0..ns {
                let dx = if ns == 2 { if s == 0 { -1.0 } else { 1.0 } } else { 0.0 };
                openpt_dsp::pan::surround_gains(layout, &surround_params(sp, dx), &mut g);
                for (d, gd) in g.iter().enumerate().take(nd) {
                    set(&mut m, s, d, *gd);
                }
                if let Some(lfe) = find(Speaker::Lfe)
                    && sp.lfe_db > -143.9
                {
                    set(&mut m, s, lfe, db_to_gain(sp.lfe_db.min(12.0)) / ns as f32);
                }
            }
        }
        (1, None) => {
            let (gl, gr) = gains(pan.stereo[0], law);
            if let (Some(l), Some(r)) = (li, ri) {
                set(&mut m, 0, l, gl);
                set(&mut m, 0, r, gr);
            }
        }
        (2, None) if !src_amb => {
            let (a_l, a_r) = gains(pan.stereo[0], PanLaw::Zero);
            let (b_l, b_r) = gains(pan.stereo[1], PanLaw::Zero);
            if let (Some(l), Some(r)) = (li, ri) {
                set(&mut m, 0, l, a_l);
                set(&mut m, 0, r, a_r);
                set(&mut m, 1, l, b_l);
                set(&mut m, 1, r, b_r);
            }
        }
        _ if src_amb => {
            let n_spk = dst_sp.iter().filter(|s| !s.is_lfe()).count();
            for (d, sp) in dst_sp.iter().enumerate().take(nd) {
                if sp.is_lfe() {
                    continue;
                }
                let dec = openpt_dsp::pan::ambisonic_decode(sp.azimuth(), sp.elevation(), n_spk);
                for (c, gc) in dec.iter().enumerate() {
                    set(&mut m, c, d, *gc);
                }
            }
        }
        _ if src_sp.len() == ns => {
            for (s, sp) in src_sp.iter().enumerate() {
                if let Some(d) = find(*sp) {
                    set(&mut m, s, d, 1.0);
                } else if sp.is_lfe() {
                } else {
                    openpt_dsp::pan::direction_gains(layout, sp.azimuth(), sp.elevation(), &mut g);
                    for (d, gd) in g.iter().enumerate().take(nd) {
                        set(&mut m, s, d, *gd);
                    }
                }
            }
        }
        _ => return identity_matrix(ns.min(nd)),
    }
    m
}

fn apply_matrix(src: &[Vec<f32>], frames: usize, dst: &mut [Vec<f32>], m: &Matrix, gain: f32) {
    apply_matrix_span(src, 0..frames, dst, m, gain);
}

fn apply_matrix_span(src: &[Vec<f32>], span: std::ops::Range<usize>, dst: &mut [Vec<f32>], m: &Matrix, gain: f32) {
    let ns = src.len().min(MAX_CHANNELS);
    for (d, out) in dst.iter_mut().enumerate().take(MAX_CHANNELS) {
        let mut idx = [(0usize, 0.0f32); MAX_CHANNELS];
        let mut k = 0;
        for (s, row) in m.iter().enumerate().take(ns) {
            let g = row.get(d).copied().unwrap_or(0.0);
            if g != 0.0
                && let Some(slot) = idx.get_mut(k)
            {
                *slot = (s, g);
                k += 1;
            }
        }
        let taps = idx.get(..k).unwrap_or(&[]);
        if taps.is_empty() {
            continue;
        }
        for (i, o) in out.iter_mut().enumerate().take(span.end).skip(span.start) {
            let mut acc = 0.0f32;
            for &(s, g) in taps {
                acc += src.get(s).and_then(|c| c.get(i)).copied().unwrap_or(0.0) * gain * g;
            }
            *o += acc;
        }
    }
}

pub const MAX_RENDER_SAMPLES: usize = 1 << 29;

pub fn render_range(s: &Session, range: Range, block: usize) -> Vec<Vec<f32>> {
    render_in_memory(s, None, range, block)
}

pub fn render_bus(s: &Session, bus: BusId, range: Range, block: usize) -> Vec<Vec<f32>> {
    render_in_memory(s, Some(bus), range, block)
}

fn render_in_memory(s: &Session, bus: Option<BusId>, range: Range, block: usize) -> Vec<Vec<f32>> {
    let nch = source_channels(s, bus);
    let len = usize::try_from(range.len().max(0)).unwrap_or(0).min(MAX_RENDER_SAMPLES / nch.max(1));
    let mut out: Vec<Vec<f32>> = (0..nch).map(|_| Vec::with_capacity(len)).collect();
    let mut render = RangeRender::new(s, bus, Range::new(range.start, range.start + len as i64), block);
    while let Some((chunk, n)) = render.next_block(s) {
        for (o, c) in out.iter_mut().zip(chunk) {
            o.extend_from_slice(c.get(..n).unwrap_or_default());
        }
    }
    out
}

pub fn source_channels(s: &Session, bus: Option<BusId>) -> usize {
    match bus {
        Some(id) => s.bus(id).map_or(0, |b| b.format.channels()),
        None => main_channels(s),
    }
}

pub struct RangeRender {
    eng: MixEngine,
    bus: Option<BusId>,
    tmp: Vec<Vec<f32>>,
    start: Samples,
    done: usize,
    total: usize,
    latency: usize,
}

impl RangeRender {
    pub fn new(s: &Session, bus: Option<BusId>, range: Range, block: usize) -> Self {
        let nch = source_channels(s, bus);
        let eng = MixEngine::new(s.sample_rate.as_f64() as f32, block);
        let tmp = vec![vec![0.0f32; eng.max_block()]; nch];
        let len = if nch == 0 { 0 } else { usize::try_from(range.len().max(0)).unwrap_or(0) };
        RangeRender { eng, bus, tmp, start: range.start, done: 0, total: len, latency: 0 }
    }

    fn render(&mut self, s: &Session, pos: Samples, n: usize) {
        match self.bus {
            Some(id) => {
                self.eng.render(s, pos, n, &mut []);
                self.eng.copy_bus(id, n, &mut self.tmp);
            }
            None => self.eng.render(s, pos, n, &mut self.tmp),
        }
    }

    pub fn next_block(&mut self, s: &Session) -> Option<(&[Vec<f32>], usize)> {
        if self.done >= self.total {
            return None;
        }
        let n = (self.total - self.done).min(self.eng.max_block());
        self.render(s, self.start + self.done as i64, n);
        if self.done == 0 {
            self.latency = match self.bus {
                Some(b) => self.eng.bus_latency(b),
                None => self.eng.latency,
            };
            self.total = self.total.saturating_add(self.latency);
        }
        let skip = self.latency.saturating_sub(self.done).min(n);
        self.done += n;
        if skip > 0 && skip < n {
            for c in &mut self.tmp {
                c.copy_within(skip..n, 0);
            }
        }
        Some((&self.tmp, n - skip))
    }
}

pub fn fold_down_stereo(fmt: ChannelFormat, ch: &[Vec<f32>]) -> Vec<Vec<f32>> {
    if ch.len() <= 2 {
        return ch.to_vec();
    }
    let len = ch.iter().map(Vec::len).max().unwrap_or(0);
    let mut out = vec![vec![0.0f32; len]; 2];
    let sp = fmt.speakers();
    let h = std::f32::consts::FRAC_1_SQRT_2;
    for (c, src) in ch.iter().enumerate() {
        let (gl, gr) = match sp.get(c) {
            Some(x) if sp.len() == ch.len() => itu_stereo_fold(*x),
            _ if c == 0 => (h, h),
            _ => (0.0, 0.0),
        };
        for (g, dst) in [(gl, 0usize), (gr, 1)] {
            if g == 0.0 {
                continue;
            }
            if let Some(d) = out.get_mut(dst) {
                for (o, x) in d.iter_mut().zip(src.iter()) {
                    *o += x * g;
                }
            }
        }
    }
    out
}

pub fn render_clips(s: &Session, track: TrackId, range: Range) -> Vec<Vec<f32>> {
    let Some(t) = s.track(track) else { return Vec::new() };
    let len = usize::try_from(range.len().max(0)).unwrap_or(0).min(MAX_RENDER_SAMPLES / t.channels().max(1));
    let mut out = vec![vec![0.0f32; len]; t.channels().max(1)];
    let mut scratch = Vec::new();
    for c in t.clips().iter().filter(|c| !c.muted && c.range().overlaps(&range)) {
        match clip_fx_params(s, c.id.0) {
            None => render_clip(s, c, range.start, len, &mut out, &mut scratch),
            Some(params) => {
                let mut tmp = vec![vec![0.0f32; len]; out.len()];
                render_clip(s, c, range.start, len, &mut tmp, &mut scratch);
                let mut fx = ClipFx::new(s.sample_rate.as_f64() as f32, tmp.len());
                fx.configure(params);
                let mut done = 0;
                while done < len {
                    let n = (len - done).min(4096);
                    let mut block: Vec<Vec<f32>> = tmp.iter().map(|ch| ch.get(done..done + n).map(<[f32]>::to_vec).unwrap_or_default()).collect();
                    fx.process(&mut block, n);
                    for (o, b) in out.iter_mut().zip(block.iter()) {
                        if let Some(dst) = o.get_mut(done..done + n) {
                            for (x, y) in dst.iter_mut().zip(b.iter()) {
                                *x += *y;
                            }
                        }
                    }
                    done += n;
                }
            }
        }
    }
    out
}

pub fn render_track_pre_fader(s: &Session, track: TrackId, range: Range) -> Vec<Vec<f32>> {
    let mut solo = s.clone();
    let fmt = s.track(track).map_or(ChannelFormat::Stereo, |t| if t.channels() > 2 { t.format } else { ChannelFormat::Stereo });
    solo.outputs = vec![openpt_model::OutputPath { name: "Out".into(), first_channel: 0, format: fmt }];
    for t in &mut solo.tracks {
        if t.id == track {
            t.mixer.volume_db = 0.0;
            t.mixer.mute = false;
            t.mixer.solo = false;
            t.mixer.output = Route::Main;
            t.mixer.pan = match t.channels() {
                1 => vec![0.0],
                _ => vec![-1.0, 1.0],
            };
            t.automation.retain(|l| matches!(l.param, AutoParam::Plugin { .. }));
            t.mixer.sends.iter_mut().for_each(|x| *x = None);
            t.mixer.surround = None;
        } else {
            t.inactive = true;
        }
    }
    let mut out = render_range(&solo, range, 1024);
    if s.track(track).is_some_and(|t| t.channels() == 1)
        && let Some(l) = out.first().cloned()
    {
        let g = std::f32::consts::SQRT_2;
        out = vec![l.iter().map(|x| x * g).collect()];
    }
    out
}

fn create_plugin(id: &str) -> Option<Box<dyn Plugin>> {
    openpt_dsp::create(id).or_else(|| openpt_vst3_host::create(id)).or_else(|| openpt_au_host::create(id))
}

#[cfg(test)]
mod audio_thread_tests {
    use super::*;

    #[test]
    fn only_a_marked_thread_is_the_audio_thread() {
        assert!(!on_audio_thread());
        let marked = std::thread::spawn(|| {
            let before = on_audio_thread();
            mark_audio_thread();
            (before, on_audio_thread())
        })
        .join()
        .unwrap();
        assert_eq!(marked, (false, true));
        assert!(!on_audio_thread(), "marking another thread leaves this one alone");
    }

    fn seen_by_strips(audio: bool) -> Vec<bool> {
        std::thread::spawn(move || {
            if audio {
                mark_audio_thread();
            }
            let mut work = vec![None; 64];
            run_strips(&mut work, |w| {
                std::thread::sleep(std::time::Duration::from_millis(1));
                *w = Some(on_audio_thread());
            });
            work.into_iter().map(|w| w.unwrap()).collect()
        })
        .join()
        .unwrap()
    }

    #[test]
    fn strips_rendered_for_the_audio_thread_count_as_the_audio_thread_and_only_then() {
        assert!(seen_by_strips(true).iter().all(|&rt| rt), "a worker rendering for the audio thread is on it");
        assert!(seen_by_strips(false).iter().all(|&rt| !rt), "the workers don't stay marked afterwards");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpt_audio_io::AudioBuffer;
    use openpt_model::{Insert, SourceAudio, SourceId};
    use std::sync::Arc;

    const CENTER: f32 = 0.749_894_2;

    fn synth_session(level_db: f32) -> Session {
        use openpt_midi::{Note, Sequence};
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        let mut seq = Sequence::default();
        seq.notes.push(Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start: 0, length: 960 });
        let cid = s.new_clip_id();
        let tr = s.track_mut(t).unwrap();
        tr.playlist_mut().unwrap().clips.push(Clip::midi(cid, "n", 0, 24_000, seq));
        let mut ins = Insert::new("subtractive_synth");
        ins.params.insert("level".into(), level_db);
        tr.instrument = Some(ins);
        s
    }

    #[test]
    fn stored_instrument_params_reach_the_plugin() {
        let rms = |db: f32| {
            let out = render_range(&synth_session(db), Range::new(0, 12_000), 512);
            let tail = out[0].get(6_000..).unwrap_or_default();
            (tail.iter().map(|x| x * x).sum::<f32>() / tail.len().max(1) as f32).sqrt()
        };
        let (loud, quiet) = (rms(-6.0), rms(-12.0));
        assert!(loud > 0.0, "the synth should sound");
        assert!((quiet / loud - 0.5).abs() < 0.05, "-12 dB vs -6 dB should halve the level: {quiet} / {loud}");
    }

    #[test]
    fn insert_slot_a_automation_does_not_drive_the_instrument() {
        let mut s = synth_session(-6.0);
        let reference = render_range(&s, Range::new(0, 12_000), 512);
        let tr = &mut s.tracks[0];
        let mut lane = openpt_model::AutomationLane::new(AutoParam::Plugin { slot: 0, param: "level".into() });
        lane.set_point(0, -48.0);
        tr.automation.push(lane);
        assert_eq!(render_range(&s, Range::new(0, 12_000), 512), reference);
    }

    fn session_with_dc(level: f32, frames: usize) -> (Session, TrackId) {
        let mut s = Session::default();
        let t = add_dc_track(&mut s, level, frames);
        (s, t)
    }

    fn add_dc_track(s: &mut Session, level: f32, frames: usize) -> TrackId {
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let buf = AudioBuffer { sample_rate: 48_000, channels: vec![vec![level; frames]] };
        s.pool.insert(SourceId(900), Arc::new(SourceAudio::new(buf)));
        let id = s.new_clip_id();
        if let Some(pl) = s.track_mut(t).and_then(|t| t.playlist_mut()) {
            pl.clips.push(Clip::audio(id, "dc", SourceId(900), 0, 0, frames as i64));
        }
        t
    }

    #[test]
    fn mono_center_pan_is_minus_2_5db() {
        let (s, _) = session_with_dc(0.5, 4800);
        let out = render_range(&s, Range::new(0, 4800), 512);
        let expect = 0.5 * CENTER;
        assert!((out[0][1000] - expect).abs() < 1e-3, "{}", out[0][1000]);
        assert!((out[1][1000] - expect).abs() < 1e-3);
    }

    #[test]
    fn fader_mute_and_solo() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.track_mut(t).unwrap().mixer.volume_db = -6.0;
        let out = render_range(&s, Range::new(0, 4800), 512);
        let expect = 0.5 * CENTER * db_to_gain(-6.0);
        assert!((out[0][2000] - expect).abs() < 1e-3);
        s.track_mut(t).unwrap().mixer.mute = true;
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][2000].abs() < 1e-6);
        s.track_mut(t).unwrap().mixer.mute = false;
        let other = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        s.track_mut(other).unwrap().mixer.solo = true;
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][2000].abs() < 1e-6);
    }

    #[test]
    fn hard_pans() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.track_mut(t).unwrap().mixer.pan = vec![-1.0];
        let out = render_range(&s, Range::new(0, 4800), 256);
        assert!(out[0][100] > 0.49 && out[1][100].abs() < 1e-4);
    }

    #[test]
    fn fade_curvature_reaches_the_rendered_audio() {
        for fade_in in [true, false] {
            for curvature in [-0.5, 0.0, 0.5] {
                let (mut s, t) = session_with_dc(1.0, 8192);
                s.track_mut(t).unwrap().mixer.pan = vec![-1.0];
                let fade = openpt_model::Fade { len: 8192, shape: openpt_model::FadeShape::SCurve, curvature };
                let c = &mut s.track_mut(t).unwrap().playlist_mut().unwrap().clips[0];
                if fade_in {
                    c.fade_in = fade;
                } else {
                    c.fade_out = fade;
                }
                let out = render_range(&s, Range::new(0, 8192), 512);
                let expected = if curvature < 0.0 {
                    0.2
                } else if curvature > 0.0 {
                    0.8
                } else {
                    0.5
                };
                assert!((out[0][4096] - expected).abs() < 1e-5, "{fade_in}, {curvature}: {} vs {expected}", out[0][4096]);
                assert!(out[1][4096].abs() < 1e-6);
                assert!(out[0][if fade_in { 0 } else { 8191 }].abs() < 1e-4);
            }
        }
    }

    #[test]
    fn pan_automation_does_not_depend_on_the_block_size() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        {
            let l = s.track_mut(t).unwrap().lane_mut(&AutoParam::Pan(0));
            l.set_point(0, -1.0);
            l.set_point(480, 1.0);
        }
        let bounce = render_range(&s, Range::new(0, 4800), 1024);
        assert!(bounce[0][600].abs() < 1e-4 && bounce[1][600] > 0.49, "{} {}", bounce[0][600], bounce[1][600]);
        for block in [512, 100, 37] {
            assert_eq!(render_range(&s, Range::new(0, 4800), block), bounce, "block {block}");
        }
    }

    #[test]
    fn fades_and_clip_gain_apply() {
        let (mut s, t) = session_with_dc(1.0, 48_000);
        {
            let c = &mut s.track_mut(t).unwrap().playlist_mut().unwrap().clips[0];
            c.fade_in = openpt_model::Fade { len: 1000, shape: openpt_model::FadeShape::Linear, curvature: 0.0 };
            c.gain_db = -6.0;
        }
        let out = render_range(&s, Range::new(0, 2000), 512);
        let g = CENTER * db_to_gain(-6.0);
        assert!(out[0][0].abs() < 1e-3);
        assert!((out[0][500] - 0.5 * g).abs() < 0.02, "{}", out[0][500]);
        assert!((out[0][1500] - g).abs() < 1e-3);
    }

    #[test]
    fn sends_feed_aux_via_bus() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let bus = s.add_bus("Verb", ChannelFormat::Stereo);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Verb"));
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        {
            let tr = s.track_mut(t).unwrap();
            tr.mixer.output = Route::None;
            let mut snd = openpt_model::SendSlot::new(Route::Bus(bus));
            snd.level_db = 0.0;
            tr.mixer.sends[0] = Some(snd);
        }
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][1000] > 0.1, "aux should carry the send: {}", out[0][1000]);
    }

    #[test]
    fn render_bus_is_the_bus_not_the_main_mix() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let stem = s.add_bus("Stem", ChannelFormat::Stereo);
        let empty = s.add_bus("Empty", ChannelFormat::Stereo);
        s.track_mut(t).unwrap().mixer.output = Route::Bus(stem);
        let main = render_range(&s, Range::new(0, 4800), 512);
        let bus = render_bus(&s, stem, Range::new(0, 4800), 512);
        let silent = render_bus(&s, empty, Range::new(0, 4800), 512);
        assert!(main[0][1000].abs() < 1e-4, "main should not carry a track routed to the bus: {}", main[0][1000]);
        assert!(bus[0][1000].abs() > 0.1, "bus should carry the track: {}", bus[0][1000]);
        assert!(silent.iter().all(|c| c.iter().all(|x| x.abs() < 1e-6)));
    }

    #[test]
    fn volume_automation_ramps() {
        let (mut s, t) = session_with_dc(0.5, 48_000);
        {
            let tr = s.track_mut(t).unwrap();
            let l = tr.lane_mut(&AutoParam::Volume);
            l.set_point(0, 0.0);
            l.set_point(24_000, -144.0);
        }
        let out = render_range(&s, Range::new(0, 48_000), 512);
        assert!(out[0][100] > 0.3);
        assert!(out[0][30_000].abs() < 1e-4);
    }

    #[test]
    fn inserts_process_and_survive_unknown_plugins() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("does-not-exist"));
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][100] > 0.3);
        let id = openpt_dsp::plugins().iter().find(|p| p.id.contains("gain")).map(|p| p.id).unwrap_or("gain");
        let mut ins = Insert::new(id);
        ins.params.insert("gain".into(), -6.0);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(ins);
        let _ = render_range(&s, Range::new(0, 4800), 512);
    }

    struct Stateful {
        inner: Box<dyn Plugin>,
        blob: Vec<u8>,
        sets: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Plugin for Stateful {
        fn info(&self) -> &'static openpt_dsp::PluginInfo {
            self.inner.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.inner.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.inner.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.sets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.inner.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.inner.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.inner.process(io, frames);
        }
        fn state_reader(&mut self) -> Option<Box<dyn openpt_dsp::PluginStateReader>> {
            let blob = self.blob.clone();
            Some(Box::new(move || Some(blob.clone())))
        }
    }

    struct Clock {
        inner: Box<dyn Plugin>,
        seen: Arc<std::sync::Mutex<Vec<Transport>>>,
    }

    impl Plugin for Clock {
        fn info(&self) -> &'static openpt_dsp::PluginInfo {
            self.inner.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.inner.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.inner.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.inner.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.inner.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.inner.process(io, frames);
        }
        fn set_transport(&mut self, t: &Transport) {
            self.seen.lock().unwrap().push(*t);
        }
    }

    #[test]
    fn plugins_get_the_session_transport() {
        let (mut s, t) = session_with_dc(0.5, 48_000 * 4);
        s.tempo.set_tempo(0, 90.0).unwrap();
        s.tempo.set_meter(0, 3, 4).unwrap();
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("vst3:test.clock"));
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let clock = Clock { inner: openpt_dsp::create("gain").unwrap(), seen: Arc::clone(&seen) };
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "vst3:test.clock".into(), ch: 1, plugin: Box::new(clock) }]);
        render_block(&mut eng, &s, 96_000);
        eng.monitor_only = true;
        render_block(&mut eng, &s, 96_000);
        let seen = seen.lock().unwrap();
        let t0 = seen[0];
        assert!(t0.playing && !t0.recording);
        assert_eq!((t0.tempo, t0.numerator, t0.denominator, t0.samples, t0.bar), (90.0, 3, 4, 96_000, 1));
        assert!((t0.seconds - 2.0).abs() < 1e-9 && (t0.quarters - 3.0).abs() < 1e-9 && (t0.bar_start - 3.0).abs() < 1e-9);
        assert!(!seen[1].playing);
    }

    struct Ears {
        inner: Box<dyn Plugin>,
        heard: Arc<std::sync::Mutex<Vec<(usize, [u8; 3])>>>,
    }

    impl Plugin for Ears {
        fn info(&self) -> &'static openpt_dsp::PluginInfo {
            self.inner.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.inner.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.inner.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.inner.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.inner.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.inner.process(io, frames);
        }
        fn midi(&mut self, offset: usize, msg: [u8; 3]) {
            self.heard.lock().unwrap().push((offset, msg));
        }
    }

    #[test]
    fn controllers_play_at_their_offsets_and_are_chased_after_a_seek() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().instrument = Some(Insert::new("vst3:test.ears"));
        let cc = |tick, kind, value| openpt_midi::CtrlEvent { tick, channel: 0, kind, value };
        let ctrls = vec![cc(4, CtrlKind::Cc(64), 127), cc(960, CtrlKind::PitchBend, -8192), cc(1920, CtrlKind::Cc(64), 0)];
        let seq = openpt_midi::Sequence { ctrls, ..Default::default() };
        let cid = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::midi(cid, "m", 0, 48_000 * 4, seq));
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ears = Ears { inner: openpt_dsp::create("gain").unwrap(), heard: Arc::clone(&heard) };
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        eng.adopt(vec![PreparedInstance { track: t, slot: INSTRUMENT_SLOT, id: "vst3:test.ears".into(), ch: 2, plugin: Box::new(ears) }]);
        render_block(&mut eng, &s, 0);
        assert_eq!(std::mem::take(&mut *heard.lock().unwrap()), vec![(0, [0xB0, 121, 0]), (100, [0xB0, 64, 127])]);
        render_block(&mut eng, &s, 36_000);
        assert_eq!(std::mem::take(&mut *heard.lock().unwrap()), vec![(0, [0xB0, 121, 0]), (0, [0xB0, 64, 127]), (0, [0xE0, 0, 0])]);
        render_block(&mut eng, &s, 36_512);
        assert!(heard.lock().unwrap().is_empty());
    }

    struct Keys {
        inner: Box<dyn Plugin>,
        heard: Arc<std::sync::Mutex<Vec<(u8, u8)>>>,
    }

    impl Plugin for Keys {
        fn info(&self) -> &'static openpt_dsp::PluginInfo {
            self.inner.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.inner.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.inner.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.inner.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.inner.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            self.inner.process(io, frames);
        }
        fn note_on(&mut self, _offset: usize, note: u8, velocity: u8) {
            self.heard.lock().unwrap().push((note, velocity));
        }
    }

    #[test]
    fn real_time_properties_reach_the_instrument() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, None);
        s.track_mut(t).unwrap().instrument = Some(Insert::new("vst3:test.keys"));
        let mut seq = openpt_midi::Sequence::default();
        seq.notes.push(openpt_midi::Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start: 0, length: 480 });
        let cid = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::midi(cid, "m", 0, 48_000, seq));
        s.edit.values.insert(format!("rtp.{}.transpose", t.0), 12.0);
        s.edit.values.insert(format!("rtp.{}.velocity", t.0), -20.0);
        let heard = Arc::new(std::sync::Mutex::new(Vec::new()));
        let keys = Keys { inner: openpt_dsp::create("gain").unwrap(), heard: Arc::clone(&heard) };
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        eng.adopt(vec![PreparedInstance { track: t, slot: INSTRUMENT_SLOT, id: "vst3:test.keys".into(), ch: 2, plugin: Box::new(keys) }]);
        render_block(&mut eng, &s, 0);
        assert_eq!(*heard.lock().unwrap(), vec![(72, 80)]);
    }

    #[test]
    fn heat_colours_tracks_unless_bypassed() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let sine: Vec<f32> = (0..48_000).map(|i| 0.8 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 48_000.0).sin()).collect();
        s.pool.insert(SourceId(901), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![sine] })));
        let id = s.new_clip_id();
        if let Some(pl) = s.track_mut(t).and_then(|t| t.playlist_mut()) {
            pl.clips.push(Clip::audio(id, "sine", SourceId(901), 0, 0, 48_000));
        }
        let render = |s: &Session| {
            let mut eng = MixEngine::new(48_000.0, 512);
            let mut out = vec![vec![0.0f32; 512]; 2];
            for k in 0..20 {
                eng.render(s, k * 512, 512, &mut out);
            }
            out[0].clone()
        };
        let dry = render(&s);
        s.heat = openpt_model::HeatSettings { active: true, bypass: false, drive: 1.0, tone: 0.0 };
        let hot = render(&s);
        let diff = dry.iter().zip(&hot).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(diff > 0.01, "HEAT changes a loud track: {diff}");
        s.track_mut(t).unwrap().mixer.heat_bypass = true;
        assert_eq!(render(&s), dry, "bypassed on this track");
    }

    fn render_block(eng: &mut MixEngine, s: &Session, pos: i64) -> f32 {
        let mut out = vec![vec![0.0f32; 512]; 2];
        eng.render(s, pos, 512, &mut out);
        out[0][500]
    }

    #[test]
    fn external_instances_are_adopted_and_retired() {
        let (mut s, t) = session_with_dc(0.5, 48_000);
        let mut ins = Insert::new("vst3:test.fake");
        ins.params.insert("gain".into(), -6.0);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(ins);
        let specs = instance_specs(&s);
        assert_eq!(specs, vec![InstanceSpec { track: t, slot: 0, id: "vst3:test.fake".into(), ch: 1, state: None }]);
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        let dry = render_block(&mut eng, &s, 0);
        assert!(dry > 0.3, "{dry}");
        let sets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut gain = openpt_dsp::create("gain").unwrap();
        gain.prepare(48_000.0, 512, 1);
        let fake = Stateful { inner: gain, blob: Vec::new(), sets: Arc::clone(&sets) };
        let wrong = Stateful { inner: openpt_dsp::create("gain").unwrap(), blob: Vec::new(), sets: Arc::clone(&sets) };
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "vst3:test.fake".into(), ch: 2, plugin: Box::new(wrong) }]);
        assert!(render_block(&mut eng, &s, 512) > 0.3);
        assert_eq!(eng.take_retired().len(), 1);
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "vst3:test.fake".into(), ch: 1, plugin: Box::new(fake) }]);
        let mut wet = 0.0;
        for b in 2..20 {
            wet = render_block(&mut eng, &s, b * 512);
        }
        assert!((wet / dry - 0.501).abs() < 0.01, "{wet} vs {dry}");
        assert!(sets.load(std::sync::atomic::Ordering::Relaxed) > 0, "no stored state: session params are pushed");
        assert!(eng.take_retired().is_empty());
        s.track_mut(t).unwrap().mixer.inserts[0] = None;
        render_block(&mut eng, &s, 20 * 512);
        assert_eq!(eng.take_retired().len(), 1);
        eng.adopt(vec![PreparedInstance { track: t, slot: 3, id: "vst3:x".into(), ch: 1, plugin: openpt_dsp::create("gain").unwrap() }]);
        render_block(&mut eng, &s, 21 * 512);
        assert_eq!(eng.take_retired().len(), 1);
        assert!(eng.drain_plugins().is_empty());
    }

    struct Nan(Box<dyn Plugin>);

    impl Plugin for Nan {
        fn info(&self) -> &'static openpt_dsp::PluginInfo {
            self.0.info()
        }
        fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
            self.0.prepare(sr, mb, ch);
        }
        fn reset(&mut self) {
            self.0.reset();
        }
        fn set_param(&mut self, id: &str, v: f32) -> bool {
            self.0.set_param(id, v)
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.0.param(id)
        }
        fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
            io.iter_mut().for_each(|c| c.iter_mut().take(frames).for_each(|x| *x = f32::NAN));
        }
    }

    fn nan(track: TrackId, slot: usize, ch: usize) -> Vec<PreparedInstance> {
        vec![PreparedInstance { track, slot, id: "vst3:test.nan".into(), ch, plugin: Box::new(Nan(openpt_dsp::create("gain").unwrap())) }]
    }

    #[test]
    fn an_insert_that_puts_out_nan_is_taken_out_until_the_next_reset() {
        let (mut s, a) = session_with_dc(0.5, 48_000);
        let b = add_dc_track(&mut s, 0.5, 48_000);
        s.track_mut(a).unwrap().mixer.inserts[2] = Some(Insert::new("vst3:test.nan"));
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        let both = render_block(&mut eng, &s, 0);
        eng.adopt(nan(a, 2, 1));
        let only_b = render_block(&mut eng, &s, 512);
        assert!(only_b > 0.3 && (only_b - both / 2.0).abs() < 1e-6, "{only_b} vs {both}");
        assert_eq!(eng.meters[&a].plugin_failures, PluginFailures { count: 1, last: 2 });
        assert_eq!(eng.meters[&b].plugin_failures, PluginFailures::default());
        assert_eq!(render_block(&mut eng, &s, 1024), both);
        eng.reset();
        assert_eq!(render_block(&mut eng, &s, 1536), only_b);
        assert_eq!(eng.meters[&a].plugin_failures.count, 2);
    }

    #[test]
    fn nan_from_an_instrument_or_a_master_insert_is_contained() {
        let mut s = synth_session(-6.0);
        let synth = s.tracks[0].id;
        s.track_mut(synth).unwrap().instrument = Some(Insert::new("vst3:test.nan"));
        let b = add_dc_track(&mut s, 0.5, 48_000);
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        eng.adopt(nan(synth, INSTRUMENT_SLOT, 2));
        let only_b = render_block(&mut eng, &s, 0);
        assert!(only_b > 0.3, "{only_b}");
        assert_eq!(eng.meters[&synth].plugin_failures, PluginFailures { count: 1, last: INSTRUMENT_SLOT });
        assert_eq!(eng.meters[&b].plugin_failures, PluginFailures::default());
        let m = s.add_track(TrackKind::Master, ChannelFormat::Stereo, None);
        s.track_mut(m).unwrap().mixer.inserts[0] = Some(Insert::new("vst3:test.nan"));
        eng.adopt(nan(m, 0, 2));
        assert_eq!(render_block(&mut eng, &s, 512), 0.0);
        assert_eq!(eng.meters[&m].plugin_failures, PluginFailures { count: 1, last: 0 });
        assert_eq!(render_block(&mut eng, &s, 1024), only_b);
    }

    #[test]
    fn a_new_instance_replaces_a_matching_one_in_its_slot() {
        let (mut s, t) = session_with_dc(0.5, 48_000);
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(Insert::new("vst3:test.fake"));
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        let sets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let make = |blob: &[u8]| {
            let mut gain = openpt_dsp::create("gain").unwrap();
            gain.prepare(48_000.0, 512, 1);
            Box::new(Stateful { inner: gain, blob: blob.to_vec(), sets: Arc::clone(&sets) })
        };
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "vst3:test.fake".into(), ch: 1, plugin: make(b"old") }]);
        render_block(&mut eng, &s, 0);
        assert!(eng.take_retired().is_empty());
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "vst3:test.fake".into(), ch: 1, plugin: make(b"new") }]);
        render_block(&mut eng, &s, 512);
        let mut retired = eng.take_retired();
        assert_eq!(retired.len(), 1, "the old instance is retired");
        assert_eq!(retired[0].save_state(), Some(b"old".to_vec()), "the new instance stays");
    }

    #[test]
    fn stored_state_comes_first_then_session_params() {
        let (mut s, t) = session_with_dc(0.5, 48_000);
        let mut ins = Insert::new("vst3:00000000000000000000000000000001");
        ins.params.insert("gain".into(), -6.0);
        ins.set_state_bytes(b"stored");
        s.track_mut(t).unwrap().mixer.inserts[0] = Some(ins);
        assert_eq!(instance_specs(&s)[0].state.as_deref(), Some(&b"stored"[..]));
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        let sets = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fake = Stateful { inner: openpt_dsp::create("gain").unwrap(), blob: Vec::new(), sets: Arc::clone(&sets) };
        eng.adopt(vec![PreparedInstance { track: t, slot: 0, id: "vst3:00000000000000000000000000000001".into(), ch: 1, plugin: Box::new(fake) }]);
        render_block(&mut eng, &s, 0);
        assert_eq!(sets.load(std::sync::atomic::Ordering::Relaxed), 1, "session params apply once");
        render_block(&mut eng, &s, 512);
        assert_eq!(sets.load(std::sync::atomic::Ordering::Relaxed), 1);
        s.track_mut(t).unwrap().mixer.inserts[0].as_mut().unwrap().params.insert("gain".into(), -12.0);
        render_block(&mut eng, &s, 1024);
        assert_eq!(sets.load(std::sync::atomic::Ordering::Relaxed), 2, "later session edits apply");
        let out = render_range(&s, Range::new(0, 1024), 512);
        assert!(out[0][100] > 0.3);
    }

    #[test]
    fn clip_effects_gain_and_trim_automation_apply() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let cid = s.track(t).unwrap().clips()[0].id.0;
        s.edit.values.insert(format!("clip_fx.{cid}.gain"), -6.0);
        let out = render_range(&s, Range::new(0, 4800), 512);
        let expect = 0.5 * CENTER * db_to_gain(-6.0);
        assert!((out[0][2000] - expect).abs() < 0.01, "{}", out[0][2000]);
        s.edit.set_flag(&format!("clip_fx.bypass.{cid}"), true);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!((out[0][2000] - 0.5 * CENTER).abs() < 0.01);
        s.track_mut(t).unwrap().lane_mut(&AutoParam::Plugin { slot: u8::MAX, param: "trim".into() }).set_point(0, -12.0);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][2000] < 0.2, "{}", out[0][2000]);
    }

    #[test]
    fn delay_compensation_aligns_latent_tracks() {
        let mut s = Session::default();
        let mut imp = vec![0.0f32; 48_000];
        imp[1000] = 1.0;
        s.pool.insert(SourceId(900), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![imp] })));
        let mut ids = Vec::new();
        for (pan, fx) in [(-1.0f32, true), (1.0, false)] {
            let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
            let cid = s.new_clip_id();
            let tr = s.track_mut(t).unwrap();
            tr.playlist_mut().unwrap().clips.push(Clip::audio(cid, "i", SourceId(900), 0, 0, 48_000));
            tr.mixer.pan = vec![pan];
            if fx {
                tr.mixer.inserts[0] = Some(Insert::new("maximizer"));
            }
            ids.push(t);
        }
        let argmax = |c: &[f32]| c.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i).unwrap();
        let lat = openpt_dsp::create("maximizer").unwrap().latency();
        assert!(lat > 0, "test needs a latent plugin");
        let out = render_range(&s, Range::new(0, 8000), 512);
        assert_eq!(argmax(&out[0]), argmax(&out[1]), "PDC should align L and R");
        s.edit.delay_compensation = false;
        let out = render_range(&s, Range::new(0, 8000), 512);
        assert_ne!(argmax(&out[0]), argmax(&out[1]));
    }

    #[test]
    fn reset_clears_delay_compensation_history() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.edit.click = false;
        s.track_mut(t).unwrap().mixer.pan = vec![1.0];
        let latent = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let mut comp = Insert::new("compressor");
        comp.params.insert("lookahead".into(), 5.0);
        s.track_mut(latent).unwrap().mixer.inserts[0] = Some(comp);
        let mut eng = MixEngine::new(48_000.0, 512);
        let mut out = vec![vec![0.0f32; 512]; 2];
        for b in 0..9 {
            eng.render(&s, b * 512, 512, &mut out);
        }
        assert!(eng.latency > 0 && out[1][500] > 0.3, "the fixture plays through the alignment delay");
        eng.reset();
        eng.render(&s, 1_000_000, 512, &mut out);
        assert!(out.iter().flatten().all(|x| *x == 0.0), "old aligned audio after a reset");
    }

    fn impulse_at(ch: &[f32]) -> Vec<usize> {
        ch.iter().enumerate().filter(|(_, v)| v.abs() > 1e-4).map(|(i, _)| i).collect()
    }

    #[test]
    fn delay_compensation_aligns_an_aux_insert() {
        let mut s = Session::default();
        let n = 8192;
        let mut left = vec![0.0f32; n];
        for at in [0usize, 2048, 8000] {
            left[at] = 0.25;
        }
        s.pool.insert(SourceId(901), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![left.clone(), left] })));
        let bus = s.add_bus("Test Bus", ChannelFormat::Stereo);
        let source = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Source"));
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Return"));
        let cid = s.new_clip_id();
        s.track_mut(source).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(cid, "impulses", SourceId(901), 0, 0, n as i64));
        s.track_mut(source).unwrap().mixer.output = Route::Bus(bus);
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        let render = |s: &Session, start: i64, end: i64| render_range(s, Range::new(start, end), 1024);
        let control = render(&s, 0, n as i64);
        assert_eq!(impulse_at(&control[0]), vec![0, 2048, 8000]);
        assert_eq!(impulse_at(&control[1]), impulse_at(&control[0]));

        let mut comp = Insert::new("compressor");
        comp.params.insert("lookahead".into(), 5.0);
        comp.params.insert("ratio".into(), 1.0);
        s.track_mut(source).unwrap().mixer.inserts[0] = Some(comp.clone());
        let on_source = render(&s, 0, n as i64);
        assert_eq!(impulse_at(&on_source[0]), impulse_at(&control[0]));

        s.track_mut(source).unwrap().mixer.inserts[0] = None;
        s.track_mut(aux).unwrap().mixer.inserts[0] = Some(comp);
        let on_aux = render(&s, 0, n as i64);
        assert_eq!(impulse_at(&on_aux[0]), vec![0, 2048, 8000], "aux lookahead must stay inside the bounce");
        assert_eq!(impulse_at(&on_aux[1]), impulse_at(&on_aux[0]));
        assert!((on_aux[0][8000] - 0.25).abs() < 1e-3, "{}", on_aux[0][8000]);
        let partial = render(&s, 2048, 3072);
        assert_eq!(impulse_at(&partial[0]), vec![0]);

        s.track_mut(aux).unwrap().mixer.inserts[0].as_mut().unwrap().params.insert("lookahead".into(), 0.0);
        let zero = render(&s, 0, n as i64);
        assert_eq!(impulse_at(&zero[0]), impulse_at(&control[0]));
    }

    #[test]
    fn offline_renders_line_up_with_the_timeline_in_any_block_size() {
        let mut s = Session::default();
        let mut imp = vec![0.0f32; 48_000];
        imp[1000] = 1.0;
        s.pool.insert(SourceId(900), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![imp] })));
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let bus = s.add_bus("Stem", ChannelFormat::Stereo);
        let cid = s.new_clip_id();
        let tr = s.track_mut(t).unwrap();
        tr.playlist_mut().unwrap().clips.push(Clip::audio(cid, "i", SourceId(900), 0, 0, 48_000));
        tr.mixer.inserts[0] = Some(Insert::new("maximizer"));
        let mut send = openpt_model::SendSlot::new(Route::Bus(bus));
        send.level_db = 0.0;
        tr.mixer.sends[0] = Some(send);
        let latency = openpt_dsp::create("maximizer").unwrap().latency();
        assert!(latency > 16);
        for source in [None, Some(bus)] {
            for block in [16, 100, 512] {
                let mut got = Vec::new();
                let mut skipped = 0;
                let mut render = RangeRender::new(&s, source, Range::new(900, 2900), block);
                while let Some((chunk, n)) = render.next_block(&s) {
                    assert!(n <= block);
                    skipped += usize::from(n == 0);
                    got.extend_from_slice(&chunk[0][..n]);
                }
                assert_eq!(got.len(), 2000, "{source:?} block {block}");
                assert_eq!(skipped, latency / block, "{source:?} block {block}");
                let peak = got.iter().enumerate().max_by(|a, b| a.1.abs().total_cmp(&b.1.abs())).map(|(i, _)| i);
                assert_eq!(peak, Some(100), "{source:?} block {block}");
            }
        }
    }

    #[test]
    fn a_plugins_bogus_latency_is_capped() {
        struct Latent(Box<dyn Plugin>);
        impl Plugin for Latent {
            fn info(&self) -> &'static openpt_dsp::PluginInfo {
                self.0.info()
            }
            fn prepare(&mut self, sr: f32, mb: usize, ch: usize) {
                self.0.prepare(sr, mb, ch);
            }
            fn reset(&mut self) {
                self.0.reset();
            }
            fn set_param(&mut self, id: &str, v: f32) -> bool {
                self.0.set_param(id, v)
            }
            fn param(&self, id: &str) -> Option<f32> {
                self.0.param(id)
            }
            fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
                self.0.process(io, frames);
            }
            fn latency(&self) -> usize {
                u32::MAX as usize
            }
        }
        let (mut s, t) = session_with_dc(0.5, 48_000);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Mono, None);
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.set_external_instances(true);
        for track in [t, aux] {
            s.track_mut(track).unwrap().mixer.inserts[0] = Some(Insert::new("vst3:test.latent"));
            let mut gain = openpt_dsp::create("gain").unwrap();
            gain.prepare(48_000.0, 512, 1);
            eng.adopt(vec![PreparedInstance { track, slot: 0, id: "vst3:test.latent".into(), ch: 1, plugin: Box::new(Latent(gain)) }]);
        }
        render_block(&mut eng, &s, 0);
        assert_eq!((eng.track_latency, eng.latency), (MAX_LATENCY, 2 * MAX_LATENCY));
    }

    #[test]
    fn input_monitoring_replaces_clips() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.input = vec![vec![0.25; 512]];
        let mut out = vec![vec![0.0; 512]; 2];
        eng.render(&s, 0, 512, &mut out);
        assert!((out[0][100] - 0.5 * CENTER).abs() < 1e-3);
        s.track_mut(t).unwrap().mixer.input_monitor = true;
        eng.render(&s, 512, 512, &mut out);
        assert!((out[0][100] - 0.25 * CENTER).abs() < 1e-3, "{}", out[0][100]);
        eng.monitor_only = true;
        eng.render(&s, 1024, 512, &mut out);
        assert!(out[0][100] > 0.1);
        s.track_mut(t).unwrap().mixer.input_monitor = false;
        eng.render(&s, 1024, 512, &mut out);
        assert!(out[0][100].abs() < 1e-6, "stopped, unmonitored: silence");
    }

    #[test]
    fn empty_session_is_silent() {
        let s = Session::default();
        let out = render_range(&s, Range::new(0, 1000), 128);
        assert!(out.iter().all(|c| c.iter().all(|x| *x == 0.0)));
    }

    #[test]
    fn strip_workspace_keeps_its_allocation_between_blocks() {
        let mut s = Session::default();
        for _ in 0..6 {
            s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        }
        let mut eng = MixEngine::new(48_000.0, 512);
        let mut out = vec![vec![0.0; 512]; 2];
        eng.render(&s, 0, 512, &mut out);
        let capacity = eng.work.capacity();
        assert!(capacity >= 6);
        eng.render(&s, 512, 512, &mut out);
        assert_eq!(eng.work.capacity(), capacity);
    }

    #[test]
    fn stopped_monitoring_does_not_click() {
        let mut s = Session::default();
        s.edit.click = true;
        let mut eng = MixEngine::new(48_000.0, 512);
        eng.metronome = true;
        eng.monitor_only = true;
        let mut out = vec![vec![0.0f32; 512]; 2];
        for _ in 0..4 {
            eng.render(&s, 0, 512, &mut out);
            assert!(out.iter().all(|c| c.iter().all(|x| *x == 0.0)));
        }
    }

    fn render_playback(s: &Session, len: usize, block: usize, recording: bool) -> Vec<Vec<f32>> {
        let mut eng = MixEngine::new(s.sample_rate.as_f64() as f32, block);
        eng.metronome = true;
        eng.recording = recording;
        let mut out = vec![vec![0.0f32; len]; 2];
        let mut tmp = vec![vec![0.0f32; block]; 2];
        let mut done = 0usize;
        while done < len {
            let n = (len - done).min(block);
            eng.render(s, done as i64, n, &mut tmp);
            for (o, t) in out.iter_mut().zip(tmp.iter()) {
                if let (Some(d), Some(src)) = (o.get_mut(done..done + n), t.get(..n)) {
                    d.copy_from_slice(src);
                }
            }
            done += n;
        }
        out
    }

    #[test]
    fn click_sounds_on_every_beat_only_in_playback() {
        let mut s = Session::default();
        let beat = s.sample_rate.as_f64() as usize * 60 / 120;
        let len = beat * 8;
        s.edit.click = true;
        let offline = render_range(&s, Range::new(0, len as i64), 512);
        assert!(offline.iter().all(|c| c.iter().all(|x| *x == 0.0)));

        let out = render_playback(&s, len, 512, false);
        for b in 0..8usize {
            let at = b * beat;
            let on: f32 = out[0][at..at + 200].iter().map(|x| x.abs()).fold(0.0, f32::max);
            let off: f32 = out[0][at + beat / 2..at + beat / 2 + 200].iter().map(|x| x.abs()).fold(0.0, f32::max);
            assert!(on > 0.1, "beat {b} should click, peak {on}");
            assert_eq!(off, 0.0, "between beats must stay silent");
            assert_eq!(out[0][at], out[1][at], "both channels get the click");
        }
        let split = render_playback(&s, len, 97, false);
        assert_eq!(out, split);
    }

    #[test]
    fn click_only_during_record_follows_recording() {
        let mut s = Session::default();
        s.edit.click = true;
        s.edit.set_flag("click.only_during_record", true);
        let idle = render_playback(&s, 48_000, 512, false);
        assert!(idle.iter().all(|c| c.iter().all(|x| *x == 0.0)));
        let rec = render_playback(&s, 48_000, 512, true);
        assert!(rec.iter().any(|c| c.iter().any(|x| x.abs() > 0.1)));
    }

    fn multichannel_session(fmt: ChannelFormat, main: ChannelFormat, frames: usize) -> (Session, TrackId) {
        let mut s = Session::default();
        s.outputs[0].format = main;
        let t = s.add_track(TrackKind::Audio, fmt, None);
        let chans: Vec<Vec<f32>> = (0..fmt.channels()).map(|c| (0..frames).map(|i| 0.1 * (c + 1) as f32 + (i % 7) as f32 * 1e-3).collect()).collect();
        s.pool.insert(SourceId(901), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: chans })));
        let id = s.new_clip_id();
        s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(id, "mc", SourceId(901), 0, 0, frames as i64));
        (s, t)
    }

    #[test]
    fn surround_track_passes_through_bit_exact() {
        let (s, _) = multichannel_session(ChannelFormat::Surround51, ChannelFormat::Surround51, 4800);
        let src = s.pool.get(SourceId(901)).unwrap().buffer.channels.clone();
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert_eq!(out.len(), 6);
        for (o, x) in out.iter().zip(src.iter()) {
            assert_eq!(o, x, "5.1 → 5.1 must be bit-exact");
        }
    }

    #[test]
    fn surround_to_stereo_folds_down_itu() {
        let (s, _) = multichannel_session(ChannelFormat::Surround51, ChannelFormat::Stereo, 4800);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert_eq!(out.len(), 2);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let x = |c: usize| 0.1 * (c + 1) as f32 + (1000 % 7) as f32 * 1e-3;
        let l = x(0) + h * x(2) + h * x(4);
        let r = x(1) + h * x(2) + h * x(5);
        assert!((out[0][1000] - l).abs() < 1e-5, "{} vs {l}", out[0][1000]);
        assert!((out[1][1000] - r).abs() < 1e-5, "{} vs {r}", out[1][1000]);
    }

    #[test]
    fn mono_track_on_surround_main_pans() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        s.outputs[0].format = ChannelFormat::Surround51;
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert_eq!(out.len(), 6);
        let h = 0.5 * gains(0.0, PanLaw::Minus2_5).0;
        assert!((out[0][100] - h).abs() < 1e-6 && (out[1][100] - h).abs() < 1e-6);
        assert!(out[2..].iter().all(|c| c[100].abs() < 1e-7));
        s.track_mut(t).unwrap().mixer.surround = Some(SurroundPan::default());
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!((out[2][100] - 0.5).abs() < 1e-4, "{}", out[2][100]);
        assert!(out.iter().enumerate().all(|(c, x)| c == 2 || x[100].abs() < 1e-4));
        s.track_mut(t).unwrap().mixer.surround = Some(SurroundPan { x: 1.0, y: -1.0, lfe_db: -6.0, ..SurroundPan::default() });
        let mut eng = MixEngine::new(48_000.0, 512);
        let mut buf = vec![vec![0.0; 512]; 6];
        eng.render(&s, 0, 512, &mut buf);
        assert!((buf[5][100] - 0.5).abs() < 1e-4 && (buf[3][100] - 0.5 * db_to_gain(-6.0)).abs() < 1e-4);
        assert_eq!(eng.main_meter.peaks.len(), 6);
        assert_eq!(eng.meters.get(&t).map(|m| m.peaks.len()), Some(1));
        let mut two = vec![vec![1.0; 512]; 2];
        eng.render(&s, 512, 512, &mut two);
        assert!(two[0][10].abs() < 1e-6);
    }

    #[test]
    fn surround_busses_and_master_fader() {
        let (mut s, t) = multichannel_session(ChannelFormat::Surround51, ChannelFormat::Surround51, 4800);
        let bus = s.add_bus("Stem", ChannelFormat::Surround51);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Surround51, Some("Stem"));
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        s.track_mut(t).unwrap().mixer.output = Route::Bus(bus);
        let m = s.add_track(TrackKind::Master, ChannelFormat::Surround51, None);
        s.track_mut(m).unwrap().mixer.volume_db = -6.0;
        let out = render_range(&s, Range::new(0, 4800), 512);
        let g = db_to_gain(-6.0);
        for (c, ch) in out.iter().enumerate() {
            let x = 0.1 * (c + 1) as f32 + (1000 % 7) as f32 * 1e-3;
            assert!((ch[1000] - x * g).abs() < 1e-5, "channel {c}: {} vs {}", ch[1000], x * g);
        }
        let (mut s, t) = session_with_dc(0.5, 4800);
        let bus = s.add_bus("Mono", ChannelFormat::Mono);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Mono, Some("M"));
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        s.track_mut(aux).unwrap().mixer.pan = vec![-1.0];
        s.track_mut(t).unwrap().mixer.output = Route::Bus(bus);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!((out[0][1000] - 0.5 * gains(0.0, PanLaw::Minus2_5).0).abs() < 1e-6, "{}", out[0][1000]);
    }

    #[test]
    fn a_send_to_a_bus_is_heard_through_the_aux_listening_to_it() {
        let (mut s, t) = session_with_dc(0.5, 4800);
        let bus = s.add_bus("Bus 7", ChannelFormat::Stereo);
        let aux = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Aux 7"));
        s.track_mut(aux).unwrap().mixer.input = Route::Bus(bus);
        let tr = s.track_mut(t).unwrap();
        tr.mixer.output = Route::None;
        let mut snd = openpt_model::SendSlot::new(Route::Bus(bus));
        snd.level_db = 0.0;
        tr.mixer.sends[0] = Some(snd);
        let out = render_range(&s, Range::new(0, 4800), 512);
        assert!(out[0][1000].abs() > 0.1 && out[1][1000].abs() > 0.1, "{} {}", out[0][1000], out[1][1000]);
    }

    #[test]
    fn surround_upmix_and_downmix_between_layouts() {
        let (s, _) = multichannel_session(ChannelFormat::Surround51, ChannelFormat::Surround71, 2000);
        let out = render_range(&s, Range::new(0, 2000), 512);
        assert_eq!(out.len(), 8);
        let x = |c: usize| 0.1 * (c + 1) as f32 + (100 % 7) as f32 * 1e-3;
        for (c, ch) in out.iter().enumerate().take(4) {
            assert!((ch[100] - x(c)).abs() < 1e-6);
        }
        let e_ls = out[4][100].powi(2) + out[6][100].powi(2);
        assert!((e_ls - x(4).powi(2)).abs() < 1e-5, "Ls energy is preserved");
    }

    #[test]
    fn live_midi_events_trigger_instrument_when_monitored_or_armed() {
        use openpt_model::Insert;
        let mut s = Session::new("LiveMidiTest", openpt_time::SampleRate::new(48_000).unwrap());
        let trk = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("SynthTrack"));
        let mut ins = Insert::new("subtractive_synth");
        ins.params.insert("level".into(), -6.0);
        s.track_mut(trk).unwrap().instrument = Some(ins);

        let mut eng = MixEngine::new(48_000.0, 512);

        eng.monitor_only = true;
        eng.midi_input = vec![(0, LiveMidiEvent { port_id: 0, timestamp_us: 10, len: 3, data: [0x90, 60, 100] })];
        let mut out = vec![vec![0.0; 512]; 2];
        eng.render(&s, 0, 512, &mut out);
        assert!(out[0].iter().all(|x| x.abs() < 1e-6), "unarmed track should not produce sound when stopped");

        s.track_mut(trk).unwrap().mixer.record_arm = true;
        eng.midi_input = vec![(0, LiveMidiEvent { port_id: 0, timestamp_us: 20, len: 3, data: [0x90, 60, 100] })];
        let mut out = vec![vec![0.0; 512]; 2];
        eng.render(&s, 0, 512, &mut out);
        assert!(out[0].iter().any(|x| x.abs() > 1e-4), "armed track must synthesize live note on");

        eng.midi_input = vec![(0, LiveMidiEvent { port_id: 0, timestamp_us: 30, len: 3, data: [0x80, 60, 0] })];
        let mut out_off = vec![vec![0.0; 512]; 2];
        eng.render(&s, 512, 512, &mut out_off);
        assert!(eng.strips.get(&trk).unwrap().held.is_empty(), "held notes must be cleared after note off");

        s.track_mut(trk).unwrap().mixer.record_arm = false;
        s.edit.set_flag("options.midi_thru", true);
        s.edit.selected_tracks = vec![trk];
        eng.midi_input = vec![(0, LiveMidiEvent { port_id: 0, timestamp_us: 40, len: 3, data: [0x90, 64, 110] })];
        let mut out_thru = vec![vec![0.0; 512]; 2];
        eng.render(&s, 1024, 512, &mut out_thru);
        assert!(out_thru[0].iter().any(|x| x.abs() > 1e-4), "MIDI Thru on selected track must synthesize note");
    }

    #[test]
    fn a_midi_track_can_take_its_input_from_another_track() {
        use openpt_midi::{Note, Sequence};
        use openpt_model::{Insert, MidiFrom};
        let mut s = Session::new("MidiFrom", openpt_time::SampleRate::new(48_000).unwrap());
        let src = s.add_track(TrackKind::Midi, ChannelFormat::Stereo, Some("Notes"));
        let mut seq = Sequence::default();
        seq.notes.push(Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start: 0, length: 480 });
        let cid = s.new_clip_id();
        s.track_mut(src).unwrap().playlist_mut().unwrap().clips.push(Clip::midi(cid, "n", 0, 48_000, seq));
        let dst = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("Synth"));
        s.track_mut(dst).unwrap().instrument = Some(Insert::new("subtractive_synth"));
        let mut eng = MixEngine::new(48_000.0, 512);
        let mut out = vec![vec![0.0; 512]; 2];
        s.track_mut(dst).unwrap().mixer.record_arm = true;
        s.track_mut(dst).unwrap().midi_from = Some(MidiFrom::Track(src));
        s.track_mut(src).unwrap().mixer.mute = true;
        eng.monitor_only = true;
        eng.midi_input = vec![(0, LiveMidiEvent { port_id: 0, timestamp_us: 1, len: 3, data: [0x90, 64, 100] })];
        eng.render(&s, 0, 512, &mut out);
        assert!(out[0].iter().all(|x| x.abs() < 1e-6));
        eng.midi_input.clear();
        eng.monitor_only = false;
        eng.render(&s, 0, 512, &mut out);
        assert_eq!(eng.midi_routed.first().map(|r| (r.0, r.2)), Some((dst, [0x90, 60, 100])));
        assert!(out[0].iter().any(|x| x.abs() > 1e-4), "the other track's note plays the instrument");
        s.track_mut(dst).unwrap().mixer.record_arm = false;
        eng.render(&s, 0, 512, &mut out);
        assert!(eng.midi_routed.is_empty());
    }

    #[test]
    fn an_audio_track_hears_its_input_bus_when_monitoring() {
        let mut s = Session::new("BusIn", openpt_time::SampleRate::new(48_000).unwrap());
        let src = s.add_track(TrackKind::Aux, ChannelFormat::Stereo, Some("Tone"));
        s.track_mut(src).unwrap().mixer.inserts[0] = Some(openpt_model::Insert::new("signal_generator"));
        let bus = s.add_bus("Print", ChannelFormat::Stereo);
        let dst = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Print"));
        s.track_mut(src).unwrap().mixer.output = Route::Bus(bus);
        s.track_mut(dst).unwrap().mixer.input = Route::Bus(bus);
        let mut eng = MixEngine::new(48_000.0, 512);
        let mut out = vec![vec![0.0; 512]; 2];
        let peak = |o: &[Vec<f32>]| o[0].iter().fold(0f32, |m, x| m.max(x.abs()));
        eng.render(&s, 0, 512, &mut out);
        eng.render(&s, 512, 512, &mut out);
        assert!(peak(&out) < 1e-6, "not monitoring: the bus is not heard");
        s.track_mut(dst).unwrap().mixer.input_monitor = true;
        eng.render(&s, 1024, 512, &mut out);
        eng.render(&s, 1536, 512, &mut out);
        assert!(peak(&out) > 1e-3, "monitoring: the track plays its input bus");
    }

    #[test]
    fn held_notes_tracks_multi_channel_voices_independently() {
        let mut held = HeldNotes::new();
        assert!(held.is_empty());
        assert_eq!(held.len(), 0);

        held.insert(0, 60);
        held.insert(1, 60);
        assert_eq!(held.len(), 2);
        assert!(held.contains(0, 60));
        assert!(held.contains(1, 60));
        assert!(!held.contains(2, 60));

        held.remove(0, 60);
        assert_eq!(held.len(), 1);
        assert!(!held.contains(0, 60));
        assert!(held.contains(1, 60));

        let items: Vec<(u8, u8)> = held.iter().collect();
        assert_eq!(items, vec![(1, 60)]);

        held.clear();
        assert!(held.is_empty());
    }

    #[test]
    fn monitored_tracks_bypass_pdc_alignment() {
        use openpt_model::Insert;
        let mut s = Session::new("PdcBypassTest", openpt_time::SampleRate::new(48_000).unwrap());
        let t_mix = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("MixTrack"));
        let t_mon = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("MonTrack"));

        let mut ins = Insert::new("eq_7band");
        ins.params.insert("frequency".into(), 1000.0);
        s.track_mut(t_mix).unwrap().mixer.inserts.push(Some(ins));

        s.track_mut(t_mon).unwrap().mixer.record_arm = true;

        let mut eng = MixEngine::new(48_000.0, 512);
        let mut out = vec![vec![0.0; 512]; 2];
        eng.render(&s, 0, 512, &mut out);

        if let Some(strip_mon) = eng.strips.get(&t_mon) {
            assert_eq!(strip_mon.align.len(), 0, "Monitored track must sever from PDC alignment delay");
        }
    }

    #[test]
    fn per_track_midi_channel_filtering() {
        use openpt_model::Insert;
        let mut s = Session::new("MidiFilterTest", openpt_time::SampleRate::new(48_000).unwrap());
        let t_ch0 = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("SynthCh0"));
        let t_ch1 = s.add_track(TrackKind::Instrument, ChannelFormat::Stereo, Some("SynthCh1"));

        s.track_mut(t_ch0).unwrap().instrument = Some(Insert::new("subtractive_synth"));
        s.track_mut(t_ch0).unwrap().mixer.record_arm = true;
        s.track_mut(t_ch0).unwrap().midi_channel = Some(0);

        s.track_mut(t_ch1).unwrap().instrument = Some(Insert::new("subtractive_synth"));
        s.track_mut(t_ch1).unwrap().mixer.record_arm = true;
        s.track_mut(t_ch1).unwrap().midi_channel = Some(1);

        let mut eng = MixEngine::new(48_000.0, 512);
        eng.monitor_only = true;

        eng.midi_input = vec![(0, LiveMidiEvent { port_id: 0, timestamp_us: 10, len: 3, data: [0x91, 60, 100] })];
        let mut out = vec![vec![0.0; 512]; 2];
        eng.render(&s, 0, 512, &mut out);

        assert!(eng.strips.get(&t_ch0).unwrap().held.is_empty(), "Track filtered to Ch 0 must ignore Ch 1 event");
        assert!(eng.strips.get(&t_ch1).unwrap().held.contains(1, 60), "Track filtered to Ch 1 must receive Ch 1 event");
    }
}
