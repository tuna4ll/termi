//! # Language servers
//!
//! **Purpose:** borrow a compiler's understanding of the code — errors as they
//! are typed, completions, hover documentation, definitions, references,
//! renames and formatting — from whatever language server the user has.
//!
//! **Responsibility:** the protocol and nothing else. [`client`] runs one
//! server and keeps the conversation with it straight, [`transport`] frames
//! the messages, and [`protocol`] reads and writes the few shapes involved.
//! When to start a server, which buffer it hears about, and what the editor
//! does with an answer is the application's business, in `app::lsp`.
//!
//! **Public API:** [`Client`], [`ServerConfig`], [`Event`], [`RequestKind`],
//! [`default_server`].

pub mod client;
pub mod protocol;
pub mod transport;

pub use client::{Client, Event, RequestKind, ServerConfig};

/// The server most people use for a language, started when the config does
/// not name one and the program is installed.
#[must_use]
pub fn default_server(language: &str) -> Option<ServerConfig> {
    let (command, args): (&str, &[&str]) = match language {
        "rust" => ("rust-analyzer", &[]),
        "c" | "cpp" => ("clangd", &[]),
        "python" => ("pylsp", &[]),
        "go" => ("gopls", &[]),
        "javascript" | "typescript" => ("typescript-language-server", &["--stdio"]),
        "zig" => ("zls", &[]),
        "lua" => ("lua-language-server", &[]),
        "shell" => ("bash-language-server", &["start"]),
        "json" => ("vscode-json-language-server", &["--stdio"]),
        "html" => ("vscode-html-language-server", &["--stdio"]),
        "css" => ("vscode-css-language-server", &["--stdio"]),
        "yaml" => ("yaml-language-server", &["--stdio"]),
        "toml" => ("taplo", &["lsp", "stdio"]),
        "markdown" => ("marksman", &["server"]),
        _ => return None,
    };
    Some(ServerConfig {
        command: command.to_string(),
        args: args.iter().map(ToString::to_string).collect(),
    })
}

/// A language server played by a thread, for tests that need one to answer.
#[cfg(test)]
pub mod mock {
    use std::io::Write;

    use serde_json::{Value, json};

    use super::transport::{Transport, read_message};

    fn reply(writer: &mut impl Write, message: &Value) {
        let body = serde_json::to_string(message).expect("serialisable");
        let _ = write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len());
        let _ = writer.flush();
    }

    /// A transport connected to a fake server that knows a little about
    /// every request the editor makes.
    #[must_use]
    pub fn server() -> Transport {
        let (server_in, client_out) = std::io::pipe().expect("pipe");
        let (client_in, mut server_out) = std::io::pipe().expect("pipe");
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(server_in);
            while let Some(message) = read_message(&mut reader) {
                let method = message.get("method").and_then(Value::as_str).unwrap_or("");
                let id = message.get("id").cloned();
                let uri = message
                    .pointer("/params/textDocument/uri")
                    .cloned()
                    .unwrap_or(Value::Null);
                let span = |line: u64, from: u64, to: u64| json!({"start": {"line": line, "character": from}, "end": {"line": line, "character": to}});
                let result = match method {
                    "initialize" => json!({"capabilities": {
                        "positionEncoding": "utf-16",
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "renameProvider": true,
                        "documentFormattingProvider": true,
                        "completionProvider": {"triggerCharacters": ["."]},
                        "textDocumentSync": 1
                    }}),
                    "textDocument/didOpen" => {
                        reply(
                            &mut server_out,
                            &json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                            "params": {"uri": uri, "diagnostics": [
                                {"range": span(0, 0, 3), "severity": 1, "message": "bad thing", "source": "mock"}
                            ]}}),
                        );
                        continue;
                    }
                    "textDocument/hover" => {
                        json!({"contents": {"kind": "plaintext", "value": "a hover"}})
                    }
                    "textDocument/definition" => json!({"uri": uri, "range": span(1, 2, 3)}),
                    "textDocument/references" => json!([
                        {"uri": uri, "range": span(0, 0, 1)},
                        {"uri": uri, "range": span(1, 0, 1)}
                    ]),
                    "textDocument/completion" => json!({"isIncomplete": false, "items": [
                        {"label": "alpha"},
                        {"label": "alphabet", "detail": "fn() -> Alphabet"},
                        {"label": "beta"},
                        {"label": "push", "insertText": "push(${1:value})", "insertTextFormat": 2}
                    ]}),
                    "textDocument/rename" => {
                        let name = message
                            .pointer("/params/newName")
                            .cloned()
                            .unwrap_or(Value::Null);
                        let key = uri.as_str().unwrap_or_default().to_string();
                        json!({"changes": {key: [{"range": span(0, 0, 3), "newText": name}]}})
                    }
                    "textDocument/formatting" => {
                        json!([{"range": span(0, 0, 0), "newText": "// formatted\n"}])
                    }
                    _ => Value::Null,
                };
                if let Some(id) = id {
                    reply(
                        &mut server_out,
                        &json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    );
                }
            }
        });
        Transport::new(client_in, Box::new(client_out))
    }
}
