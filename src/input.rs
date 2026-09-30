use crate::vim::Key;
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey};

/// Turns a key press into the keys the vim engine takes: none for a bare modifier or a key we
/// have no use for, and several for text that arrives in one piece, like an IME commit.
/// `text` is what the press types, if anything, which takes shifting and dead keys into account.
/// Ctrl or Cmd with a letter is a shortcut, and Ctrl+[ is Esc.
pub fn keys(logical: &WinitKey, text: Option<&str>, mods: ModifiersState) -> Vec<Key> {
    match logical {
        WinitKey::Named(named) => match named {
            NamedKey::Escape => vec![Key::Esc],
            NamedKey::Enter => vec![Key::Enter],
            NamedKey::Backspace => vec![Key::Backspace],
            NamedKey::Delete => vec![Key::Delete],
            NamedKey::ArrowLeft => vec![Key::Left],
            NamedKey::ArrowRight => vec![Key::Right],
            NamedKey::ArrowUp => vec![Key::Up],
            NamedKey::ArrowDown => vec![Key::Down],
            NamedKey::Tab => vec![Key::Char('\t')],
            NamedKey::F1 => vec![Key::Help],
            NamedKey::Space => vec![Key::Char(' ')],
            _ => Vec::new(),
        },
        WinitKey::Character(typed) => {
            if mods.control_key() || mods.super_key() {
                match typed.chars().next() {
                    Some('[') => vec![Key::Esc],
                    Some(c) if c.is_ascii_alphabetic() => vec![Key::Ctrl(c.to_ascii_lowercase())],
                    _ => Vec::new(),
                }
            } else {
                text.unwrap_or(typed).chars().map(Key::Char).collect()
            }
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(s: &str) -> WinitKey {
        WinitKey::Character(s.into())
    }

    #[test]
    fn named_keys() {
        let none = ModifiersState::empty();
        for (named, want) in [
            (NamedKey::Escape, Key::Esc),
            (NamedKey::Enter, Key::Enter),
            (NamedKey::Backspace, Key::Backspace),
            (NamedKey::Delete, Key::Delete),
            (NamedKey::ArrowLeft, Key::Left),
            (NamedKey::ArrowDown, Key::Down),
            (NamedKey::Tab, Key::Char('\t')),
            (NamedKey::Space, Key::Char(' ')),
        ] {
            assert_eq!(keys(&WinitKey::Named(named), None, none), [want]);
        }
        // Keys with no meaning here, and bare modifiers, give nothing.
        assert!(keys(&WinitKey::Named(NamedKey::F2), None, none).is_empty());
        assert!(
            keys(
                &WinitKey::Named(NamedKey::Shift),
                None,
                ModifiersState::SHIFT
            )
            .is_empty()
        );
    }

    #[test]
    fn typed_text_uses_what_the_press_produced() {
        let none = ModifiersState::empty();
        assert_eq!(keys(&character("a"), Some("a"), none), [Key::Char('a')]);
        // Shift, dead keys, and IME commits change the text but not the key.
        assert_eq!(
            keys(&character("a"), Some("A"), ModifiersState::SHIFT),
            [Key::Char('A')]
        );
        assert_eq!(keys(&character("e"), Some("é"), none), [Key::Char('é')]);
        assert_eq!(
            keys(&character("x"), Some("日本"), none),
            [Key::Char('日'), Key::Char('本')]
        );
        assert_eq!(keys(&character("!"), None, none), [Key::Char('!')]);
    }

    #[test]
    fn ctrl_and_cmd_make_shortcuts() {
        for mods in [ModifiersState::CONTROL, ModifiersState::SUPER] {
            assert_eq!(keys(&character("b"), Some("\u{2}"), mods), [Key::Ctrl('b')]);
            assert_eq!(keys(&character("B"), None, mods), [Key::Ctrl('b')]);
            assert_eq!(keys(&character("["), None, mods), [Key::Esc]);
            assert!(keys(&character("1"), None, mods).is_empty());
        }
    }
}
