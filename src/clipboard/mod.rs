//! # Clipboard
//!
//! **Purpose:** yank and paste, with or without a desktop session.
//!
//! **Responsibility:** wrap [`arboard`] and fall back to an internal register
//! when the system clipboard is unavailable — over SSH, in a container, or when
//! the user turns it off. Every copy also lands in the internal register, so
//! paste keeps working even if the system clipboard fails between the two.
//!
//! Whether a yank was *line-wise* travels with the text: pasting a whole line
//! should put it on its own line, while pasting a fragment should splice it in
//! where the caret is. The system clipboard cannot carry that flag, so it is
//! tracked here.
//!
//! **Public API:** [`Clipboard`], [`RegisterKind`], [`registers::Registers`].

pub mod registers;

/// The shape of copied text, which decides how it is put back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegisterKind {
    /// A fragment, spliced in at the caret.
    #[default]
    Charwise,
    /// Whole lines, put above or below the caret's line.
    Linewise,
    /// A rectangle, one line of text per screen line, put at the caret's
    /// column on successive lines.
    Blockwise,
}

/// The editor's copy buffer.
pub struct Clipboard {
    system: Option<arboard::Clipboard>,
    register: String,
    kind: RegisterKind,
}

impl std::fmt::Debug for Clipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Clipboard")
            .field("system", &self.system.is_some())
            .field("register", &self.register.len())
            .field("kind", &self.kind)
            .finish()
    }
}

impl Clipboard {
    /// Connect to the system clipboard when asked, and when one exists.
    #[must_use]
    pub fn new(use_system: bool) -> Self {
        Self {
            system: use_system.then(|| arboard::Clipboard::new().ok()).flatten(),
            register: String::new(),
            kind: RegisterKind::Charwise,
        }
    }

    /// Store `text`, remembering its shape.
    pub fn set(&mut self, text: String, kind: RegisterKind) {
        if let Some(system) = self.system.as_mut() {
            // A clipboard failure must not lose the yank, so ignore the error
            // and rely on the register below.
            let _ = system.set_text(text.clone());
        }
        self.register = text;
        self.kind = kind;
    }

    /// Retrieve the clipboard contents.
    ///
    /// Prefers the system clipboard so that copying from another application
    /// works, but falls back to the register when it is empty or unreadable.
    pub fn get(&mut self) -> String {
        if let Some(system) = self.system.as_mut()
            && let Ok(text) = system.get_text()
            && !text.is_empty()
        {
            // Text from elsewhere has no line-wise flag; treat it as a fragment
            // unless it is exactly what we put there.
            if text != self.register {
                self.kind = if text.ends_with('\n') {
                    RegisterKind::Linewise
                } else {
                    RegisterKind::Charwise
                };
            }
            return text;
        }
        self.register.clone()
    }

    /// The shape of what [`Clipboard::get`] last returned.
    #[must_use]
    pub const fn kind(&self) -> RegisterKind {
        self.kind
    }
}
