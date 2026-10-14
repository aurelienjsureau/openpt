use crate::{AudioBuffer, AudioError, Result};
use mp3lame_encoder::{Bitrate, Builder, DualPcm, FlushNoGap, MonoPcm, Quality};

const CHUNK: usize = 1152 * 32;

fn err(e: impl std::fmt::Display) -> AudioError {
    AudioError::Encode(format!("MP3: {e}"))
}

pub fn encode(buf: &AudioBuffer) -> Result<Vec<u8>> {
    let channels = buf.channels.len().min(2);
    if channels == 0 || buf.frames() == 0 {
        return Err(AudioError::Encode("nothing to encode".into()));
    }
    let mut b = Builder::new().ok_or_else(|| err("cannot start the encoder"))?;
    b.set_num_channels(channels as u8).map_err(err)?;
    b.set_sample_rate(buf.sample_rate).map_err(err)?;
    b.set_brate(Bitrate::Kbps320).map_err(err)?;
    b.set_quality(Quality::Best).map_err(err)?;
    let mut enc = b.build().map_err(err)?;
    let clip = |v: &[f32]| -> Vec<f32> { v.iter().map(|x| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 }).collect() };
    let left = clip(buf.channels.first().map_or(&[][..], Vec::as_slice));
    let right = if channels == 2 { clip(buf.channels.get(1).map_or(&[][..], Vec::as_slice)) } else { Vec::new() };
    let mut out = Vec::new();
    let frames = buf.frames();
    let mut at = 0;
    while at < frames {
        let end = (at + CHUNK).min(frames);
        out.reserve(mp3lame_encoder::max_required_buffer_size(end - at));
        let l = left.get(at..end).unwrap_or(&[]);
        if channels == 2 {
            let r = right.get(at..end).unwrap_or(&[]);
            enc.encode_to_vec(DualPcm { left: l, right: r }, &mut out).map_err(err)?;
        } else {
            enc.encode_to_vec(MonoPcm(l), &mut out).map_err(err)?;
        }
        at = end;
    }
    out.reserve(7200);
    enc.flush_to_vec::<FlushNoGap>(&mut out).map_err(err)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stereo_tone_encodes_to_a_decodable_mp3() {
        let n = 44_100;
        let tone: Vec<f32> = (0..n).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44_100.0).sin()).collect();
        let buf = AudioBuffer { sample_rate: 44_100, channels: vec![tone.clone(), tone] };
        let bytes = encode(&buf).unwrap();
        assert!(bytes.len() > 30_000, "about 40 kB a second at 320 kb/s: {}", bytes.len());
        let (_, back) = crate::decode(&bytes, Some("mp3")).unwrap();
        assert_eq!(back.channels.len(), 2);
        assert!(back.frames() >= n - 2_000, "{} frames", back.frames());
    }
}
