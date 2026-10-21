use openpt_vst3_host::keyhook::{self, KeyEvent, vk};
pub use openpt_vst3_host::keyhook::{Held, PadKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    Start { key: egui::Key, held: Held },
    Pad { key: PadKey, down: bool, held: Held },
}

pub fn install() {
    keyhook::install();
}

pub(crate) fn set_context(ctx: &egui::Context) {
    let ctx = ctx.clone();
    keyhook::set_waker(Box::new(move || ctx.request_repaint()));
}

pub(crate) fn set_text_focus(on: bool) {
    keyhook::set_text_focus(on);
}

pub(crate) fn drain() -> Vec<HookEvent> {
    keyhook::drain()
        .into_iter()
        .filter_map(|e| match e {
            KeyEvent::Start { vk, held } => main_key(vk).map(|key| HookEvent::Start { key, held }),
            KeyEvent::Pad { key, down, held } => Some(HookEvent::Pad { key, down, held }),
        })
        .collect()
}

fn main_key(code: u16) -> Option<egui::Key> {
    let name = match code {
        0x30..=0x39 | 0x41..=0x5A => return egui::Key::from_name(&char::from(code as u8).to_string()),
        vk::F1..=vk::F20 => return egui::Key::from_name(&format!("F{}", code - vk::F1 + 1)),
        vk::OEM_PLUS => "=",
        vk::OEM_MINUS => "-",
        vk::OEM_COMMA => ",",
        vk::OEM_PERIOD => ".",
        vk::OEM_2 => "/",
        vk::OEM_1 => ";",
        vk::OEM_7 => "'",
        vk::OEM_4 => "[",
        vk::OEM_6 => "]",
        vk::OEM_5 => "\\",
        vk::SPACE => "Space",
        vk::TAB => "Tab",
        vk::RETURN => "Enter",
        vk::BACK => "Backspace",
        vk::DELETE => "Delete",
        vk::HOME => "Home",
        vk::END => "End",
        vk::LEFT => "ArrowLeft",
        vk::RIGHT => "ArrowRight",
        vk::UP => "ArrowUp",
        vk::DOWN => "ArrowDown",
        _ => return None,
    };
    egui::Key::from_name(name)
}
