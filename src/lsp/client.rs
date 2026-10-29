//! One running language server and the conversation with it.
//!
//! The client keeps the bookkeeping the protocol asks for — request ids,
//! document versions, the handshake — so the editor only ever says "this file
//! changed" or "what is under the caret", and later picks the answers up as
//! [`Event`]s.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use super::protocol::{self, Encoding};
use super::transport::Transport;

/// How a server is started, from `[languages.<name>]` or the built-in list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// The program.
    pub command: String,
    /// Its arguments.
    #[serde(default)]
    pub args: Vec<String>,
}

/// What a request was for, so its answer can be put to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Initialize,
    Hover,
    Definition,
    References,
    Completion,
    Rename,
    Formatting,
}

/// Something the server said that the editor should act on.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The answer to a request about `path`.
    Response {
        kind: RequestKind,
        path: PathBuf,
        result: Value,
    },
    /// A request failed.
    Failed { kind: RequestKind, message: String },
    /// The full set of diagnostics for a file, replacing the last.
    Diagnostics { path: PathBuf, items: Vec<Value> },
    /// A message the server wants the user to see.
    Message(String),
}

/// An answer still to come.
#[derive(Debug)]
struct Pending {
    kind: RequestKind,
    path: PathBuf,
}

/// A connection to one language server.
#[derive(Debug)]
pub struct Client {
    transport: Transport,
    child: Option<Child>,
    /// The program, for `:lsp`.
    pub name: String,
    next_id: i64,
    pending: HashMap<i64, Pending>,
    /// Whether the handshake has finished.
    ready: bool,
    /// What was sent before it had, to go out as soon as it does.
    queued: Vec<Value>,
    capabilities: Value,
    /// How the server counts columns.
    pub encoding: Encoding,
    versions: HashMap<PathBuf, i64>,
}

