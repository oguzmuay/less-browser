//! egui input -> CEF events, and page-scroll JavaScript.

use cef::sys::cef_event_flags_t as Flags;
use eframe::egui::{Key, Modifiers, PointerButton};
use less_input::PageAction;

/// JavaScript that performs a page scroll (a line is 60px, a half page is `innerHeight / 2`).
pub fn scroll_js(action: PageAction) -> String {
    match action {
        PageAction::ScrollLines(n) => format!("window.scrollBy(0, {})", n.saturating_mul(60)),
        PageAction::HalfPages(n) => format!("window.scrollBy(0, innerHeight / 2 * {n})"),
        PageAction::Top => "window.scrollTo(0, 0)".into(),
        PageAction::Bottom => "window.scrollTo(0, document.scrollingElement.scrollHeight)".into(),
    }
}

/// CEF `EVENTFLAG_*` bits for egui modifiers.
pub fn flags(m: Modifiers) -> u32 {
    let mut f = 0;
    for (on, flag) in [
        (m.shift, Flags::EVENTFLAG_SHIFT_DOWN),
        (m.ctrl, Flags::EVENTFLAG_CONTROL_DOWN),
        (m.alt, Flags::EVENTFLAG_ALT_DOWN),
        (m.mac_cmd, Flags::EVENTFLAG_COMMAND_DOWN),
    ] {
        if on {
            f |= flag.0;
        }
    }
    f
}

/// CEF mouse button and its `EVENTFLAG_*` bit.
pub fn mouse_button(b: PointerButton) -> Option<(cef::MouseButtonType, u32)> {
    match b {
        PointerButton::Primary => Some((cef::MouseButtonType::LEFT, Flags::EVENTFLAG_LEFT_MOUSE_BUTTON.0)),
        PointerButton::Middle => Some((cef::MouseButtonType::MIDDLE, Flags::EVENTFLAG_MIDDLE_MOUSE_BUTTON.0)),
        PointerButton::Secondary => Some((cef::MouseButtonType::RIGHT, Flags::EVENTFLAG_RIGHT_MOUSE_BUTTON.0)),
        _ => None,
    }
}

/// Builds the RAWKEYDOWN/KEYUP event for an egui key.
///
/// On macOS CEF rebuilds an `NSEvent` from `native_key_code` (a physical `kVK_*` code) and
/// `character`, so both are filled; `windows_key_code` is derived by CEF. Pass the egui
/// `physical_key` when available so non-US layouts (Turkish Q/F) still hit the right key.
pub fn key_event(key: Key, physical: Option<Key>, mods: Modifiers, pressed: bool) -> Option<cef::KeyEvent> {
    let (native, special) = mac_key(physical.unwrap_or(key)).or_else(|| mac_key(key))?;
    let character = special.unwrap_or_else(|| letter_char(key, mods.shift));
    Some(cef::KeyEvent {
        size: size_of::<cef::sys::_cef_key_event_t>(),
        type_: if pressed { cef::KeyEventType::RAWKEYDOWN } else { cef::KeyEventType::KEYUP },
        modifiers: flags(mods),
        native_key_code: native,
        character,
        unmodified_character: character,
        ..Default::default()
    })
}

/// Builds a CHAR event that inserts `c` (from egui `Text`, so layout/Option-composed chars work).
pub fn char_event(c: u16, mods: Modifiers) -> cef::KeyEvent {
    cef::KeyEvent {
        size: size_of::<cef::sys::_cef_key_event_t>(),
        type_: cef::KeyEventType::CHAR,
        modifiers: flags(mods),
        character: c,
        unmodified_character: c,
        ..Default::default()
    }
}

fn letter_char(key: Key, shift: bool) -> u16 {
    let c = key.name().chars().next().unwrap_or('\0');
    let c = if shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
    if key.name().len() == 1 { c as u16 } else { 0 }
}

