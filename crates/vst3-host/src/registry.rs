use crate::ffi::{Bundle, Instance, PARAM_BYPASS, PARAM_HIDDEN, PARAM_READ_ONLY, RawParam};
use crate::plugin::Vst3Plugin;
use crate::scan as paths;
use crate::{AUDIO_MODULE_CLASS, Vst3Descriptor, Vst3Error, parse_id};
use openpt_dsp::{ParamInfo, Plugin, PluginInfo, Taper, Unit};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError, TryLockError};

const MAX_CHOICES: i32 = 128;

#[derive(Default)]
struct Registry {
    scanned: Option<Vec<Vst3Descriptor>>,
    extra_dirs: Vec<PathBuf>,
    infos: HashMap<String, &'static PluginInfo>,
    failed: HashSet<String>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry::default()))
}

pub fn load_bundle(path: &Path) -> Result<Vec<Vst3Descriptor>, Vst3Error> {
    let b = Bundle::load(path)?;
    Ok(b.classes()
        .into_iter()
        .filter(|c| c.category == AUDIO_MODULE_CLASS)
        .map(|c| {
            let raw = crate::isolated::RawClass {
                class_id: crate::ffi::hex(&c.cid),
                name: c.name,
                vendor: c.vendor,
                version: c.version,
                sdk_version: c.sdk_version,
                sub_categories: c.sub_categories,
            };
            descriptor(path, raw)
        })
        .collect())
}

fn descriptor(path: &Path, c: crate::isolated::RawClass) -> Vst3Descriptor {
    let subs = paths::split_sub_categories(&c.sub_categories);
    Vst3Descriptor {
        id: format!("{}{}", crate::ID_PREFIX, c.class_id),
        category: paths::category_from_sub_categories(&subs),
        is_instrument: paths::is_instrument(&subs),
        class_id: c.class_id,
        name: if c.name.is_empty() { "VST3 plugin".to_string() } else { c.name },
        vendor: c.vendor,
        version: c.version,
        sdk_version: c.sdk_version,
        sub_categories: subs,
        path: path.to_path_buf(),
    }
}

pub fn scan_paths(dirs: &[PathBuf]) -> Vec<Vst3Descriptor> {
    let bundles = paths::find_bundles(dirs);
    let found: Vec<Vec<Vst3Descriptor>> = match crate::isolated::scanner() {
        Some(sc) => crate::isolated::scan(sc, &bundles)
            .into_iter()
            .map(|(p, o)| match o {
                crate::isolated::Outcome::Loaded(cs) => cs.into_iter().map(|c| descriptor(&p, c)).collect(),
                crate::isolated::Outcome::Failed(_) => Vec::new(),
            })
            .collect(),
        None => bundles
            .iter()
            .filter_map(|p| match load_bundle(p) {
                Ok(ds) => Some(ds),
                Err(e) => {
                    log::warn!("{e}");
                    None
                }
            })
            .collect(),
    };
    let mut out: Vec<Vst3Descriptor> = Vec::new();
    for d in found.into_iter().flatten() {
        if !out.iter().any(|o| o.id == d.id) {
            out.push(d);
        }
    }
    out.sort_by_key(|d| d.name.to_lowercase());
    out
}

fn ensure_scanned(r: &mut Registry) -> &Vec<Vst3Descriptor> {
    if r.scanned.is_none() {
        let mut dirs = paths::default_search_paths();
        dirs.extend(r.extra_dirs.iter().cloned());
        r.scanned = Some(scan_paths(&dirs));
    }
    r.scanned.get_or_insert_with(Vec::new)
}

pub fn scan() -> Vec<Vst3Descriptor> {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    ensure_scanned(&mut r).clone()
}

pub fn cached() -> Option<Vec<Vst3Descriptor>> {
    match registry().try_lock() {
        Ok(r) => r.scanned.clone(),
        Err(TryLockError::Poisoned(p)) => p.into_inner().scanned.clone(),
        Err(TryLockError::WouldBlock) => None,
    }
}