impl Client {
    /// Start `config` in `root` and begin the handshake.
    ///
    /// # Errors
    /// Returns an error if the program cannot be found or started.
    pub fn spawn(config: &ServerConfig, root: &Path) -> Result<Self> {
        if find_program(&config.command).is_none() {
            bail!("{} is not installed", config.command);
        }
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("cannot start {}", config.command))?;
        let stdout = child.stdout.take().context("no output from the server")?;
        let stdin = child.stdin.take().context("no input to the server")?;
        let mut client = Self::over(
            Transport::new(stdout, Box::new(stdin)),
            &config.command,
            root,
        );
        client.child = Some(child);
        Ok(client)
    }

    /// A client over an existing connection, handshake begun.
    #[must_use]
    pub fn over(transport: Transport, name: &str, root: &Path) -> Self {
        let mut client = Self {
            transport,
            child: None,
            name: name.to_string(),
            next_id: 1,
            pending: HashMap::new(),
            ready: false,
            queued: Vec::new(),
            capabilities: Value::Null,
            encoding: Encoding::default(),
            versions: HashMap::new(),
        };
        let params = json!({
            "processId": std::process::id(),
            "clientInfo": { "name": "termi", "version": env!("CARGO_PKG_VERSION") },
            "rootUri": protocol::uri(root),
            "workspaceFolders": [{ "uri": protocol::uri(root), "name": root.file_name().map_or_else(|| "root".into(), |n| n.to_string_lossy().into_owned()) }],
            "capabilities": {
                "general": { "positionEncodings": ["utf-32", "utf-16"] },
                "textDocument": {
                    "synchronization": { "didSave": true },
                    "hover": { "contentFormat": ["plaintext", "markdown"] },
                    "completion": { "completionItem": { "snippetSupport": true } },
                    "publishDiagnostics": {},
                    "definition": {},
                    "references": {},
                    "rename": {},
                    "formatting": {}
                },
                "window": { "showMessage": {} }
            }
        });
        client.call("initialize", params, RequestKind::Initialize, root);
        client
    }

    fn send(&mut self, message: Value) {
        let handshake = message.get("method").and_then(Value::as_str) == Some("initialize");
        if self.ready || handshake {
            let _ = self.transport.send(&message);
        } else {
            self.queued.push(message);
        }
    }

    fn call(&mut self, method: &str, params: Value, kind: RequestKind, path: &Path) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(
            id,
            Pending {
                kind,
                path: path.to_path_buf(),
            },
        );
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        id
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Ask something about a position in a file.
    pub fn request(&mut self, kind: RequestKind, path: &Path, params: Value) -> i64 {
        let method = match kind {
            RequestKind::Hover => "textDocument/hover",
            RequestKind::Definition => "textDocument/definition",
            RequestKind::References => "textDocument/references",
            RequestKind::Completion => "textDocument/completion",
            RequestKind::Rename => "textDocument/rename",
            RequestKind::Formatting => "textDocument/formatting",
            RequestKind::Initialize => "initialize",
        };
        self.call(method, params, kind, path)
    }

    /// Whether a request of `kind` is already waiting for its answer.
    #[must_use]
    pub fn is_waiting(&self, kind: RequestKind) -> bool {
        self.pending.values().any(|pending| pending.kind == kind)
    }

    /// Tell the server a file is open, with its text.
    pub fn did_open(&mut self, path: &Path, language: &str, text: &str) {
        self.versions.insert(path.to_path_buf(), 1);
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": {
                "uri": protocol::uri(path), "languageId": language_id(language), "version": 1, "text": text
            }}),
        );
    }

    /// Whether the server has been told about `path`.
    #[must_use]
    pub fn is_open(&self, path: &Path) -> bool {
        self.versions.contains_key(path)
    }

    /// Send a file's whole new text.
    pub fn did_change(&mut self, path: &Path, text: &str) {
        let version = self.versions.entry(path.to_path_buf()).or_insert(1);
        *version += 1;
        let version = *version;
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": protocol::uri(path), "version": version },
                "contentChanges": [{ "text": text }]
            }),
        );
    }

    /// Tell the server a file was written.
    pub fn did_save(&mut self, path: &Path) {
        self.notify(
            "textDocument/didSave",
            json!({ "textDocument": { "uri": protocol::uri(path) } }),
        );
    }

    /// Tell the server a file was closed.
    pub fn did_close(&mut self, path: &Path) {
        if self.versions.remove(path).is_some() {
            self.notify(
                "textDocument/didClose",
                json!({ "textDocument": { "uri": protocol::uri(path) } }),
            );
        }
    }

    /// Whether the server offers a feature, by its capability name such as
    /// `hoverProvider`.
    #[must_use]
    pub fn supports(&self, capability: &str) -> bool {
        self.capabilities
            .get(capability)
            .is_some_and(|value| !matches!(value, Value::Null | Value::Bool(false)))
    }

    /// Characters that should open completion as soon as they are typed.
    #[must_use]
    pub fn trigger_characters(&self) -> Vec<char> {
        self.capabilities
            .pointer("/completionProvider/triggerCharacters")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .filter_map(|text| text.chars().next())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether the handshake has finished.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    /// Whether the server has gone away.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.transport.is_closed()
    }

    /// Everything the server has said since the last call.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for message in self.transport.receive() {
            self.handle(message, &mut events);
        }
        events
    }

    fn handle(&mut self, message: Value, events: &mut Vec<Event>) {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let id = message.get("id").cloned();
        match (method, id) {
            // A request from the server. The few servers use need an answer
            // to carry on; anything else is politely declined.
            (Some(method), Some(id)) => {
                let result = match method.as_str() {
                    "workspace/configuration" => {
                        let count = message
                            .pointer("/params/items")
                            .and_then(Value::as_array)
                            .map_or(0, Vec::len);
                        Value::Array(vec![Value::Null; count])
                    }
                    _ => Value::Null,
                };
                let _ = self
                    .transport
                    .send(&json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            }
            (Some(method), None) => match method.as_str() {
                "textDocument/publishDiagnostics" => {
                    let path = message
                        .pointer("/params/uri")
                        .and_then(Value::as_str)
                        .and_then(protocol::path_of);
                    let items = message
                        .pointer("/params/diagnostics")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    if let Some(path) = path {
                        events.push(Event::Diagnostics { path, items });
                    }
                }
                "window/showMessage" => {
                    if let Some(text) = message.pointer("/params/message").and_then(Value::as_str) {
                        events.push(Event::Message(text.to_string()));
                    }
                }
                _ => {}
            },
            (None, Some(id)) => {
                let Some(pending) = id.as_i64().and_then(|id| self.pending.remove(&id)) else {
                    return;
                };
                if let Some(error) = message.get("error") {
                    let text = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("request failed")
                        .to_string();
                    events.push(Event::Failed {
                        kind: pending.kind,
                        message: text,
                    });
                    return;
                }
                let result = message.get("result").cloned().unwrap_or(Value::Null);
                if pending.kind == RequestKind::Initialize {
                    self.finish_handshake(&result);
                    return;
                }
                events.push(Event::Response {
                    kind: pending.kind,
                    path: pending.path,
                    result,
                });
            }
            (None, None) => {}
        }
    }

    fn finish_handshake(&mut self, result: &Value) {
        self.capabilities = result.get("capabilities").cloned().unwrap_or(Value::Null);
        self.encoding = self
            .capabilities
            .get("positionEncoding")
            .and_then(Value::as_str)
            .map_or(Encoding::Utf16, Encoding::from_name);
        self.ready = true;
        let _ = self
            .transport
            .send(&json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));
        for message in std::mem::take(&mut self.queued) {
            let _ = self.transport.send(&message);
        }
    }

    /// Ask the server to finish, and make sure it does.
    pub fn stop(&mut self) {
        if self.ready {
            let _ = self
                .transport
                .send(&json!({ "jsonrpc": "2.0", "id": 0, "method": "shutdown" }));
            let _ = self
                .transport
                .send(&json!({ "jsonrpc": "2.0", "method": "exit" }));
        }
        if let Some(child) = self.child.as_mut() {
            // A well-behaved server exits on `exit`; one that does not is not
            // allowed to outlive the editor.
            for _ in 0..20 {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// The protocol's name for a language, where it differs from the editor's.
fn language_id(language: &str) -> &str {
    match language {
        "shell" => "shellscript",
        "cpp" => "cpp",
        other => other,
    }
}

/// Where `program` is on the `PATH`, if anywhere.
#[must_use]
pub fn find_program(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".into())
            .split(';')
            .map(str::to_string)
            .chain(std::iter::once(String::new()))
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        extensions.iter().find_map(|extension| {
            let path = directory.join(format!("{program}{extension}"));
            path.is_file().then_some(path)
        })
    })
}
