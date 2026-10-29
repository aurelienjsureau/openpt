use crate::TrackId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearnSource {
    Cc { channel: u8, number: u8 },
    Note { channel: u8, number: u8 },
    PitchBend { channel: u8 },
}

impl LearnSource {
    pub fn parse(msg: &[u8]) -> Option<(LearnSource, f32, bool)> {
        let (&status, rest) = msg.split_first()?;
        let channel = status & 0x0F;
        let a = rest.first().copied().unwrap_or(0) & 0x7F;
        let b = rest.get(1).copied().unwrap_or(0) & 0x7F;
        match status & 0xF0 {
            0xB0 => Some((LearnSource::Cc { channel, number: a }, f32::from(b) / 127.0, b >= 64)),
            0x90 if b > 0 => Some((LearnSource::Note { channel, number: a }, f32::from(b) / 127.0, true)),
            0x80 | 0x90 => Some((LearnSource::Note { channel, number: a }, 0.0, false)),
            0xE0 => Some((LearnSource::PitchBend { channel }, f32::from(u16::from(b) << 7 | u16::from(a)) / 16383.0, true)),
            _ => None,
        }
    }

    pub fn label(&self) -> String {
        match self {
            LearnSource::Cc { channel, number } => format!("CC {number} · ch {}", channel + 1),
            LearnSource::Note { channel, number } => format!("Note {number} · ch {}", channel + 1),
            LearnSource::PitchBend { channel } => format!("Pitch Bend · ch {}", channel + 1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum LearnTarget {
    Volume { track: TrackId },
    Pan { track: TrackId, index: usize },
    Mute { track: TrackId },
    Solo { track: TrackId },
    RecordArm { track: TrackId },
    SendLevel { track: TrackId, slot: usize },
    PluginParam { track: TrackId, slot: usize, param: String },
    Transport { action: String },
}

impl LearnTarget {
    pub fn track(&self) -> Option<TrackId> {
        match self {
            LearnTarget::Volume { track }
            | LearnTarget::Pan { track, .. }
            | LearnTarget::Mute { track }
            | LearnTarget::Solo { track }
            | LearnTarget::RecordArm { track }
            | LearnTarget::SendLevel { track, .. }
            | LearnTarget::PluginParam { track, .. } => Some(*track),
            LearnTarget::Transport { .. } => None,
        }
    }

    pub fn is_toggle(&self) -> bool {
        matches!(self, LearnTarget::Mute { .. } | LearnTarget::Solo { .. } | LearnTarget::RecordArm { .. } | LearnTarget::Transport { .. })
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MidiLearn {
    pub source: LearnSource,
    pub target: LearnTarget,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_parse_to_sources_and_values() {
        assert_eq!(LearnSource::parse(&[0xB2, 7, 127]), Some((LearnSource::Cc { channel: 2, number: 7 }, 1.0, true)));
        assert_eq!(LearnSource::parse(&[0x90, 60, 0]), Some((LearnSource::Note { channel: 0, number: 60 }, 0.0, false)));
        let (s, v, _) = LearnSource::parse(&[0xE0, 0x7F, 0x7F]).unwrap();
        assert_eq!((s, v), (LearnSource::PitchBend { channel: 0 }, 1.0));
        assert_eq!(LearnSource::parse(&[0xF8]), None);
    }
}
