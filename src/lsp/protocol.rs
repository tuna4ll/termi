//! The parts of the Language Server Protocol the editor speaks, in its own
//! terms: positions in characters, paths rather than URIs, and plain structs
//! read out of the JSON by hand — the handful of shapes used here do not
//! justify a generated type for every message in the specification.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::editor::cursor::Position;
use crate::editor::document::Document;

/// How a server counts columns, as agreed at initialisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// Bytes of UTF-8.
    Utf8,
    /// UTF-16 code units — the protocol's default.
    #[default]
    Utf16,
    /// Characters, which is how the editor counts too.
    Utf32,
}

impl Encoding {
    /// The encoding a server announced.
    #[must_use]
    pub fn from_name(name: &str) -> Self {
        match name {
            "utf-8" => Self::Utf8,
            "utf-32" => Self::Utf32,
            _ => Self::Utf16,
        }
    }

    /// A character column on `line` in the server's units.
    #[must_use]
    pub fn encode(self, line: &str, col: usize) -> usize {
        let chars = line.chars().take(col);
        match self {
            Self::Utf32 => col,
            Self::Utf16 => chars.map(char::len_utf16).sum(),
            Self::Utf8 => chars.map(char::len_utf8).sum(),
        }
    }

    /// A column in the server's units on `line` back as characters.
    #[must_use]
    pub fn decode(self, line: &str, units: usize) -> usize {
        if self == Self::Utf32 {
            return units.min(line.chars().count());
        }
        let mut used = 0;
        for (index, ch) in line.chars().enumerate() {
            if used >= units {
                return index;
            }
            used += match self {
                Self::Utf16 => ch.len_utf16(),
                _ => ch.len_utf8(),
            };
        }
        line.chars().count()
    }
}

/// `file://` URI for a path.
#[must_use]
pub fn uri(path: &Path) -> String {
    let absolute = path.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    });
    let text = absolute.to_string_lossy().replace('\\', "/");
    let mut encoded = String::from("file://");
    if !text.starts_with('/') {
        encoded.push('/');
    }
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The path a `file://` URI names.
#[must_use]
pub fn path_of(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%'
            && let Some(hex) = rest.get(index + 1..index + 3)
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            bytes.push(byte);
            index += 3;
            continue;
        }
        bytes.push(raw[index]);
        index += 1;
    }
    let text = String::from_utf8(bytes).ok()?;
    // `file:///C:/x` on Windows has a slash before the drive letter.
    let text = if cfg!(windows) {
        text.trim_start_matches('/').to_string()
    } else {
        text
    };
    Some(PathBuf::from(text))
}

/// A position as the server writes it.
#[must_use]
pub fn position(document: &Document, at: Position, encoding: Encoding) -> Value {
    let line = document.line_string(at.line);
    json!({ "line": at.line, "character": encoding.encode(&line, at.col) })
}

/// A position the server wrote, in characters. Without the document to hand
/// the column is taken as it is.
#[must_use]
pub fn read_position(
    value: &Value,
    document: Option<&Document>,
    encoding: Encoding,
) -> Option<Position> {
    let line = usize::try_from(value.get("line")?.as_u64()?).ok()?;
    let units = usize::try_from(value.get("character")?.as_u64()?).ok()?;
    let col = match document {
        Some(document) if line <= document.last_line() => {
            encoding.decode(&document.line_string(line), units)
        }
        _ => units,
    };
    Some(Position::new(line, col))
}

/// A range the server wrote, in characters.
#[must_use]
pub fn read_range(
    value: &Value,
    document: Option<&Document>,
    encoding: Encoding,
) -> Option<(Position, Position)> {
    Some((
        read_position(value.get("start")?, document, encoding)?,
        read_position(value.get("end")?, document, encoding)?,
    ))
}

/// How serious a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Will not build.
    Error,
    /// Builds, but suspicious.
    Warning,
    /// For information.
    Information,
    /// A suggestion.
    Hint,
}

impl Severity {
    fn from_number(number: u64) -> Self {
        match number {
            1 => Self::Error,
            2 => Self::Warning,
            3 => Self::Information,
            _ => Self::Hint,
        }
    }

    /// One letter for the status bar.
    #[must_use]
    pub const fn letter(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warning => 'W',
            Self::Information => 'I',
            Self::Hint => 'H',
        }
    }
}

