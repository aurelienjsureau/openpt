use rustfft::{Fft, FftPlanner, num_complex::Complex};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

pub const RING: usize = 1 << 15;
const MASK: usize = RING - 1;
pub const HISTORY: usize = 1024;
pub const BANDS: usize = 24;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Column {
    pub input: f32,
    pub output: f32,
    pub gr: f32,
    pub aux: f32,
    pub hit: f32,
}

#[derive(Debug, Clone)]
pub struct ScopeData {
    pub rate: f32,
    pub pre: [Vec<f32>; 2],
    pub post: [Vec<f32>; 2],
    pub pos: usize,
    pub bands: [f32; BANDS],
    pub history: Vec<Column>,
    pub hist_pos: usize,
    pub columns: u64,
    pub column_secs: f32,
    pub key_live: bool,
    pub readout: f32,
}

impl Default for ScopeData {
    fn default() -> Self {
        ScopeData {
            rate: 48_000.0,
            pre: [vec![0.0; RING], vec![0.0; RING]],
            post: [vec![0.0; RING], vec![0.0; RING]],
            pos: 0,
            bands: [0.0; BANDS],
            history: vec![Column { input: -120.0, output: -120.0, gr: 0.0, aux: -120.0, hit: f32::NEG_INFINITY }; HISTORY],
            hist_pos: 0,
            columns: 0,
            column_secs: 0.01,
            key_live: false,
            readout: 0.0,
        }
    }
}

impl ScopeData {
    pub fn push_audio(&mut self, io: &[Vec<f32>], frames: usize, post: bool) {
        let ring = if post { &mut self.post } else { &mut self.pre };
        let l = io.first().map(|c| &c[..frames.min(c.len())]).unwrap_or(&[]);
        let r = io.get(1).map(|c| &c[..frames.min(c.len())]).unwrap_or(l);
        let mut p = self.pos;
        for (i, &x) in l.iter().enumerate() {
            ring[0][p] = x;
            ring[1][p] = r.get(i).copied().unwrap_or(x);
            p = (p + 1) & MASK;
        }
        if post {
            self.pos = p;
        }
    }

    pub fn push_column(&mut self, c: Column) {
        if let Some(slot) = self.history.get_mut(self.hist_pos) {
            *slot = c;
        }
        self.hist_pos = (self.hist_pos + 1) % HISTORY;
        self.columns = self.columns.wrapping_add(1);
    }

    pub fn latest(&self, post: bool, ch: usize, out: &mut [f32]) {
        let ring = if post { &self.post } else { &self.pre };
        let Some(src) = ring.get(ch) else { return };
        let n = out.len().min(RING);
        let start = self.pos.wrapping_sub(n) & MASK;
        for (i, o) in out.iter_mut().take(n).enumerate() {
            *o = src[(start + i) & MASK];
        }
    }

    pub fn columns_in_order(&self) -> impl Iterator<Item = &Column> {
        self.history[self.hist_pos..].iter().chain(self.history[..self.hist_pos].iter())
    }
}

#[derive(Debug, Default)]
pub struct Scope {
    data: Mutex<ScopeData>,
}

impl Scope {
    pub fn new() -> Arc<Scope> {
        Arc::new(Scope::default())
    }

    pub fn try_write(&self) -> Option<MutexGuard<'_, ScopeData>> {
        self.data.try_lock().ok()
    }

    pub fn read<R>(&self, f: impl FnOnce(&ScopeData) -> R) -> R {
        match self.data.lock() {
            Ok(g) => f(&g),
            Err(p) => f(&p.into_inner()),
        }
    }
}

fn registry() -> &'static Mutex<HashMap<(u64, usize), Vec<Weak<Scope>>>> {
    static R: OnceLock<Mutex<HashMap<(u64, usize), Vec<Weak<Scope>>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

pub fn publish(track: u64, slot: usize, scope: &Arc<Scope>) {
    if let Ok(mut r) = registry().lock() {
        let v = r.entry((track, slot)).or_default();
        v.retain(|w| w.strong_count() > 0);
        if !v.iter().any(|w| w.as_ptr() == Arc::as_ptr(scope)) {
            v.push(Arc::downgrade(scope));
        }
    }
}

pub fn find(track: u64, slot: usize) -> Option<Arc<Scope>> {
    let r = registry().lock().ok()?;
    r.get(&(track, slot))?.iter().rev().find_map(Weak::upgrade)
}

pub struct Analyzer {
    n: usize,
    fft: Arc<dyn Fft<f32>>,
    win: Vec<f32>,
    buf: Vec<Complex<f32>>,
    tmp: Vec<f32>,
    pub power: Vec<f32>,
}

impl Analyzer {
    pub fn new(n: usize) -> Analyzer {
        let n = n.clamp(256, RING);
        let fft = FftPlanner::<f32>::new().plan_fft_forward(n);
        let win = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
        Analyzer { n, fft, win, buf: vec![Complex::new(0.0, 0.0); n], tmp: vec![0.0; n], power: vec![0.0; n / 2 + 1] }
    }

    pub fn size(&self) -> usize {
        self.n
    }

    pub fn run(&mut self, d: &ScopeData, post: bool) {
        self.power.iter_mut().for_each(|p| *p = 0.0);
        let norm = 4.0 / self.n as f32;
        for ch in 0..2 {
            d.latest(post, ch, &mut self.tmp);
            for ((b, &x), &w) in self.buf.iter_mut().zip(self.tmp.iter()).zip(self.win.iter()) {
                *b = Complex::new(if x.is_finite() { x * w } else { 0.0 }, 0.0);
            }
            self.fft.process(&mut self.buf);
            for (p, c) in self.power.iter_mut().zip(self.buf.iter()) {
                let m = c.norm() * norm;
                *p += 0.5 * m * m;
            }
        }
    }

    pub fn db_at(&self, f: f32, f_lo: f32, f_hi: f32, rate: f32) -> f32 {
        let bin = rate / self.n as f32;
        if f >= 0.5 * rate || bin <= 0.0 {
            return -200.0;
        }
        let (k_lo, k_hi) = (f_lo / bin, f_hi / bin);
        let last = self.power.len().saturating_sub(2);
        let pw = if k_hi.floor() < k_lo.ceil() {
            let k = f / bin;
            let k0 = (k as usize).min(last);
            let t = (k - k0 as f32).clamp(0.0, 1.0);
            self.power[k0] * (1.0 - t) + self.power[k0 + 1] * t
        } else {
            let a = k_lo.ceil().max(0.0) as usize;
            let b = (k_hi.floor() as usize).min(self.power.len() - 1);
            self.power[a.min(b)..=b].iter().copied().fold(0.0, f32::max)
        };
        10.0 * (pw + 1.0e-20).log10()
    }
}
