use crate::clip::{bound, bound_sequence};
use crate::{
    Bus, BusId, ChannelFormat, Clip, ClipId, Group, GroupId, MarkerId, MarkerKind, MemoryLocation, OutputPath, Source, SourceId, SourcePool,
    TRACK_COLORS, Track, TrackId, TrackKind, VideoSource,
};
use openpt_time::{FrameRate, GridValue, Range, SampleRate, Samples, TempoMap, TimeFormat};

pub const SESSION_EXTENSION: &str = "openpt";

pub const FEET_FRAMES_START: &str = "feet_frames.start";

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SessionError {
    #[error("not a OpenPT session: {0}")]
    Format(String),
    #[error("session file version {0} is newer than this OpenPT supports")]
    Version(u32),
    #[error("{0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum BitDepthSetting {
    Int16,
    #[default]
    Int24,
    Float32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum EditMode {
    Shuffle,
    #[default]
    Slip,
    Spot,
    Grid,
    GridRelative,
}

impl EditMode {
    pub fn label(self) -> &'static str {
        match self {
            EditMode::Shuffle => "SHUFFLE",
            EditMode::Slip => "SLIP",
            EditMode::Spot => "SPOT",
            EditMode::Grid => "GRID",
            EditMode::GridRelative => "REL GRID",
        }
    }
    pub fn from_id(s: &str) -> Option<EditMode> {
        match s.to_ascii_lowercase().as_str() {
            "shuffle" => Some(EditMode::Shuffle),
            "slip" => Some(EditMode::Slip),
            "spot" => Some(EditMode::Spot),
            "grid" | "absolute_grid" => Some(EditMode::Grid),
            "relative_grid" | "rel_grid" | "grid_relative" => Some(EditMode::GridRelative),
            _ => None,
        }
    }
    pub fn is_grid(self) -> bool {
        matches!(self, EditMode::Grid | EditMode::GridRelative)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum Tool {
    Zoom,
    Trim,
    #[default]
    Selector,
    Grabber,
    Scrubber,
    Pencil,
    Smart,
}

impl Tool {
    pub const ALL: [Tool; 7] = [Tool::Zoom, Tool::Trim, Tool::Selector, Tool::Grabber, Tool::Scrubber, Tool::Pencil, Tool::Smart];
    pub fn id(self) -> &'static str {
        match self {
            Tool::Zoom => "zoom",
            Tool::Trim => "trim",
            Tool::Selector => "selector",
            Tool::Grabber => "grabber",
            Tool::Scrubber => "scrubber",
            Tool::Pencil => "pencil",
            Tool::Smart => "smart",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Tool::Zoom => "Zoomer",
            Tool::Trim => "Trim",
            Tool::Selector => "Selector",
            Tool::Grabber => "Grabber",
            Tool::Scrubber => "Scrubber",
            Tool::Pencil => "Pencil",
            Tool::Smart => "Smart Tool",
        }
    }
    pub fn from_id(s: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.id() == s.to_ascii_lowercase())
    }
    pub fn fkey(self) -> Option<&'static str> {
        match self {
            Tool::Zoom => Some("F5"),
            Tool::Trim => Some("F6"),
            Tool::Selector => Some("F7"),
            Tool::Grabber => Some("F8"),
            Tool::Scrubber => Some("F9"),
            Tool::Pencil => Some("F10"),
            Tool::Smart => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TrackView {
    Waveform,
    Blocks,
    Volume,
    Pan,
    Mute,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ZoomState {
    pub samples_per_px: f64,
    pub scroll: Samples,
    pub waveform_zoom: f32,
    pub presets: [f64; 5],
}

impl Default for ZoomState {
    fn default() -> Self {
        ZoomState { samples_per_px: 1024.0, scroll: 0, waveform_zoom: 1.0, presets: [64.0, 256.0, 1024.0, 4096.0, 16384.0] }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ZoomToggle {
    pub samples_per_px: f64,
    pub scroll: Samples,
    pub heights: Vec<(TrackId, crate::TrackHeight)>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EditState {
    pub tool: Tool,
    pub edit_mode: EditMode,
    pub selection: Range,
    pub selected_tracks: Vec<TrackId>,
    #[serde(default)]
    pub selected_clips: Vec<ClipId>,
    #[serde(default)]
    pub selected_sources: Vec<SourceId>,
    pub playhead: Samples,
    pub grid: GridValue,
    pub nudge: GridValue,
    #[serde(default = "yes")]
    pub grid_lines: bool,
    pub main_counter: TimeFormat,
    pub sub_counter: Option<TimeFormat>,
    pub zoom: ZoomState,
    pub zoom_toggle: Option<ZoomToggle>,
    pub rulers: Vec<String>,
    pub link_timeline_edit: bool,
    pub link_track_edit: bool,
    pub insertion_follows_playback: bool,
    pub loop_playback: bool,
    pub pre_post_roll: bool,
    pub pre_roll: Samples,
    pub post_roll: Samples,
    pub tab_to_transient: bool,
    pub automation_follows_edit: bool,
    pub markers_follow_edit: bool,
    pub layered_editing: bool,
    pub mirrored_midi: bool,
    pub click: bool,
    pub countoff: bool,
    pub countoff_bars: u32,
    pub midi_merge: bool,
    pub record_mode: String,
    pub loop_record: bool,
    pub quickpunch: bool,
    pub scrolling: String,
    pub solo_mode: String,
    #[serde(default)]
    pub afl_pfl_path: String,
    pub pre_fader_metering: bool,
    pub delay_compensation: bool,
    pub keyboard_focus: String,
    pub timeline_selection: Range,
    #[serde(default)]
    pub flags: std::collections::BTreeSet<String>,
    #[serde(default)]
    pub values: std::collections::BTreeMap<String, f64>,
}

impl EditState {
    pub fn solo_listen(&self) -> &str {
        self.solo_mode.split('+').next().filter(|m| matches!(*m, "afl" | "pfl")).unwrap_or("sip")
    }
    pub fn solo_latch(&self) -> &str {
        self.solo_mode.split('+').nth(1).filter(|m| matches!(*m, "xor" | "momentary")).unwrap_or("latch")
    }
    pub fn flag(&self, id: &str) -> bool {
        self.flags.contains(id)
    }
    pub fn set_flag(&mut self, id: &str, on: bool) {
        if on {
            self.flags.insert(id.to_string());
        } else {
            self.flags.remove(id);
        }
    }
    pub fn value(&self, id: &str, default: f64) -> f64 {
        self.values.get(id).copied().filter(|v| v.is_finite()).unwrap_or(default)
    }
}

fn yes() -> bool {
    true
}

impl Default for EditState {
    fn default() -> Self {
        EditState {
            tool: Tool::Smart,
            edit_mode: EditMode::Slip,
            selection: Range::default(),
            selected_tracks: Vec::new(),
            selected_clips: Vec::new(),
            selected_sources: Vec::new(),
            playhead: 0,
            grid: GridValue::default(),
            nudge: GridValue::Seconds(1.0),
            grid_lines: true,
            main_counter: TimeFormat::MinSecs,
            sub_counter: Some(TimeFormat::Samples),
            zoom: ZoomState::default(),
            zoom_toggle: None,
            rulers: vec!["bars_beats".into(), "min_secs".into(), "tempo".into(), "meter".into(), "markers".into()],
            link_timeline_edit: true,
            link_track_edit: false,
            insertion_follows_playback: false,
            loop_playback: false,
            pre_post_roll: false,
            pre_roll: 96_000,
            post_roll: 96_000,
            tab_to_transient: false,
            automation_follows_edit: true,
            markers_follow_edit: false,
            layered_editing: false,
            mirrored_midi: false,
            click: false,
            countoff: false,
            countoff_bars: 2,
            midi_merge: false,
            record_mode: "normal".into(),
            loop_record: false,
            quickpunch: false,
            scrolling: "page".into(),
            solo_mode: "sip".into(),
            afl_pfl_path: String::new(),
            pre_fader_metering: false,
            delay_compensation: true,
            keyboard_focus: "commands".into(),
            timeline_selection: Range::default(),
            flags: [
                "view.clip.name",
                "view.clip.gain_line",
                "waveform.peak",
                "view.clip.overlap_shadows",
                "view.marker.ruler_lines",
                "edit_view.inserts_ae",
                "edit_view.sends_ae",
                "edit_view.io",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            values: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SessionLink {
    pub path: String,
    pub tracks: Vec<String>,
    #[serde(default)]
    pub data: serde_json::Value,
    pub modified: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub format_version: u32,
    pub name: String,
    pub sample_rate: SampleRate,
    pub bit_depth: BitDepthSetting,
    pub frame_rate: FrameRate,
    pub timecode_start: Samples,
    pub tempo: TempoMap,
    #[serde(default)]
    pub key_signatures: Vec<(Samples, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ara_archives: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_sessions: Vec<SessionLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub midi_learn: Vec<crate::MidiLearn>,
    pub tracks: Vec<Track>,
    pub sources: Vec<Source>,
    #[serde(default)]
    pub videos: Vec<VideoSource>,
    pub busses: Vec<Bus>,
    pub outputs: Vec<OutputPath>,
    #[serde(default = "crate::default_inputs")]
    pub inputs: Vec<crate::InputPath>,
    #[serde(default)]
    pub heat: crate::HeatSettings,
    #[serde(default)]
    pub atmos: crate::AtmosSettings,
    pub markers: Vec<MemoryLocation>,
    pub groups: Vec<Group>,
    pub edit: EditState,
    pub next_id: u64,
    #[serde(default)]
    pub comments: String,
    #[serde(default)]
    pub midi_originals: std::collections::BTreeMap<u64, openpt_midi::Sequence>,
    #[serde(skip)]
    pub pool: SourcePool,
    #[serde(skip)]
    pub folder: Option<std::path::PathBuf>,
}

impl Default for Session {
    fn default() -> Self {
        Session::new("Untitled", SampleRate::HZ_48000)
    }
}

impl Session {
    pub fn input_first_channel(&self, name: &str) -> usize {
        if let Some(p) = self.inputs.iter().find(|p| p.name == name) {
            return usize::from(p.first_channel);
        }
        name.trim_start_matches("In ").split(['-', ' ']).next().and_then(|n| n.parse::<usize>().ok()).map_or(0, |n| n.saturating_sub(1))
    }

    pub const FORMAT_VERSION: u32 = 1;

    pub fn new(name: impl Into<String>, sample_rate: SampleRate) -> Self {
        let mut s = Session {
            format_version: Self::FORMAT_VERSION,
            name: name.into(),
            sample_rate,
            bit_depth: BitDepthSetting::Int24,
            frame_rate: FrameRate::Fps30,
            timecode_start: 0,
            tempo: TempoMap::default(),
            key_signatures: Vec::new(),
            ara_archives: Vec::new(),
            linked_sessions: Vec::new(),
            midi_learn: Vec::new(),
            tracks: Vec::new(),
            sources: Vec::new(),
            videos: Vec::new(),
            busses: Vec::new(),
            outputs: vec![OutputPath { name: "Out 1-2".into(), first_channel: 0, format: ChannelFormat::Stereo }],
            inputs: crate::default_inputs(),
            heat: crate::HeatSettings::default(),
            atmos: crate::AtmosSettings::default(),
            markers: Vec::new(),
            groups: Vec::new(),
            edit: EditState::default(),
            next_id: 1,
            comments: String::new(),
            midi_originals: std::collections::BTreeMap::new(),
            pool: SourcePool::default(),
            folder: None,
        };
        s.edit.pre_roll = sample_rate.samples(2.0);
        s.edit.post_roll = sample_rate.samples(2.0);
        s
    }

    pub fn alloc(&mut self) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id.saturating_add(1);
        id
    }

    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }
    pub fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }
    pub fn track_by_name(&self, name: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.name == name).or_else(|| self.tracks.iter().find(|t| t.name.eq_ignore_ascii_case(name)))
    }
    pub fn master(&self) -> Option<&Track> {
        self.tracks.iter().find(|t| t.kind == TrackKind::Master)
    }
    pub fn source(&self, id: SourceId) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }
    pub fn video(&self, id: SourceId) -> Option<&VideoSource> {
        self.videos.iter().find(|v| v.id == id)
    }
    pub fn bus(&self, id: BusId) -> Option<&Bus> {
        self.busses.iter().find(|b| b.id == id)
    }
    pub fn bus_by_name(&self, name: &str) -> Option<&Bus> {
        self.busses.iter().find(|b| b.name.eq_ignore_ascii_case(name))
    }
    pub fn busses_in_track_order(&self) -> Vec<&Bus> {
        let on = |r: &crate::Route, id: BusId| matches!(r, crate::Route::Bus(b) if *b == id);
        let rank = |bus: &Bus| {
            let listener = self.tracks.iter().position(|t| on(&t.mixer.input, bus.id));
            let source =
                || self.tracks.iter().position(|t| on(&t.mixer.output, bus.id) || t.mixer.sends.iter().flatten().any(|s| on(&s.target, bus.id)));
            listener.or_else(source).unwrap_or(usize::MAX)
        };
        let mut v: Vec<&Bus> = self.busses.iter().collect();
        v.sort_by_cached_key(|b| rank(b));
        v
    }
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn vca_controllers<'a>(&'a self, t: &'a Track) -> impl Iterator<Item = &'a Track> + 'a {
        self.tracks.iter().filter(move |v| {
            v.kind == TrackKind::Vca
                && v.id != t.id
                && match v.mixer.vca_group.and_then(|g| self.group(g)) {
                    Some(g) => g.members.contains(&t.id),
                    None => t.mixer.vca == Some(v.id.0),
                }
        })
    }

    pub fn vca_masters<'a>(&'a self, t: &'a Track) -> Vec<&'a Track> {
        let mut out: Vec<&Track> = Vec::new();
        let mut next = 0;
        out.extend(self.vca_controllers(t));
        out.retain(|v| v.id != t.id);
        while let Some(v) = out.get(next).copied() {
            next += 1;
            for up in self.vca_controllers(v) {
                if up.id != t.id && !out.iter().any(|x| x.id == up.id) {
                    out.push(up);
                }
            }
        }
        out
    }

    pub fn vca_members(&self, v: TrackId) -> Vec<TrackId> {
        self.tracks.iter().filter(|t| self.vca_controllers(t).any(|x| x.id == v)).map(|t| t.id).collect()
    }

    pub fn sync_vcas(&mut self, adopt: bool) -> bool {
        let mut changed = adopt && self.adopt_vcas_in_groups();
        let plan = self.vca_plan(adopt);
        changed |= !plan.is_empty();
        for (i, vca, group) in plan {
            if let Some(t) = self.tracks.get_mut(i) {
                t.mixer.vca = vca;
                t.mixer.vca_group = group;
            }
        }
        changed
    }

    fn adopt_vcas_in_groups(&mut self) -> bool {
        let taken: Vec<GroupId> = self.tracks.iter().filter_map(|t| t.mixer.vca_group).collect();
        let mut binds: Vec<(TrackId, GroupId)> = Vec::new();
        for v in self.tracks.iter().filter(|t| t.kind == TrackKind::Vca && t.mixer.vca_group.is_none()) {
            let mut inside = self.groups.iter().filter(|g| g.mix && g.members.contains(&v.id) && !taken.contains(&g.id));
            if let (Some(g), None) = (inside.next(), inside.next())
                && !binds.iter().any(|(_, x)| *x == g.id)
            {
                binds.push((v.id, g.id));
            }
        }
        for (v, g) in &binds {
            if let Some(t) = self.track_mut(*v) {
                t.mixer.vca_group = Some(*g);
            }
            if let Some(gr) = self.groups.iter_mut().find(|x| x.id == *g) {
                gr.members.retain(|m| m != v);
            }
        }
        !binds.is_empty()
    }

    pub fn vca_plan(&self, adopt: bool) -> Vec<(usize, Option<u64>, Option<GroupId>)> {
        let vca = |id: u64| self.tracks.iter().find(|t| t.id.0 == id && t.kind == TrackKind::Vca);
        let mut group: Vec<Option<GroupId>> =
            self.tracks.iter().map(|t| t.mixer.vca_group.filter(|g| t.kind == TrackKind::Vca && self.group(*g).is_some())).collect();
        if adopt {
            for (i, v) in self.tracks.iter().enumerate() {
                if v.kind != TrackKind::Vca || group[i].is_some() {
                    continue;
                }
                let mut mine: Vec<u64> = self.tracks.iter().filter(|t| t.mixer.vca == Some(v.id.0)).map(|t| t.id.0).collect();
                mine.sort_unstable();
                group[i] = self
                    .groups
                    .iter()
                    .find(|g| {
                        let mut m: Vec<u64> = g.members.iter().map(|x| x.0).filter(|x| *x != v.id.0).collect();
                        m.sort_unstable();
                        g.mix && !mine.is_empty() && m == mine
                    })
                    .map(|g| g.id);
            }
        }
        let grouped = |id: u64| self.tracks.iter().position(|t| t.id.0 == id).is_some_and(|i| group[i].is_some());
        let link: Vec<Option<u64>> = self.tracks.iter().map(|t| t.mixer.vca.filter(|v| vca(*v).is_some() && !grouped(*v))).collect();
        self.tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| t.mixer.vca != link[*i] || t.mixer.vca_group != group[*i])
            .map(|(i, _)| (i, link[i], group[i]))
            .collect()
    }
    pub fn marker(&self, id: MarkerId) -> Option<&MemoryLocation> {
        self.markers.iter().find(|m| m.id == id)
    }

    pub fn find_clip(&self, id: ClipId) -> Option<(TrackId, &Clip)> {
        self.tracks.iter().find_map(|t| t.clips().iter().find(|c| c.id == id).map(|c| (t.id, c)))
    }

    pub fn find_clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.tracks.iter_mut().find_map(|t| t.playlist_mut().and_then(|p| p.clip_mut(id)))
    }

    pub fn all_clips(&self) -> impl Iterator<Item = &Clip> {
        self.tracks.iter().flat_map(|t| t.playlists.iter().flat_map(|p| &p.clips))
    }

    pub fn clips_using(&self, src: SourceId) -> impl Iterator<Item = &Clip> {
        self.all_clips().filter(move |c| c.source() == Some(src))
    }

    pub fn content_end(&self) -> Samples {
        self.tracks.iter().filter_map(|t| t.playlist().map(|p| p.end())).max().unwrap_or(0)
    }

    pub fn feet_frames_start(&self) -> i64 {
        self.edit.value(FEET_FRAMES_START, 0.0) as i64
    }

    pub fn format_position(&self, at: Samples, fmt: TimeFormat) -> String {
        openpt_time::format_position(at, fmt, self.sample_rate, &self.tempo, self.frame_rate, self.timecode_start, self.feet_frames_start())
    }

    pub fn parse_position(&self, text: &str, fmt: TimeFormat) -> Result<Samples, openpt_time::TimeError> {
        openpt_time::parse_position(text, fmt, self.sample_rate, &self.tempo, self.frame_rate, self.timecode_start, self.feet_frames_start())
    }

    pub fn next_track_color(&self) -> [u8; 3] {
        let n = self.tracks.iter().filter(|t| t.kind != TrackKind::Master).count();
        TRACK_COLORS.get(n % TRACK_COLORS.len()).copied().unwrap_or([128, 128, 128])
    }

    pub fn unique_track_name(&self, base: &str) -> String {
        let taken = |n: &str| self.tracks.iter().any(|t| t.name == n);
        if !base.is_empty() && !taken(base) && !matches!(base, "Audio" | "Aux" | "MIDI" | "Inst" | "VCA" | "Folder" | "Master") {
            return base.to_string();
        }
        let prefix = format!("{base} ");
        let used: std::collections::HashSet<u64> = self
            .tracks
            .iter()
            .filter_map(|t| t.name.strip_prefix(&prefix))
            .filter_map(|n| n.parse().ok().filter(|i: &u64| i.to_string() == n))
            .collect();
        let i = (1..).find(|i| !used.contains(i)).unwrap_or(1);
        format!("{base} {i}")
    }

    pub fn add_track(&mut self, kind: TrackKind, format: ChannelFormat, name: Option<&str>) -> TrackId {
        let id = TrackId(self.alloc());
        let name = self.unique_track_name(name.unwrap_or(kind.default_name()));
        let color = if kind == TrackKind::Master { [140, 140, 140] } else { self.next_track_color() };
        let mut t = Track::new(id, name, kind, format, color);
        if kind == TrackKind::Master {
            t.mixer.output = crate::Route::Main;
            t.mixer.input = crate::Route::Main;
        }
        let pos = if kind == TrackKind::Master {
            self.tracks.len()
        } else {
            self.tracks.iter().position(|t| t.kind == TrackKind::Master).unwrap_or(self.tracks.len())
        };
        self.tracks.insert(pos, t);
        id
    }

    pub fn grid_anchors(&self) -> Vec<Samples> {
        let mut a: Vec<Samples> = self.tracks.iter().flat_map(|t| t.clips().iter().flat_map(|c| [c.start, c.end()])).collect();
        a.extend(self.markers.iter().map(|m| m.start));
        a.sort_unstable();
        a.dedup();
        a
    }

    pub fn snap_to_grid(&self, pos: Samples) -> Samples {
        let g = self.edit.grid;
        let anchors = if g == openpt_time::GridValue::ClipsMarkers { self.grid_anchors() } else { Vec::new() };
        g.snap_with(pos, self.sample_rate, &self.tempo, self.frame_rate, &anchors)
    }

    pub fn grid_line_at_or_before(&self, pos: Samples) -> Samples {
        let g = self.edit.grid;
        let anchors = if g == openpt_time::GridValue::ClipsMarkers { self.grid_anchors() } else { Vec::new() };
        g.line_at_or_before_with(pos, self.sample_rate, &self.tempo, self.frame_rate, &anchors)
    }

    pub fn grid_lines(&self, start: Samples, end: Samples, max: usize) -> Vec<Samples> {
        let g = self.edit.grid;
        let anchors = if g == openpt_time::GridValue::ClipsMarkers { self.grid_anchors() } else { Vec::new() };
        g.lines_with(start, end, self.sample_rate, &self.tempo, self.frame_rate, max, &anchors)
    }

    pub fn add_bus(&mut self, name: &str, format: ChannelFormat) -> BusId {
        if let Some(b) = self.bus_by_name(name) {
            return b.id;
        }
        let id = BusId(self.alloc());
        self.busses.push(Bus { id, name: name.to_string(), format });
        id
    }

    pub fn add_marker(&mut self, name: &str, kind: MarkerKind, start: Samples, end: Samples) -> MarkerId {
        let id = MarkerId(self.alloc());
        let number = self.markers.iter().map(|m| m.number).max().unwrap_or(0).saturating_add(1);
        self.markers.push(MemoryLocation {
            id,
            number,
            name: name.to_string(),
            kind,
            start,
            end: end.max(start),
            comments: String::new(),
            color: None,
            ruler: 1,
            track: None,
        });
        self.markers.sort_by_key(|m| m.start);
        id
    }

    pub fn new_clip_id(&mut self) -> ClipId {
        ClipId(self.alloc())
    }

    pub fn to_json(&self) -> Result<String, SessionError> {
        serde_json::to_string_pretty(self).map_err(|e| SessionError::Format(e.to_string()))
    }

    pub fn from_json(text: &str) -> Result<Session, SessionError> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| SessionError::Format(e.to_string()))?;
        let ver = v.get("format_version").and_then(serde_json::Value::as_u64).ok_or_else(|| SessionError::Format("missing format_version".into()))?;
        if ver > u64::from(Self::FORMAT_VERSION) {
            return Err(SessionError::Version(u32::try_from(ver).unwrap_or(u32::MAX)));
        }
        let mut s: Session = serde_json::from_value(v).map_err(|e| SessionError::Format(e.to_string()))?;
        s.sanitize();
        Ok(s)
    }

    pub fn sanitize(&mut self) {
        let max_id = self
            .tracks
            .iter()
            .map(|t| t.id.0)
            .chain(self.tracks.iter().flat_map(|t| t.playlists.iter().flat_map(|p| p.clips.iter().map(|c| c.id.0))))
            .chain(self.sources.iter().map(|s| s.id.0))
            .chain(self.videos.iter().map(|v| v.id.0))
            .chain(self.busses.iter().map(|b| b.id.0))
            .chain(self.markers.iter().map(|m| m.id.0))
            .chain(self.groups.iter().map(|g| g.id.0))
            .max()
            .unwrap_or(0);
        self.next_id = self.next_id.max(max_id.saturating_add(1));
        truncate_chars(&mut self.name, MAX_NAME_CHARS);
        truncate_chars(&mut self.comments, MAX_COMMENT_CHARS);
        for t in &mut self.tracks {
            t.mixer.sanitize();
            truncate_chars(&mut t.name, MAX_NAME_CHARS);
            truncate_chars(&mut t.comments, MAX_COMMENT_CHARS);
            if t.playlists.is_empty() {
                t.playlists.push(crate::Playlist::new(t.name.clone()));
            }
            if t.active_playlist >= t.playlists.len() {
                t.active_playlist = 0;
            }
            for p in &mut t.playlists {
                truncate_chars(&mut p.name, MAX_NAME_CHARS);
                p.clips.retain(|c| c.length > 0);
                p.clips.iter_mut().for_each(Clip::sanitize);
                p.sort();
            }
            for l in &mut t.automation {
                l.points.retain(|p| p.value.is_finite());
                l.points.iter_mut().for_each(|p| bound(&mut p.at));
                l.points.sort_by_key(|p| p.at);
            }
        }
        for m in &mut self.markers {
            bound(&mut m.start);
            bound(&mut m.end);
            truncate_chars(&mut m.name, MAX_NAME_CHARS);
            truncate_chars(&mut m.comments, MAX_COMMENT_CHARS);
        }
        for (at, key) in &mut self.key_signatures {
            bound(at);
            truncate_chars(key, MAX_NAME_CHARS);
        }
        self.midi_originals.values_mut().for_each(bound_sequence);
        let e = &mut self.edit;
        for at in [
            &mut e.selection.start,
            &mut e.selection.end,
            &mut e.timeline_selection.start,
            &mut e.timeline_selection.end,
            &mut e.playhead,
            &mut e.pre_roll,
            &mut e.post_roll,
            &mut e.zoom.scroll,
            &mut self.timecode_start,
        ] {
            bound(at);
        }
        for v in &mut self.videos {
            if !v.frame_rate.is_finite() || v.frame_rate < 0.0 {
                v.frame_rate = 0.0;
            }
            if !v.duration.is_finite() || v.duration < 0.0 {
                v.duration = 0.0;
            }
            truncate_chars(&mut v.name, MAX_NAME_CHARS);
        }
        self.busses.iter_mut().for_each(|b| truncate_chars(&mut b.name, MAX_NAME_CHARS));
        self.outputs.iter_mut().for_each(|o| truncate_chars(&mut o.name, MAX_NAME_CHARS));
        self.groups.iter_mut().for_each(|g| truncate_chars(&mut g.name, MAX_NAME_CHARS));
        self.sources.iter_mut().for_each(|s| truncate_chars(&mut s.name, MAX_NAME_CHARS));
        let clip_ids: std::collections::BTreeSet<u64> =
            self.tracks.iter().flat_map(|t| t.playlists.iter().flat_map(|p| p.clips.iter().map(|c| c.id.0))).collect();
        self.midi_originals.retain(|id, _| clip_ids.contains(id));
        if !self.edit.zoom.samples_per_px.is_finite() || self.edit.zoom.samples_per_px <= 0.0 {
            self.edit.zoom.samples_per_px = 1024.0;
        }
        if SampleRate::new(self.sample_rate.hz()).is_err() {
            self.sample_rate = SampleRate::default();
        }
        self.tempo.sanitize();
    }
}

pub const MAX_NAME_CHARS: usize = 256;
pub const MAX_COMMENT_CHARS: usize = 4096;

pub fn truncate_chars(s: &mut String, max: usize) {
    if let Some((end, _)) = s.char_indices().nth(max) {
        s.truncate(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_tracks_names_and_order() {
        let mut s = Session::default();
        let m = s.add_track(TrackKind::Master, ChannelFormat::Stereo, None);
        let a = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let b = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        let v = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Vocal"));
        assert_eq!(s.track(a).map(|t| t.name.as_str()), Some("Audio 1"));
        assert_eq!(s.track(b).map(|t| t.name.as_str()), Some("Audio 2"));
        assert_eq!(s.track(v).map(|t| t.name.as_str()), Some("Vocal"));
        assert_eq!(s.tracks.last().map(|t| t.id), Some(m));
        let v2 = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, Some("Vocal"));
        assert_eq!(s.track(v2).map(|t| t.name.as_str()), Some("Vocal 1"));
    }

    #[test]
    fn json_round_trip() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Stereo, None);
        let cid = s.new_clip_id();
        if let Some(p) = s.track_mut(t).and_then(Track::playlist_mut) {
            p.clips.push(Clip::audio(cid, "c", SourceId(99), 0, 10, 100));
        }
        s.add_marker("Verse", MarkerKind::Marker, 48_000, 48_000);
        let text = s.to_json().unwrap();
        let back = Session::from_json(&text).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn video_clip_round_trips_and_old_sessions_load() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Video, ChannelFormat::Mono, None);
        let vid = SourceId(s.alloc());
        s.videos.push(VideoSource {
            id: vid,
            name: "cut.mov".into(),
            path: "/films/cut.mov".into(),
            width: 1920,
            height: 1080,
            frame_rate: 23.976,
            duration: 12.5,
            codec: "prores".into(),
        });
        let cid = s.new_clip_id();
        if let Some(p) = s.track_mut(t).and_then(Track::playlist_mut) {
            p.clips.push(Clip::video(cid, "cut", vid, 480, 0, 600_000));
        }
        let back = Session::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back, s);
        let c = &back.track(t).unwrap().clips()[0];
        assert_eq!((c.video_source(), c.source(), c.source_offset(), c.is_audio()), (Some(vid), None, 480, false));
        assert_eq!(back.track(t).unwrap().kind.label(), "Video Track");
        assert!(back.video(vid).is_some());
        let mut v = serde_json::to_value(Session::default()).unwrap();
        v.as_object_mut().unwrap().remove("videos");
        assert!(Session::from_json(&v.to_string()).unwrap().videos.is_empty());
        let mut c = c.clone();
        c.trim_start_to(100);
        assert_eq!(c.source_offset(), 580);
    }

    #[test]
    fn unique_track_names_take_the_first_free_number() {
        let mut s = Session::default();
        for name in ["Piano", "Piano 1", "Piano 3", "Piano 02", "Pianos 2"] {
            s.add_track(TrackKind::Audio, ChannelFormat::Mono, Some(name));
        }
        assert_eq!(s.unique_track_name("Piano"), "Piano 2");
        assert_eq!(s.unique_track_name("Organ"), "Organ");
        assert_eq!(s.unique_track_name("Audio"), "Audio 1");
    }

    #[test]
    fn names_from_a_file_are_cut_to_a_length_that_can_be_drawn() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        if let Some(tr) = s.track_mut(t) {
            tr.name = "é".repeat(1_000);
        }
        s.comments = "c".repeat(10_000);
        let back = Session::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back.track(t).map(|tr| tr.name.chars().count()), Some(MAX_NAME_CHARS));
        assert_eq!(back.comments.len(), MAX_COMMENT_CHARS);
    }

    #[test]
    fn a_bad_sample_rate_or_tempo_is_repaired_on_load() {
        let mut v = serde_json::to_value(Session::default()).unwrap();
        v["sample_rate"] = serde_json::json!(0);
        v["tempo"]["tempos"] = serde_json::json!([{"tick": 0, "bpm": 4e9}]);
        let s = Session::from_json(&v.to_string()).unwrap();
        assert_eq!((s.sample_rate, s.tempo), (SampleRate::HZ_48000, TempoMap::default()));
    }

    #[test]
    fn rejects_garbage_and_future_versions() {
        assert!(Session::from_json("not json").is_err());
        assert!(Session::from_json("{}").is_err());
        let mut v = serde_json::to_value(Session::default()).unwrap();
        v["format_version"] = serde_json::json!(999);
        assert!(matches!(Session::from_json(&v.to_string()), Err(SessionError::Version(999))));
    }

    #[test]
    fn sanitize_repairs() {
        let mut s = Session::default();
        let t = s.add_track(TrackKind::Audio, ChannelFormat::Mono, None);
        if let Some(tr) = s.track_mut(t) {
            tr.active_playlist = 7;
            tr.mixer.volume_db = f32::NAN;
        }
        s.next_id = 0;
        s.sanitize();
        let tr = s.track(t).unwrap();
        assert_eq!(tr.active_playlist, 0);
        assert_eq!(tr.mixer.volume_db, 0.0);
        assert!(s.next_id > t.0);
    }
}