pub fn scan_in_background() -> bool {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if crate::isolated::scanner().is_none() {
        return false;
    }
    if STARTED.swap(true, Ordering::AcqRel) {
        return true;
    }
    let spawned = std::thread::Builder::new().name("vst3-scan".into()).spawn(|| {
        scan();
    });
    if let Err(e) = spawned {
        log::warn!("VST3 scan thread: {e}");
        STARTED.store(false, Ordering::Release);
        return false;
    }
    true
}

pub fn rescan() -> Vec<Vst3Descriptor> {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    r.scanned = None;
    r.failed.clear();
    crate::isolated::clear_cache();
    ensure_scanned(&mut r).clone()
}

pub fn set_search_dirs(dirs: Vec<PathBuf>) {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if r.extra_dirs == dirs {
        return;
    }
    r.extra_dirs = dirs;
    let rescan = r.scanned.take().is_some();
    drop(r);
    if rescan && crate::isolated::scanner().is_some() {
        let _ = std::thread::Builder::new().name("vst3-scan".into()).spawn(|| {
            scan();
        });
    }
}

pub fn add_search_dir(dir: &Path) -> usize {
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    if !r.extra_dirs.iter().any(|d| d == dir) {
        r.extra_dirs.push(dir.to_path_buf());
    }
    let found = scan_paths(&[dir.to_path_buf()]);
    let n = found.len();
    let list = r.scanned.get_or_insert_with(Vec::new);
    for d in found {
        if !list.iter().any(|o| o.id == d.id) {
            list.push(d);
        }
    }
    n
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

pub(crate) fn unit_from_label(units: &str) -> Unit {
    match units.trim().to_ascii_lowercase().as_str() {
        "db" => Unit::Db,
        "hz" => Unit::Hz,
        "ms" => Unit::Ms,
        "%" => Unit::Percent,
        "s" | "sec" | "secs" | "seconds" => Unit::Seconds,
        "st" | "semi" | "semitones" => Unit::Semitones,
        "ct" | "cent" | "cents" => Unit::Cents,
        _ => Unit::None,
    }
}

fn param_info(inst: &Instance, p: &RawParam) -> ParamInfo {
    let name = if p.title.is_empty() { format!("Param {}", p.id) } else { p.title.clone() };
    let base = ParamInfo {
        id: leak(p.id.to_string()),
        name: leak(name),
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        taper: Taper::Linear,
        choices: &[],
    };
    if p.step_count == 1 {
        return ParamInfo { unit: Unit::Toggle, default: p.default_normalized.round() as f32, ..base };
    }
    if (2..=MAX_CHOICES).contains(&p.step_count) {
        let steps = f64::from(p.step_count);
        let labels: Vec<&'static str> =
            (0..=p.step_count).map(|i| leak(inst.value_text(p.id, f64::from(i) / steps).unwrap_or_else(|| i.to_string()))).collect();
        return ParamInfo {
            max: p.step_count as f32,
            default: (p.default_normalized * steps).round() as f32,
            unit: Unit::Choice,
            choices: Box::leak(labels.into_boxed_slice()),
            ..base
        };
    }
    let unit = unit_from_label(&p.units);
    match (inst.to_plain(p.id, 0.0), inst.to_plain(p.id, 1.0)) {
        (Some(lo), Some(hi)) if hi > lo && (lo as f32).is_finite() && (hi as f32).is_finite() && (hi as f32) > (lo as f32) => {
            let default = inst.to_plain(p.id, p.default_normalized).map_or(lo, |d| d.clamp(lo, hi));
            let taper = match inst.to_plain(p.id, 0.5) {
                Some(mid)
                    if lo > 0.0
                        && ((mid - (lo * hi).sqrt()).abs() <= 0.01 * (lo * hi).sqrt())
                        && (mid - (lo + hi) / 2.0).abs() > 0.01 * (hi - lo) =>
                {
                    Taper::Log
                }
                _ => Taper::Linear,
            };
            ParamInfo { min: lo as f32, max: hi as f32, default: default as f32, unit, taper, ..base }
        }
        _ => ParamInfo { default: p.default_normalized as f32, unit, ..base },
    }
}

fn build_info(d: &Vst3Descriptor, inst: &Instance) -> &'static PluginInfo {
    let params: Vec<ParamInfo> =
        inst.params().iter().filter(|p| p.flags & (PARAM_HIDDEN | PARAM_READ_ONLY | PARAM_BYPASS) == 0).map(|p| param_info(inst, p)).collect();
    let short: String = d.name.chars().take(8).collect();
    Box::leak(Box::new(PluginInfo {
        id: leak(d.id.clone()),
        name: leak(d.name.clone()),
        short_name: leak(short),
        category: d.category,
        params: Box::leak(params.into_boxed_slice()),
        is_instrument: d.is_instrument,
        audiosuite: false,
    }))
}

