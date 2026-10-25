//! # Key notation
//!
//! **Purpose:** write keys down as text and read them back — `<C-s>`, `<Esc>`,
//! `<Space>f`, `jk`.
//!
//! **Responsibility:** the one textual form for keys in the editor. Macros are
//! stored in registers in it, so `"ap` shows a recording and `"ay$` turns an
//! edited line back into one, and the `[keys]` tables in the config use it so
//! a mapping reads the way the help writes keys.
//!
//! **Public API:** [`encode`], [`parse`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Named keys, as written between angle brackets.
const NAMES: &[(&str, KeyCode)] = &[
    ("esc", KeyCode::Esc),
    ("cr", KeyCode::Enter),
    ("enter", KeyCode::Enter),
    ("return", KeyCode::Enter),
    ("tab", KeyCode::Tab),
    ("bs", KeyCode::Backspace),
    ("backspace", KeyCode::Backspace),
    ("del", KeyCode::Delete),
    ("delete", KeyCode::Delete),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
    ("insert", KeyCode::Insert),
    ("space", KeyCode::Char(' ')),
    ("lt", KeyCode::Char('<')),
    ("gt", KeyCode::Char('>')),
    ("bar", KeyCode::Char('|')),
    ("bslash", KeyCode::Char('\\')),
];

/// Write keys down.
#[must_use]
pub fn encode(keys: &[KeyEvent]) -> String {
    keys.iter().map(|key| encode_one(*key)).collect()
}

fn encode_one(key: KeyEvent) -> String {
    let mut prefix = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        prefix.push_str("C-");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        prefix.push_str("A-");
    }
    let name = match key.code {
        KeyCode::Char('<') => "lt".to_string(),
        KeyCode::Char(ch) if prefix.is_empty() => return ch.to_string(),
        KeyCode::Char(' ') => "Space".to_string(),
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::F(number) => format!("F{number}"),
        KeyCode::BackTab => {
            prefix.push_str("S-");
            "Tab".to_string()
        }
        code => {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                prefix.push_str("S-");
            }
            NAMES
                .iter()
                .find(|(_, named)| *named == code)
                .map(|(name, _)| capitalise(name))
                .unwrap_or_else(|| "Nop".to_string())
        }
    };
    format!("<{prefix}{name}>")
}

fn capitalise(name: &str) -> String {
    match name {
        "cr" => "CR".to_string(),
        "bs" => "BS".to_string(),
        "pageup" => "PageUp".to_string(),
        "pagedown" => "PageDown".to_string(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
                .unwrap_or_default()
        }
    }
}

/// Read keys back.
///
/// # Errors
/// Returns a message naming the part that is not a key.
pub fn parse(text: &str) -> Result<Vec<KeyEvent>, String> {
    let mut keys = Vec::new();
    let mut rest = text;
    while let Some(ch) = rest.chars().next() {
        if ch == '<'
            && let Some(close) = rest.find('>')
            && close > 1
            && let Some(key) = parse_bracketed(&rest[1..close])
        {
            keys.push(key);
            rest = &rest[close + 1..];
            continue;
        }
        let modifiers = if ch.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        keys.push(KeyEvent::new(KeyCode::Char(ch), modifiers));
        rest = &rest[ch.len_utf8()..];
    }
    if keys.is_empty() {
        return Err(format!("no keys in {text:?}"));
    }
    Ok(keys)
}

fn parse_bracketed(inner: &str) -> Option<KeyEvent> {
    let mut modifiers = KeyModifiers::NONE;
    let mut name = inner;
    loop {
        let lower = name.to_ascii_lowercase();
        let stripped = if lower.starts_with("c-") {
            modifiers |= KeyModifiers::CONTROL;
            2
        } else if lower.starts_with("a-") || lower.starts_with("m-") {
            modifiers |= KeyModifiers::ALT;
            2
        } else if lower.starts_with("s-") {
            modifiers |= KeyModifiers::SHIFT;
            2
        } else {
            break;
        };
        if name.len() <= stripped {
            break;
        }
        name = &name[stripped..];
    }
    let lower = name.to_ascii_lowercase();
    let code = if let Some((_, code)) = NAMES.iter().find(|(named, _)| *named == lower) {
        *code
    } else if let Some(number) = lower.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        KeyCode::F(number)
    } else {
        let mut chars = name.chars();
        let ch = chars.next()?;
        if chars.next().is_some() {
            return None;
        }
        // `<C-S>` and `<C-s>` are the same chord: terminals report the letter
        // in lower case with Ctrl held.
        if modifiers.contains(KeyModifiers::CONTROL) {
            KeyCode::Char(ch.to_ascii_lowercase())
        } else {
            KeyCode::Char(ch)
        }
    };
    if code == KeyCode::Tab && modifiers.contains(KeyModifiers::SHIFT) {
        return Some(KeyEvent::new(
            KeyCode::BackTab,
            modifiers - KeyModifiers::SHIFT,
        ));
    }
    Some(KeyEvent::new(code, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_a_run_of_characters() {
        let keys = parse("dw").expect("valid");
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].code, KeyCode::Char('d'));
    }

    #[test]
    fn bracketed_names_and_modifiers() {
        let keys = parse("<C-s><Esc><Space>f<lt>").expect("valid");
        assert_eq!(
            keys[0],
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        );
        assert_eq!(keys[1].code, KeyCode::Esc);
        assert_eq!(keys[2].code, KeyCode::Char(' '));
        assert_eq!(keys[3].code, KeyCode::Char('f'));
        assert_eq!(keys[4].code, KeyCode::Char('<'));
    }

    #[test]
    fn a_lone_angle_bracket_is_just_a_character() {
        let keys = parse("a<b").expect("valid");
        assert_eq!(keys.len(), 3);
    }

    #[test]
    fn encoding_round_trips() {
        let original = "ihello<Esc>:w<CR><C-w>v<A-Up><lt>";
        let keys = parse(original).expect("valid");
        assert_eq!(encode(&keys), original);
    }
}
