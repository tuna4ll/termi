//! Every language must highlight any text without panicking, and return spans
//! inside the line.

#![no_main]

use libfuzzer_sys::fuzz_target;
use termi::syntax::{self, BlockState};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    syntax::builtin_only();
    for name in syntax::names() {
        let highlighter = syntax::by_name(name).expect("known language");
        let mut state = BlockState::Normal;
        for line in text.lines() {
            let (spans, next) = highlighter.highlight_line(line, state);
            let length = line.chars().count();
            assert!(spans.iter().all(|span| span.start <= span.end && span.end <= length));
            state = next;
        }
    }
});