fn instantiate_raw(id: &str) -> Result<(Instance, &'static PluginInfo), Vst3Error> {
    let cid = parse_id(id).ok_or_else(|| Vst3Error::NotFound(id.to_string()))?;
    let desc = {
        let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
        if r.failed.contains(id) {
            return Err(Vst3Error::Instantiate(id.to_string(), "failed before".into()));
        }
        ensure_scanned(&mut r).iter().find(|d| d.id == id).cloned().ok_or_else(|| Vst3Error::NotFound(id.to_string()))?
    };
    let made = Bundle::load(&desc.path).and_then(|b| b.instantiate(&cid));
    let mut r = registry().lock().unwrap_or_else(PoisonError::into_inner);
    match made {
        Ok(inst) => {
            let info = match r.infos.get(id) {
                Some(i) => *i,
                None => {
                    let i = build_info(&desc, &inst);
                    r.infos.insert(id.to_string(), i);
                    i
                }
            };
            Ok((inst, info))
        }
        Err(e) => {
            r.failed.insert(id.to_string());
            Err(e)
        }
    }
}

pub fn instantiate(id: &str) -> Result<Box<dyn Plugin>, Vst3Error> {
    let (inst, info) = instantiate_raw(id)?;
    Ok(Box::new(Vst3Plugin::new(inst, info)))
}

pub fn instantiate_plugin(id: &str) -> Result<Vst3Plugin, Vst3Error> {
    let (inst, info) = instantiate_raw(id)?;
    let mut p = Vst3Plugin::new(inst, info);
    p.prepare(48_000.0, 1024, 2);
    Ok(p)
}

pub fn plugin_info(id: &str) -> Option<&'static PluginInfo> {
    parse_id(id)?;
    {
        let r = registry().lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(i) = r.infos.get(id) {
            return Some(*i);
        }
        if r.failed.contains(id) {
            return None;
        }
    }
    instantiate_raw(id).ok().map(|(_, info)| info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs an installed plug-in (OPENPT_PROBE)"]
    fn probe_installed_plugin() {
        let Ok(path) = std::env::var("OPENPT_PROBE") else { return };
        let b = Bundle::load(Path::new(&path)).unwrap();
        for c in b.classes().into_iter().filter(|c| c.category == AUDIO_MODULE_CLASS) {
            let inst = b.instantiate(&c.cid).unwrap();
            println!("{} — PreSonus GR: {}", c.name, inst.reports_gain_reduction());
            for p in inst.params() {
                println!("{:>11} {:#06x} {:?} [{}]", p.id, p.flags, p.title, p.units);
            }
        }
    }

    #[test]
    fn unit_labels_map() {
        assert_eq!(unit_from_label("dB"), Unit::Db);
        assert_eq!(unit_from_label(" Hz "), Unit::Hz);
        assert_eq!(unit_from_label("%"), Unit::Percent);
        assert_eq!(unit_from_label("ms"), Unit::Ms);
        assert_eq!(unit_from_label("sec"), Unit::Seconds);
        assert_eq!(unit_from_label("cents"), Unit::Cents);
        assert_eq!(unit_from_label("st"), Unit::Semitones);
        assert_eq!(unit_from_label("furlongs"), Unit::None);
    }
}
