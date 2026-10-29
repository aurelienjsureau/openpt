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

#[cfg(not(target_arch = "wasm32"))]
pub struct HuiPort {
    _input: midir::MidiInputConnection<()>,
    output: midir::MidiOutputConnection,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
    decoder: HuiDecoder,
}

#[cfg(not(target_arch = "wasm32"))]
impl HuiPort {
    pub fn open(input: &str, output: &str) -> Result<HuiPort, String> {
        let mut mi = midir::MidiInput::new("OpenPT HUI").map_err(|e| e.to_string())?;
        mi.ignore(midir::Ignore::None);
        let ip =
            mi.ports().into_iter().find(|p| mi.port_name(p).is_ok_and(|n| n == input)).ok_or_else(|| format!("MIDI input “{input}” not found"))?;
        let (tx, rx) = std::sync::mpsc::channel();
        let input_conn = mi
            .connect(
                &ip,
                "OpenPT HUI in",
                move |_, bytes, _| {
                    let _ = tx.send(bytes.to_vec());
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        let mo = midir::MidiOutput::new("OpenPT HUI").map_err(|e| e.to_string())?;
        let op =
            mo.ports().into_iter().find(|p| mo.port_name(p).is_ok_and(|n| n == output)).ok_or_else(|| format!("MIDI output “{output}” not found"))?;
        let output = mo.connect(&op, "OpenPT HUI out").map_err(|e| e.to_string())?;
        Ok(HuiPort { _input: input_conn, output, rx, decoder: HuiDecoder::default() })
    }

    pub fn poll(&mut self) -> Vec<SurfaceEvent> {
        let mut out = Vec::new();
        while let Ok(bytes) = self.rx.try_recv() {
            let mut status = 0u8;
            let mut i = 0;
            while i < bytes.len() {
                let b = bytes.get(i).copied().unwrap_or(0);
                if b == 0xF0 {
                    i = bytes.iter().skip(i).position(|x| *x == 0xF7).map_or(bytes.len(), |p| i + p + 1);
                    continue;
                }
                let (msg, next) = if b & 0x80 != 0 {
                    status = b;
                    (vec![b, bytes.get(i + 1).copied().unwrap_or(0), bytes.get(i + 2).copied().unwrap_or(0)], i + 3)
                } else {
                    (vec![status, b, bytes.get(i + 1).copied().unwrap_or(0)], i + 2)
                };
                if let Some(ev) = self.decoder.feed(&msg) {
                    out.push(ev);
                }
                i = next;
            }
        }
        out
    }

    pub fn send(&mut self, msg: &[u8]) {
        let _ = self.output.send(msg);
    }
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
}