/// macOS virtual key code (`kVK_*`) and, for non-printing keys, the `NSEvent` character.
fn mac_key(key: Key) -> Option<(i32, Option<u16>)> {
    use Key::*;
    let printable = |code| Some((code, None));
    let special = |code, ch| Some((code, Some(ch)));
    match key {
        A => printable(0x00), S => printable(0x01), D => printable(0x02), F => printable(0x03),
        H => printable(0x04), G => printable(0x05), Z => printable(0x06), X => printable(0x07),
        C => printable(0x08), V => printable(0x09), B => printable(0x0B), Q => printable(0x0C),
        W => printable(0x0D), E => printable(0x0E), R => printable(0x0F), Y => printable(0x10),
        T => printable(0x11), O => printable(0x1F), U => printable(0x20), I => printable(0x22),
        P => printable(0x23), L => printable(0x25), J => printable(0x26), K => printable(0x28),
        N => printable(0x2D), M => printable(0x2E),
        Num1 => printable(0x12), Num2 => printable(0x13), Num3 => printable(0x14),
        Num4 => printable(0x15), Num6 => printable(0x16), Num5 => printable(0x17),
        Num9 => printable(0x19), Num7 => printable(0x1A), Num8 => printable(0x1C),
        Num0 => printable(0x1D),
        Equals | Plus => printable(0x18), Minus => printable(0x1B), CloseBracket => printable(0x1E),
        OpenBracket => printable(0x21), Quote => printable(0x27), Semicolon | Colon => printable(0x29),
        Backslash | Pipe => printable(0x2A), Comma => printable(0x2B), Slash | Questionmark => printable(0x2C),
        Period => printable(0x2F), Backtick => printable(0x32),
        Enter => special(0x24, 0x0D),
        Tab => special(0x30, 0x09),
        Space => special(0x31, 0x20),
        Backspace => special(0x33, 0x7F),
        Escape => special(0x35, 0x1B),
        Delete => special(0x75, 0xF728),
        Home => special(0x73, 0xF729),
        End => special(0x77, 0xF72B),
        PageUp => special(0x74, 0xF72C),
        PageDown => special(0x79, 0xF72D),
        ArrowLeft => special(0x7B, 0xF702),
        ArrowRight => special(0x7C, 0xF703),
        ArrowDown => special(0x7D, 0xF701),
        ArrowUp => special(0x7E, 0xF700),
        F1 => special(0x7A, 0xF704), F2 => special(0x78, 0xF705), F3 => special(0x63, 0xF706),
        F4 => special(0x76, 0xF707), F5 => special(0x60, 0xF708), F6 => special(0x61, 0xF709),
        F7 => special(0x62, 0xF70A), F8 => special(0x64, 0xF70B), F9 => special(0x65, 0xF70C),
        F10 => special(0x6D, 0xF70D), F11 => special(0x67, 0xF70E), F12 => special(0x6F, 0xF70F),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_actions_become_js() {
        assert_eq!(scroll_js(PageAction::ScrollLines(1)), "window.scrollBy(0, 60)");
        assert_eq!(scroll_js(PageAction::ScrollLines(-3)), "window.scrollBy(0, -180)");
        assert_eq!(scroll_js(PageAction::HalfPages(-2)), "window.scrollBy(0, innerHeight / 2 * -2)");
        assert_eq!(scroll_js(PageAction::Top), "window.scrollTo(0, 0)");
        assert_eq!(scroll_js(PageAction::Bottom), "window.scrollTo(0, document.scrollingElement.scrollHeight)");
    }

    #[test]
    fn key_events_carry_mac_codes() {
        let e = key_event(Key::A, None, Modifiers::NONE, true).unwrap();
        assert_eq!((e.native_key_code, e.character), (0x00, 'a' as u16));
        let e = key_event(Key::ArrowUp, None, Modifiers::NONE, false).unwrap();
        assert_eq!((e.native_key_code, e.character, e.type_), (0x7E, 0xF700, cef::KeyEventType::KEYUP));
        let cmd = Modifiers { mac_cmd: true, command: true, ..Default::default() };
        assert_eq!(flags(cmd), Flags::EVENTFLAG_COMMAND_DOWN.0);
    }
}
