const CURVE_ODD: [f32; 8] = [-0.0001568, 0.4345272, -0.0001483, -0.4488479, 0.0000083, 0.0145985, -0.0000003, -0.0002216];
const CURVE_TRIODE: [f32; 12] = [0.0680458, 0.3897605, -0.0119493, -0.4574004, -0.0670536, 0.0739828, 0.0117812, -0.0066401, -0.0011019, 0.0003232, 0.0000582, -0.0000061];

const DRIVE_ODD: [[f32; 4]; 5] = [
    [0.064041, 0.939772, 0.113410, -0.238140],
    [0.092015, 0.836364, 0.250630, -0.225548],
    [0.107007, 0.666949, 0.404277, -0.212227],
    [0.111209, 0.410454, 0.566274, -0.198182],
    [0.105468, 0.051222, 0.727864, -0.183410],
];
const DRIVE_TRIODE: [[f32; 4]; 5] = [
    [0.060481, 0.919807, 0.081917, 0.544682],
    [0.086903, 0.805980, 0.175770, 0.500863],
    [0.101062, 0.645518, 0.273451, 0.454528],
    [0.105029, 0.430848, 0.366091, 0.405688],
    [0.099609, 0.164091, 0.444066, 0.354285],
];
const TONE: [f32; 7] = [0.25, 0.375, 0.515, 0.67, 0.78, 0.875, 1.0];

#[derive(Debug, Clone, Default)]
struct Channel {
    previous: f32,
    low: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Heat {
    ch: Vec<Channel>,
}

struct Law<'a> {
    curve: &'a [f32],
    mix: f32,
    gain: f32,
    pre: f32,
    feed: f32,
    low: f32,
    dry: f32,
    wet: f32,
    edge: f32,
}

impl Law<'_> {
    fn new(drive: i32, tone: i32, sample_rate: f32) -> Option<Self> {
        let step = usize::try_from(drive.unsigned_abs().checked_sub(1)?).ok()?.min(4);
        let tone = TONE[(tone.clamp(-3, 3) + 3) as usize];
        let rate = if sample_rate.is_finite() && sample_rate > 1000.0 { 48_000.0 / sample_rate } else { 1.0 };
        let (curve, row, pre, feed, low): (&[f32], _, _, _, _) = if drive < 0 {
            (&CURVE_ODD, DRIVE_ODD[step], 0.31579, 1.0, 1.144848)
        } else {
            (&CURVE_TRIODE, DRIVE_TRIODE[step], 0.832605, 0.625, 1.288404)
        };
        Some(Law {
            curve,
            mix: row[0],
            gain: 1.46875 + 0.09375 * drive.abs().min(5) as f32,
            pre: 1.0 + pre * tone * rate,
            feed: feed * tone,
            low: low * tone * rate,
            dry: row[1],
            wet: row[2],
            edge: row[3] * tone,
        })
    }
}

fn wrap(x: f32) -> f32 {
    (x + 1.0).rem_euclid(2.0) - 1.0
}

fn curve(c: &[f32], x: f32) -> f32 {
    let (mut b1, mut b2) = (0.0f32, 0.0f32);
    for a in c.iter().skip(1).rev() {
        let b = a + 2.0 * x * b1 - b2;
        b2 = b1;
        b1 = b;
    }
    c[0] + x * b1 - b2
}

impl Heat {
    pub fn reset(&mut self) {
        self.ch.iter_mut().for_each(|c| *c = Channel::default());
    }

    pub fn process(&mut self, buf: &mut [Vec<f32>], frames: usize, drive: i32, tone: i32, sample_rate: f32) -> f32 {
        if self.ch.len() < buf.len() {
            self.ch.resize(buf.len(), Channel::default());
        }
        let Some(l) = Law::new(drive, tone, sample_rate) else {
            self.reset();
            return 0.0;
        };
        let mut peak = 0.0f32;
        for (c, st) in buf.iter_mut().zip(self.ch.iter_mut()) {
            for x in c.iter_mut().take(frames) {
                let input = if x.is_finite() { *x } else { 0.0 };
                let w = (1.0 - l.mix) * input;
                let edge = l.mix * curve(l.curve, wrap(l.gain * (l.pre * w - st.previous)));
                st.previous = w;
                let shaped = curve(l.curve, wrap(w + edge)) + l.feed * edge;
                st.low += l.low * (shaped - st.low);
                let out = l.dry * w + l.wet * st.low + l.edge * edge;
                peak = peak.max((out - input).abs());
                *x = out;
            }
        }
        peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(amp: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| amp * (2.0 * std::f32::consts::PI * 100.0 * i as f32 / 48_000.0).sin()).collect()
    }

    fn harmonics(x: &[f32]) -> (f32, f32) {
        let bin = |f: f32| {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, v) in x.iter().enumerate() {
                let p = 2.0 * std::f32::consts::PI * f * i as f32 / 48_000.0;
                re += v * p.cos();
                im += v * p.sin();
            }
            (re * re + im * im).sqrt()
        };
        let f = bin(100.0);
        (bin(200.0) / f, bin(300.0) / f)
    }

    #[test]
    fn centre_drive_leaves_the_signal_untouched() {
        let src = sine(0.8, 4_800);
        let mut buf = vec![src.clone()];
        let peak = Heat::default().process(&mut buf, 4_800, 0, 3, 48_000.0);
        assert_eq!(buf[0], src);
        assert_eq!(peak, 0.0);
    }

    #[test]
    fn left_is_odd_and_right_adds_even_harmonics() {
        let render = |drive| {
            let mut buf = vec![sine(0.5, 48_000)];
            Heat::default().process(&mut buf, 48_000, drive, 0, 48_000.0);
            harmonics(&buf[0][24_000..])
        };
        let (even_left, odd_left) = render(-5);
        let (even_right, odd_right) = render(5);
        assert!(odd_left > 0.001, "odd harmonics on the left: {odd_left}");
        assert!(even_left < odd_left / 100.0, "no even harmonics on the left: {even_left}");
        assert!(even_right > 0.005, "even harmonics on the right: {even_right}");
        assert!(odd_right > 0.001, "odd harmonics on the right: {odd_right}");
    }

    #[test]
    fn tone_brightens_the_top_end() {
        let render = |tone| {
            let mut buf = vec![(0..4_800).map(|i| 0.1 * (2.0 * std::f32::consts::PI * 5_000.0 * i as f32 / 48_000.0).sin()).collect::<Vec<f32>>()];
            Heat::default().process(&mut buf, 4_800, 3, tone, 48_000.0);
            buf[0][2_400..].iter().fold(0.0f32, |m, v| m.max(v.abs()))
        };
        assert!(render(3) > render(-3) * 1.2);
    }

    #[test]
    fn garbage_input_stays_finite() {
        let mut buf = vec![vec![f32::NAN, f32::INFINITY, 40.0, -40.0, 0.0]];
        Heat::default().process(&mut buf, 5, 5, 3, 48_000.0);
        assert!(buf[0].iter().all(|v| v.is_finite()));
    }
}
