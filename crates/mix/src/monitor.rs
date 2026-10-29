use crate::hrir_kemar;
use openpt_model::{AtmosMonitor, AtmosSettings, ChannelFormat, Speaker};

const HEAD_GAIN: f32 = 0.78 / 32768.0;

struct Ear {
    left: Vec<f32>,
    right: Vec<f32>,
}

pub struct Monitor {
    key: (AtmosMonitor, ChannelFormat, u32, i32, i32, i32),
    mode: AtmosMonitor,
    speakers: Vec<Speaker>,
    ears: Vec<Option<Ear>>,
    history: Vec<Vec<f32>>,
    matrix: Vec<[f32; 6]>,
    out: Vec<Vec<f32>>,
    scratch: Vec<f32>,
    gain: f32,
}

fn db(v: f32) -> f32 {
    10f32.powf(v / 20.0)
}

fn nearest(elev: f32, az: f32) -> Option<&'static (f32, f32, [i16; 128], [i16; 128])> {
    hrir_kemar::HRIRS
        .iter()
        .filter(|h| (h.0 - elev).abs() < 25.0)
        .min_by(|a, b| (a.1 - az.abs()).abs().total_cmp(&(b.1 - az.abs()).abs()))
}

fn ear_for(sp: Speaker, rate: u32) -> Option<Ear> {
    if sp.is_lfe() {
        return None;
    }
    let elev = if sp.is_height() { 40.0 } else { 0.0 };
    let az = sp.azimuth();
    let h = nearest(elev, az)?;
    let energy = |h: &(f32, f32, [i16; 128], [i16; 128])| h.2.iter().chain(h.3.iter()).map(|v| f32::from(*v).powi(2)).sum::<f32>();
    let front = hrir_kemar::HRIRS.first().map_or(1.0, energy);
    let level = HEAD_GAIN * (front / energy(h).max(1.0)).sqrt();
    let (near, far): (Vec<f32>, Vec<f32>) = (h.3.iter().map(|v| f32::from(*v) * level).collect(), h.2.iter().map(|v| f32::from(*v) * level).collect());
    let (right, left) = if az >= 0.0 { (near, far) } else { (far, near) };
    let at_rate = |ir: Vec<f32>| {
        if rate == hrir_kemar::RATE || rate == 0 {
            ir
        } else {
            let scale = rate as f32 / hrir_kemar::RATE as f32;
            openpt_dsp::offline::resample(&[ir], hrir_kemar::RATE, rate).into_iter().next().unwrap_or_default().into_iter().map(|x| x / scale).collect()
        }
    };
    Some(Ear { left: at_rate(left), right: at_rate(right) })
}

fn side(sp: Speaker) -> f32 {
    let a = sp.azimuth();
    if a.abs() < 1.0 || (a.abs() - 180.0).abs() < 1.0 { 0.0 } else { a.signum() }
}

fn downmix_row(sp: Speaker, to51: bool, surround: f32, height: f32) -> [f32; 6] {
    use Speaker::*;
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let lr = |g: f32, s: f32| if s < 0.0 { [g, 0.0] } else if s > 0.0 { [0.0, g] } else { [g * h, g * h] };
    let mut row = [0.0f32; 6];
    if to51 {
        match sp {
            L | Lw | Lc => row[0] = 1.0,
            R | Rw | Rc => row[1] = 1.0,
            C => row[2] = 1.0,
            Lfe => row[3] = 1.0,
            Ls | Lss | Lrs => row[4] = 1.0,
            Rs | Rss | Rrs => row[5] = 1.0,
            Cs => {
                row[4] = h;
                row[5] = h;
            }
            Ltf => row[0] = height,
            Rtf => row[1] = height,
            Ltr => row[4] = height,
            Rtr => row[5] = height,
            Ltm => {
                row[0] = height * h;
                row[4] = height * h;
            }
            Rtm => {
                row[1] = height * h;
                row[5] = height * h;
            }
        }
        return row;
    }
    let [l, r] = match sp {
        Lfe => [0.0, 0.0],
        C => [h, h],
        L | R | Lw | Rw | Lc | Rc => lr(1.0, side(sp)),
        Ls | Rs | Lss | Rss | Lrs | Rrs | Cs => lr(surround, side(sp)),
        _ => lr(height, side(sp)),
    };
    row[0] = l;
    row[1] = r;
    row
}

