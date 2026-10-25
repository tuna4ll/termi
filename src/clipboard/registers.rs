//! # Registers
//!
//! **Purpose:** vi's named places to keep text — `"a` to `"z`, the yank
//! register `"0`, the delete history `"1` to `"9`, the black hole `"_` and the
//! system clipboard `"+`.
//!
//! **Responsibility:** decide where a yank or a delete lands and where a put
//! reads from. The unnamed register *is* the [`Clipboard`], so someone who never
//! types `"` gets exactly the copy-and-paste behaviour they had before, and the
//! system clipboard keeps working with other applications.
//!
//! Macros are stored here too, as text in the key notation, which is what lets
//! `"ap` show a recorded macro and `"ayy` turn an edited line back into one.
//!
//! **Public API:** [`Register`], [`Registers`].

use std::collections::HashMap;

use super::{Clipboard, RegisterKind};

/// The contents of one register.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Register {
    /// The text, with block lines joined by `\n`.
    pub text: String,
    /// How it was copied, and so how it goes back.
    pub kind: RegisterKind,
}

impl Register {
    /// A register holding `text` of shape `kind`.
    #[must_use]
    pub fn new(text: impl Into<String>, kind: RegisterKind) -> Self {
        Self {
            text: text.into(),
            kind,
        }
    }
}

/// Every register, plus the one the next command was told to use.
#[derive(Debug)]
pub struct Registers {
    clipboard: Clipboard,
    named: HashMap<char, Register>,
    selected: Option<char>,
}

impl Registers {
    /// Registers backed by the system clipboard when `use_system` is set.
    #[must_use]
    pub fn new(use_system: bool) -> Self {
        Self {
            clipboard: Clipboard::new(use_system),
            named: HashMap::new(),
            selected: None,
        }
    }

    /// Aim the next yank, delete or put at `name` — what `"a` does.
    pub fn select(&mut self, name: char) {
        self.selected = Some(name);
    }

    /// Forget a selection the command did not use.
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    /// The register the next command was aimed at, if any.
    #[must_use]
    pub const fn selected(&self) -> Option<char> {
        self.selected
    }

    /// Store yanked text.
    pub fn yank(&mut self, text: String, kind: RegisterKind) {
        self.store(Register::new(text, kind), true);
    }

    /// Store deleted text.
    pub fn delete(&mut self, text: String, kind: RegisterKind) {
        self.store(Register::new(text, kind), false);
    }

    fn store(&mut self, register: Register, yank: bool) {
        match self.selected.take() {
            Some('_') => {}
            Some('+' | '*') => self.clipboard.set(register.text, register.kind),
            Some(name) if name.is_ascii_uppercase() => {
                let lower = name.to_ascii_lowercase();
                let entry = self.named.entry(lower).or_default();
                if entry.kind == RegisterKind::Linewise && !entry.text.ends_with('\n') {
                    entry.text.push('\n');
                }
                entry.text.push_str(&register.text);
                if register.kind == RegisterKind::Linewise {
                    entry.kind = RegisterKind::Linewise;
                }
                let combined = entry.clone();
                self.clipboard.set(combined.text, combined.kind);
            }
            Some(name) if name != '"' => {
                self.named.insert(name, register.clone());
                self.clipboard.set(register.text, register.kind);
            }
            _ => {
                if yank {
                    self.named.insert('0', register.clone());
                } else if register.kind == RegisterKind::Linewise || register.text.contains('\n') {
                    // `"1` is the newest of nine; each delete pushes the rest
                    // down one.
                    for slot in (1..9).rev() {
                        let from = char::from_digit(slot, 10).unwrap_or('1');
                        let to = char::from_digit(slot + 1, 10).unwrap_or('9');
                        if let Some(older) = self.named.remove(&from) {
                            self.named.insert(to, older);
                        }
                    }
                    self.named.insert('1', register.clone());
                } else {
                    self.named.insert('-', register.clone());
                }
                self.clipboard.set(register.text, register.kind);
            }
        }
    }

    /// What a put should insert: the selected register, or the clipboard.
    ///
    /// Consumes the selection, so the register only applies to one command.
    pub fn get(&mut self) -> Option<Register> {
        match self.selected.take() {
            Some('_') => None,
            None | Some('"' | '+' | '*') => {
                let text = self.clipboard.get();
                (!text.is_empty()).then(|| Register::new(text, self.clipboard.kind()))
            }
            Some(name) => self.named.get(&name.to_ascii_lowercase()).cloned(),
        }
    }

    /// Read a named register without consuming any selection — for macros.
    #[must_use]
    pub fn named(&self, name: char) -> Option<&Register> {
        self.named.get(&name.to_ascii_lowercase())
    }

    /// Set a register directly — for macros, `:` and `/`.
    pub fn set_named(&mut self, name: char, register: Register) {
        self.named.insert(name, register);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registers() -> Registers {
        Registers::new(false)
    }

    #[test]
    fn the_unnamed_register_is_the_clipboard() {
        let mut registers = registers();
        registers.yank("hello".to_string(), RegisterKind::Charwise);
        assert_eq!(registers.get().map(|r| r.text), Some("hello".to_string()));
        assert_eq!(registers.named('0').map(|r| r.text.as_str()), Some("hello"));
    }

    #[test]
    fn a_named_register_keeps_its_text_after_other_yanks() {
        let mut registers = registers();
        registers.select('a');
        registers.yank("kept".to_string(), RegisterKind::Charwise);
        registers.yank("other".to_string(), RegisterKind::Charwise);
        registers.select('a');
        assert_eq!(registers.get().map(|r| r.text), Some("kept".to_string()));
    }

    #[test]
    fn an_uppercase_name_appends() {
        let mut registers = registers();
        registers.select('a');
        registers.yank("one\n".to_string(), RegisterKind::Linewise);
        registers.select('A');
        registers.yank("two\n".to_string(), RegisterKind::Linewise);
        assert_eq!(
            registers.named('a').map(|r| r.text.as_str()),
            Some("one\ntwo\n")
        );
    }

    #[test]
    fn line_deletes_shift_through_the_numbered_registers() {
        let mut registers = registers();
        registers.delete("first\n".to_string(), RegisterKind::Linewise);
        registers.delete("second\n".to_string(), RegisterKind::Linewise);
        assert_eq!(
            registers.named('1').map(|r| r.text.as_str()),
            Some("second\n")
        );
        assert_eq!(
            registers.named('2').map(|r| r.text.as_str()),
            Some("first\n")
        );
        registers.delete("x".to_string(), RegisterKind::Charwise);
        assert_eq!(registers.named('-').map(|r| r.text.as_str()), Some("x"));
    }

    #[test]
    fn the_black_hole_keeps_nothing() {
        let mut registers = registers();
        registers.yank("before".to_string(), RegisterKind::Charwise);
        registers.select('_');
        registers.delete("gone".to_string(), RegisterKind::Charwise);
        assert_eq!(registers.get().map(|r| r.text), Some("before".to_string()));
    }
}
