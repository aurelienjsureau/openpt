#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SurfaceEvent {
    Fader(u8, f32),
    Touch(u8, bool),
    Strip(u8, StripButton),
    Pot(u8, i8),
    Transport(TransportButton),
    Bank(BankButton),
    Alive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripButton {
    Select,
    Mute,
    Solo,
    Auto,
    VSelect,
    Insert,
    RecReady,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportButton {
    Rewind,
    FastForward,
    Stop,
    Play,
    Record,
    Loop,
    ReturnToZero,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankButton {
    ChannelLeft,
    BankLeft,
    ChannelRight,
    BankRight,
}

const STRIP_PORTS: [Option<StripButton>; 8] = [
    None,
    Some(StripButton::Select),
    Some(StripButton::Mute),
    Some(StripButton::Solo),
    Some(StripButton::Auto),
    Some(StripButton::VSelect),
    Some(StripButton::Insert),
    Some(StripButton::RecReady),
];

fn strip_port(b: StripButton) -> u8 {
    STRIP_PORTS.iter().position(|p| *p == Some(b)).unwrap_or(0) as u8
}

const ZONE_BANK: u8 = 0x0A;
const ZONE_TRANSPORT: u8 = 0x0E;
const ZONE_TRANSPORT_2: u8 = 0x0F;

fn transport_of(zone: u8, port: u8) -> Option<TransportButton> {
    use TransportButton as T;
    match (zone, port) {
        (ZONE_TRANSPORT, 1) => Some(T::Rewind),
        (ZONE_TRANSPORT, 2) => Some(T::FastForward),
        (ZONE_TRANSPORT, 3) => Some(T::Stop),
        (ZONE_TRANSPORT, 4) => Some(T::Play),
        (ZONE_TRANSPORT, 5) => Some(T::Record),
        (ZONE_TRANSPORT_2, 0) => Some(T::ReturnToZero),
        (ZONE_TRANSPORT_2, 1) => Some(T::End),
        (ZONE_TRANSPORT_2, 3) => Some(T::Loop),
        _ => None,
    }
}

fn transport_port(b: TransportButton) -> (u8, u8) {
    use TransportButton as T;
    match b {
        T::Rewind => (ZONE_TRANSPORT, 1),
        T::FastForward => (ZONE_TRANSPORT, 2),
        T::Stop => (ZONE_TRANSPORT, 3),
        T::Play => (ZONE_TRANSPORT, 4),
        T::Record => (ZONE_TRANSPORT, 5),
        T::ReturnToZero => (ZONE_TRANSPORT_2, 0),
        T::End => (ZONE_TRANSPORT_2, 1),
        T::Loop => (ZONE_TRANSPORT_2, 3),
    }
}

#[derive(Debug, Default, Clone)]
pub struct HuiDecoder {
    zone: u8,
    fader_hi: [u8; 8],
}

impl HuiDecoder {
    pub fn feed(&mut self, msg: &[u8]) -> Option<SurfaceEvent> {
        match *msg {
            [0x90, 0x00, 0x7F] => Some(SurfaceEvent::Alive),
            [0xB0, cc, v] if cc < 8 => {
                if let Some(h) = self.fader_hi.get_mut(usize::from(cc)) {
                    *h = v & 0x7F;
                }
                None
            }
            [0xB0, cc, v] if (0x20..0x28).contains(&cc) => {
                let n = cc - 0x20;
                let hi = self.fader_hi.get(usize::from(n)).copied().unwrap_or(0);
                let value = (u16::from(hi) << 7) | u16::from(v & 0x7F);
                Some(SurfaceEvent::Fader(n, f32::from(value) / 16383.0))
            }
            [0xB0, 0x0F, z] => {
                self.zone = z & 0x7F;
                None
            }
            [0xB0, 0x2F, p] => {
                let (port, down) = (p & 0x0F, p & 0x40 != 0);
                let zone = self.zone;
                if zone < 8 {
                    if port == 0 {
                        return Some(SurfaceEvent::Touch(zone, down));
                    }
                    return down.then(|| STRIP_PORTS.get(usize::from(port)).copied().flatten().map(|b| SurfaceEvent::Strip(zone, b))).flatten();
                }
                if !down {
                    return None;
                }
                if zone == ZONE_BANK {
                    use BankButton as B;
                    return [B::ChannelLeft, B::BankLeft, B::ChannelRight, B::BankRight].get(usize::from(port)).map(|b| SurfaceEvent::Bank(*b));
                }
                transport_of(zone, port).map(SurfaceEvent::Transport)
            }
            [0xB0, cc, v] if (0x40..0x48).contains(&cc) => {
                let steps = (v & 0x3F) as i8;
                Some(SurfaceEvent::Pot(cc - 0x40, if v & 0x40 != 0 { -steps } else { steps }))
            }
            _ => None,
        }
    }
}

pub fn ping() -> [u8; 3] {
    [0x90, 0x00, 0x00]
}

pub fn fader(n: u8, pos: f32) -> [[u8; 3]; 2] {
    let v = (pos.clamp(0.0, 1.0) * 16383.0).round() as u16;
    [[0xB0, n & 7, (v >> 7) as u8 & 0x7F], [0xB0, 0x20 + (n & 7), v as u8 & 0x7F]]
}

fn led(zone: u8, port: u8, on: bool) -> [[u8; 3]; 2] {
    [[0xB0, 0x0C, zone & 0x7F], [0xB0, 0x2C, (port & 0x0F) | if on { 0x40 } else { 0 }]]
}

pub fn strip_led(strip: u8, b: StripButton, on: bool) -> [[u8; 3]; 2] {
    led(strip & 7, strip_port(b), on)
}

pub fn transport_led(b: TransportButton, on: bool) -> [[u8; 3]; 2] {
    let (z, p) = transport_port(b);
    led(z, p, on)
}

pub fn strip_name(strip: u8, name: &str) -> Vec<u8> {
    let mut v = vec![0xF0, 0x00, 0x00, 0x66, 0x05, 0x00, 0x10, strip & 7];
    let mut chars: Vec<u8> = name.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).map(|c| c as u8).take(4).collect();
    chars.resize(4, b' ');
    v.extend(chars);
    v.push(0xF7);
    v
}

#[derive(Debug, Default, Clone)]
pub struct McuDecoder;

const MCU_TOUCH: u8 = 0x68;
const MCU_BANK_LEFT: u8 = 0x2E;
const MCU_BANK_RIGHT: u8 = 0x2F;
const MCU_CHANNEL_LEFT: u8 = 0x30;
const MCU_CHANNEL_RIGHT: u8 = 0x31;

fn mcu_strip_base(b: StripButton) -> Option<u8> {
    match b {
        StripButton::RecReady => Some(0x00),
        StripButton::Solo => Some(0x08),
        StripButton::Mute => Some(0x10),
        StripButton::Select => Some(0x18),
        StripButton::VSelect => Some(0x20),
        StripButton::Auto | StripButton::Insert => None,
    }
}

fn mcu_transport_note(b: TransportButton) -> Option<u8> {
    use TransportButton as T;
    match b {
        T::Loop => Some(0x56),
        T::Rewind => Some(0x5B),
        T::FastForward => Some(0x5C),
        T::Stop => Some(0x5D),
        T::Play => Some(0x5E),
        T::Record => Some(0x5F),
        T::ReturnToZero | T::End => None,
    }
}

impl McuDecoder {
    pub fn feed(&mut self, msg: &[u8]) -> Option<SurfaceEvent> {
        match *msg {
            [s, lsb, msb] if s & 0xF0 == 0xE0 => {
                let ch = s & 0x0F;
                let value = (u16::from(msb & 0x7F) << 7) | u16::from(lsb & 0x7F);
                (ch < 8).then(|| SurfaceEvent::Fader(ch, f32::from(value) / 16383.0))
            }
            [s, note, vel] if matches!(s & 0xF0, 0x80 | 0x90) => {
                let down = s & 0xF0 == 0x90 && vel > 0;
                if (MCU_TOUCH..MCU_TOUCH + 8).contains(&note) {
                    return Some(SurfaceEvent::Touch(note - MCU_TOUCH, down));
                }
                if !down {
                    return None;
                }
                use BankButton as B;
                use StripButton as S;
                let strip = |b: S| Some(SurfaceEvent::Strip(note & 7, b));
                match note {
                    0x00..=0x07 => strip(S::RecReady),
                    0x08..=0x0F => strip(S::Solo),
                    0x10..=0x17 => strip(S::Mute),
                    0x18..=0x1F => strip(S::Select),
                    0x20..=0x27 => strip(S::VSelect),
                    MCU_BANK_LEFT => Some(SurfaceEvent::Bank(B::BankLeft)),
                    MCU_BANK_RIGHT => Some(SurfaceEvent::Bank(B::BankRight)),
                    MCU_CHANNEL_LEFT => Some(SurfaceEvent::Bank(B::ChannelLeft)),
                    MCU_CHANNEL_RIGHT => Some(SurfaceEvent::Bank(B::ChannelRight)),
                    _ => {
                        use TransportButton as T;
                        [T::Loop, T::Rewind, T::FastForward, T::Stop, T::Play, T::Record]
                            .into_iter()
                            .find(|b| mcu_transport_note(*b) == Some(note))
                            .map(SurfaceEvent::Transport)
                    }
                }
            }
            [0xB0, cc, v] if (0x10..0x18).contains(&cc) => {
                let steps = (v & 0x3F) as i8;
                Some(SurfaceEvent::Pot(cc - 0x10, if v & 0x40 != 0 { -steps } else { steps }))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Hui,
    Mcu,
    McuExtender,
}

pub const MCU_STRIP_CHARS: usize = 7;

fn mcu_lcd(device: u8, offset: usize, text: &str, width: usize) -> Vec<u8> {
    let mut v = vec![0xF0, 0x00, 0x00, 0x66, device, 0x12, (offset & 0x7F) as u8];
    let mut chars: Vec<u8> = text.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).map(|c| c as u8).take(width.saturating_sub(1)).collect();
    chars.resize(width, b' ');
    v.extend(chars);
    v.push(0xF7);
    v
}

fn seven_segment(c: char) -> u8 {
    let b = if c.is_ascii() { c as u8 } else { b' ' };
    if b >= 0x40 { b - 0x40 } else { b }
}

impl Protocol {
    pub const KINDS: [&'static str; 3] = ["HUI", "Mackie Control", "Mackie Control XT"];

    pub fn from_kind(kind: &str) -> Option<Protocol> {
        match kind {
            "HUI" => Some(Protocol::Hui),
            "Mackie Control" => Some(Protocol::Mcu),
            "Mackie Control XT" => Some(Protocol::McuExtender),
            _ => None,
        }
    }

    fn device_id(self) -> u8 {
        if self == Protocol::McuExtender { 0x15 } else { 0x14 }
    }

    pub fn has_transport(self) -> bool {
        self != Protocol::McuExtender
    }

    pub fn ping(self) -> Option<Vec<u8>> {
        (self == Protocol::Hui).then(|| ping().to_vec())
    }

    pub fn fader(self, n: u8, pos: f32) -> Vec<Vec<u8>> {
        match self {
            Protocol::Hui => fader(n, pos).iter().map(|m| m.to_vec()).collect(),
            _ => {
                let v = (pos.clamp(0.0, 1.0) * 16383.0).round() as u16;
                vec![vec![0xE0 | (n & 7), v as u8 & 0x7F, (v >> 7) as u8 & 0x7F]]
            }
        }
    }

    pub fn strip_led(self, n: u8, b: StripButton, on: bool) -> Vec<Vec<u8>> {
        match self {
            Protocol::Hui => strip_led(n, b, on).iter().map(|m| m.to_vec()).collect(),
            _ => mcu_strip_base(b).map(|base| vec![vec![0x90, base + (n & 7), if on { 0x7F } else { 0x00 }]]).unwrap_or_default(),
        }
    }

    pub fn transport_led(self, b: TransportButton, on: bool) -> Vec<Vec<u8>> {
        match self {
            Protocol::Hui => transport_led(b, on).iter().map(|m| m.to_vec()).collect(),
            Protocol::Mcu => mcu_transport_note(b).map(|note| vec![vec![0x90, note, if on { 0x7F } else { 0x00 }]]).unwrap_or_default(),
            Protocol::McuExtender => Vec::new(),
        }
    }

    pub fn strip_name(self, n: u8, name: &str) -> Vec<Vec<u8>> {
        match self {
            Protocol::Hui => vec![strip_name(n, name)],
            _ => vec![mcu_lcd(self.device_id(), usize::from(n & 7) * MCU_STRIP_CHARS, name, MCU_STRIP_CHARS)],
        }
    }

    pub fn strip_value(self, n: u8, text: &str) -> Vec<Vec<u8>> {
        match self {
            Protocol::Hui => Vec::new(),
            _ => vec![mcu_lcd(self.device_id(), 56 + usize::from(n & 7) * MCU_STRIP_CHARS, text, MCU_STRIP_CHARS)],
        }
    }

    pub fn pan(self, n: u8, pan: f32) -> Vec<Vec<u8>> {
        match self {
            Protocol::Hui => Vec::new(),
            _ => {
                let pos = 1 + ((pan.clamp(-1.0, 1.0) + 1.0) * 5.0).round() as u8;
                vec![vec![0xB0, 0x30 + (n & 7), pos.min(11)]]
            }
        }
    }

    pub fn meter(self, n: u8, peak: f32) -> Vec<Vec<u8>> {
        match self {
            Protocol::Hui => Vec::new(),
            _ => {
                let db = if peak > 1e-6 { 20.0 * peak.log10() } else { -120.0 };
                let level = if db >= 0.0 { 0x0E } else { (((db + 60.0) / 60.0) * 12.0).clamp(0.0, 12.0) as u8 };
                vec![vec![0xD0, ((n & 7) << 4) | level]]
            }
        }
    }

    pub fn timecode(self, text: &str) -> Vec<Vec<u8>> {
        if self != Protocol::Mcu {
            return Vec::new();
        }
        let digits: Vec<char> = text.chars().filter(|c| c.is_ascii_alphanumeric() || *c == ' ').collect();
        (0..10u8)
            .map(|i| {
                let c = digits.len().checked_sub(usize::from(i) + 1).and_then(|k| digits.get(k)).copied().unwrap_or(' ');
                vec![0xB0, 0x40 + i, seven_segment(c)]
            })
            .collect()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct SurfacePort {
    pub protocol: Protocol,
    _input: midir::MidiInputConnection<()>,
    output: midir::MidiOutputConnection,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    hui: HuiDecoder,
    mcu: McuDecoder,
}

#[cfg(not(target_arch = "wasm32"))]
impl SurfacePort {
    pub fn open(protocol: Protocol, input: &str, output: &str) -> Result<SurfacePort, String> {
        let mut mi = midir::MidiInput::new("OpenPT Surface").map_err(|e| e.to_string())?;
        mi.ignore(midir::Ignore::None);
        let ip =
            mi.ports().into_iter().find(|p| mi.port_name(p).is_ok_and(|n| n == input)).ok_or_else(|| format!("MIDI input “{input}” not found"))?;
        let (tx, rx) = std::sync::mpsc::channel();
        let input_conn = mi
            .connect(
                &ip,
                "OpenPT Surface in",
                move |_, bytes, _| {
                    let _ = tx.send(bytes.to_vec());
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        let mo = midir::MidiOutput::new("OpenPT Surface").map_err(|e| e.to_string())?;
        let op =
            mo.ports().into_iter().find(|p| mo.port_name(p).is_ok_and(|n| n == output)).ok_or_else(|| format!("MIDI output “{output}” not found"))?;
        let output = mo.connect(&op, "OpenPT Surface out").map_err(|e| e.to_string())?;
        Ok(SurfacePort { protocol, _input: input_conn, output, rx, hui: HuiDecoder::default(), mcu: McuDecoder })
    }

    pub fn poll(&mut self) -> Vec<SurfaceEvent> {
        let mut out = Vec::new();
        while let Ok(bytes) = self.rx.try_recv() {
            for msg in split_messages(&bytes) {
                let ev = match self.protocol {
                    Protocol::Hui => self.hui.feed(&msg),
                    _ => self.mcu.feed(&msg),
                };
                out.extend(ev);
            }
        }
        out
    }

    pub fn send(&mut self, msg: &[u8]) {
        let _ = self.output.send(msg);
    }
}

pub fn split_messages(bytes: &[u8]) -> Vec<[u8; 3]> {
    let mut out = Vec::new();
    let mut status = 0u8;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes.get(i).copied().unwrap_or(0);
        if b == 0xF0 {
            i = bytes.iter().skip(i).position(|x| *x == 0xF7).map_or(bytes.len(), |p| i + p + 1);
            continue;
        }
        if b >= 0xF8 {
            i += 1;
            continue;
        }
        if b & 0x80 != 0 {
            status = b;
            i += 1;
            if status >= 0xF4 {
                out.push([status, 0, 0]);
                status = 0;
                continue;
            }
        }
        if status == 0 {
            i += 1;
            continue;
        }
        let len = if matches!(status & 0xF0, 0xC0 | 0xD0) || matches!(status, 0xF1 | 0xF3) { 1 } else { 2 };
        out.push([status, bytes.get(i).copied().unwrap_or(0), if len == 2 { bytes.get(i + 1).copied().unwrap_or(0) } else { 0 }]);
        i += len;
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
pub struct LearnPort {
    _input: midir::MidiInputConnection<()>,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl LearnPort {
    pub fn open(input: &str) -> Result<LearnPort, String> {
        let mut mi = midir::MidiInput::new("OpenPT MIDI Learn").map_err(|e| e.to_string())?;
        mi.ignore(midir::Ignore::All);
        let ip =
            mi.ports().into_iter().find(|p| mi.port_name(p).is_ok_and(|n| n == input)).ok_or_else(|| format!("MIDI input “{input}” not found"))?;
        let (tx, rx) = std::sync::mpsc::channel();
        let conn = mi
            .connect(
                &ip,
                "OpenPT MIDI Learn in",
                move |_, bytes, _| {
                    let _ = tx.send(bytes.to_vec());
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        Ok(LearnPort { _input: conn, rx })
    }

    pub fn poll(&mut self) -> Vec<[u8; 3]> {
        let mut out = Vec::new();
        while let Ok(bytes) = self.rx.try_recv() {
            let mut status = 0u8;
            let mut i = 0;
            while i < bytes.len() {
                let b = bytes.get(i).copied().unwrap_or(0);
                if b >= 0xF0 {
                    i += 1;
                    continue;
                }
                if b & 0x80 != 0 {
                    status = b;
                    i += 1;
                }
                let len = if matches!(status & 0xF0, 0xC0 | 0xD0) { 1 } else { 2 };
                if status != 0 {
                    out.push([status, bytes.get(i).copied().unwrap_or(0), if len == 2 { bytes.get(i + 1).copied().unwrap_or(0) } else { 0 }]);
                }
                i += len;
            }
        }
        out
    }
}

pub fn list_output_devices() -> Vec<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let Ok(mo) = midir::MidiOutput::new("OpenPT") else { return Vec::new() };
        mo.ports().iter().filter_map(|p| mo.port_name(p).ok()).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_faders_switches_pots_and_the_ping_answer() {
        let mut d = HuiDecoder::default();
        assert_eq!(d.feed(&[0x90, 0, 0x7F]), Some(SurfaceEvent::Alive));
        assert_eq!(d.feed(&[0xB0, 0x02, 0x7F]), None);
        assert_eq!(d.feed(&[0xB0, 0x22, 0x7F]), Some(SurfaceEvent::Fader(2, 1.0)));
        d.feed(&[0xB0, 0x0F, 0x03]);
        assert_eq!(d.feed(&[0xB0, 0x2F, 0x42]), Some(SurfaceEvent::Strip(3, StripButton::Mute)));
        assert_eq!(d.feed(&[0xB0, 0x2F, 0x02]), None, "releases do nothing");
        assert_eq!(d.feed(&[0xB0, 0x2F, 0x40]), Some(SurfaceEvent::Touch(3, true)));
        d.feed(&[0xB0, 0x0F, ZONE_TRANSPORT]);
        assert_eq!(d.feed(&[0xB0, 0x2F, 0x44]), Some(SurfaceEvent::Transport(TransportButton::Play)));
        d.feed(&[0xB0, 0x0F, ZONE_BANK]);
        assert_eq!(d.feed(&[0xB0, 0x2F, 0x43]), Some(SurfaceEvent::Bank(BankButton::BankRight)));
        assert_eq!(d.feed(&[0xB0, 0x41, 0x42]), Some(SurfaceEvent::Pot(1, -2)));
    }

    #[test]
    fn encodes_what_the_surface_shows() {
        assert_eq!(fader(1, 1.0), [[0xB0, 1, 0x7F], [0xB0, 0x21, 0x7F]]);
        assert_eq!(strip_led(2, StripButton::Solo, true), [[0xB0, 0x0C, 2], [0xB0, 0x2C, 0x43]]);
        assert_eq!(transport_led(TransportButton::Record, false), [[0xB0, 0x0C, ZONE_TRANSPORT], [0xB0, 0x2C, 5]]);
        assert_eq!(strip_name(0, "Kick drum"), vec![0xF0, 0, 0, 0x66, 5, 0, 0x10, 0, b'K', b'i', b'c', b'k', 0xF7]);
        assert_eq!(strip_name(7, "Hi"), vec![0xF0, 0, 0, 0x66, 5, 0, 0x10, 7, b'H', b'i', b' ', b' ', 0xF7]);
    }

    #[test]
    fn mackie_control_decodes_faders_touch_buttons_and_pots() {
        let mut d = McuDecoder;
        assert_eq!(d.feed(&[0xE3, 0x7F, 0x7F]), Some(SurfaceEvent::Fader(3, 1.0)));
        assert_eq!(d.feed(&[0xE8, 0x00, 0x40]), None, "the master fader is not a strip");
        assert_eq!(d.feed(&[0x90, 0x69, 0x7F]), Some(SurfaceEvent::Touch(1, true)));
        assert_eq!(d.feed(&[0x90, 0x69, 0x00]), Some(SurfaceEvent::Touch(1, false)));
        assert_eq!(d.feed(&[0x90, 0x12, 0x7F]), Some(SurfaceEvent::Strip(2, StripButton::Mute)));
        assert_eq!(d.feed(&[0x90, 0x12, 0x00]), None, "releases do nothing");
        assert_eq!(d.feed(&[0x90, 0x0F, 0x7F]), Some(SurfaceEvent::Strip(7, StripButton::Solo)));
        assert_eq!(d.feed(&[0x90, 0x03, 0x7F]), Some(SurfaceEvent::Strip(3, StripButton::RecReady)));
        assert_eq!(d.feed(&[0x90, 0x18, 0x7F]), Some(SurfaceEvent::Strip(0, StripButton::Select)));
        assert_eq!(d.feed(&[0x90, 0x5E, 0x7F]), Some(SurfaceEvent::Transport(TransportButton::Play)));
        assert_eq!(d.feed(&[0x90, 0x56, 0x7F]), Some(SurfaceEvent::Transport(TransportButton::Loop)));
        assert_eq!(d.feed(&[0x90, 0x2F, 0x7F]), Some(SurfaceEvent::Bank(BankButton::BankRight)));
        assert_eq!(d.feed(&[0x90, 0x30, 0x7F]), Some(SurfaceEvent::Bank(BankButton::ChannelLeft)));
        assert_eq!(d.feed(&[0xB0, 0x12, 0x43]), Some(SurfaceEvent::Pot(2, -3)));
        assert_eq!(d.feed(&[0xB0, 0x10, 0x01]), Some(SurfaceEvent::Pot(0, 1)));
    }

    #[test]
    fn mackie_control_encodes_faders_leds_and_the_display() {
        let m = Protocol::Mcu;
        assert_eq!(m.fader(2, 1.0), vec![vec![0xE2, 0x7F, 0x7F]]);
        assert_eq!(m.fader(0, 0.0), vec![vec![0xE0, 0, 0]]);
        assert_eq!(m.strip_led(4, StripButton::Solo, true), vec![vec![0x90, 0x0C, 0x7F]]);
        assert!(m.strip_led(4, StripButton::Insert, true).is_empty());
        assert_eq!(m.transport_led(TransportButton::Record, false), vec![vec![0x90, 0x5F, 0]]);
        assert!(Protocol::McuExtender.transport_led(TransportButton::Play, true).is_empty());
        let name = m.strip_name(1, "Kick drum");
        assert_eq!(name[0], [vec![0xF0, 0, 0, 0x66, 0x14, 0x12, 7], b"Kick d ".to_vec(), vec![0xF7]].concat());
        assert_eq!(Protocol::McuExtender.strip_value(0, "-6.0")[0][..7], [0xF0, 0, 0, 0x66, 0x15, 0x12, 56]);
        assert_eq!(m.pan(0, 0.0), vec![vec![0xB0, 0x30, 6]]);
        assert_eq!(m.pan(3, -1.0), vec![vec![0xB0, 0x33, 1]]);
        assert_eq!(m.pan(3, 1.0), vec![vec![0xB0, 0x33, 11]]);
        assert_eq!(m.meter(5, 1.0), vec![vec![0xD0, 0x5E]]);
        assert_eq!(m.meter(5, 0.0), vec![vec![0xD0, 0x50]]);
        let tc = m.timecode("  1020304 ");
        assert_eq!(tc.len(), 10);
        assert_eq!(tc[0], vec![0xB0, 0x40, b' ']);
        assert_eq!(tc[1], vec![0xB0, 0x41, b'4']);
        assert_eq!(tc[7], vec![0xB0, 0x47, b'1']);
        assert_eq!(tc[8], vec![0xB0, 0x48, b' ']);
        assert_eq!(m.timecode("A")[0], vec![0xB0, 0x40, 0x01]);
        assert!(Protocol::Hui.timecode("01:00:00:00").is_empty());
        assert_eq!(Protocol::Hui.ping(), Some(vec![0x90, 0, 0]));
        assert_eq!(Protocol::Mcu.ping(), None);
    }

    #[test]
    fn running_status_and_realtime_bytes_split_into_messages() {
        assert_eq!(split_messages(&[0x90, 0x10, 0x7F, 0x11, 0x7F, 0xF8, 0xE0, 1, 2]), vec![[0x90, 0x10, 0x7F], [0x90, 0x11, 0x7F], [0xE0, 1, 2]]);
        assert_eq!(split_messages(&[0xF0, 0, 0x66, 0xF7, 0xB0, 0x10, 0x41]), vec![[0xB0, 0x10, 0x41]]);
        assert_eq!(split_messages(&[0xF1, 0x23, 0xD0, 0x45]), vec![[0xF1, 0x23, 0], [0xD0, 0x45, 0]]);
    }
}
