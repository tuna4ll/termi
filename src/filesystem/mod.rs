//! # Filesystem
//!
//! **Purpose:** every read from and write to disk goes through here.
//!
//! **Responsibility:** turn raw I/O into `anyhow` errors carrying the path (a
//! bare "access denied" is useless in a status bar), and make saving atomic so
//! that a crash mid-write cannot leave the user with a truncated source file.
//!
//! **Public API:** [`read_file`], [`read_text`], [`encode`], [`write_file`],
//! [`write_bytes`], [`is_writable`], [`create_file`], [`create_directory`],
//! [`Encoding`], [`tree::Tree`], [`editorconfig`], [`swap`].

pub mod editorconfig;
pub mod swap;
pub mod tree;
pub mod watcher;

use std::fs::{self, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result, bail};

/// How a file's text is stored as bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// UTF-8, the default for every new file.
    #[default]
    Utf8,
    /// UTF-8 with a byte-order mark, kept when a file already has one.
    Utf8Bom,
    /// ISO-8859-1: one byte per character. Every byte sequence is valid
    /// Latin-1, which makes it the fallback that opens anything that is not
    /// UTF-8 without losing a byte on the way back out.
    Latin1,
}

impl Encoding {
    /// The name shown in the status bar and accepted by `:set encoding`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf8Bom => "utf-8-bom",
            Self::Latin1 => "latin1",
        }
    }

    /// The encoding a name refers to, as `:set encoding` and `.editorconfig`
    /// spell them.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().replace('_', "-").as_str() {
            "utf-8" | "utf8" => Some(Self::Utf8),
            "utf-8-bom" | "utf8-bom" => Some(Self::Utf8Bom),
            "latin1" | "latin-1" | "iso-8859-1" => Some(Self::Latin1),
            _ => None,
        }
    }
}

const BOM: &[u8] = &[0xef, 0xbb, 0xbf];

/// How much of a file is inspected for the NUL bytes that give a binary away.
const BINARY_SNIFF: usize = 8 * 1024;

/// Read a text file in whichever encoding it is in.
///
/// # Errors
/// Returns an error if the file cannot be read or looks like a binary file.
pub fn read_text(path: &Path) -> Result<(String, Encoding)> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    decode(&bytes).with_context(|| format!("cannot open {}", path.display()))
}

/// Turn a file's bytes into text, working out the encoding on the way.
///
/// # Errors
/// Returns an error for binary data, which no text encoding can round-trip.
pub fn decode(bytes: &[u8]) -> Result<(String, Encoding)> {
    if bytes.iter().take(BINARY_SNIFF).any(|byte| *byte == 0) {
        bail!("it looks like a binary file");
    }
    if let Some(rest) = bytes.strip_prefix(BOM)
        && let Ok(text) = std::str::from_utf8(rest)
    {
        return Ok((text.to_string(), Encoding::Utf8Bom));
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok((text.to_string(), Encoding::Utf8)),
        Err(_) => Ok((
            bytes.iter().map(|byte| char::from(*byte)).collect(),
            Encoding::Latin1,
        )),
    }
}

/// Turn text back into the bytes `encoding` stores it as.
///
/// # Errors
/// Returns an error naming the first character Latin-1 cannot hold.
pub fn encode(text: &str, encoding: Encoding) -> Result<Vec<u8>> {
    Ok(match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => {
            let mut bytes = BOM.to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes
        }
        Encoding::Latin1 => {
            let mut bytes = Vec::with_capacity(text.len());
            for ch in text.chars() {
                let Ok(byte) = u8::try_from(u32::from(ch)) else {
                    bail!(
                        "{ch:?} (U+{:04X}) cannot be saved as latin1 — :set encoding=utf-8 first",
                        u32::from(ch)
                    );
                };
                bytes.push(byte);
            }
            bytes
        }
    })
}

/// Whether the current user can write to `path`.
///
/// Asked by opening the file for appending, which changes nothing: the
/// permission bits alone cannot answer it, since ownership, groups and ACLs
/// all have a say.
#[must_use]
pub fn is_writable(path: &Path) -> bool {
    if !path.exists() {
        return path
            .parent()
            .is_none_or(|parent| parent.as_os_str().is_empty() || parent.exists());
    }
    OpenOptions::new().append(true).open(path).is_ok()
}

/// Read a UTF-8 file into memory.
///
/// # Errors
/// Returns an error if the file cannot be read or is not valid UTF-8.
pub fn read_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    String::from_utf8(bytes).with_context(|| format!("{} is not valid UTF-8", path.display()))
}

/// Create a new, empty file without replacing an existing one.
///
/// # Errors
/// Returns an error when the path already exists or its parent is not writable.
pub fn create_file(path: &Path) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    Ok(())
}

/// Create one directory without filling in missing parents.
///
/// # Errors
/// Returns an error when the path already exists, its parent does not exist, or
/// its parent is not writable.
pub fn create_directory(path: &Path) -> Result<()> {
    fs::create_dir(path).with_context(|| format!("cannot create {}", path.display()))
}

pub fn move_path(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        bail!("{} already exists", destination.display());
    }
    fs::rename(source, destination).with_context(|| {
        format!(
            "cannot move {} to {}",
            source.display(),
            destination.display()
        )
    })
}

pub fn copy_path(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        bail!("{} already exists", destination.display());
    }
    if source.is_dir() && destination.starts_with(source) {
        bail!("cannot copy a directory inside itself");
    }
    if source.is_dir() {
        copy_directory(source, destination)
    } else {
        fs::copy(source, destination).map(|_| ()).with_context(|| {
            format!(
                "cannot copy {} to {}",
                source.display(),
                destination.display()
            )
        })
    }
}

