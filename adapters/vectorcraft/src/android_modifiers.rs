//! Android hardware modifier tracking for VectorCraft's physical keyboard.
//! Adapted from the already-tested PhotoCraft Android implementation.
//! Track left/right Ctrl, Shift and Alt independently and clear on focus loss.
//! Do not rewrite IME composition; forward egui's original shortcut events.

#[derive(Default)]
pub struct AndroidModifiers {
    held: [bool; 6],
}

impl AndroidModifiers {
    fn modifiers(&self) -> egui::Modifiers {
        egui::Modifiers {
            shift: self.held[0] || self.held[1],
            ctrl: self.held[2] || self.held[3],
            alt: self.held[4] || self.held[5],
            mac_cmd: false,
            command: self.held[2] || self.held[3],
        }
    }
}

pub fn apply_android_modifiers(state: &mut AndroidModifiers, raw: &mut egui::RawInput) {
    let mut kept = Vec::with_capacity(raw.events.len());
    for mut event in raw.events.drain(..) {
        match &mut event {
            egui::Event::Key { key, physical_key, pressed, modifiers, .. } => {
                let tracked = physical_key.unwrap_or(*key);
                let before = state.modifiers();
                let side = match tracked {
                    egui::Key::ShiftLeft => Some(0),
                    egui::Key::ShiftRight => Some(1),
                    egui::Key::ControlLeft => Some(2),
                    egui::Key::ControlRight => Some(3),
                    egui::Key::AltLeft => Some(4),
                    egui::Key::AltRight => Some(5),
                    _ => None,
                };
                if let Some(side) = side { state.held[side] = *pressed; }
                let current = state.modifiers();
                if current != before { kept.push(egui::Event::ModifiersChanged(current)); }
                *modifiers = current;
                kept.push(event);
            }
            egui::Event::PointerButton { modifiers, .. }
            | egui::Event::MouseWheel { modifiers, .. } => {
                *modifiers = state.modifiers();
                kept.push(event);
            }
            egui::Event::Text(_) if state.modifiers().command => {}
            egui::Event::WindowFocused(false) => {
                *state = AndroidModifiers::default();
                kept.push(egui::Event::ModifiersChanged(state.modifiers()));
                kept.push(event);
            }
            _ => kept.push(event),
        }
    }
    raw.events = kept;
}

/// Minimal event counters for tablet debugging; never retain actual typed text.
#[derive(Default)]
pub struct KeyboardDiagnostics {
    pub key_downs: u64,
    pub text_events: u64,
    pub ime_events: u64,
    pub fallback_events: u64,
    pub last_key: String,
}

impl KeyboardDiagnostics {
    pub fn observe(&mut self, raw: &egui::RawInput) {
        for event in &raw.events {
            match event {
                egui::Event::Key { key, physical_key, pressed: true, .. } => {
                    self.key_downs = self.key_downs.saturating_add(1);
                    self.last_key = format!("{:?}", physical_key.unwrap_or(*key));
                }
                egui::Event::Text(_) => {
                    self.text_events = self.text_events.saturating_add(1);
                }
                egui::Event::Ime(_) => {
                    self.ime_events = self.ime_events.saturating_add(1);
                }
                _ => {}
            }
        }
    }
}

