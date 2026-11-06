use crate::{Engine, Result};
use openpt_model::{BinauralDistance, ChannelFormat, Route, Session, Speaker, SurroundPan, TrackId};
use openpt_time::{Range, Samples};
use serde_json::{Value, json};
use std::fmt::Write as _;

const META_STEP: Samples = 1024;
const MAX_CHANNELS: usize = 128;
const DBMD_VERSION: u32 = 0x0100_0006;
const DBMD_SUPPLEMENTAL_SEGMENT: u8 = 10;
const DBMD_SYNC: u32 = 0xF872_6FBD;
const DBMD_TRIM_CONFIGS: usize = 9;
const DBMD_HEAD_TRACK_DEFAULT: u8 = 0x80;

#[derive(Debug, Clone, PartialEq)]
pub struct ObjectBlock {
    pub start: Samples,
    pub length: Samples,
    pub pan: SurroundPan,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdmObject {
    pub name: String,
    pub blocks: Vec<ObjectBlock>,
    pub binaural: BinauralDistance,
}

pub fn is_object(s: &Session, t: TrackId) -> bool {
    s.edit.flag(&format!("object.{}", t.0))
}

fn speaker_label(sp: Speaker) -> String {
    if sp.is_lfe() {
        return "LFE1".into();
    }
    let az = -sp.azimuth();
    let layer = if sp.is_height() { 'U' } else { 'M' };
    let sign = if az < 0.0 { '-' } else { '+' };
    format!("{layer}{sign}{:03}", az.abs().round() as i32)
}

fn ticks(samples: Samples, rate: u32) -> i64 {
    (samples.max(0) as f64 * 100_000.0 / f64::from(rate.max(1))).round() as i64
}

fn time_ticks(t: i64) -> String {
    let secs = t.max(0) as f64 / 100_000.0;
    let h = (secs / 3600.0).floor();
    let m = ((secs - h * 3600.0) / 60.0).floor();
    let s = secs - h * 3600.0 - m * 60.0;
    format!("{:02}:{:02}:{:08.5}", h as u64, m as u64, s)
}

fn time(samples: Samples, rate: u32) -> String {
    time_ticks(ticks(samples, rate))
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn blocks_for(s: &Session, t: TrackId, range: Range) -> Vec<ObjectBlock> {
    let Some(track) = s.track(t) else { return Vec::new() };
    let mut out: Vec<ObjectBlock> = Vec::new();
    let mut at = range.start;
    while at < range.end {
        let len = META_STEP.min(range.end - at);
        let centre = if track.channels() >= 2 { (openpt_mix::pan_at(track, 0, at) + openpt_mix::pan_at(track, 1, at)) * 0.5 } else { openpt_mix::pan_at(track, 0, at) };
        let pan = openpt_mix::surround_at(track, at).unwrap_or_else(|| SurroundPan::from_stereo(centre));
        match out.last_mut() {
            Some(b) if b.pan == pan => b.length += len,
            _ => out.push(ObjectBlock { start: at - range.start, length: len, pan }),
        }
        at += len;
    }
    out
}

pub fn axml(name: &str, rate: u32, bed: ChannelFormat, objects: &[AdmObject], length: Samples) -> String {
    let speakers: Vec<Speaker> = bed.speakers().to_vec();
    let mut x = String::new();
    let _ = writeln!(x, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(x, r#"<ebuCoreMain xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns="urn:ebu:metadata-schema:ebuCore_2014" schema="EBU_CORE_20140201.xsd" xml:lang="en">"#);
    let _ = writeln!(x, "<coreMetadata><format><audioFormatExtended version=\"ITU-R_BS.2076-2\">");
    let end = time(length, rate);
    let _ = writeln!(x, r#"<audioProgramme audioProgrammeID="APR_1001" audioProgrammeName="{}" start="00:00:00.00000" end="{end}">"#, esc(name));
    for k in 0..=objects.len() {
        let _ = writeln!(x, "<audioContentIDRef>ACO_{:04X}</audioContentIDRef>", 0x1001 + k);
    }
    let _ = writeln!(x, "</audioProgramme>");
    let bed_name = format!("Bed {}", bed.label());
    for (k, label) in std::iter::once(bed_name.as_str()).chain(objects.iter().map(|o| o.name.as_str())).enumerate() {
        let _ = writeln!(
            x,
            r#"<audioContent audioContentID="ACO_{:04X}" audioContentName="{}"><audioObjectIDRef>AO_{:04X}</audioObjectIDRef></audioContent>"#,
            0x1001 + k,
            esc(label),
            0x1001 + k
        );
    }
    let mut uid = 0usize;
    let _ = writeln!(x, r#"<audioObject audioObjectID="AO_1001" audioObjectName="{}" start="00:00:00.00000" duration="{end}">"#, esc(&bed_name));
    let _ = writeln!(x, "<audioPackFormatIDRef>AP_00011001</audioPackFormatIDRef>");
    for _ in &speakers {
        uid += 1;
        let _ = writeln!(x, "<audioTrackUIDRef>ATU_{uid:08X}</audioTrackUIDRef>");
    }
    let _ = writeln!(x, "</audioObject>");
    for (k, o) in objects.iter().enumerate() {
        uid += 1;
        let _ = writeln!(
            x,
            r#"<audioObject audioObjectID="AO_{:04X}" audioObjectName="{}" start="00:00:00.00000" duration="{end}"><audioPackFormatIDRef>AP_0003{:04X}</audioPackFormatIDRef><audioTrackUIDRef>ATU_{uid:08X}</audioTrackUIDRef></audioObject>"#,
            0x1002 + k,
            esc(&o.name),
            0x1001 + k
        );
    }
    let _ = writeln!(x, r#"<audioPackFormat audioPackFormatID="AP_00011001" audioPackFormatName="{}" typeLabel="0001" typeDefinition="DirectSpeakers">"#, esc(&bed_name));
    for i in 0..speakers.len() {
        let _ = writeln!(x, "<audioChannelFormatIDRef>AC_0001{:04X}</audioChannelFormatIDRef>", 0x1001 + i);
    }
    let _ = writeln!(x, "</audioPackFormat>");
    for (k, o) in objects.iter().enumerate() {
        let _ = writeln!(
            x,
            r#"<audioPackFormat audioPackFormatID="AP_0003{:04X}" audioPackFormatName="{}" typeLabel="0003" typeDefinition="Objects"><audioChannelFormatIDRef>AC_0003{:04X}</audioChannelFormatIDRef></audioPackFormat>"#,
            0x1001 + k,
            esc(&o.name),
            0x1001 + k
        );
    }
    for (i, sp) in speakers.iter().enumerate() {
        let id = format!("0001{:04X}", 0x1001 + i);
        let label = speaker_label(*sp);
        let _ = write!(x, r#"<audioChannelFormat audioChannelFormatID="AC_{id}" audioChannelFormatName="{label}" typeLabel="0001" typeDefinition="DirectSpeakers">"#);
        if sp.is_lfe() {
            let _ = write!(x, r#"<frequency typeDefinition="lowPass">120</frequency>"#);
        }
        let _ = writeln!(
            x,
            r#"<audioBlockFormat audioBlockFormatID="AB_{id}_00000001"><speakerLabel>{label}</speakerLabel><position coordinate="azimuth">{:.1}</position><position coordinate="elevation">{:.1}</position><position coordinate="distance">1.0</position></audioBlockFormat></audioChannelFormat>"#,
            if sp.is_lfe() { 0.0 } else { -sp.azimuth() },
            if sp.is_lfe() { -30.0 } else { sp.elevation() }
        );
    }
    for (k, o) in objects.iter().enumerate() {
        let id = format!("0003{:04X}", 0x1001 + k);
        let _ = writeln!(x, r#"<audioChannelFormat audioChannelFormatID="AC_{id}" audioChannelFormatName="{}" typeLabel="0003" typeDefinition="Objects">"#, esc(&o.name));
        for (b, blk) in o.blocks.iter().enumerate() {
            let p = blk.pan;
            let size = p.divergence.clamp(0.0, 1.0);
            let _ = writeln!(
                x,
                r#"<audioBlockFormat audioBlockFormatID="AB_{id}_{:08X}" rtime="{}" duration="{}"><cartesian>1</cartesian><position coordinate="X">{:.4}</position><position coordinate="Y">{:.4}</position><position coordinate="Z">{:.4}</position><width>{size:.4}</width><depth>{size:.4}</depth><height>{size:.4}</height><jumpPosition interpolationLength="0.005">1</jumpPosition></audioBlockFormat>"#,
                b + 1,
                time(blk.start, rate),
                time_ticks(ticks(blk.start + blk.length, rate) - ticks(blk.start, rate)),
                p.x.clamp(-1.0, 1.0),
                p.y.clamp(-1.0, 1.0),
                p.z.clamp(0.0, 1.0)
            );
        }
        let _ = writeln!(x, "</audioChannelFormat>");
    }
    let streams = speakers.len() + objects.len();
    for i in 0..streams {
        let (ty, n) = if i < speakers.len() { ("0001", 0x1001 + i) } else { ("0003", 0x1001 + i - speakers.len()) };
        let _ = writeln!(
            x,
            r#"<audioStreamFormat audioStreamFormatID="AS_{ty}{n:04X}" audioStreamFormatName="PCM_{n:04X}" formatLabel="0001" formatDefinition="PCM"><audioChannelFormatIDRef>AC_{ty}{n:04X}</audioChannelFormatIDRef><audioTrackFormatIDRef>AT_{ty}{n:04X}_01</audioTrackFormatIDRef></audioStreamFormat>"#
        );
        let _ = writeln!(
            x,
            r#"<audioTrackFormat audioTrackFormatID="AT_{ty}{n:04X}_01" audioTrackFormatName="PCM_{n:04X}" formatLabel="0001" formatDefinition="PCM"><audioStreamFormatIDRef>AS_{ty}{n:04X}</audioStreamFormatIDRef></audioTrackFormat>"#
        );
    }
    for i in 0..streams {
        let (ty, n, pack) = if i < speakers.len() { ("0001", 0x1001 + i, "AP_00011001".to_string()) } else {
            let k = i - speakers.len();
            ("0003", 0x1001 + k, format!("AP_0003{:04X}", 0x1001 + k))
        };
        let _ = writeln!(
            x,
            r#"<audioTrackUID UID="ATU_{:08X}" sampleRate="{rate}" bitDepth="24"><audioTrackFormatIDRef>AT_{ty}{n:04X}_01</audioTrackFormatIDRef><audioPackFormatIDRef>{pack}</audioPackFormatIDRef></audioTrackUID>"#,
            i + 1
        );
    }
    let _ = writeln!(x, "</audioFormatExtended></format></coreMetadata></ebuCoreMain>");
    x
}

pub fn chna(bed: ChannelFormat, objects: usize) -> Vec<u8> {
    let speakers = bed.speakers().len();
    let n = speakers + objects;
    let mut v = Vec::with_capacity(4 + 40 * n);
    v.extend_from_slice(&u16::try_from(n).unwrap_or(u16::MAX).to_le_bytes());
    v.extend_from_slice(&u16::try_from(n).unwrap_or(u16::MAX).to_le_bytes());
    for i in 0..n {
        let (ty, k, pack) = if i < speakers { ("0001", 0x1001 + i, "AP_00011001".to_string()) } else {
            let o = i - speakers;
            ("0003", 0x1001 + o, format!("AP_0003{:04X}", 0x1001 + o))
        };
        v.extend_from_slice(&u16::try_from(i + 1).unwrap_or(u16::MAX).to_le_bytes());
        v.extend_from_slice(format!("ATU_{:08X}", i + 1).as_bytes());
        v.extend_from_slice(format!("AT_{ty}{k:04X}_01").as_bytes());
        v.extend_from_slice(pack.as_bytes());
        v.push(0);
    }
    v
}

fn binaural_code(d: BinauralDistance) -> u8 {
    match d {
        BinauralDistance::Off => 0,
        BinauralDistance::Near => 1,
        BinauralDistance::Far => 2,
        BinauralDistance::Mid => 3,
    }
}

pub fn binaural_modes(s: &Session, bed: ChannelFormat, objects: &[AdmObject]) -> Vec<BinauralDistance> {
    let beds = bed.speakers().iter().enumerate().map(|(c, sp)| if sp.is_lfe() { BinauralDistance::Off } else { s.atmos.binaural_distance.get(c).copied().unwrap_or_default() });
    beds.chain(objects.iter().map(|o| o.binaural)).collect()
}

pub fn dbmd(modes: &[BinauralDistance]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(142 + 2 * modes.len());
    payload.extend_from_slice(&DBMD_SYNC.to_le_bytes());
    payload.extend_from_slice(&u16::try_from(modes.len()).unwrap_or(u16::MAX).to_le_bytes());
    payload.push(0);
    for _ in 0..DBMD_TRIM_CONFIGS {
        payload.push(1);
        payload.extend_from_slice(&[0; 14]);
    }
    payload.resize(payload.len() + modes.len(), 0);
    payload.extend(modes.iter().map(|m| DBMD_HEAD_TRACK_DEFAULT | binaural_code(*m)));
    let size = u16::try_from(payload.len()).unwrap_or(u16::MAX);
    let sum = payload.iter().fold(size as u8, |sum, b| sum.wrapping_add(*b));
    let mut out = Vec::with_capacity(payload.len() + 9);
    out.extend_from_slice(&DBMD_VERSION.to_le_bytes());
    out.push(DBMD_SUPPLEMENTAL_SEGMENT);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&payload);
    out.push(sum.wrapping_neg());
    out.push(0);
    out
}

pub fn wav(rate: u32, channels: &[Vec<f32>], chna: &[u8], axml: &str, dbmd: &[u8]) -> Vec<u8> {
    let nch = channels.len();
    let frames = channels.iter().map(Vec::len).max().unwrap_or(0);
    let data_len = frames * nch * 3;
    let chunk = |id: &[u8; 4], body: &[u8], out: &mut Vec<u8>| {
        out.extend_from_slice(id);
        out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
    };
    let mut fmt = Vec::with_capacity(16);
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&u16::try_from(nch).unwrap_or(u16::MAX).to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * 3 * nch as u32).to_le_bytes());
    fmt.extend_from_slice(&u16::try_from(3 * nch).unwrap_or(u16::MAX).to_le_bytes());
    fmt.extend_from_slice(&24u16.to_le_bytes());
    let mut data = Vec::with_capacity(data_len);
    for i in 0..frames {
        for ch in channels {
            let v = ch.get(i).copied().unwrap_or(0.0);
            let q = (f64::from(if v.is_finite() { v } else { 0.0 }).clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
            data.extend_from_slice(&q.to_le_bytes()[..3]);
        }
    }
    let mut body = Vec::with_capacity(data_len + axml.len() + chna.len() + dbmd.len() + 64);
    body.extend_from_slice(b"WAVE");
    chunk(b"fmt ", &fmt, &mut body);
    chunk(b"chna", chna, &mut body);
    chunk(b"data", &data, &mut body);
    chunk(b"axml", axml.as_bytes(), &mut body);
    chunk(b"dbmd", dbmd, &mut body);
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

pub fn export(e: &mut Engine, p: &Value) -> Result<Value> {
    let id = "file.export_adm";
    let path = p.get("path").and_then(Value::as_str).filter(|x| !x.trim().is_empty()).ok_or_else(|| crate::cmd::bad(id, "`path` required"))?.to_string();
    let s = e.session().clone();
    let bed = s.main_format();
    if bed.speakers().is_empty() || bed.channels() < 2 {
        return Err(crate::cmd::bad(id, "the main output must be a speaker format (e.g. 7.1.4) to export a bed"));
    }
    let range = crate::cmd::range_param(e, id, p).ok().filter(|r| !r.is_empty()).unwrap_or_else(|| Range::new(0, s.content_end().max(1)));
    let objects: Vec<TrackId> = s.tracks.iter().filter(|t| !t.inactive && is_object(&s, t.id) && t.mixer.output != Route::None).map(|t| t.id).collect();
    let mut bed_session = s.clone();
    for t in &objects {
        if let Some(tr) = bed_session.track_mut(*t) {
            tr.mixer.output = Route::None;
        }
    }
    let mut channels = openpt_mix::render_range(&bed_session, range, 1024);
    channels.truncate(bed.channels());
    let mut meta = Vec::new();
    for t in &objects {
        let mut solo = s.clone();
        let Some(track) = solo.track(*t) else { continue };
        let (name, width, binaural) = (track.name.clone(), track.channels().clamp(1, 2), track.mixer.binaural_distance);
        let bus = solo.add_bus(&format!("ADM object {}", t.0), ChannelFormat::for_channels(width));
        if let Some(tr) = solo.track_mut(*t) {
            tr.mixer.output = Route::Bus(bus);
        }
        let audio = openpt_mix::render_bus(&solo, bus, range, 1024);
        let blocks = blocks_for(&s, *t, range);
        if audio.len() == 2 {
            for (side, ch) in audio.into_iter().enumerate() {
                let shift = if side == 0 { -0.5 } else { 0.5 };
                let blocks = blocks.iter().map(|b| ObjectBlock { pan: SurroundPan { x: (b.pan.x + shift).clamp(-1.0, 1.0), ..b.pan }, ..b.clone() }).collect();
                channels.push(ch);
                meta.push(AdmObject { name: format!("{name} {}", if side == 0 { "L" } else { "R" }), blocks, binaural });
            }
        } else if let Some(ch) = audio.into_iter().next() {
            channels.push(ch);
            meta.push(AdmObject { name, blocks, binaural });
        }
    }
    if channels.len() > MAX_CHANNELS {
        return Err(crate::cmd::bad(id, format!("{} channels: ADM masters hold at most {MAX_CHANNELS}", channels.len())));
    }
    let rate = s.sample_rate.hz();
    let length = range.len();
    let xml = axml(&s.name, rate, bed, &meta, length);
    let bytes = wav(rate, &channels, &chna(bed, meta.len()), &xml, &dbmd(&binaural_modes(&s, bed, &meta)));
    crate::io::write_file(&path, &bytes)?;
    let warn = (![48_000, 96_000].contains(&rate)).then(|| format!("Dolby Atmos masters are 48 or 96 kHz; this session runs at {rate} Hz"));
    Ok(json!({"path": path, "bed": bed.label(), "objects": meta.len(), "channels": channels.len(), "warning": warn}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_labels_follow_bs2051() {
        assert_eq!(speaker_label(Speaker::L), "M+030");
        assert_eq!(speaker_label(Speaker::R), "M-030");
        assert_eq!(speaker_label(Speaker::C), "M+000");
        assert_eq!(speaker_label(Speaker::Lfe), "LFE1");
        assert_eq!(speaker_label(Speaker::Rtf), "U-045");
        assert_eq!(time(48_000 * 3725 + 24_000, 48_000), "01:02:05.50000");
    }

    #[test]
    fn the_chna_rows_point_at_the_axml_ids() {
        let c = chna(ChannelFormat::Atmos714, 2);
        assert_eq!(u16::from_le_bytes([c[0], c[1]]), 14);
        assert_eq!(c.len(), 4 + 40 * 14);
        let row = |i: usize| String::from_utf8_lossy(&c[4 + 40 * i + 2..4 + 40 * i + 39]).to_string();
        assert_eq!(row(0), "ATU_00000001AT_00011001_01AP_00011001");
        assert_eq!(row(13), "ATU_0000000EAT_00031002_01AP_00031002");
        let x = axml("t", 48_000, ChannelFormat::Atmos714, &[AdmObject { name: "a".into(), blocks: vec![], binaural: BinauralDistance::Mid }, AdmObject { name: "b".into(), blocks: vec![], binaural: BinauralDistance::Mid }], 48_000);
        for i in 0..14 {
            let r = row(i);
            assert!(x.contains(&format!("UID=\"{}\"", &r[..12])), "{r}");
            assert!(x.contains(&format!("audioTrackFormatID=\"{}\"", &r[12..26])), "{r}");
            assert!(x.contains(&format!("audioPackFormatID=\"{}\"", &r[26..])), "{r}");
        }
    }

    #[test]
    fn the_dbmd_chunk_carries_one_binaural_mode_per_track() {
        let modes = [BinauralDistance::Off, BinauralDistance::Near, BinauralDistance::Far, BinauralDistance::Mid];
        let d = dbmd(&modes);
        assert_eq!(u32::from_le_bytes([d[0], d[1], d[2], d[3]]), 0x0100_0006);
        assert_eq!(d[4], 10);
        let size = usize::from(u16::from_le_bytes([d[5], d[6]]));
        let payload = &d[7..7 + size];
        assert_eq!(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]), 0xF872_6FBD);
        assert_eq!(u16::from_le_bytes([payload[4], payload[5]]), 4);
        let sum = payload.iter().fold(size as u8, |s, b| s.wrapping_add(*b));
        assert_eq!(sum.wrapping_add(d[7 + size]), 0, "two's complement checksum");
        assert_eq!(d[8 + size], 0, "segment list terminator");
        assert_eq!(d.len(), 9 + size);
        let headphone = &payload[size - 4..];
        assert_eq!(headphone.iter().map(|b| b & 7).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        assert!(payload[7..142].chunks(15).all(|cfg| cfg[0] == 1 && cfg[1..].iter().all(|b| *b == 0)), "automatic trims");
    }

    #[test]
    fn a_session_writes_lfe_off_then_bed_then_objects() {
        let mut s = Session::default();
        s.atmos.binaural_distance = [BinauralDistance::Far; openpt_model::BINAURAL_CHANNELS];
        let o = AdmObject { name: "o".into(), blocks: vec![], binaural: BinauralDistance::Near };
        let modes = binaural_modes(&s, ChannelFormat::Atmos714, &[o]);
        assert_eq!(modes.len(), 13);
        let lfe = ChannelFormat::Atmos714.speakers().iter().position(|sp| sp.is_lfe()).unwrap();
        assert_eq!(modes[lfe], BinauralDistance::Off);
        assert_eq!(modes[0], BinauralDistance::Far);
        assert_eq!(modes[12], BinauralDistance::Near);
    }
}

#[cfg(test)]
mod export_tests {
    use crate::Engine;
    use openpt_audio_io::AudioBuffer;
    use openpt_model::{ChannelFormat, Clip, SourceAudio, SourceId, TrackKind};
    use serde_json::json;
    use std::sync::Arc;

    #[test]
    fn exports_a_bed_and_moving_objects() {
        let mut e = Engine::default();
        e.execute("setup.main_format", &json!({"format": "7.1.4"})).unwrap();
        let n = 48_000usize;
        let tone = |f: f32| (0..n).map(|i| (i as f32 * f * std::f32::consts::TAU / 48_000.0).sin() * 0.3).collect::<Vec<f32>>();
        let mut ids = Vec::new();
        for (k, (name, fmt, src)) in [("Pad", ChannelFormat::Stereo, 91u64), ("Fly", ChannelFormat::Mono, 92), ("Keys", ChannelFormat::Stereo, 93)].into_iter().enumerate() {
            let s = e.session_mut();
            let ch = fmt.channels();
            s.pool.insert(SourceId(src), Arc::new(SourceAudio::new(AudioBuffer { sample_rate: 48_000, channels: vec![tone(220.0 * (k + 1) as f32); ch] })));
            let t = s.add_track(TrackKind::Audio, fmt, Some(name));
            let cid = s.new_clip_id();
            s.track_mut(t).unwrap().playlist_mut().unwrap().clips.push(Clip::audio(cid, name, SourceId(src), 0, 0, n as i64));
            ids.push(t);
        }
        e.execute("track.object", &json!({"tracks": [ids[1].0, ids[2].0], "object": true})).unwrap();
        e.execute("track.binaural_distance", &json!({"tracks": [ids[1].0], "distance": "near"})).unwrap();
        e.execute("track.binaural_distance", &json!({"tracks": [ids[2].0], "distance": "far"})).unwrap();
        e.execute("setup.atmos", &json!({"binaural_distance": "off", "channel": 0})).unwrap();
        assert!(e.execute("track.binaural_distance", &json!({"tracks": [ids[1].0], "distance": "nope"})).is_err());
        e.execute("mix.surround_pan", &json!({"track": ids[1].0, "x": -1.0, "y": 1.0})).unwrap();
        for (at, x) in [(0, -1.0), (n as i64, 1.0)] {
            e.execute("automation.set_point", &json!({"track": ids[1].0, "param": "surround_x", "at": at, "value": x})).unwrap();
        }
        let dir = std::env::temp_dir().join(format!("openpt-adm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("master.wav");
        let r = e.execute("file.export_adm", &json!({"path": path.to_string_lossy(), "start": 0, "end": n})).unwrap();
        assert_eq!((r["objects"].as_u64(), r["channels"].as_u64()), (Some(3), Some(15)), "{r}");
        let bytes = std::fs::read(&path).unwrap();
        let has = |id: &[u8]| bytes.windows(4).any(|w| w == id);
        assert!(has(b"chna") && has(b"axml") && has(b"data"));
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.matches("<audioBlockFormat audioBlockFormatID=\"AB_00031001_").count() > 10, "the moving object gets many blocks");
        assert_eq!(text.matches("<audioBlockFormat audioBlockFormatID=\"AB_00031002_").count(), 1, "a still object gets one");
        let at = bytes.windows(4).position(|w| w == b"dbmd").unwrap();
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
        assert_eq!(at + 8 + size + size % 2, bytes.len(), "dbmd is the last chunk");
        let end = at + 8 + size - 2;
        let modes: Vec<u8> = bytes[end - 15..end].iter().map(|b| b & 7).collect();
        assert_eq!(modes[0], 0, "channel 0 set to off");
        assert_eq!(modes[3], 0, "the LFE is off");
        assert_eq!(modes[11], 3, "an untouched bed channel stays on mid");
        assert_eq!(&modes[12..], &[1, 2, 2], "fly is near, both sides of keys are far");
        if let Some(out) = std::env::var_os("OPENPT_ADM_OUT") {
            std::fs::copy(&path, out).unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
