//! The wire: JSON-RPC messages framed with `Content-Length` headers, over a
//! server's standard input and output.
//!
//! Reading happens on a thread of its own that blocks on the server's output
//! and hands whole messages to the editor through a channel, so a slow or
//! silent server never stalls a keystroke.

use std::io::{BufRead, BufReader, Read, Write};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use anyhow::{Context, Result};
use serde_json::Value;

/// One end of a connection to a server.
pub struct Transport {
    writer: Box<dyn Write + Send>,
    incoming: Receiver<Value>,
    closed: bool,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport")
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl Transport {
    /// Talk to a server through `reader` and `writer` — a child process's
    /// pipes, or in tests, a server running on a thread.
    pub fn new(reader: impl Read + Send + 'static, writer: Box<dyn Write + Send>) -> Self {
        let (sender, incoming) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(reader);
            while let Some(message) = read_message(&mut reader) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        Self {
            writer,
            incoming,
            closed: false,
        }
    }

    /// Send one message.
    ///
    /// # Errors
    /// Returns an error once the server has stopped reading.
    pub fn send(&mut self, message: &Value) -> Result<()> {
        let body = serde_json::to_string(message)?;
        write!(self.writer, "Content-Length: {}\r\n\r\n{body}", body.len())
            .and_then(|()| self.writer.flush())
            .context("the language server stopped listening")
    }

    /// Every message that has arrived since the last call, without waiting.
    pub fn receive(&mut self) -> Vec<Value> {
        let mut messages = Vec::new();
        loop {
            match self.incoming.try_recv() {
                Ok(message) => messages.push(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.closed = true;
                    break;
                }
            }
        }
        messages
    }

    /// Whether the server's output has ended.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.closed
    }
}

/// Read one framed message, or `None` once the stream ends.
pub fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    loop {
        let mut length = None;
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header).ok()? == 0 {
                return None;
            }
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let Some(length) = length else {
            continue;
        };
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        // A message that is not JSON is skipped, not fatal: the next one may
        // well be fine.
        if let Ok(message) = serde_json::from_slice(&body) {
            return Some(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_are_framed_and_unframed() {
        let (reader, mut writer) = std::io::pipe().expect("pipe");
        let (_, sink) = std::io::pipe().expect("pipe");
        let mut transport = Transport::new(reader, Box::new(sink));
        let body = serde_json::to_string(&json!({"jsonrpc": "2.0", "method": "ping"})).unwrap();
        write!(writer, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        write!(writer, "Content-Type: x\r\nContent-Length: 2\r\n\r\n{{}}").unwrap();
        drop(writer);
        let mut received = Vec::new();
        for _ in 0..200 {
            received.extend(transport.receive());
            if transport.is_closed() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            received,
            vec![json!({"jsonrpc": "2.0", "method": "ping"}), json!({})]
        );
    }
}
