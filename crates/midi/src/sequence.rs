use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub pitch: u8,
    pub velocity: u8,
    pub release_velocity: u8,
    pub channel: u8,
    pub start: i64,
    pub length: i64,
}

impl Note {
    pub fn end(&self) -> i64 {
        self.start.saturating_add(self.length)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CtrlKind {
    Cc(u8),
    PitchBend,
    ChannelPressure,
    PolyPressure(u8),
    Program,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CtrlEvent {
    pub tick: i64,
    pub channel: u8,
    pub kind: CtrlKind,
    pub value: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sequence {
    pub notes: Vec<Note>,
    pub ctrls: Vec<CtrlEvent>,
}

fn kind_key(k: CtrlKind) -> (u8, u8) {
    match k {
        CtrlKind::Cc(n) => (0, n),
        CtrlKind::PitchBend => (1, 0),
        CtrlKind::ChannelPressure => (2, 0),
        CtrlKind::PolyPressure(p) => (3, p),
        CtrlKind::Program => (4, 0),
    }
}

impl Sequence {
    pub fn sort(&mut self) {
        self.notes.sort_by_key(|a| (a.start, a.pitch, a.channel, a.length, a.velocity));
        self.ctrls.sort_by_key(|c| c.tick);
    }

    pub fn end_tick(&self) -> i64 {
        let n = self.notes.iter().map(Note::end).max();
        let c = self.ctrls.iter().map(|c| c.tick).max();
        match (n, c) {
            (Some(a), Some(b)) => a.max(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => 0,
        }
    }

    pub fn notes_in(&self, start: i64, end: i64) -> impl Iterator<Item = &Note> + '_ {
        self.notes.iter().filter(move |n| n.start >= start && n.start < end)
    }

    pub fn notes_overlapping(&self, start: i64, end: i64) -> impl Iterator<Item = &Note> + '_ {
        self.notes.iter().filter(move |n| n.start < end && n.end() > start)
    }

    pub fn remove_duplicates(&mut self) -> usize {
        let before = self.notes.len() + self.ctrls.len();
        self.notes.sort_by(|a, b| (a.start, a.pitch, a.channel, b.length).cmp(&(b.start, b.pitch, b.channel, a.length)));
        self.notes.dedup_by(|later, kept| later.start == kept.start && later.pitch == kept.pitch && later.channel == kept.channel);

        self.ctrls.sort_by_key(|c| c.tick);
        let mut kept: Vec<CtrlEvent> = Vec::with_capacity(self.ctrls.len());
        for c in self.ctrls.drain(..) {
            let slot =
                kept.iter_mut().rev().take_while(|k| k.tick == c.tick).find(|k| k.channel == c.channel && kind_key(k.kind) == kind_key(c.kind));
            match slot {
                Some(slot) => *slot = c,
                None => kept.push(c),
            }
        }
        self.ctrls = kept;
        self.sort();
        before - (self.notes.len() + self.ctrls.len())
    }
}
