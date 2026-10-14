#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioBuffer {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f32>>,
}

impl AudioBuffer {
    pub fn new(sample_rate: u32, channels: usize, frames: usize) -> Self {
        Self { sample_rate, channels: vec![vec![0.0; frames]; channels] }
    }

    pub fn frames(&self) -> usize {
        self.channels.iter().map(Vec::len).min().unwrap_or(0)
    }

    pub fn num_channels(&self) -> usize {
        self.channels.len()
    }

    pub fn duration_secs(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames() as f64 / f64::from(self.sample_rate)
    }

    pub fn interleaved(&self) -> Vec<f32> {
        let frames = self.frames();
        let mut out = Vec::with_capacity(frames.saturating_mul(self.channels.len()));
        for f in 0..frames {
            for ch in &self.channels {
                out.push(ch.get(f).copied().unwrap_or(0.0));
            }
        }
        out
    }

    pub fn from_interleaved(sr: u32, channels: usize, data: &[f32]) -> Self {
        if channels == 0 {
            return Self { sample_rate: sr, channels: Vec::new() };
        }
        let frames = data.len() / channels;
        let mut out: Vec<Vec<f32>> = (0..channels).map(|_| Vec::with_capacity(frames)).collect();
        for frame in data.chunks_exact(channels) {
            for (ch, &s) in out.iter_mut().zip(frame) {
                ch.push(s);
            }
        }
        Self { sample_rate: sr, channels: out }
    }

    pub fn peak(&self) -> f32 {
        self.channels.iter().flat_map(|c| c.iter()).fold(0.0f32, |acc, &s| acc.max(s.abs()))
    }
}
