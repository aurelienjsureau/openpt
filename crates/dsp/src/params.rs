use crate::{ParamInfo, Taper, Unit};

pub(crate) const fn p(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: Unit, taper: Taper) -> ParamInfo {
    ParamInfo { id, name, min, max, default, unit, taper, choices: &[] }
}

pub(crate) const fn lin(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> ParamInfo {
    p(id, name, min, max, default, unit, Taper::Linear)
}

pub(crate) const fn log(id: &'static str, name: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> ParamInfo {
    p(id, name, min, max, default, unit, Taper::Log)
}

pub(crate) const fn toggle(id: &'static str, name: &'static str, default_on: bool) -> ParamInfo {
    ParamInfo { id, name, min: 0.0, max: 1.0, default: if default_on { 1.0 } else { 0.0 }, unit: Unit::Toggle, taper: Taper::Linear, choices: &[] }
}

pub(crate) const fn choice(id: &'static str, name: &'static str, choices: &'static [&'static str], default: usize) -> ParamInfo {
    let max = if choices.is_empty() { 0.0 } else { (choices.len() - 1) as f32 };
    ParamInfo { id, name, min: 0.0, max, default: default as f32, unit: Unit::Choice, taper: Taper::Linear, choices }
}

#[derive(Debug, Clone)]
pub(crate) struct Params {
    info: &'static [ParamInfo],
    values: Vec<f32>,
}

impl Params {
    pub(crate) fn new(info: &'static [ParamInfo]) -> Self {
        Self { info, values: info.iter().map(|p| p.default).collect() }
    }

    pub(crate) fn set(&mut self, id: &str, value: f32) -> Option<usize> {
        let idx = self.info.iter().position(|p| p.id == id)?;
        let info = self.info.get(idx)?;
        let slot = self.values.get_mut(idx)?;
        *slot = info.clamp(value);
        Some(idx)
    }

    pub(crate) fn get(&self, id: &str) -> Option<f32> {
        let idx = self.info.iter().position(|p| p.id == id)?;
        self.values.get(idx).copied()
    }

    #[inline]
    pub(crate) fn v(&self, idx: usize) -> f32 {
        self.values.get(idx).copied().unwrap_or(0.0)
    }

    #[inline]
    pub(crate) fn on(&self, idx: usize) -> bool {
        self.v(idx) >= 0.5
    }

    #[inline]
    pub(crate) fn choice(&self, idx: usize) -> usize {
        let v = self.v(idx);
        if v.is_finite() && v > 0.0 { v.round() as usize } else { 0 }
    }
}

macro_rules! param_plumbing {
    ($info:expr) => {
        fn info(&self) -> &'static $crate::PluginInfo {
            &$info
        }
        fn set_param(&mut self, id: &str, value: f32) -> bool {
            if self.p.set(id, value).is_some() {
                self.update();
                true
            } else {
                false
            }
        }
        fn param(&self, id: &str) -> Option<f32> {
            self.p.get(id)
        }
    };
}
pub(crate) use param_plumbing;
