use crate::bitstream::{BitWriter, escape_rbsp};
use crate::isobmff::{AvcConfig, Brand, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
use crate::{Result, VideoError};
use std::io::Cursor;

pub const MB_W: usize = 4;
pub const MB_H: usize = 3;

fn err(e: impl std::fmt::Display) -> VideoError {
    VideoError::Container(e.to_string())
}

fn nal_body(ref_idc: u8, t: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut v = vec![(ref_idc << 5) | t];
    v.extend(escape_rbsp(rbsp));
    v
}

fn sps(mb_w: usize, mb_h: usize) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_bits(66, 8);
    w.write_bits(0, 8);
    w.write_bits(30, 8);
    w.write_ue(0);
    w.write_ue(0);
    w.write_ue(2);
    w.write_ue(1);
    w.write_bit(false);
    w.write_ue(mb_w.saturating_sub(1) as u32);
    w.write_ue(mb_h.saturating_sub(1) as u32);
    w.write_bit(true);
    w.write_bit(true);
    w.write_bit(false);
    w.write_bit(false);
    w.rbsp_trailing();
    nal_body(3, 7, &w.finish())
}

fn pps() -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(false);
    w.write_bit(false);
    w.write_ue(0);
    w.write_ue(0);
    w.write_ue(0);
    w.write_bit(false);
    w.write_bits(0, 2);
    w.write_se(0);
    w.write_se(0);
    w.write_se(0);
    w.write_bit(true);
    w.write_bit(false);
    w.write_bit(false);
    w.rbsp_trailing();
    nal_body(3, 8, &w.finish())
}

fn pcm_picture(idr: bool, frame_num: u32, mb_w: usize, mb_h: usize, color: &dyn Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.write_ue(0);
    w.write_ue(7);
    w.write_ue(0);
    w.write_bits(frame_num % 16, 4);
    if idr {
        w.write_ue(0);
        w.write_bit(false);
        w.write_bit(false);
    } else {
        w.write_bit(false);
    }
    w.write_se(0);
    w.write_ue(0);
    w.write_se(0);
    w.write_se(0);
    for my in 0..mb_h {
        for mx in 0..mb_w {
            let [y, cb, cr] = color(mx, my);
            let mut mb = vec![y; 256];
            mb.extend([cb; 64]);
            mb.extend([cr; 64]);
            w.write_ue(25);
            w.align_zero();
            w.write_bytes(&mb);
        }
    }
    w.rbsp_trailing();
    nal_body(3, if idr { 5 } else { 1 }, &w.finish())
}

fn length_prefixed(nal: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(nal.len() + 4);
    v.extend(u32::try_from(nal.len()).unwrap_or(u32::MAX).to_be_bytes());
    v.extend(nal);
    v
}

pub fn frame_color(i: usize) -> [u8; 3] {
    [(20 + (i * 7) % 200) as u8, (60 + (i * 13) % 130) as u8, (200 - (i * 11) % 130) as u8]
}

pub fn h264_mp4_with(n: usize, gop: usize, mb_w: usize, mb_h: usize, fps: u32, color: &dyn Fn(usize, usize, usize) -> [u8; 3]) -> Result<Vec<u8>> {
    let (mb_w, mb_h) = (mb_w.clamp(1, 120), mb_h.clamp(1, 68));
    let fps = fps.clamp(1, 240);
    let avcc = AvcConfig::new(vec![sps(mb_w, mb_h)], vec![pps()], 4);
    let mut mw = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).map_err(err)?;
    let entry = SampleEntry::avc(avcc, (mb_w * 16) as u16, (mb_h * 16) as u16);
    let t = mw.add_track(TrackConfig::new(entry, fps * 100)).map_err(err)?;
    let mut frame_num = 0;
    for i in 0..n.min(100_000) {
        let idr = i % gop.max(1) == 0;
        if idr {
            frame_num = 0;
        }
        let au = length_prefixed(&pcm_picture(idr, frame_num, mb_w, mb_h, &|x, y| color(i, x, y)));
        frame_num += 1;
        mw.write_sample(t, WriteSample { data: &au, duration: 100, composition_offset: 0, is_sync: idr }).map_err(err)?;
    }
    Ok(mw.finish().map_err(err)?.into_inner())
}

pub fn h264_mp4(n: usize, gop: usize) -> Result<Vec<u8>> {
    h264_mp4_with(n, gop, MB_W, MB_H, 24, &|i, _, _| frame_color(i))
}

pub fn test_pattern_mp4(secs: u32) -> Result<Vec<u8>> {
    const BARS: [[u8; 3]; 7] = [[180, 128, 128], [162, 44, 142], [131, 156, 44], [112, 72, 58], [84, 184, 198], [65, 100, 212], [35, 212, 114]];
    let (w, h) = (20usize, 12usize);
    h264_mp4_with(secs.clamp(1, 600) as usize * 24, 24, w, h, 24, &|i, x, y| {
        if y == 9 && x == i % w {
            [235, 128, 128]
        } else if y >= 8 {
            [16 + (x * 219 / w) as u8, 128, 128]
        } else {
            BARS.get(x * BARS.len() / w).copied().unwrap_or([16, 128, 128])
        }
    })
}

pub fn prores_mov(n: usize) -> Result<Vec<u8>> {
    use crate::prores::{ChromaFormat, Encoder, Frame, Profile};
    let (w, h) = (64u32, 48u32);
    let mut enc = Encoder::new(Profile::Hq, w, h);
    let mut mw = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).map_err(err)?;
    let t = mw.add_track(TrackConfig::new(SampleEntry::prores(FourCc(*b"apch"), w as u16, h as u16), 25)).map_err(err)?;
    for i in 0..n.min(10_000) {
        let mut f = Frame::new(w, h, ChromaFormat::Yuv422, 10, false);
        let y = ((16 + 8 * (i as u32 % 27)) << 2) as u16;
        f.y.iter_mut().for_each(|s| *s = y);
        let data = enc.encode(&f).map_err(err)?;
        mw.write_sample(t, WriteSample { data: &data, duration: 1, composition_offset: 0, is_sync: true }).map_err(err)?;
    }
    Ok(mw.finish().map_err(err)?.into_inner())
}

pub fn mjpeg_mov(n: usize) -> Result<Vec<u8>> {
    let (w, h) = (32u32, 24u32);
    let mut mw = Mp4Writer::new(Cursor::new(Vec::new()), WriterOptions::new(Brand::Mov)).map_err(err)?;
    let t = mw.add_track(TrackConfig::new(SampleEntry::jpeg(w as u16, h as u16), 10)).map_err(err)?;
    for i in 0..n.min(10_000) {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([(30 * (i % 9)) as u8; 3]));
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 95).encode_image(&img).map_err(err)?;
        mw.write_sample(t, WriteSample { data: &jpg, duration: 1, composition_offset: 0, is_sync: true }).map_err(err)?;
    }
    Ok(mw.finish().map_err(err)?.into_inner())
}