/// Some Android hardware keyboards emit an egui Key event but no Text event.
/// Official VectorCraft v0.14 tool switching (V=Selection, T=Type, etc.) listens
/// for Text, so such hardware cannot switch tools. Add a plain ASCII letter
/// only when that frame has NO native Text or IME event and no editing focus.
/// Do not change or suppress any original event. Never synthesize Ctrl/Alt
/// chords, repeated keys or ongoing text editing/IME: no duplicate letters.
pub fn restore_tool_key_text(raw: &mut egui::RawInput, tool_shortcuts_allowed: bool) -> usize {
    if !tool_shortcuts_allowed || raw.events.iter().any(|event| {
        matches!(event, egui::Event::Text(_) | egui::Event::Ime(_))
    }) {
        return 0;
    }
    let mut restored = 0;
    let mut events = Vec::with_capacity(raw.events.len() + 2);
    for event in raw.events.drain(..) {
        let generated = match &event {
            egui::Event::Key { key, physical_key, pressed: true, repeat: false, modifiers } 
                if !modifiers.command && !modifiers.ctrl && !modifiers.alt =>
            {
                let key = physical_key.unwrap_or(*key);
                let name = key.name();
                if name.len() == 1 && name.as_bytes()[0].is_ascii_alphabetic() {
                    Some(if modifiers.shift {
                        name.to_ascii_uppercase()
                    } else {
                        name.to_ascii_lowercase()
                    })
                } else {
                    None
                }
            }
            _ => None,
        };
        events.push(event);
        if let Some(letter) = generated {
            events.push(egui::Event::Text(letter));
            restored += 1;
        }
    }
    raw.events = events;
    restored
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, RawInput};

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE }
    }
    fn apply(state: &mut AndroidModifiers, events: Vec<Event>) -> Vec<Event> {
        let mut raw = RawInput { events, ..Default::default() };
        apply_android_modifiers(state, &mut raw);
        raw.events
    }
    #[test]
    fn bare_v_and_t_are_repaired_from_key_only_frames() {
        let mut state = AndroidModifiers::default();
        for (pressed_key, expected) in [(Key::V, "v"), (Key::T, "t")] {
            let mut raw = RawInput { events: apply(&mut state, vec![key(pressed_key, true)]), ..Default::default() };
            assert_eq!(restore_tool_key_text(&mut raw, true), 1);
            assert!(matches!(&raw.events[0], Event::Key { pressed: true, .. }));
            assert!(matches!(&raw.events[1], Event::Text(text) if text == expected));
        }
    }

    #[test]
    fn never_double_native_text_or_editing_ime_events() {
        let mut state = AndroidModifiers::default();
        let events = [
            vec![key(Key::V, true), Event::Text("v".into())],
            vec![key(Key::T, true), Event::Ime(egui::ImeEvent::Commit("你好".into()))],
        ];
        for original in events {
            let mut raw = RawInput { events: apply(&mut state, original), ..Default::default() };
            let unchanged = format!("{:?}", raw.events);
            assert_eq!(restore_tool_key_text(&mut raw, true), 0);
            assert_eq!(format!("{:?}", raw.events), unchanged);
        }
        let mut raw = RawInput { events: vec![key(Key::T, true)], ..Default::default() };
        assert_eq!(restore_tool_key_text(&mut raw, false), 0);
        assert_eq!(raw.events.len(), 1);
    }

    #[test]
    fn modifier_chords_repeat_and_shift_are_respected() {
        let mut state = AndroidModifiers::default();
        let events = apply(&mut state, vec![key(Key::ControlLeft, true), key(Key::V, true)]);
        let mut raw = RawInput { events, ..Default::default() };
        assert_eq!(restore_tool_key_text(&mut raw, true), 0);
        apply(&mut state, vec![key(Key::ControlLeft, false)]);
        let events = apply(&mut state, vec![key(Key::ShiftRight, true), key(Key::T, true)]);
        let mut raw = RawInput { events, ..Default::default() };
        assert_eq!(restore_tool_key_text(&mut raw, true), 1);
        assert!(matches!(raw.events.last(), Some(Event::Text(letter)) if letter == "T"));
        let mut repeated = RawInput { events: vec![
            Event::Key { key: Key::V, physical_key: Some(Key::V), pressed: true, repeat: true, modifiers: Modifiers::NONE }
        ], ..Default::default() };
        assert_eq!(restore_tool_key_text(&mut repeated, true), 0);
        assert_eq!(repeated.events.len(), 1);
    }

    #[test]
    fn repair_uses_physical_letter_when_logical_key_is_unknown() {
        let mut raw = RawInput { events: vec![
            Event::Key { key: Key::Z, physical_key: Some(Key::V), pressed: true, repeat: false, modifiers: Modifiers::NONE }
        ], ..Default::default() };
        assert_eq!(restore_tool_key_text(&mut raw, true), 1);
        assert!(matches!(raw.events.last(), Some(Event::Text(letter)) if letter == "v"));
    }

    #[test]
    fn releasing_one_side_preserves_the_other() {
        for (left, right, field) in [
            (Key::ControlLeft, Key::ControlRight, 0),
            (Key::ShiftLeft, Key::ShiftRight, 1),
            (Key::AltLeft, Key::AltRight, 2),
        ] {
            for (first, second) in [(left, right), (right, left)] {
                let mut state = AndroidModifiers::default();
                apply(&mut state, vec![key(first, true), key(second, true), key(first, false)]);
                let mods = state.modifiers();
                assert!([mods.ctrl, mods.shift, mods.alt][field]);
                apply(&mut state, vec![key(second, false)]);
                assert_eq!(state.modifiers(), Modifiers::NONE);
            }
        }
    }
    #[test]
    fn command_and_chord_text_across_frames() {
        let mut state = AndroidModifiers::default();
        apply(&mut state, vec![key(Key::ControlRight, true)]);
        let events = apply(&mut state, vec![key(Key::Z, true), Event::Text("z".into())]);
        assert!(matches!(events.as_slice(), [Event::Key { modifiers, .. }] if modifiers.ctrl && modifiers.command && !modifiers.mac_cmd));
        apply(&mut state, vec![key(Key::ControlRight, false)]);
        assert!(matches!(apply(&mut state, vec![Event::Text("z".into())]).as_slice(), [Event::Text(_)]));
    }
    #[test]
    fn repeats_are_idempotent_and_focus_loss_clears_all_sides() {
        let mut state = AndroidModifiers::default();
        apply(&mut state, vec![key(Key::ControlLeft, true), key(Key::ControlLeft, true), key(Key::ShiftRight, true)]);
        assert!(state.modifiers().ctrl && state.modifiers().shift);
        let events = apply(&mut state, vec![Event::WindowFocused(false), key(Key::B, true), Event::Text("b".into())]);
        assert_eq!(state.modifiers(), Modifiers::NONE);
        assert!(matches!(events.last(), Some(Event::Text(_))));
    }
    #[test]
    fn physical_key_and_event_order_are_preserved() {
        let mut state = AndroidModifiers::default();
        let ctrl = Event::Key { key: Key::A, physical_key: Some(Key::ControlLeft), pressed: true, repeat: false, modifiers: Modifiers::NONE };
        let events = apply(&mut state, vec![ctrl, key(Key::S, true), key(Key::ControlLeft, false)]);
        assert!(matches!(&events[0], Event::ModifiersChanged(mods) if mods.command));
        assert!(matches!(&events[2], Event::Key { key: Key::S, modifiers, .. } if modifiers.command));
        assert!(matches!(&events[3], Event::ModifiersChanged(mods) if !mods.command));
    }
    #[test]
    fn ime_composition_is_never_rewritten() {
        let mut state = AndroidModifiers::default();
        apply(&mut state, vec![key(Key::ControlLeft, true)]);
        let events = apply(&mut state, vec![Event::Ime(egui::ImeEvent::Commit("你好".into()))]);
        assert!(matches!(events.as_slice(), [Event::Ime(egui::ImeEvent::Commit(text))] if text == "你好"));
    }
}
