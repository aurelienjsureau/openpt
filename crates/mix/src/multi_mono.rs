use openpt_dsp::{Plugin, PluginEditor, PluginInfo, PluginStateReader, Transport};

pub(crate) struct MultiMono {
    id: String,
    info: &'static PluginInfo,
    state: Option<Vec<u8>>,
    inner: Vec<Box<dyn Plugin>>,
}

impl MultiMono {
    pub(crate) fn new(id: &str, state: Option<&[u8]>) -> Option<(MultiMono, bool)> {
        let mut first = super::create_plugin(id)?;
        let refused = state.is_some_and(|st| !first.load_state(st));
        let info = first.info();
        let state = if refused { None } else { state.map(<[u8]>::to_vec) };
        Some((MultiMono { id: id.to_string(), info, state, inner: vec![first] }, refused))
    }

    fn grow(&mut self, channels: usize) {
        while self.inner.len() < channels {
            let Some(mut p) = super::create_plugin(&self.id) else { break };
            if let Some(st) = &self.state {
                p.load_state(st);
            }
            if let Some(first) = self.inner.first() {
                for pi in self.info.params {
                    if let Some(v) = first.param(pi.id) {
                        p.set_param(pi.id, v);
                    }
                }
            }
            self.inner.push(p);
        }
        self.inner.truncate(channels.max(1));
    }
}

impl Plugin for MultiMono {
    fn info(&self) -> &'static PluginInfo {
        self.info
    }

    fn prepare(&mut self, sample_rate: f32, max_block: usize, channels: usize) {
        self.grow(channels.max(1));
        for p in &mut self.inner {
            p.prepare(sample_rate, max_block, 1);
        }
    }

    fn reset(&mut self) {
        self.inner.iter_mut().for_each(|p| p.reset());
    }

    fn set_param(&mut self, id: &str, value: f32) -> bool {
        let mut known = false;
        for p in &mut self.inner {
            known |= p.set_param(id, value);
        }
        known
    }

    fn param(&self, id: &str) -> Option<f32> {
        self.inner.first().and_then(|p| p.param(id))
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        for (p, ch) in self.inner.iter_mut().zip(io.chunks_mut(1)) {
            p.process(ch, frames);
        }
    }

    fn accepts_key(&self) -> bool {
        self.inner.first().is_some_and(|p| p.accepts_key())
    }

    fn process_keyed(&mut self, io: &mut [Vec<f32>], key: &[Vec<f32>], frames: usize) {
        if key.is_empty() {
            return self.process(io, frames);
        }
        for (c, (p, ch)) in self.inner.iter_mut().zip(io.chunks_mut(1)).enumerate() {
            let k = c % key.len();
            p.process_keyed(ch, key.get(k..=k).unwrap_or(&[]), frames);
        }
    }

    fn latency(&self) -> usize {
        self.inner.iter().map(|p| p.latency()).max().unwrap_or(0)
    }

    fn tail_samples(&self) -> usize {
        self.inner.iter().map(|p| p.tail_samples()).max().unwrap_or(0)
    }

    fn gain_reduction_db(&self) -> f32 {
        self.inner.iter().map(|p| p.gain_reduction_db()).fold(0.0, f32::max)
    }

    fn set_transport(&mut self, transport: &Transport) {
        self.inner.iter_mut().for_each(|p| p.set_transport(transport));
    }

    fn save_state(&mut self) -> Option<Vec<u8>> {
        self.inner.first_mut().and_then(|p| p.save_state())
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        let mut ok = !self.inner.is_empty();
        for p in &mut self.inner {
            ok &= p.load_state(data);
        }
        if ok {
            self.state = Some(data.to_vec());
        }
        ok
    }

    fn open_editor(&mut self) -> Result<(), String> {
        self.inner.first_mut().map_or_else(|| Err("no instance".into()), |p| p.open_editor())
    }

    fn close_editor(&mut self) {
        if let Some(p) = self.inner.first_mut() {
            p.close_editor();
        }
    }

    fn editor(&mut self) -> Option<Box<dyn PluginEditor>> {
        self.inner.first_mut().and_then(|p| p.editor())
    }

    fn state_reader(&mut self) -> Option<Box<dyn PluginStateReader>> {
        self.inner.first_mut().and_then(|p| p.state_reader())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_block(left: f32, right: f32) -> Vec<Vec<f32>> {
        let n = 4_800;
        let tone = |a: f32| (0..n).map(|i| a * (i as f32 * 0.05).sin()).collect::<Vec<f32>>();
        vec![tone(left), tone(right)]
    }

    fn run(p: &mut dyn Plugin, io: &mut [Vec<f32>]) {
        let frames = io.first().map_or(0, Vec::len);
        for start in (0..frames).step_by(480) {
            let mut block: Vec<Vec<f32>> = io.iter().map(|c| c[start..start + 480].to_vec()).collect();
            p.process(&mut block, 480);
            for (c, b) in io.iter_mut().zip(block) {
                c[start..start + 480].copy_from_slice(&b);
            }
        }
    }

    fn peak(c: &[f32]) -> f32 {
        c.iter().skip(2_400).fold(0.0, |m, x| m.max(x.abs()))
    }

    #[test]
    fn one_mono_instance_per_channel() {
        let (mut mm, refused) = MultiMono::new("gain", None).unwrap();
        assert!(!refused);
        mm.prepare(48_000.0, 512, 2);
        assert_eq!(mm.inner.len(), 2);
        mm.prepare(48_000.0, 512, 6);
        assert_eq!(mm.inner.len(), 6);
    }

    #[test]
    fn parameters_are_linked() {
        let (mut mm, _) = MultiMono::new("gain", None).unwrap();
        mm.prepare(48_000.0, 512, 2);
        assert!(mm.set_param("gain", -6.0));
        assert!(mm.inner.iter().all(|p| p.param("gain") == Some(-6.0)));
        assert!(!mm.set_param("nope", 1.0));
    }

    #[test]
    fn linear_plugin_matches_multichannel() {
        let mut stereo = super::super::create_plugin("gain").unwrap();
        stereo.prepare(48_000.0, 512, 2);
        stereo.set_param("gain", -6.0);
        let (mut mm, _) = MultiMono::new("gain", None).unwrap();
        mm.prepare(48_000.0, 512, 2);
        mm.set_param("gain", -6.0);
        let mut a = stereo_block(0.5, 0.25);
        let mut b = a.clone();
        run(stereo.as_mut(), &mut a);
        run(&mut mm, &mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn channels_are_processed_independently() {
        let setup = |p: &mut dyn Plugin| {
            p.set_param("threshold", -30.0);
            p.set_param("ratio", 10.0);
            p.set_param("attack", 0.1);
        };
        let mut stereo = super::super::create_plugin("compressor").unwrap();
        stereo.prepare(48_000.0, 512, 2);
        setup(stereo.as_mut());
        let (mut mm, _) = MultiMono::new("compressor", None).unwrap();
        mm.prepare(48_000.0, 512, 2);
        setup(&mut mm);
        let mut a = stereo_block(0.9, 0.02);
        let mut b = a.clone();
        run(stereo.as_mut(), &mut a);
        run(&mut mm, &mut b);
        assert!(peak(&b[1]) > peak(&a[1]) * 1.5, "right channel: multi-mono {} vs stereo {}", peak(&b[1]), peak(&a[1]));
    }
}