/// An error, warning or hint the server reported for a span of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Where it starts.
    pub start: Position,
    /// Where it ends.
    pub end: Position,
    /// How serious it is.
    pub severity: Severity,
    /// What it says.
    pub message: String,
    /// Which tool said it, such as `rustc` or `clippy`.
    pub source: Option<String>,
}

/// Read a `publishDiagnostics` list.
#[must_use]
pub fn read_diagnostics(
    list: &[Value],
    document: Option<&Document>,
    encoding: Encoding,
) -> Vec<Diagnostic> {
    let mut found: Vec<Diagnostic> = list
        .iter()
        .filter_map(|item| {
            let (start, end) = read_range(item.get("range")?, document, encoding)?;
            Some(Diagnostic {
                start,
                end,
                severity: Severity::from_number(
                    item.get("severity").and_then(Value::as_u64).unwrap_or(1),
                ),
                message: item.get("message")?.as_str()?.to_string(),
                source: item
                    .get("source")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect();
    found.sort_by_key(|diagnostic| (diagnostic.start, diagnostic.severity));
    found
}

/// A place in a file, as definitions and references are reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The file.
    pub path: PathBuf,
    /// Where in it, in the server's columns until a document is at hand.
    pub start: Value,
}

/// Read a `Location`, a `LocationLink`, or a list of either.
#[must_use]
pub fn read_locations(value: &Value) -> Vec<Location> {
    let items: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::Null => Vec::new(),
        single => vec![single],
    };
    items
        .into_iter()
        .filter_map(|item| {
            let uri = item
                .get("uri")
                .or_else(|| item.get("targetUri"))?
                .as_str()?;
            let range = item
                .get("range")
                .or_else(|| item.get("targetSelectionRange"))
                .or_else(|| item.get("targetRange"))?;
            Some(Location {
                path: path_of(uri)?,
                start: range.get("start")?.clone(),
            })
        })
        .collect()
}

/// One edit to a file's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEdit {
    /// The span replaced, as the server wrote it.
    pub range: Value,
    /// What goes there.
    pub text: String,
}

/// Read a list of `TextEdit`s.
#[must_use]
pub fn read_edits(value: &Value) -> Vec<TextEdit> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(TextEdit {
                        range: item.get("range")?.clone(),
                        text: item.get("newText")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Read a `WorkspaceEdit` as edits per file, in either of its two shapes.
#[must_use]
pub fn read_workspace_edit(value: &Value) -> Vec<(PathBuf, Vec<TextEdit>)> {
    let mut files = Vec::new();
    if let Some(changes) = value.get("changes").and_then(Value::as_object) {
        for (uri, edits) in changes {
            if let Some(path) = path_of(uri) {
                files.push((path, read_edits(edits)));
            }
        }
    }
    if let Some(changes) = value.get("documentChanges").and_then(Value::as_array) {
        for change in changes {
            let Some(uri) = change
                .get("textDocument")
                .and_then(|document| document.get("uri"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            if let (Some(path), Some(edits)) = (path_of(uri), change.get("edits")) {
                files.push((path, read_edits(edits)));
            }
        }
    }
    files
}

/// One completion offered by the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    /// What the menu shows.
    pub label: String,
    /// The extra column: a type, a signature, a module.
    pub detail: String,
    /// What is inserted, with snippet placeholders reduced to their text.
    pub insert: String,
    /// Where the inserted text starts, when the server said.
    pub replace_from: Option<Value>,
    /// What typing filters against.
    pub filter: String,
}

/// Read a completion response: a list, or a `CompletionList`.
#[must_use]
pub fn read_completions(value: &Value) -> Vec<CompletionItem> {
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .or_else(|| value.as_array());
    let Some(items) = items else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let label = item.get("label")?.as_str()?.to_string();
            let edit = item.get("textEdit");
            let raw = edit
                .and_then(|edit| edit.get("newText"))
                .or_else(|| item.get("insertText"))
                .and_then(Value::as_str)
                .unwrap_or(&label);
            let snippet = item.get("insertTextFormat").and_then(Value::as_u64) == Some(2);
            let insert = if snippet {
                strip_snippet(raw)
            } else {
                raw.to_string()
            };
            let replace_from = edit
                .and_then(|edit| edit.get("range").or_else(|| edit.get("insert")))
                .and_then(|range| range.get("start"))
                .cloned();
            Some(CompletionItem {
                detail: item
                    .get("detail")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                filter: item
                    .get("filterText")
                    .and_then(Value::as_str)
                    .unwrap_or(&label)
                    .to_string(),
                label,
                insert,
                replace_from,
            })
        })
        .collect()
}

/// A snippet as plain text: `${1:name}` becomes `name`, `$1` disappears.
#[must_use]
pub fn strip_snippet(snippet: &str) -> String {
    let mut out = String::with_capacity(snippet.len());
    let chars: Vec<char> = snippet.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '\\' if index + 1 < chars.len() => {
                out.push(chars[index + 1]);
                index += 2;
            }
            '$' if chars.get(index + 1) == Some(&'{') => {
                // `${1:default}` keeps the default; `${1}` keeps nothing.
                let mut depth = 0;
                let mut end = index + 1;
                while end < chars.len() {
                    match chars[end] {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    end += 1;
                }
                let inner: String = chars[index + 2..end.min(chars.len())].iter().collect();
                if let Some((_, default)) = inner.split_once(':') {
                    out.push_str(&strip_snippet(default));
                }
                index = end + 1;
            }
            '$' if chars.get(index + 1).is_some_and(char::is_ascii_digit) => {
                index += 1;
                while chars.get(index).is_some_and(char::is_ascii_digit) {
                    index += 1;
                }
            }
            ch => {
                out.push(ch);
                index += 1;
            }
        }
    }
    out
}

/// The text of a hover response, whichever of its three shapes it came in.
#[must_use]
pub fn hover_text(value: &Value) -> Option<String> {
    let contents = value.get("contents")?;
    let piece = |item: &Value| -> Option<String> {
        match item {
            Value::String(text) => Some(text.clone()),
            Value::Object(object) => object
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        }
    };
    let text = match contents {
        Value::Array(items) => items
            .iter()
            .filter_map(piece)
            .collect::<Vec<_>>()
            .join("\n\n"),
        other => piece(other)?,
    };
    // Code fences are noise in a plain-text popup.
    let text: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect();
    let text = text.join("\n").trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_convert_between_encodings() {
        let line = "a𝄞b";
        assert_eq!(Encoding::Utf16.encode(line, 2), 3);
        assert_eq!(Encoding::Utf16.decode(line, 3), 2);
        assert_eq!(Encoding::Utf8.encode(line, 2), 5);
        assert_eq!(Encoding::Utf8.decode(line, 5), 2);
        assert_eq!(Encoding::Utf32.decode(line, 2), 2);
    }

    #[test]
    fn uris_round_trip_with_awkward_characters() {
        let path = std::env::temp_dir().join("a dir/ağaç.rs");
        let uri = uri(&path);
        assert!(uri.starts_with("file://"));
        assert!(uri.contains("%20"));
        assert_eq!(path_of(&uri), Some(path));
    }

    #[test]
    fn snippets_become_plain_text() {
        assert_eq!(strip_snippet("push(${1:value})$0"), "push(value)");
        assert_eq!(strip_snippet("fn ${1:name}(${2}) {$0}"), "fn name() {}");
        assert_eq!(strip_snippet(r"cost \$5"), "cost $5");
    }

    #[test]
    fn hover_text_comes_in_three_shapes() {
        assert_eq!(
            hover_text(&json!({"contents": "plain"})).as_deref(),
            Some("plain")
        );
        assert_eq!(
            hover_text(
                &json!({"contents": {"kind": "markdown", "value": "```rust\nfn f()\n```\ndoc"}})
            )
            .as_deref(),
            Some("fn f()\ndoc")
        );
        assert_eq!(
            hover_text(&json!({"contents": [{"language": "c", "value": "int x"}, "note"]}))
                .as_deref(),
            Some("int x\n\nnote")
        );
    }

    #[test]
    fn workspace_edits_in_both_shapes() {
        let edit = json!({"newText": "x", "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}});
        let changes = json!({"changes": {"file:///tmp/a.rs": [edit.clone()]}});
        assert_eq!(
            read_workspace_edit(&changes)[0].0,
            PathBuf::from("/tmp/a.rs")
        );
        let document_changes = json!({"documentChanges": [{"textDocument": {"uri": "file:///tmp/b.rs", "version": 1}, "edits": [edit]}]});
        assert_eq!(read_workspace_edit(&document_changes)[0].1.len(), 1);
    }
}
