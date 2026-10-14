const TILT_HZ: f32 = 1500.0;
const DC_HZ: f32 = 8.0;

#[derive(Debug, Clone, Default)]
struct Channel {
    pre_lp: f32,
    post_lp: f32,
    dc_x: f32,
    dc_y: f32,
}

#[derive(Debug, Clone, Default)]
pub struct Heat {
    ch: Vec<Channel>,
}

impl Heat {
    pub fn reset(&mut self) {
        self.ch.iter_mut().for_each(|c| *c = Channel::default());
    }

    pub fn process(&mut self, buf: &mut [Vec<f32>], frames: usize, drive: f32, tone: f32, sample_rate: f32) {
        if self.ch.len() < buf.len() {
            self.ch.resize(buf.len(), Channel::default());
        }
        let sr = if sample_rate.is_finite() && sample_rate > 1000.0 { sample_rate } else { 48_000.0 };
        let drive = if drive.is_finite() { drive.clamp(0.0, 1.0) } else { 0.0 };
        let tone = if tone.is_finite() { tone.clamp(-1.0, 1.0) } else { 0.0 };
        let g = 1.0 + drive * 5.0;
        let tilt = 0.6 * tone.max(0.0);
        let bias = 0.35 * drive * (-tone).max(0.0);
        let tb = (g * bias).tanh();
        let slope = g * (1.0 - tb * tb);
        let a_lp = 1.0 - (-2.0 * std::f32::consts::PI * TILT_HZ / sr).exp();
        let r_dc = (-2.0 * std::f32::consts::PI * DC_HZ / sr).exp();
        for (c, st) in buf.iter_mut().zip(self.ch.iter_mut()) {
            for x in c.iter_mut().take(frames) {
                st.pre_lp += a_lp * (*x - st.pre_lp);
                let into = st.pre_lp + (*x - st.pre_lp) * (1.0 + tilt);
                let y = ((g * (into + bias)).tanh() - tb) / slope;
                st.post_lp += a_lp * (y - st.post_lp);
                let out = st.post_lp + (y - st.post_lp) / (1.0 + tilt);
                let dc = out - st.dc_x + r_dc * st.dc_y;
                st.dc_x = out;
                st.dc_y = dc;
                *x = if dc.is_finite() { dc } else { 0.0 };
            }
        }
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
    fn quiet_signals_pass_unchanged_and_loud_ones_gain_harmonics() {
        let mut h = Heat::default();
        let mut quiet = vec![sine(0.001, 48_000)];
        h.process(&mut quiet, 48_000, 1.0, 0.0, 48_000.0);
        let tail = &quiet[0][24_000..];
        let peak = tail.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.001).abs() < 0.0001, "unity for small signals: {peak}");
        let mut loud = vec![sine(0.8, 48_000)];
        Heat::default().process(&mut loud, 48_000, 1.0, 0.0, 48_000.0);
        let (_, h3) = harmonics(&loud[0][24_000..]);
        assert!(h3 > 0.01, "odd harmonics when driven: {h3}");
    }

    #[test]
    fn dark_tone_brings_even_harmonics() {
        let mut dark = vec![sine(0.6, 48_000)];
        Heat::default().process(&mut dark, 48_000, 1.0, -1.0, 48_000.0);
        let mut neutral = vec![sine(0.6, 48_000)];
        Heat::default().process(&mut neutral, 48_000, 1.0, 0.0, 48_000.0);
        let (h2_dark, _) = harmonics(&dark[0][24_000..]);
        let (h2_neutral, _) = harmonics(&neutral[0][24_000..]);
        assert!(h2_dark > h2_neutral * 5.0, "even harmonics: {h2_dark} vs {h2_neutral}");
        let mean: f32 = dark[0][24_000..].iter().sum::<f32>() / 24_000.0;
        assert!(mean.abs() < 1e-3, "no DC: {mean}");
    }
}
