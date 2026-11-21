// input.rs

use libservo::{
    Code, DevicePoint, InputEvent, Key, KeyState, KeyboardEvent, Location, Modifiers,
    MouseButton, NamedKey, WebViewPoint,
};

pub(crate) fn point(x: f32, y: f32) -> WebViewPoint {
    WebViewPoint::Device(DevicePoint::new(x, y))
}

pub(crate) fn mouse_button(button: i32) -> MouseButton {
    match button {
        0 => MouseButton::Primary,
        1 => MouseButton::Secondary,
        2 => MouseButton::Auxiliary,
        other => MouseButton::Other(other as u16),
    }
}

// Numbering is shared with SpecialKey() in cl_dll/cs_web/csw_input.cpp.
fn special_key(special: i32) -> Option<Key> {
    Some(Key::Named(match special {
        1 => NamedKey::Enter,
        2 => NamedKey::Backspace,
        3 => NamedKey::Tab,
        4 => NamedKey::Escape,
        5 => NamedKey::ArrowUp,
        6 => NamedKey::ArrowDown,
        7 => NamedKey::ArrowLeft,
        8 => NamedKey::ArrowRight,
        9 => NamedKey::Home,
        10 => NamedKey::End,
        11 => NamedKey::PageUp,
        12 => NamedKey::PageDown,
        13 => NamedKey::Insert,
        14 => NamedKey::Delete,
        _ => return None,
    }))
}

/// `codepoint` wins over `special`; None if neither is valid.
pub(crate) fn resolve_key(codepoint: u32, special: i32) -> Option<Key> {
    if codepoint != 0 {
        char::from_u32(codepoint).map(|c| Key::Character(c.to_string()))
    } else {
        special_key(special)
    }
}

pub(crate) fn modifiers_from_bits(bits: u32) -> Modifiers {
    let mut modifiers = Modifiers::empty();
    if bits & 1 != 0 {
        modifiers |= Modifiers::SHIFT;
    }
    if bits & 2 != 0 {
        modifiers |= Modifiers::CONTROL;
    }
    if bits & 4 != 0 {
        modifiers |= Modifiers::ALT;
    }
    if bits & 8 != 0 {
        modifiers |= Modifiers::META;
    }
    modifiers
}

pub(crate) fn key_event(state: KeyState, key: Key, modifiers: Modifiers) -> InputEvent {
    // The physical `code` is not known from the engine's key numbers, so it is left unidentified.
    InputEvent::Keyboard(KeyboardEvent::new_without_event(
        state,
        key,
        Code::Unidentified,
        Location::Standard,
        modifiers,
        false,
        false,
    ))
}