impl Monitor {
    pub fn wanted(settings: &AtmosSettings, fmt: ChannelFormat) -> bool {
        settings.monitor != AtmosMonitor::Speakers && fmt.channels() > 2 && !fmt.speakers().is_empty()
    }

    pub fn new(settings: &AtmosSettings, fmt: ChannelFormat, rate: u32) -> Monitor {
        let speakers = fmt.speakers().to_vec();
        let to51 = settings.monitor == AtmosMonitor::Surround51;
        let (surround, height) = (db(settings.surround_trim_db), db(settings.height_trim_db));
        let binaural = settings.monitor == AtmosMonitor::Binaural;
        let ears = if binaural { speakers.iter().map(|s| ear_for(*s, rate)).collect() } else { Vec::new() };
        let taps = ears.iter().flatten().map(|e| e.left.len().max(e.right.len())).max().unwrap_or(1);
        Monitor {
            key: Monitor::key_of(settings, fmt, rate),
            mode: settings.monitor,
            matrix: speakers.iter().map(|s| downmix_row(*s, to51, surround, height)).collect(),
            history: vec![vec![0.0; taps]; if binaural { speakers.len() } else { 0 }],
            ears,
            speakers,
            out: vec![Vec::new(); 6],
            scratch: Vec::new(),
            gain: db(settings.binaural_gain_db),
        }
    }

    fn key_of(settings: &AtmosSettings, fmt: ChannelFormat, rate: u32) -> (AtmosMonitor, ChannelFormat, u32, i32, i32, i32) {
        let k = |v: f32| (v * 100.0).round() as i32;
        (settings.monitor, fmt, rate, k(settings.surround_trim_db), k(settings.height_trim_db), k(settings.binaural_gain_db))
    }

    pub fn matches(&self, settings: &AtmosSettings, fmt: ChannelFormat, rate: u32) -> bool {
        self.key == Monitor::key_of(settings, fmt, rate)
    }

    pub fn outputs(&self) -> usize {
        match self.mode {
            AtmosMonitor::Surround51 => 6,
            _ => 2,
        }
    }

    pub fn process(&mut self, main: &mut [Vec<f32>], frames: usize) {
        let n_out = self.outputs();
        for o in self.out.iter_mut().take(n_out) {
            o.clear();
            o.resize(frames, 0.0);
        }
        if self.mode == AtmosMonitor::Binaural {
            self.binaural(main, frames);
        } else {
            for (c, row) in self.matrix.iter().enumerate() {
                let Some(src) = main.get(c) else { continue };
                for (o, g) in self.out.iter_mut().zip(row).take(n_out) {
                    if *g == 0.0 {
                        continue;
                    }
                    for (d, s) in o.iter_mut().zip(src.iter().take(frames)) {
                        *d += s * g;
                    }
                }
            }
        }
        for (c, ch) in main.iter_mut().enumerate().take(self.speakers.len()) {
            let src = self.out.get(c).filter(|_| c < n_out);
            for (i, x) in ch.iter_mut().take(frames).enumerate() {
                *x = src.and_then(|s| s.get(i)).copied().unwrap_or(0.0);
            }
        }
    }

