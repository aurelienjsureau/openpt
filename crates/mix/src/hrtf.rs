use crate::hrir_kemar;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const HEAD_GAIN: f32 = 0.78 / 32768.0;
#[cfg(not(target_arch = "wasm32"))]
const SOFA_RATE: u32 = 48_000;
#[cfg(not(target_arch = "wasm32"))]
const SOFA_TAPS: usize = 288;
#[cfg(not(target_arch = "wasm32"))]
const SOFA_DIRECTIONS: [(f32, f32); 21] = [
    (0.0, 0.0),
    (0.0, 15.0),
    (0.0, -15.0),
    (0.0, 30.0),
    (0.0, -30.0),
    (0.0, 60.0),
    (0.0, -60.0),
    (0.0, 90.0),
    (0.0, -90.0),
    (0.0, 110.0),
    (0.0, -110.0),
    (0.0, 150.0),
    (0.0, -150.0),
    (0.0, 180.0),
    (45.0, 45.0),
    (45.0, -45.0),
    (45.0, 90.0),
    (45.0, -90.0),
    (45.0, 135.0),
    (45.0, -135.0),
    (45.0, 0.0),
];

pub struct Hrir {
    pub elevation: f32,
    pub azimuth: f32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

pub struct HrirSet {
    pub name: String,
    pub rate: u32,
    pub irs: Vec<Hrir>,
}

fn angle_between(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

impl HrirSet {
    pub fn at(&self, elevation: f32, azimuth: f32) -> Option<&Hrir> {
        let high = elevation > 20.0;
        self.irs
            .iter()
            .filter(|h| (h.elevation > 20.0) == high)
            .min_by(|a, b| angle_between(a.azimuth, azimuth).total_cmp(&angle_between(b.azimuth, azimuth)))
    }
}

fn energy(h: &Hrir) -> f32 {
    h.left.iter().chain(h.right.iter()).map(|v| v * v).sum()
}

pub fn kemar() -> Arc<HrirSet> {
    static SET: OnceLock<Arc<HrirSet>> = OnceLock::new();
    SET.get_or_init(|| {
        let scaled = |x: &[i16; 128]| x.iter().map(|v| f32::from(*v) * HEAD_GAIN).collect::<Vec<f32>>();
        let mut irs = Vec::new();
        for (elevation, azimuth, far, near) in &hrir_kemar::HRIRS {
            irs.push(Hrir { elevation: *elevation, azimuth: *azimuth, left: scaled(far), right: scaled(near) });
            if *azimuth > 0.0 && *azimuth < 180.0 {
                irs.push(Hrir { elevation: *elevation, azimuth: -*azimuth, left: scaled(near), right: scaled(far) });
            }
        }
        Arc::new(HrirSet { name: "KEMAR".to_string(), rate: hrir_kemar::RATE, irs })
    })
    .clone()
}

static CUSTOM: Mutex<Option<Arc<HrirSet>>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

pub fn current() -> Arc<HrirSet> {
    match CUSTOM.lock() {
        Ok(c) => c.clone().unwrap_or_else(kemar),
        Err(_) => kemar(),
    }
}

fn install(set: Option<Arc<HrirSet>>) {
    if let Ok(mut c) = CUSTOM.lock() {
        *c = set;
    }
    GENERATION.fetch_add(1, Ordering::AcqRel);
}

pub fn use_kemar() {
    install(None);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn sofa_position(elevation: f32, azimuth: f32) -> [f32; 3] {
    let (el, az) = (elevation.to_radians(), azimuth.to_radians());
    [el.cos() * az.cos(), -el.cos() * az.sin(), el.sin()]
}

#[cfg(not(target_arch = "wasm32"))]
fn fitted(ir: &[f32], delay_seconds: f32) -> Vec<f32> {
    let delay = (delay_seconds.max(0.0) * SOFA_RATE as f32).round() as usize;
    let mut out = vec![0.0f32; delay.min(SOFA_TAPS)];
    let room = SOFA_TAPS - out.len();
    out.extend(ir.iter().take(room));
    let fade = 32.min(out.len());
    let n = out.len();
    for (i, v) in out.iter_mut().skip(n - fade).enumerate() {
        *v *= 1.0 - i as f32 / fade.max(1) as f32;
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_sofa(path: &std::path::Path) -> Result<HrirSet, String> {
    let sofa = sofar::reader::OpenOptions::new().sample_rate(SOFA_RATE as f32).normalized(false).open(path).map_err(|e| e.to_string())?;
    let mut filter = sofar::reader::Filter::new(sofa.filter_len());
    let mut irs = Vec::with_capacity(SOFA_DIRECTIONS.len());
    for (elevation, azimuth) in SOFA_DIRECTIONS {
        let [x, y, z] = sofa_position(elevation, azimuth);
        sofa.filter_nointerp(x, y, z, &mut filter);
        irs.push(Hrir { elevation, azimuth, left: fitted(&filter.left, filter.ldelay), right: fitted(&filter.right, filter.rdelay) });
    }
    let front = irs.first().map_or(0.0, energy);
    if !front.is_finite() || front <= 0.0 {
        return Err("this SOFA file has no usable response for the front direction".to_string());
    }
    let reference = kemar();
    let target = reference.at(0.0, 0.0).map_or(1.0, energy) * reference.rate as f32 / SOFA_RATE as f32;
    let scale = (target / front).sqrt();
    for h in &mut irs {
        h.left.iter_mut().chain(h.right.iter_mut()).for_each(|v| *v *= scale);
    }
    let name = path.file_stem().map_or_else(|| "SOFA".to_string(), |s| s.to_string_lossy().into_owned());
    Ok(HrirSet { name, rate: SOFA_RATE, irs })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn load_sofa(path: &std::path::Path) -> Result<String, String> {
    let set = read_sofa(path)?;
    let name = set.name.clone();
    install(Some(Arc::new(set)));
    Ok(name)
}

#[cfg(target_arch = "wasm32")]
pub fn load_sofa(_: &std::path::Path) -> Result<String, String> {
    Err("SOFA files cannot be loaded in the browser".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kemar_covers_both_sides_with_the_near_ear_on_the_source_side() {
        let set = kemar();
        let right = set.at(0.0, 90.0).unwrap();
        let left = set.at(0.0, -90.0).unwrap();
        let e = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>();
        assert!(e(&right.right) > e(&right.left) * 4.0);
        assert!(e(&left.left) > e(&left.right) * 4.0);
        assert_eq!(set.at(0.0, 180.0).unwrap().azimuth, 180.0);
        assert_eq!(set.at(40.0, -135.0).unwrap().azimuth, -135.0);
    }

    #[test]
    fn sofa_positions_put_positive_azimuth_on_the_listener_right() {
        let [x, y, z] = sofa_position(0.0, 90.0);
        assert!(x.abs() < 1e-5 && (y + 1.0).abs() < 1e-5 && z.abs() < 1e-5);
        let [x, y, _] = sofa_position(0.0, 180.0);
        assert!((x + 1.0).abs() < 1e-5 && y.abs() < 1e-5);
    }

    #[test]
    #[ignore = "needs OPENPT_TEST_SOFA pointing at a SOFA file"]
    fn a_real_sofa_file_keeps_sides_and_front_back_apart() {
        let path = std::env::var("OPENPT_TEST_SOFA").unwrap();
        let set = read_sofa(std::path::Path::new(&path)).unwrap();
        let e = |x: &[f32]| x.iter().map(|v| v * v).sum::<f32>();
        let right = set.at(0.0, 90.0).unwrap();
        assert!(e(&right.right) > e(&right.left) * 2.0, "the right ear is on the right");
        let onset = |x: &[f32]| {
            let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            x.iter().position(|v| v.abs() > 0.15 * peak).unwrap()
        };
        assert!(onset(&right.right) + 10 < onset(&right.left), "the far ear is later");
        let kem = kemar();
        for (el, az) in [(0.0f32, 0.0f32), (0.0, 30.0), (0.0, 150.0), (0.0, 180.0), (0.0, -150.0)] {
            let (a, b) = (set.at(el, az).unwrap(), kem.at(el, az).unwrap());
            println!(
                "az {az}: L {:.1} dB R {:.1} dB, KEMAR L {:.1} R {:.1}",
                10.0 * e(&a.left).log10(),
                10.0 * e(&a.right).log10(),
                10.0 * e(&b.left).log10(),
                10.0 * e(&b.right).log10()
            );
        }
    }
}
