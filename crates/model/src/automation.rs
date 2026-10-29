use openpt_time::Samples;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
pub enum AutomationMode {
    Off,
    #[default]
    Read,
    Touch,
    Latch,
    TouchLatch,
    Write,
    Trim,
}

impl AutomationMode {
    pub const ALL: [AutomationMode; 7] = [
        AutomationMode::Off,
        AutomationMode::Read,
        AutomationMode::Touch,
        AutomationMode::Latch,
        AutomationMode::TouchLatch,
        AutomationMode::Write,
        AutomationMode::Trim,
    ];
    pub fn label(self) -> &'static str {
        match self {
            AutomationMode::Off => "off",
            AutomationMode::Read => "read",
            AutomationMode::Touch => "touch",
            AutomationMode::Latch => "latch",
            AutomationMode::TouchLatch => "touch/latch",
            AutomationMode::Write => "write",
            AutomationMode::Trim => "trim",
        }
    }
    pub fn from_id(s: &str) -> Option<AutomationMode> {
        let s = s.to_ascii_lowercase();
        AutomationMode::ALL.into_iter().find(|m| m.label() == s || m.label().replace('/', "_") == s || format!("{m:?}").to_ascii_lowercase() == s)
    }
    pub fn reads(self) -> bool {
        !matches!(self, AutomationMode::Off)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum AutoParam {
    Volume,
    Pan(u8),
    Mute,
    SendLevel(u8),
    SendPan(u8),
    SendMute(u8),
    Surround(u8),
    Plugin {
        slot: u8,
        param: String,
    },
}

pub const BYPASS_PARAM: &str = "master_bypass";
pub const SURROUND_AXES: [&str; 4] = ["x", "y", "z", "size"];

impl AutoParam {
    pub fn label(&self) -> String {
        match self {
            AutoParam::Volume => "volume".into(),
            AutoParam::Pan(0) => "pan".into(),
            AutoParam::Pan(i) => format!("pan {}", i + 1),
            AutoParam::Mute => "mute".into(),
            AutoParam::SendLevel(i) => format!("send {} level", slot_letter(*i)),
            AutoParam::SendPan(i) => format!("send {} pan", slot_letter(*i)),
            AutoParam::SendMute(i) => format!("send {} mute", slot_letter(*i)),
            AutoParam::Surround(a) => format!("surround {}", SURROUND_AXES.get(usize::from(*a)).copied().unwrap_or("x")),
            AutoParam::Plugin { slot, param } => format!("insert {} {param}", slot_letter(*slot)),
        }
    }
    pub fn parse(s: &str) -> Option<AutoParam> {
        let s = s.trim().to_ascii_lowercase();
        match s.as_str() {
            "volume" | "vol" => return Some(AutoParam::Volume),
            "pan" | "pan1" => return Some(AutoParam::Pan(0)),
            "pan2" => return Some(AutoParam::Pan(1)),
            "mute" => return Some(AutoParam::Mute),
            "surround_x" => return Some(AutoParam::Surround(0)),
            "surround_y" => return Some(AutoParam::Surround(1)),
            "surround_z" => return Some(AutoParam::Surround(2)),
            "surround_size" => return Some(AutoParam::Surround(3)),
            _ => {}
        }
        if let Some(rest) = s.strip_prefix("send_") {
            let mut it = rest.splitn(2, '_');
            let slot = letter_slot(it.next()?)?;
            return match it.next()? {
                "level" => Some(AutoParam::SendLevel(slot)),
                "pan" => Some(AutoParam::SendPan(slot)),
                "mute" => Some(AutoParam::SendMute(slot)),
                _ => None,
            };
        }
        if let Some(rest) = s.strip_prefix("plugin:") {
            let (slot, param) = rest.split_once(':')?;
            return Some(AutoParam::Plugin { slot: letter_slot(slot)?, param: param.to_string() });
        }
        None
    }
    pub fn range(&self) -> (f32, f32, f32) {
        match self {
            AutoParam::Volume | AutoParam::SendLevel(_) => (-144.0, 12.0, 0.0),
            AutoParam::Pan(_) | AutoParam::SendPan(_) => (-1.0, 1.0, 0.0),
            AutoParam::Mute | AutoParam::SendMute(_) => (0.0, 1.0, 0.0),
            AutoParam::Surround(0) => (-1.0, 1.0, 0.0),
            AutoParam::Surround(1) => (-1.0, 1.0, 1.0),
            AutoParam::Surround(_) => (0.0, 1.0, 0.0),
            AutoParam::Plugin { .. } => (f32::MIN, f32::MAX, 0.0),
        }
    }
}

pub fn slot_letter(i: u8) -> char {
    char::from(b'A'.saturating_add(i.min(25)))
}

fn letter_slot(s: &str) -> Option<u8> {
    let c = s.chars().next()?.to_ascii_uppercase();
    if s.len() == 1 && c.is_ascii_uppercase() { Some(c as u8 - b'A') } else { s.parse().ok() }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AutomationPoint {
    pub at: Samples,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AutomationLane {
    pub param: AutoParam,
    pub points: Vec<AutomationPoint>,
    #[serde(default)]
    pub enabled: bool,
}

impl AutomationLane {
    pub fn new(param: AutoParam) -> Self {
        AutomationLane { param, points: Vec::new(), enabled: true }
    }

    pub fn value_at(&self, at: Samples, default: f32) -> f32 {
        let pts = &self.points;
        let Some(first) = pts.first() else { return default };
        if at <= first.at {
            return first.value;
        }
        let idx = pts.partition_point(|p| p.at <= at);
        match (pts.get(idx.wrapping_sub(1)), pts.get(idx)) {
            (Some(a), Some(b)) if b.at > a.at => {
                if matches!(self.param, AutoParam::Mute | AutoParam::SendMute(_)) {
                    return a.value;
                }
                a.value + (b.value - a.value) * ((at - a.at) as f32 / (b.at - a.at) as f32)
            }
            (Some(a), _) => a.value,
            _ => default,
        }
    }

    pub fn set_point(&mut self, at: Samples, value: f32) {
        let (lo, hi, _) = self.param.range();
        let value = if value.is_nan() { 0.0 } else { value.clamp(lo, hi) };
        match self.points.binary_search_by(|p| p.at.cmp(&at)) {
            Ok(i) => {
                if let Some(p) = self.points.get_mut(i) {
                    p.value = value;
                }
            }
            Err(i) => self.points.insert(i, AutomationPoint { at, value }),
        }
    }

    pub fn clear_range(&mut self, start: Samples, end: Samples) -> usize {
        let before = self.points.len();
        self.points.retain(|p| p.at < start || p.at >= end);
        before - self.points.len()
    }

    pub fn trim_range(&mut self, start: Samples, end: Samples, delta: f32, default: f32) {
        if end <= start || !delta.is_finite() {
            return;
        }
        let mut anchors = vec![
            (start, self.value_at(start, default)),
            (end, self.value_at(end, default)),
            (end.saturating_add(1), self.value_at(end.saturating_add(1), default)),
        ];
        if start > 0 {
            anchors.push((start - 1, self.value_at(start - 1, default)));
        }
        for (at, v) in anchors {
            if self.points.binary_search_by(|p| p.at.cmp(&at)).is_err() {
                self.set_point(at, v);
            }
        }
        let (lo, hi, _) = self.param.range();
        for p in self.points.iter_mut().filter(|p| p.at >= start && p.at <= end) {
            p.value = (p.value + delta).clamp(lo, hi);
        }
    }

    pub fn offset_points(&mut self, start: Samples, end: Samples, delta: f32) {
        if !delta.is_finite() {
            return;
        }
        let (lo, hi, _) = self.param.range();
        for p in self.points.iter_mut().filter(|p| p.at >= start && p.at <= end) {
            p.value = (p.value + delta).clamp(lo, hi);
        }
    }

    pub fn write_range(&mut self, start: Samples, end: Samples, value: f32, default: f32) {
        if end <= start {
            return;
        }
        let before = self.value_at(start.saturating_sub(1), default);
        let after = self.value_at(end, default);
        self.clear_range(start, end.saturating_add(1));
        if start > 0 {
            self.set_point(start - 1, before);
        }
        self.set_point(start, value);
        self.set_point(end, value);
        self.set_point(end.saturating_add(1), after);
    }

    pub fn thin(&mut self, tolerance: f32) -> usize {
        if self.points.len() < 3 || tolerance.is_nan() {
            return 0;
        }
        let tolerance = f64::from(tolerance);
        let mut keep = vec![true; self.points.len()];
        let mut last = 0usize;
        let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
        for i in 1..self.points.len() - 1 {
            let (Some(a), Some(b), Some(c)) = (self.points.get(last), self.points.get(i), self.points.get(i + 1)) else { continue };
            let dt = b.at.saturating_sub(a.at) as f64;
            let rise = f64::from(b.value) - f64::from(a.value);
            lo = lo.max((rise - tolerance) / dt);
            hi = hi.min((rise + tolerance) / dt);
            let slope = (f64::from(c.value) - f64::from(a.value)) / c.at.saturating_sub(a.at) as f64;
            if (lo..=hi).contains(&slope) {
                if let Some(k) = keep.get_mut(i) {
                    *k = false;
                }
            } else {
                last = i;
                (lo, hi) = (f64::NEG_INFINITY, f64::INFINITY);
            }
        }
        let before = self.points.len();
        let mut it = keep.iter();
        self.points.retain(|_| it.next().copied().unwrap_or(true));
        before - self.points.len()
    }

    pub fn shift_from(&mut self, from: Samples, delta: Samples) {
        for p in &mut self.points {
            if p.at >= from {
                p.at = p.at.saturating_add(delta).max(0);
            }
        }
        self.points.sort_by_key(|p| p.at);
        self.points.dedup_by_key(|p| p.at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_and_steps() {
        let mut l = AutomationLane::new(AutoParam::Volume);
        assert_eq!(l.value_at(10, -3.0), -3.0);
        l.set_point(0, 0.0);
        l.set_point(100, -10.0);
        assert!((l.value_at(50, 0.0) + 5.0).abs() < 1e-6);
        assert_eq!(l.value_at(1000, 0.0), -10.0);
        let mut m = AutomationLane::new(AutoParam::Mute);
        m.set_point(0, 0.0);
        m.set_point(100, 1.0);
        assert_eq!(m.value_at(99, 0.0), 0.0);
        assert_eq!(m.value_at(100, 0.0), 1.0);
    }

    #[test]
    fn write_range_preserves_outside() {
        let mut l = AutomationLane::new(AutoParam::Volume);
        l.set_point(0, 0.0);
        l.set_point(1000, 0.0);
        l.write_range(200, 400, -20.0, 0.0);
        assert_eq!(l.value_at(100, 0.0), 0.0);
        assert_eq!(l.value_at(300, 0.0), -20.0);
        assert_eq!(l.value_at(600, 0.0), 0.0);
    }

    #[test]
    fn thin_removes_collinear() {
        let mut l = AutomationLane::new(AutoParam::Volume);
        for i in 0..=10 {
            l.set_point(i * 10, -(i as f32));
        }
        assert_eq!(l.thin(0.01), 9);
        assert_eq!(l.points.len(), 2);
    }

    #[test]
    fn thin_keeps_removed_points_within_tolerance() {
        for direction in [1.0, -1.0] {
            let mut lane = AutomationLane::new(AutoParam::Volume);
            for (at, value) in [(0, 0.0), (100, 0.0), (200, 0.0), (300, 0.12), (400, 0.26)] {
                lane.set_point(at, value * direction);
            }
            let original = lane.points.clone();
            let tolerance = 0.1;
            assert!(lane.thin(tolerance) > 0);
            for point in original {
                let error = (lane.value_at(point.at, 0.0) - point.value).abs();
                assert!(error <= tolerance, "at {}: error {error} exceeds {tolerance}", point.at);
            }
        }
    }

    #[test]
    fn param_ids() {
        assert_eq!(AutoParam::parse("send_b_level"), Some(AutoParam::SendLevel(1)));
        assert_eq!(AutoParam::parse("plugin:a:threshold"), Some(AutoParam::Plugin { slot: 0, param: "threshold".into() }));
        assert_eq!(AutoParam::parse("nope"), None);
        assert_eq!(AutomationMode::from_id("touch/latch"), Some(AutomationMode::TouchLatch));
    }

    #[test]
    fn set_point_clamps_nan() {
        let mut l = AutomationLane::new(AutoParam::Pan(0));
        l.set_point(0, f32::NAN);
        l.set_point(1, 5.0);
        assert_eq!(l.points[0].value, 0.0);
        assert_eq!(l.points[1].value, 1.0);
    }
}