    fn binaural(&mut self, main: &[Vec<f32>], frames: usize) {
        let [l, r, ..] = &mut self.out[..] else { return };
        for (c, sp) in self.speakers.iter().enumerate() {
            let Some(src) = main.get(c) else { continue };
            let src = src.get(..frames).unwrap_or(src);
            match self.ears.get(c).and_then(Option::as_ref) {
                None => {
                    if sp.is_lfe() {
                        for i in 0..frames {
                            let v = src.get(i).copied().unwrap_or(0.0) * self.gain;
                            l[i] += v;
                            r[i] += v;
                        }
                    }
                }
                Some(ear) => {
                    let Some(hist) = self.history.get_mut(c) else { continue };
                    let taps = hist.len();
                    let buf = &mut self.scratch;
                    buf.clear();
                    buf.extend_from_slice(hist);
                    buf.extend_from_slice(src);
                    for i in 0..frames {
                        let window = &buf[i + 1..i + 1 + taps];
                        let (mut a, mut b) = (0.0f32, 0.0f32);
                        for (k, x) in window.iter().rev().enumerate() {
                            a += x * ear.left.get(k).copied().unwrap_or(0.0);
                            b += x * ear.right.get(k).copied().unwrap_or(0.0);
                        }
                        l[i] += a * self.gain;
                        r[i] += b * self.gain;
                    }
                    let keep = buf.len() - taps;
                    hist.copy_from_slice(&buf[keep..]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impulse_on(fmt: ChannelFormat, sp: Speaker, n: usize) -> Vec<Vec<f32>> {
        let mut main = vec![vec![0.0f32; n]; fmt.channels()];
        let c = fmt.speakers().iter().position(|s| *s == sp).unwrap();
        main[c][0] = 1.0;
        main
    }

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum()
    }

    #[test]
    fn binaural_puts_each_speaker_on_its_side() {
        let s = AtmosSettings { monitor: AtmosMonitor::Binaural, ..AtmosSettings::default() };
        for (sp, right_louder) in [(Speaker::R, true), (Speaker::Lss, false), (Speaker::Rtr, true), (Speaker::Ltf, false)] {
            let mut m = Monitor::new(&s, ChannelFormat::Atmos714, 48_000);
            let mut main = impulse_on(ChannelFormat::Atmos714, sp, 512);
            m.process(&mut main, 512);
            let (l, r) = (energy(&main[0]), energy(&main[1]));
            assert!(l > 0.0 && r > 0.0, "{sp:?}");
            assert_eq!(r > l * 2.0, right_louder, "{sp:?}: L {l} R {r}");
            assert!(main[2..].iter().all(|c| c.iter().all(|v| *v == 0.0)), "only the first two channels carry the render");
        }
    }

    #[test]
    fn binaural_carries_over_between_blocks() {
        let s = AtmosSettings { monitor: AtmosMonitor::Binaural, ..AtmosSettings::default() };
        let mut whole = Monitor::new(&s, ChannelFormat::Atmos714, 44_100);
        let mut split = Monitor::new(&s, ChannelFormat::Atmos714, 44_100);
        let mut a = impulse_on(ChannelFormat::Atmos714, Speaker::C, 256);
        whole.process(&mut a, 256);
        let mut b1 = impulse_on(ChannelFormat::Atmos714, Speaker::C, 64);
        split.process(&mut b1, 64);
        let mut b2 = vec![vec![0.0f32; 192]; 12];
        split.process(&mut b2, 192);
        for i in 0..64 {
            assert!((a[0][i] - b1[0][i]).abs() < 1e-6);
        }
        for i in 0..192 {
            assert!((a[0][64 + i] - b2[0][i]).abs() < 1e-6, "{i}");
        }
    }

    #[test]
    fn stereo_downmix_applies_the_trims() {
        let s = AtmosSettings { monitor: AtmosMonitor::Stereo, surround_trim_db: -6.0, height_trim_db: -12.0, ..AtmosSettings::default() };
        let mut m = Monitor::new(&s, ChannelFormat::Atmos714, 48_000);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Lss, 4);
        m.process(&mut main, 4);
        assert!((main[0][0] - db(-6.0)).abs() < 1e-4 && main[1][0] == 0.0);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Rtf, 4);
        m.process(&mut main, 4);
        assert!((main[1][0] - db(-12.0)).abs() < 1e-4 && main[0][0] == 0.0);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Lfe, 4);
        m.process(&mut main, 4);
        assert!(main.iter().all(|c| c[0] == 0.0), "no LFE in Lo/Ro");
    }

    #[test]
    fn surround_downmix_folds_heights_into_the_layer_below() {
        let s = AtmosSettings { monitor: AtmosMonitor::Surround51, height_trim_db: 0.0, ..AtmosSettings::default() };
        let mut m = Monitor::new(&s, ChannelFormat::Atmos714, 48_000);
        let mut main = impulse_on(ChannelFormat::Atmos714, Speaker::Rtr, 4);
        m.process(&mut main, 4);
        assert_eq!((0..6).map(|c| main[c][0]).collect::<Vec<_>>(), vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }
}
