#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LiveMidiEvent {
    pub port_id: u32,
    pub timestamp_us: u64,
    pub len: u8,
    pub data: [u8; 3],
}

impl LiveMidiEvent {
    pub fn note_on(channel: u8, pitch: u8, velocity: u8) -> Self {
        Self { port_id: 0, timestamp_us: 0, len: 3, data: [0x90 | (channel & 0x0F), pitch & 0x7F, velocity & 0x7F] }
    }

    pub fn note_off(channel: u8, pitch: u8, release_velocity: u8) -> Self {
        Self { port_id: 0, timestamp_us: 0, len: 3, data: [0x80 | (channel & 0x0F), pitch & 0x7F, release_velocity & 0x7F] }
    }

    pub fn all_notes_off(channel: u8) -> Self {
        Self { port_id: 0, timestamp_us: 0, len: 3, data: [0xB0 | (channel & 0x0F), 123, 0] }
    }

    pub fn all_sound_off(channel: u8) -> Self {
        Self { port_id: 0, timestamp_us: 0, len: 3, data: [0xB0 | (channel & 0x0F), 120, 0] }
    }

    pub fn is_note_on(&self) -> bool {
        self.len == 3 && (self.data[0] & 0xF0 == 0x90) && self.data[2] > 0
    }

    pub fn is_note_off(&self) -> bool {
        self.len == 3 && ((self.data[0] & 0xF0 == 0x80) || ((self.data[0] & 0xF0 == 0x90) && self.data[2] == 0))
    }

    pub fn note_pitch(&self) -> u8 {
        self.data[1] & 0x7F
    }

    pub fn note_velocity(&self) -> u8 {
        self.data[2] & 0x7F
    }

    pub fn channel(&self) -> u8 {
        self.data[0] & 0x0F
    }

    pub fn is_control_change(&self) -> bool {
        self.len == 3 && (self.data[0] & 0xF0 == 0xB0)
    }

    pub fn cc_controller(&self) -> u8 {
        self.data[1] & 0x7F
    }

    pub fn cc_value(&self) -> u8 {
        self.data[2] & 0x7F
    }

    pub fn is_pitch_bend(&self) -> bool {
        self.len == 3 && (self.data[0] & 0xF0 == 0xE0)
    }

    pub fn pitch_bend_value(&self) -> i32 {
        (((self.data[2] as i32 & 0x7F) << 7) | (self.data[1] as i32 & 0x7F)) - 8192
    }

    pub fn is_channel_pressure(&self) -> bool {
        self.len >= 2 && (self.data[0] & 0xF0 == 0xD0)
    }

    pub fn is_poly_pressure(&self) -> bool {
        self.len == 3 && (self.data[0] & 0xF0 == 0xA0)
    }

    pub fn is_program_change(&self) -> bool {
        self.len >= 2 && (self.data[0] & 0xF0 == 0xC0)
    }

    pub fn pack_word(&self) -> u32 {
        (self.len as u32) | ((self.data[0] as u32) << 8) | ((self.data[1] as u32) << 16) | ((self.data[2] as u32) << 24)
    }

    pub fn unpack_word(port_id: u32, timestamp_us: u64, word: u32) -> Self {
        let len = (word & 0xFF) as u8;
        let d0 = ((word >> 8) & 0xFF) as u8;
        let d1 = ((word >> 16) & 0xFF) as u8;
        let d2 = ((word >> 24) & 0xFF) as u8;
        Self { port_id, timestamp_us, len, data: [d0, d1, d2] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedMidiEvent {
    pub track_id: u64,
    pub at_sample: i64,
    pub event: LiveMidiEvent,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MidiTake {
    pub track_id: u64,
    pub start: i64,
    pub length: i64,
    pub sequence: crate::Sequence,
}

pub fn parse_midi_message(port_id: u32, timestamp_us: u64, bytes: &[u8]) -> Option<LiveMidiEvent> {
    if bytes.is_empty() {
        return None;
    }
    let status = bytes[0];
    if status == 0xFE {
        return None;
    }
    match status & 0xF0 {
        0x80 | 0x90 | 0xA0 | 0xB0 | 0xE0 => {
            if bytes.len() >= 3 {
                Some(LiveMidiEvent { port_id, timestamp_us, len: 3, data: [status, bytes[1] & 0x7F, bytes[2] & 0x7F] })
            } else {
                None
            }
        }
        0xC0 | 0xD0 => {
            if bytes.len() >= 2 {
                Some(LiveMidiEvent { port_id, timestamp_us, len: 2, data: [status, bytes[1] & 0x7F, 0] })
            } else {
                None
            }
        }
        0xF0 => {
            if status >= 0xF8 { Some(LiveMidiEvent { port_id, timestamp_us, len: 1, data: [status, 0, 0] }) } else { None }
        }
        _ => None,
    }
}