pub fn delete_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .with_context(|| format!("cannot delete {}", path.display()))
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)
        .with_context(|| format!("cannot create {}", destination.display()))?;
    for entry in
        fs::read_dir(source).with_context(|| format!("cannot read {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("cannot read {}", source.display()))?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if entry
            .file_type()
            .with_context(|| format!("cannot inspect {}", from.display()))?
            .is_dir()
        {
            copy_directory(&from, &to)?;
        } else {
            fs::copy(&from, &to)
                .with_context(|| format!("cannot copy {} to {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// Write `contents` to `path` atomically.
///
/// The data lands in a sibling temporary file first and is then renamed over the
/// target. `rename` within a directory is atomic on every platform we support,
/// so a reader either sees the old file or the new one — never a half-written
/// one.
///
/// # Errors
/// Returns an error if the temporary file cannot be written or renamed.
pub fn write_file(path: &Path, contents: &str) -> Result<()> {
    write_bytes(path, contents.as_bytes())
}

/// [`write_file`] for bytes that are already encoded.
///
/// The replacement takes over the permissions of the file it replaces, so
/// saving a script does not quietly make it non-executable.
///
/// # Errors
/// Returns an error if the temporary file cannot be written or renamed.
pub fn write_bytes(path: &Path, contents: &[u8]) -> Result<()> {
    let temp = temp_path(path);
    fs::write(&temp, contents).with_context(|| format!("cannot write {}", temp.display()))?;
    if let Ok(metadata) = fs::metadata(path) {
        let _ = fs::set_permissions(&temp, metadata.permissions());
    }
    fs::rename(&temp, path)
        .with_context(|| format!("cannot replace {}", path.display()))
        .inspect_err(|_| {
            // Do not leave debris behind if the rename failed.
            let _ = fs::remove_file(&temp);
        })
}

/// A sibling path for the staging file used by [`write_file`].
///
/// It must live in the same directory as the target, otherwise the rename would
/// cross a filesystem boundary and stop being atomic.
fn temp_path(path: &Path) -> std::path::PathBuf {
    let mut name = std::ffi::OsString::from(".");
    name.push(path.file_name().unwrap_or_else(|| "termi".as_ref()));
    name.push(".termi-tmp");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("termi-fs-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).expect("create fixture");
        root
    }

    #[test]
    fn text_round_trips_through_every_encoding() {
        for (bytes, encoding) in [
            (b"plain".to_vec(), Encoding::Utf8),
            ([BOM, "bom".as_bytes()].concat(), Encoding::Utf8Bom),
            (vec![b'c', 0xe9, b'!'], Encoding::Latin1),
        ] {
            let (text, found) = decode(&bytes).expect("decodes");
            assert_eq!(found, encoding);
            assert_eq!(encode(&text, found).expect("encodes"), bytes);
        }
        assert!(decode(b"elf\0\0data").is_err());
        assert!(encode("ğ", Encoding::Latin1).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn saving_keeps_the_permissions_of_the_file_it_replaces() {
        use std::os::unix::fs::PermissionsExt;
        let root = fixture("keep-permissions");
        let path = root.join("run.sh");
        fs::write(&path, "echo old").expect("write fixture");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");

        write_file(&path, "echo new").expect("save");
        let mode = fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn creates_new_files_without_overwriting() {
        let root = fixture("create-file");
        let path = root.join("notes.txt");

        create_file(&path).expect("create file");
        fs::write(&path, "keep me").expect("write fixture");

        assert!(create_file(&path).is_err());
        assert_eq!(fs::read_to_string(path).expect("read file"), "keep me");
    }

    #[test]
    fn creates_one_directory() {
        let root = fixture("create-directory");
        let path = root.join("drafts");

        create_directory(&path).expect("create directory");

        assert!(path.is_dir());
        assert!(create_directory(&path).is_err());
    }

    #[test]
    fn moves_without_replacing_the_destination() {
        let root = fixture("move");
        let source = root.join("old.txt");
        let destination = root.join("new.txt");
        fs::write(&source, "old").expect("write fixture");

        move_path(&source, &destination).expect("move file");
        assert_eq!(fs::read_to_string(&destination).expect("read file"), "old");

        fs::write(&source, "another").expect("write fixture");
        assert!(move_path(&source, &destination).is_err());
        assert_eq!(fs::read_to_string(&destination).expect("read file"), "old");
    }

    #[test]
    fn copies_directories_recursively() {
        let root = fixture("copy-directory");
        let source = root.join("source");
        let destination = root.join("copy");
        fs::create_dir(&source).expect("create fixture");
        fs::write(source.join("note.txt"), "hello").expect("write fixture");

        copy_path(&source, &destination).expect("copy directory");

        assert_eq!(
            fs::read_to_string(destination.join("note.txt")).expect("read copy"),
            "hello"
        );
    }

    #[test]
    fn refuses_to_copy_a_directory_inside_itself() {
        let root = fixture("copy-inside-itself");
        let source = root.join("source");
        fs::create_dir(&source).expect("create fixture");

        assert!(copy_path(&source, &source.join("nested")).is_err());
        assert!(!source.join("nested").exists());
    }

    #[test]
    fn deletes_files_and_nonempty_directories() {
        let root = fixture("delete");
        let file = root.join("note.txt");
        let directory = root.join("folder");
        fs::write(&file, "hello").expect("write fixture");
        fs::create_dir(&directory).expect("create fixture");
        fs::write(directory.join("nested.txt"), "hello").expect("write fixture");

        delete_path(&file).expect("delete file");
        delete_path(&directory).expect("delete directory");

        assert!(!file.exists());
        assert!(!directory.exists());
    }
}
