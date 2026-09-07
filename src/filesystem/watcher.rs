//! # File watching
//!
//! **Purpose:** notice when an open file changes underneath the editor.
//!
//! **Responsibility:** wrap [`notify`] in a non-blocking queue of changed paths.
//! The event loop drains it once per iteration, so watching never blocks input
//! and the editor keeps working if the watcher cannot start at all — on some
//! filesystems it simply cannot.
//!
//! What to *do* about a change is the application's decision, not this module's:
//! it only reports which paths moved.
//!
//! **Public API:** [`Watcher`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as NotifyWatcher};

/// Watches the files backing the open buffers.
pub struct Watcher {
    /// Dropping this stops the background thread, so it must be kept alive.
    inner: RecommendedWatcher,
    events: Receiver<notify::Result<notify::Event>>,
    watched: HashSet<PathBuf>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher")
            .field("watched", &self.watched.len())
            .finish()
    }
}

impl Watcher {
    /// Start a watcher, or `None` when the platform will not provide one.
    #[must_use]
    pub fn new() -> Option<Self> {
        let (sender, events) = channel();
        let inner = notify::recommended_watcher(sender).ok()?;
        Some(Self {
            inner,
            events,
            watched: HashSet::new(),
        })
    }

    /// Start watching the file at `path`, if it is not already watched.
    ///
    /// The *parent directory* is watched rather than the file itself: many
    /// editors (this one included) save by writing a temporary file and renaming
    /// it over the target, which replaces the inode and would silently detach a
    /// file-level watch.
    pub fn watch(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            self.watch_directory(parent);
        }
    }

    /// Start watching `directory` itself, if it is not already watched.
    ///
    /// Non-recursive on purpose: the caller watches the directories it actually
    /// shows, so the number of watches follows what is on screen rather than the
    /// size of the project.
    pub fn watch_directory(&mut self, directory: &Path) {
        let directory = directory.to_path_buf();
        if !self.watched.insert(directory.clone()) {
            return;
        }
        if self
            .inner
            .watch(&directory, RecursiveMode::NonRecursive)
            .is_err()
        {
            self.watched.remove(&directory);
        }
    }

    /// Take every path reported since the last call.
    ///
    /// Never blocks. Duplicate reports for one path are collapsed, because a
    /// single save typically produces several filesystem events.
    pub fn drain(&mut self) -> Vec<PathBuf> {
        let mut changed: Vec<PathBuf> = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(Ok(event)) => {
                    if !matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    ) {
                        continue;
                    }
                    for path in event.paths {
                        if !changed.contains(&path) {
                            changed.push(path);
                        }
                    }
                }
                // A watcher error is not worth interrupting the user for; the
                // next event will either arrive or it will not.
                Ok(Err(_)) => {}
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
        changed
    }
}
