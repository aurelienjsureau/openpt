use openpt_time::{FrameRate, SampleRate, Samples, TempoMap, Timecode};

pub const CLOCK_TICKS: f64 = 40.0;
pub const SPP_TICKS: f64 = 240.0;

pub fn mtc_rate(rate: FrameRate) -> FrameRate {
    match rate {
        FrameRate::Fps23_976 | FrameRate::Fps24 | FrameRate::Fps47_952 | FrameRate::Fps48 => rate_with_pace(rate, FrameRate::Fps24),
        FrameRate::Fps25 | FrameRate::Fps50 | FrameRate::Fps100 => FrameRate::Fps25,
        FrameRate::Fps29_97Drop | FrameRate::Fps59_94Drop => FrameRate::Fps29_97Drop,
        FrameRate::Fps29_97 | FrameRate::Fps59_94 | FrameRate::Fps119_88 => FrameRate::Fps29_97,
        FrameRate::Fps30 | FrameRate::Fps60 | FrameRate::Fps120 => FrameRate::Fps30,
    }
}

fn rate_with_pace(rate: FrameRate, nominal: FrameRate) -> FrameRate {
    if matches!(rate, FrameRate::Fps23_976 | FrameRate::Fps47_952) { FrameRate::Fps23_976 } else { nominal }
}

pub fn mtc_rate_code(rate: FrameRate) -> u8 {
    match mtc_rate(rate) {
        FrameRate::Fps23_976 | FrameRate::Fps24 => 0,
        FrameRate::Fps25 => 1,
        FrameRate::Fps29_97Drop => 2,
        _ => 3,
    }
}

pub fn rate_of_code(code: u8) -> FrameRate {
    match code & 3 {
        0 => FrameRate::Fps24,
        1 => FrameRate::Fps25,
        2 => FrameRate::Fps29_97Drop,
        _ => FrameRate::Fps30,
    }
}

fn field(v: i64) -> u8 {
    u8::try_from(v.rem_euclid(128)).unwrap_or(0)
}

pub fn mtc_full_frame(tc: Timecode, rate: FrameRate) -> Vec<u8> {
    let hh = (mtc_rate_code(rate) << 5) | (field(tc.hours) & 0x1F);
    vec![0xF0, 0x7F, 0x7F, 0x01, 0x01, hh, field(tc.minutes) & 0x3F, field(tc.seconds) & 0x3F, field(tc.frames) & 0x1F, 0xF7]
}

