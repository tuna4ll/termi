//! # Languages
//!
//! **Purpose:** the data that describes each supported language.
//!
//! **Responsibility:** one module per language, each exposing a single
//! `&'static Language`. Adding a language is adding a file and one line to
//! [`all`] — no engine change, no rendering change.
//!
//! **Public API:** [`all`].

pub mod c;
pub mod cpp;
pub mod diff;
pub mod markdown;
pub mod python;
pub mod rust;
pub mod zig;

use super::Language;

/// Every language definition, in the order they are searched.
///
/// C++ comes before C so that a `.h` shared by both resolves to the definition
/// with the larger keyword set, which degrades more gracefully.
#[must_use]
pub fn all() -> &'static [&'static Language] {
    static ALL: &[&Language] = &[
        &rust::RUST,
        &cpp::CPP,
        &c::C,
        &zig::ZIG,
        &python::PYTHON,
        &markdown::MARKDOWN,
        &diff::DIFF,
    ];
    ALL
}
