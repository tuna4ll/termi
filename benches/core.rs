//! How long the hot paths take on a large file: typing into it, highlighting
//! it, searching it, undoing in it.
//!
//! A plain harness rather than a benchmarking framework: each case runs until
//! a time budget is spent and reports the mean, which is enough to notice a
//! path going from microseconds to milliseconds. `cargo bench` runs it.

use std::hint::black_box;
use std::time::{Duration, Instant};

use termi::editor::buffer::Buffer;
use termi::editor::cursor::Position;
use termi::editor::document::Document;
use termi::editor::edit::Edit;
use termi::editor::window::Window;
use termi::search::Search;
use termi::syntax::{self, HighlightCache};

/// How long each case runs for.
const BUDGET: Duration = Duration::from_millis(400);

fn bench(name: &str, mut run: impl FnMut()) {
    // One untimed run warms caches and lazily built state.
    run();
    let started = Instant::now();
    let mut rounds = 0u32;
    while started.elapsed() < BUDGET {
        run();
        rounds += 1;
    }
    let mean = started.elapsed() / rounds.max(1);
    println!("{name:<44} {mean:>12.2?}  ({rounds} runs)");
}

/// A Rust-looking file of `lines` lines.
fn source(lines: usize) -> String {
    (0..lines)
        .map(|index| match index % 6 {
            0 => format!("/// Item number {index}.\n"),
            1 => format!("pub fn item_{index}(value: u32) -> u32 {{\n"),
            2 => "    let doubled = value * 2; // twice\n".to_string(),
            3 => "    if doubled > 10 { return doubled; }\n".to_string(),
            4 => "    \"a string with // inside\".len() as u32\n".to_string(),
            _ => "}\n".to_string(),
        })
        .collect()
}

fn main() {
    syntax::builtin_only();
    let text = source(100_000);

    bench("open a 100k-line document", || {
        black_box(Document::from_text(black_box(&text), None));
    });

    let mut buffer = Buffer::new(Document::from_text(&text, None));
    let mut window = Window::new(0);
    window.cursor_mut().move_to(Position::new(50_000, 4), false);
    bench("type a character in the middle", || {
        let mut edit = Edit::new(&mut buffer, &mut window);
        edit.insert_text("x");
    });
    bench("undo a typed character", || {
        let mut edit = Edit::new(&mut buffer, &mut window);
        edit.insert_text("y");
        edit.checkpoint();
        edit.undo();
    });

    let document = Document::from_text(&text, None);
    let rust = syntax::by_name("rust");
    bench("highlight the first 10k lines from cold", || {
        let mut cache = HighlightCache::new(rust);
        cache.ensure(&document, 10_000);
        black_box(cache.highlight(&document, 10_000));
    });
    let mut warm = HighlightCache::new(rust);
    warm.ensure(&document, 99_999);
    bench("highlight one screen deep in the file", || {
        for line in 90_000..90_060 {
            black_box(warm.highlight(&document, line));
        }
    });

    let mut search = Search::default();
    search.set_query("item_99999".to_string());
    bench("find a match at the far end", || {
        black_box(search.find(&document, 0, true));
    });
}