pub fn mtc_quarter_frame(piece: u8, tc: Timecode, rate: FrameRate) -> [u8; 2] {
    let (f, s, m, h) = (field(tc.frames), field(tc.seconds), field(tc.minutes), field(tc.hours));
    let nibble = match piece & 7 {
        0 => f & 0x0F,
        1 => (f >> 4) & 0x01,
        2 => s & 0x0F,
        3 => (s >> 4) & 0x03,
        4 => m & 0x0F,
        5 => (m >> 4) & 0x03,
        6 => h & 0x0F,
        _ => ((h >> 4) & 0x01) | (mtc_rate_code(rate) << 1),
    };
    [0xF1, ((piece & 7) << 4) | nibble]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TcReading {
    pub frames: i64,
    pub rate: FrameRate,
    pub at_us: u64,
    pub running: bool,
}

impl TcReading {
    pub fn samples_at(&self, us: u64, sr: SampleRate) -> f64 {
        let base = self.rate.samples_at_frame(self.frames, sr) as f64;
        if !self.running {
            return base;
        }
        base + (us as f64 - self.at_us as f64) / 1_000_000.0 * sr.as_f64()
    }
}

#[derive(Debug, Default, Clone)]
pub struct MtcReader {
    pieces: [u8; 8],
    seen: u8,
    last: Option<u8>,
}

impl MtcReader {
    pub fn feed(&mut self, msg: &[u8], at_us: u64) -> Option<TcReading> {
        match *msg {
            [0xF1, data, ..] => {
                let piece = (data >> 4) & 7;
                if self.last.is_some_and(|l| (l + 1) & 7 != piece) {
                    self.seen = 0;
                }
                self.last = Some(piece);
                if let Some(p) = self.pieces.get_mut(usize::from(piece)) {
                    *p = data & 0x0F;
                }
                self.seen |= 1 << piece;
                if piece != 7 || self.seen != 0xFF {
                    return None;
                }
                let p = self.pieces;
                let rate = rate_of_code((p[7] >> 1) & 3);
                let tc = Timecode {
                    negative: false,
                    frames: i64::from(p[0] | (p[1] << 4)),
                    seconds: i64::from(p[2] | (p[3] << 4)),
                    minutes: i64::from(p[4] | (p[5] << 4)),
                    hours: i64::from(p[6] | ((p[7] & 1) << 4)),
                };
                Some(TcReading { frames: tc.to_frames(rate) + 2, rate, at_us, running: true })
            }
            _ => None,
        }
    }

    pub fn feed_sysex(&mut self, msg: &[u8], at_us: u64) -> Option<TcReading> {
        match *msg {
            [0xF0, 0x7F, _, 0x01, 0x01, hh, mm, ss, ff, 0xF7] => {
                self.seen = 0;
                self.last = None;
                let rate = rate_of_code(hh >> 5);
                let tc =
                    Timecode { negative: false, hours: i64::from(hh & 0x1F), minutes: i64::from(mm), seconds: i64::from(ss), frames: i64::from(ff) };
                Some(TcReading { frames: tc.to_frames(rate), rate, at_us, running: false })
            }
            _ => None,
        }
    }
}

pub fn clock_position(map: &TempoMap, pos: f64, sr: SampleRate) -> f64 {
    map.samples_to_ticks_f(pos.round() as Samples, sr) / CLOCK_TICKS
}

pub fn song_position(map: &TempoMap, pos: Samples, sr: SampleRate) -> [u8; 3] {
    let beats = (map.samples_to_ticks_f(pos.max(0), sr) / SPP_TICKS).floor().clamp(0.0, 16_383.0) as u16;
    [0xF2, (beats & 0x7F) as u8, ((beats >> 7) & 0x7F) as u8]
}

const SYNC_WORD: u16 = 0b1011_1111_1111_1100;

fn bcd_split(v: i64) -> (u8, u8) {
    let v = u8::try_from(v.rem_euclid(100)).unwrap_or(0);
    (v % 10, v / 10)
}

pub fn ltc_bits(tc: Timecode, rate: FrameRate) -> [bool; 80] {
    let mut b = [false; 80];
    let mut put = |at: usize, width: usize, v: u8| {
        for i in 0..width {
            if let Some(x) = b.get_mut(at + i) {
                *x = (v >> i) & 1 == 1;
            }
        }
    };
    let (fu, ft) = bcd_split(tc.frames);
    let (su, st) = bcd_split(tc.seconds);
    let (mu, mt) = bcd_split(tc.minutes);
    let (hu, ht) = bcd_split(tc.hours);
    put(0, 4, fu);
    put(8, 2, ft);
    put(10, 1, u8::from(rate.is_drop()));
    put(16, 4, su);
    put(24, 3, st);
    put(32, 4, mu);
    put(40, 3, mt);
    put(48, 4, hu);
    put(56, 2, ht);
    for i in 0..16 {
        b[64 + i] = (SYNC_WORD >> i) & 1 == 1;
    }
    let parity = if rate.nominal() == 25 { 59 } else { 27 };
    let ones = b.iter().filter(|x| **x).count();
    b[parity] = ones % 2 == 1;
    b
}

fn ltc_decode(bits: &[bool; 80], rate: FrameRate) -> Timecode {
    let get = |at: usize, width: usize| -> i64 { (0..width).map(|i| i64::from(bits[at + i]) << i).sum() };
    let _ = rate;
    Timecode {
        negative: false,
        frames: get(0, 4) + 10 * get(8, 2),
        seconds: get(16, 4) + 10 * get(24, 3),
        minutes: get(32, 4) + 10 * get(40, 3),
        hours: get(48, 4) + 10 * get(56, 2),
    }
}

#[derive(Debug, Clone)]
pub struct LtcEncoder {
    level: f32,
    last: Option<(i64, usize, bool)>,
    bits: [bool; 80],
    amplitude: f32,
}

impl Default for LtcEncoder {
    fn default() -> Self {
        LtcEncoder { level: 1.0, last: None, bits: [false; 80], amplitude: 0.25 }
    }
}

impl LtcEncoder {
    pub fn render(&mut self, out: &mut [f32], start: Samples, sr: SampleRate, rate: FrameRate) {
        let spf = rate.samples_per_frame(sr).max(1.0);
        for (i, o) in out.iter_mut().enumerate() {
            let a = (start + i as i64) as f64;
            let frame = (a / spf).floor() as i64;
            let within = ((a - frame as f64 * spf) / spf * 80.0).clamp(0.0, 79.999);
            let bit = within as usize;
            let second_half = within - bit as f64 >= 0.5;
            match self.last {
                Some((f, b, h)) if f == frame && b == bit => {
                    if !h && second_half && self.bits[bit] {
                        self.level = -self.level;
                    }
                }
                _ => {
                    if self.last.is_none_or(|(f, _, _)| f != frame) {
                        self.bits = ltc_bits(Timecode::from_frames(frame, rate), rate);
                    }
                    self.level = -self.level;
                    if second_half && self.bits[bit] {
                        self.level = -self.level;
                    }
                }
            }
            self.last = Some((frame, bit, second_half));
            *o = self.level * self.amplitude;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LtcDecoder {
    sign: bool,
    since: f64,
    bit_len: f64,
    half_pending: bool,
    window: u128,
    count: u32,
}

impl LtcDecoder {
    pub fn feed(&mut self, samples: &[f32], rate: FrameRate, sr: SampleRate) -> Option<(Timecode, usize)> {
        let nominal = sr.as_f64() / (rate.rational().0 as f64 / rate.rational().1 as f64) / 80.0;
        if self.bit_len <= 0.0 {
            self.bit_len = nominal;
        }
        let mut found = None;
        for (i, x) in samples.iter().enumerate() {
            self.since += 1.0;
            let s = *x >= 0.0;
            if s == self.sign || x.abs() < 0.02 {
                continue;
            }
            self.sign = s;
            let gap = self.since;
            self.since = 0.0;
            if gap > self.bit_len * 1.6 || gap < self.bit_len * 0.25 {
                self.half_pending = false;
                self.count = 0;
                continue;
            }
            let bit = if gap < self.bit_len * 0.75 {
                if self.half_pending {
                    self.half_pending = false;
                    self.bit_len = self.bit_len * 0.95 + gap * 2.0 * 0.05;
                    Some(true)
                } else {
                    self.half_pending = true;
                    None
                }
            } else {
                self.half_pending = false;
                self.bit_len = self.bit_len * 0.95 + gap * 0.05;
                Some(false)
            };
            let Some(bit) = bit else { continue };
            self.window = (self.window >> 1) | (u128::from(bit) << 79);
            self.count = self.count.saturating_add(1);
            let sync = ((self.window >> 64) & 0xFFFF) as u16;
            if self.count >= 80 && sync == SYNC_WORD {
                let mut bits = [false; 80];
                for (k, b) in bits.iter_mut().enumerate() {
                    *b = (self.window >> k) & 1 == 1;
                }
                found = Some((ltc_decode(&bits, rate), i));
            }
        }
        found
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use ports::{MtcInput, SyncOutput, SyncSettings};

#[cfg(not(target_arch = "wasm32"))]
mod ports {
    use super::*;
    use crate::PlayClock;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    #[derive(Debug, Clone, PartialEq)]
    pub struct SyncSettings {
        pub tempo: TempoMap,
        pub sample_rate: SampleRate,
        pub frame_rate: FrameRate,
        pub timecode_start: Samples,
    }

    fn connect(name: &str, client: &str) -> Result<midir::MidiOutputConnection, String> {
        let mo = midir::MidiOutput::new(client).map_err(|e| e.to_string())?;
        let port =
            mo.ports().into_iter().find(|p| mo.port_name(p).is_ok_and(|n| n == name)).ok_or_else(|| format!("MIDI output “{name}” not found"))?;
        mo.connect(&port, client).map_err(|e| e.to_string())
    }

    struct Clock {
        out: midir::MidiOutputConnection,
        next: Option<i64>,
    }

    struct Mtc {
        out: midir::MidiOutputConnection,
        next: Option<i64>,
        rate: FrameRate,
    }

    pub struct SyncOutput {
        stop: Arc<AtomicBool>,
        settings: Arc<Mutex<SyncSettings>>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl SyncOutput {
        pub fn start(clock: PlayClock, settings: SyncSettings, clock_port: Option<&str>, mtc_port: Option<&str>) -> Result<SyncOutput, String> {
            let clock_out = clock_port.map(|n| connect(n, "OpenPT MIDI Clock")).transpose()?;
            let mtc_out = mtc_port.map(|n| connect(n, "OpenPT MTC")).transpose()?;
            let stop = Arc::new(AtomicBool::new(false));
            let settings = Arc::new(Mutex::new(settings));
            let (stop2, settings2) = (Arc::clone(&stop), Arc::clone(&settings));
            let handle = std::thread::Builder::new()
                .name("openpt-sync-out".into())
                .spawn(move || {
                    let mut clock_state = clock_out.map(|out| Clock { out, next: None });
                    let mut mtc_state = mtc_out.map(|out| Mtc { out, next: None, rate: FrameRate::Fps30 });
                    let mut was_playing = false;
                    let mut stopped_at = None;
                    while !stop2.load(Ordering::Relaxed) {
                        let s = settings2.lock().unwrap_or_else(PoisonError::into_inner).clone();
                        let (playing, pos) = clock.now();
                        if let Some(c) = &mut clock_state {
                            clock_tick(c, &s, playing, was_playing, pos);
                        }
                        if let Some(m) = &mut mtc_state {
                            mtc_tick(m, &s, playing, was_playing, pos, &mut stopped_at);
                        }
                        was_playing = playing;
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    if let Some(c) = &mut clock_state {
                        let _ = c.out.send(&[0xFC]);
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(SyncOutput { stop, settings, handle: Some(handle) })
        }

        pub fn update(&self, settings: SyncSettings) {
            let mut s = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
            if *s != settings {
                *s = settings;
            }
        }
    }

    impl Drop for SyncOutput {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }

    fn clock_tick(c: &mut Clock, s: &SyncSettings, playing: bool, was_playing: bool, pos: f64) {
        if !playing {
            if was_playing {
                let _ = c.out.send(&[0xFC]);
                let _ = c.out.send(&song_position(&s.tempo, pos.round() as Samples, s.sample_rate));
            }
            c.next = None;
            return;
        }
        let at = clock_position(&s.tempo, pos.max(0.0), s.sample_rate);
        let jumped = c.next.is_some_and(|n| (at - n as f64).abs() > 3.0);
        if c.next.is_none() || jumped {
            if jumped {
                let _ = c.out.send(&[0xFC]);
            }
            let spp = song_position(&s.tempo, pos.round() as Samples, s.sample_rate);
            let beats = u16::from(spp[1]) | (u16::from(spp[2]) << 7);
            let _ = c.out.send(&spp);
            let _ = c.out.send(if beats == 0 && pos < 1.0 { &[0xFA] } else { &[0xFB] });
            c.next = Some((f64::from(beats) * SPP_TICKS / CLOCK_TICKS).ceil() as i64);
        }
        while let Some(n) = c.next
            && (n as f64) <= at
        {
            let _ = c.out.send(&[0xF8]);
            c.next = Some(n + 1);
        }
    }

    fn mtc_tick(m: &mut Mtc, s: &SyncSettings, playing: bool, was_playing: bool, pos: f64, stopped_at: &mut Option<i64>) {
        m.rate = mtc_rate(s.frame_rate);
        let spf = m.rate.samples_per_frame(s.sample_rate).max(1.0);
        let quarters = (pos + s.timecode_start as f64) / spf * 4.0;
        if !playing {
            let frame = (quarters / 4.0).floor() as i64;
            if was_playing || *stopped_at != Some(frame) {
                let _ = m.out.send(&mtc_full_frame(Timecode::from_frames(frame.max(0), m.rate), m.rate));
                *stopped_at = Some(frame);
            }
            m.next = None;
            return;
        }
        *stopped_at = None;
        let jumped = m.next.is_some_and(|n| (quarters - n as f64).abs() > 8.0);
        if m.next.is_none() || jumped {
            let start = quarters.ceil() as i64;
            let start = start + (8 - start.rem_euclid(8)) % 8;
            let _ = m.out.send(&mtc_full_frame(Timecode::from_frames((start / 4).max(0), m.rate), m.rate));
            m.next = Some(start);
        }
        while let Some(q) = m.next
            && (q as f64) <= quarters
        {
            let piece = q.rem_euclid(8);
            let frame = (q - piece) / 4;
            let msg = mtc_quarter_frame(piece as u8, Timecode::from_frames(frame.max(0), m.rate), m.rate);
            let _ = m.out.send(&msg);
            m.next = Some(q + 1);
        }
    }

    pub struct MtcInput {
        _conn: midir::MidiInputConnection<()>,
        reading: Arc<Mutex<Option<TcReading>>>,
    }

    impl MtcInput {
        pub fn open(name: &str) -> Result<MtcInput, String> {
            let mut mi = midir::MidiInput::new("OpenPT MTC in").map_err(|e| e.to_string())?;
            mi.ignore(midir::Ignore::ActiveSense);
            let port =
                mi.ports().into_iter().find(|p| mi.port_name(p).is_ok_and(|n| n == name)).ok_or_else(|| format!("MIDI input “{name}” not found"))?;
            let reading = Arc::new(Mutex::new(None));
            let shared = Arc::clone(&reading);
            let mut reader = MtcReader::default();
            let conn = mi
                .connect(
                    &port,
                    "OpenPT MTC in",
                    move |_, bytes, _| {
                        let now = crate::midi::monotonic_now_us();
                        let got = if bytes.first() == Some(&0xF0) { reader.feed_sysex(bytes, now) } else { reader.feed(bytes, now) };
                        if let Some(r) = got {
                            *shared.lock().unwrap_or_else(PoisonError::into_inner) = Some(r);
                        } else if bytes.first() == Some(&0xF1)
                            && let Some(r) = shared.lock().unwrap_or_else(PoisonError::into_inner).as_mut()
                        {
                            r.running = true;
                        }
                    },
                    (),
                )
                .map_err(|e| e.to_string())?;
            Ok(MtcInput { _conn: conn, reading })
        }

        pub fn reading(&self) -> Option<TcReading> {
            *self.reading.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarter_frames_carry_the_timecode_and_rate() {
        let tc = Timecode { negative: false, hours: 1, minutes: 2, seconds: 3, frames: 20 };
        let msgs: Vec<[u8; 2]> = (0..8).map(|p| mtc_quarter_frame(p, tc, FrameRate::Fps25)).collect();
        assert_eq!(msgs[0], [0xF1, 0x04]);
        assert_eq!(msgs[1], [0xF1, 0x11]);
        assert_eq!(msgs[2], [0xF1, 0x23]);
        assert_eq!(msgs[7], [0xF1, 0x72]);
        let mut r = MtcReader::default();
        let mut got = None;
        for (k, m) in msgs.iter().enumerate() {
            got = r.feed(m, k as u64);
        }
        let got = got.unwrap();
        assert_eq!(got.rate, FrameRate::Fps25);
        assert_eq!(got.frames, tc.to_frames(FrameRate::Fps25) + 2);
        assert!(got.running);
    }

    #[test]
    fn a_full_frame_message_locates_the_reader() {
        let tc = Timecode { negative: false, hours: 10, minutes: 0, seconds: 0, frames: 0 };
        let msg = mtc_full_frame(tc, FrameRate::Fps29_97Drop);
        assert_eq!(msg, vec![0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x4A, 0, 0, 0, 0xF7]);
        let got = MtcReader::default().feed_sysex(&msg, 5).unwrap();
        assert_eq!((got.rate, got.frames, got.running), (FrameRate::Fps29_97Drop, tc.to_frames(FrameRate::Fps29_97Drop), false));
    }

    #[test]
    fn out_of_order_pieces_do_not_produce_a_reading() {
        let tc = Timecode { negative: false, hours: 0, minutes: 0, seconds: 1, frames: 0 };
        let mut r = MtcReader::default();
        for p in [0, 1, 2, 3, 5, 6, 7] {
            assert_eq!(r.feed(&mtc_quarter_frame(p, tc, FrameRate::Fps30), 0), None);
        }
    }

    #[test]
    fn session_rates_map_to_the_four_mtc_rates() {
        assert_eq!(mtc_rate_code(FrameRate::Fps24), 0);
        assert_eq!(mtc_rate_code(FrameRate::Fps23_976), 0);
        assert_eq!(mtc_rate(FrameRate::Fps23_976), FrameRate::Fps23_976);
        assert_eq!(mtc_rate_code(FrameRate::Fps50), 1);
        assert_eq!(mtc_rate_code(FrameRate::Fps29_97Drop), 2);
        assert_eq!(mtc_rate_code(FrameRate::Fps60), 3);
    }

    #[test]
    fn clock_and_song_position_follow_the_tempo_map() {
        let map = TempoMap::new(120.0, 4, 4).unwrap();
        let sr = SampleRate::HZ_48000;
        assert!((clock_position(&map, 24_000.0, sr) - 24.0).abs() < 1e-9, "one beat at 120 bpm is 24 clocks");
        assert_eq!(song_position(&map, 48_000, sr), [0xF2, 8, 0], "two beats are eight sixteenths");
        assert_eq!(song_position(&map, 0, sr), [0xF2, 0, 0]);
    }

    #[test]
    fn ltc_frames_carry_bcd_time_the_sync_word_and_even_parity() {
        let tc = Timecode { negative: false, hours: 23, minutes: 59, seconds: 58, frames: 29 };
        let bits = ltc_bits(tc, FrameRate::Fps30);
        assert_eq!(ltc_decode(&bits, FrameRate::Fps30), tc);
        assert_eq!(bits.iter().filter(|b| **b).count() % 2, 0);
        let sync: Vec<bool> = bits[64..].to_vec();
        assert_eq!(sync, [false, false, true, true, true, true, true, true, true, true, true, true, true, true, false, true]);
        assert!(ltc_bits(tc, FrameRate::Fps29_97Drop)[10]);
    }

    #[test]
    fn generated_ltc_decodes_back_to_the_same_timecode() {
        let sr = SampleRate::HZ_48000;
        for rate in [FrameRate::Fps24, FrameRate::Fps25, FrameRate::Fps29_97Drop, FrameRate::Fps30] {
            let start_frame = Timecode { negative: false, hours: 1, minutes: 0, seconds: 10, frames: 5 }.to_frames(rate);
            let start = rate.samples_at_frame(start_frame, sr);
            let mut enc = LtcEncoder::default();
            let mut dec = LtcDecoder::default();
            let mut decoded = Vec::new();
            let mut at = start;
            for _ in 0..40 {
                let mut buf = vec![0.0f32; 480];
                enc.render(&mut buf, at, sr, rate);
                if let Some((tc, _)) = dec.feed(&buf, rate, sr) {
                    decoded.push(tc.to_frames(rate));
                }
                at += 480;
            }
            assert!(decoded.len() >= 8, "{rate:?}: {decoded:?}");
            for w in decoded.windows(2) {
                assert!(w[1] > w[0] && w[1] - w[0] <= 2, "{rate:?}: frames go forward one by one: {decoded:?}");
            }
            assert!(decoded[0] >= start_frame && decoded[0] <= start_frame + 3, "{rate:?}: {} vs {start_frame}", decoded[0]);
        }
    }

    #[test]
    fn a_running_reading_advances_with_time() {
        let sr = SampleRate::HZ_48000;
        let r = TcReading { frames: 25, rate: FrameRate::Fps25, at_us: 1_000_000, running: true };
        assert!((r.samples_at(1_000_000, sr) - 48_000.0).abs() < 1e-6);
        assert!((r.samples_at(1_500_000, sr) - 72_000.0).abs() < 1e-6);
        let held = TcReading { running: false, ..r };
        assert!((held.samples_at(9_000_000, sr) - 48_000.0).abs() < 1e-6);
    }
}
