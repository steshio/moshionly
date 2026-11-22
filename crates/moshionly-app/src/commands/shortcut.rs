//! Keyboard shortcuts as text.

use egui::{Key, KeyboardShortcut};

/// A shortcut the way the platform writes it: `⇧⌘Z` on a Mac, `Ctrl+Shift+Z`
/// elsewhere.
pub fn shortcut_text(shortcut: &KeyboardShortcut, mac: bool) -> String {
    let m = shortcut.modifiers;
    let key = key_text(shortcut.logical_key, mac);
    if mac {
        // The order Apple uses: control, option, shift, command.
        let mut text = String::new();
        if m.ctrl {
            text.push('⌃');
        }
        if m.alt {
            text.push('⌥');
        }
        if m.shift {
            text.push('⇧');
        }
        if m.command || m.mac_cmd {
            text.push('⌘');
        }
        text.push_str(key);
        text
    } else {
        let mut parts = Vec::new();
        if m.ctrl || m.command {
            parts.push("Ctrl");
        }
        if m.alt {
            parts.push("Alt");
        }
        if m.shift {
            parts.push("Shift");
        }
        parts.push(key);
        parts.join("+")
    }
}

fn key_text(key: Key, mac: bool) -> &'static str {
    match key {
        Key::Backspace if mac => "⌫",
        Key::Delete if mac => "⌦",
        Key::Enter if mac => "↵",
        Key::Escape => "Esc",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::Plus => "+",
        Key::Minus => "-",
        Key::Equals => "=",
        other => other.name(),
    }
}

#[cfg(test)]
mod tests {
    use egui::Modifiers;

    use super::*;

    fn shortcut(modifiers: Modifiers, key: Key) -> KeyboardShortcut {
        KeyboardShortcut::new(modifiers, key)
    }

    #[test]
    fn mac_shortcuts_use_symbols_in_apple_order() {
        let mac = true;
        assert_eq!(
            shortcut_text(&shortcut(Modifiers::COMMAND, Key::K), mac),
            "⌘K"
        );
        assert_eq!(
            shortcut_text(
                &shortcut(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
                mac
            ),
            "⇧⌘Z"
        );
        assert_eq!(
            shortcut_text(
                &shortcut(Modifiers::COMMAND.plus(Modifiers::ALT), Key::C),
                mac
            ),
            "⌥⌘C"
        );
        assert_eq!(
            shortcut_text(&shortcut(Modifiers::NONE, Key::Backspace), mac),
            "⌫"
        );
        assert_eq!(
            shortcut_text(&shortcut(Modifiers::SHIFT, Key::Num1), mac),
            "⇧1"
        );
        assert_eq!(
            shortcut_text(&shortcut(Modifiers::NONE, Key::Space), mac),
            "Space"
        );
    }

    #[test]
    fn other_platforms_spell_modifiers_out() {
        let mac = false;
        assert_eq!(
            shortcut_text(&shortcut(Modifiers::COMMAND, Key::K), mac),
            "Ctrl+K"
        );
        assert_eq!(
            shortcut_text(
                &shortcut(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z),
                mac
            ),
            "Ctrl+Shift+Z"
        );
        assert_eq!(
            shortcut_text(&shortcut(Modifiers::NONE, Key::Backspace), mac),
            "Backspace"
        );
        assert_eq!(
            shortcut_text(&shortcut(Modifiers::COMMAND, Key::Plus), mac),
            "Ctrl++"
        );
    }
}
