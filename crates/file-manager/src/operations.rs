//! The filesystem operations a project's file explorer performs.
//!
//! Every function here takes the project root plus a *client-supplied* relative
//! string, and the first thing it does is turn that string into a [`SafePath`].
//! There is deliberately no variant that accepts an already-trusted path: a
//! caller cannot skip validation by holding on to one from an earlier request,
//! because the containment check is re-run against the live filesystem
//! immediately before the operation ([`SafePath::verify_still_within`]).
//!
//! Symbolic links are never followed. On Windows that also covers junctions and
//! mount points, which `symlink_metadata` reports as reparse points. A link
//! inside a project is listed, and is refused as the target of anything else —
//! following one is the cheapest way out of a sandbox and there is no legitimate
//! use for it in a project directory the product itself manages.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::safe_path::{PathError, SafePath};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FileError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("`{0}` does not exist")]
    NotFound(String),
    #[error("`{0}` already exists")]
    AlreadyExists(String),
    #[error("`{0}` is a directory")]
    NotAFile(String),
    #[error("`{0}` is not a directory")]
    NotADirectory(String),
    #[error("`{path}` is {size} bytes, above the {limit} byte limit for this operation")]
    TooLarge { path: String, size: u64, limit: u64 },
    #[error("`{0}` is a binary file and cannot be edited as text")]
    Binary(String),
    #[error("a directory cannot be moved or copied into itself")]
    IntoItself,
    #[error("{0}")]
    Refused(&'static str),
    #[error("the directory is not empty")]
    NotEmpty,
    #[error("filesystem error: {0}")]
    Io(String),
}

impl FileError {
    fn io(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// Ceilings that keep one request from exhausting the agent.
///
/// The editor limit is far below the download limit on purpose: reading a file
/// into an editor buffer costs memory in the desktop client as well as here,
/// while a download streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileLimits {
    /// Largest file the text editor will open.
    pub max_editable_bytes: u64,
    /// Largest single upload accepted into a project.
    pub max_upload_bytes: u64,
    /// Entries returned for one directory before the listing is truncated.
    pub max_listing_entries: usize,
    /// Matches returned by one search.
    pub max_search_results: usize,
    /// Directories a recursive walk (search, copy, size) will visit.
    pub max_walk_entries: usize,
}

impl Default for FileLimits {
    fn default() -> Self {
        Self {
            max_editable_bytes: 4 * 1024 * 1024,
            max_upload_bytes: 512 * 1024 * 1024,
            max_listing_entries: 5_000,
            max_search_results: 500,
            max_walk_entries: 200_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    /// A symbolic link, junction, device or socket. Listed so the user can see
    /// and delete it; refused as the target of every other operation.
    Other,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Other => "other",
        }
    }
}

/// One row in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    /// Relative to the project root, forward-slashed on every platform.
    pub path: String,
    pub kind: EntryKind,
    pub size_bytes: u64,
    pub modified_unix_ms: Option<i64>,
    /// True for links and other reparse points, which are also [`EntryKind::Other`].
    pub is_symlink: bool,
}

/// A directory, plus whether the listing was cut short.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub path: String,
    pub entries: Vec<FileEntry>,
    pub truncated: bool,
}

/// Progress for a local filesystem import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalImportProgress {
    pub copied_bytes: u64,
    pub total_bytes: u64,
    pub copied_files: u64,
    pub total_files: u64,
    pub current_path: String,
}

/// What a completed local filesystem import placed into the project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalImportReport {
    pub entries: Vec<FileEntry>,
    pub total_files: u64,
    pub total_bytes: u64,
    /// Symbolic links and junctions found in the source and not copied.
    ///
    /// A count rather than a list because `node_modules` alone can hold
    /// thousands, and an import that reported every one of them would be
    /// reporting noise. Zero is the ordinary case and says nothing.
    pub skipped_symlinks: u64,
    /// A few of them by name, so the report can point at something real.
    pub skipped_symlink_examples: Vec<String>,
}

/// The content of a text file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFile {
    pub path: String,
    pub text: String,
    pub size_bytes: u64,
    /// The language hint the editor should use, derived from the extension.
    pub language: &'static str,
}

fn modified_ms(metadata: &std::fs::Metadata) -> Option<i64> {
    let modified = metadata.modified().ok()?;
    let since_epoch = modified.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_millis()).ok()
}

fn kind_of(metadata: &std::fs::Metadata) -> EntryKind {
    if metadata.is_symlink() {
        EntryKind::Other
    } else if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// `symlink_metadata`, so a link is reported as a link rather than as whatever
/// it points at.
fn entry_from(relative: &str, name: &str, absolute: &Path) -> Result<FileEntry, FileError> {
    let metadata = std::fs::symlink_metadata(absolute).map_err(FileError::io)?;
    Ok(FileEntry {
        name: name.to_string(),
        path: relative.to_string(),
        kind: kind_of(&metadata),
        size_bytes: if metadata.is_dir() { 0 } else { metadata.len() },
        modified_unix_ms: modified_ms(&metadata),
        is_symlink: metadata.is_symlink(),
    })
}

/// Resolve a request path and confirm it still points where it did.
fn resolve(root: &Path, relative: &str) -> Result<SafePath, FileError> {
    let safe = if relative.is_empty() || relative == "." || relative == "/" {
        SafePath::root(root)?
    } else {
        SafePath::new(root, relative)?
    };
    safe.verify_still_within(root)?;
    Ok(safe)
}

/// The last component of a relative path, or the root's own display name.
fn name_of(relative: &str) -> String {
    relative
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(".")
        .to_string()
}

fn require_kind(safe: &SafePath, expected: EntryKind) -> Result<std::fs::Metadata, FileError> {
    let metadata = std::fs::symlink_metadata(safe.absolute())
        .map_err(|_| FileError::NotFound(safe.relative().to_string()))?;
    if metadata.is_symlink() {
        return Err(FileError::Refused(
            "symbolic links are not followed; delete the link instead",
        ));
    }
    match (expected, kind_of(&metadata)) {
        (EntryKind::Directory, EntryKind::Directory) => Ok(metadata),
        (EntryKind::File, EntryKind::File) => Ok(metadata),
        (EntryKind::Directory, _) => Err(FileError::NotADirectory(safe.relative().to_string())),
        (EntryKind::File, EntryKind::Directory) => {
            Err(FileError::NotAFile(safe.relative().to_string()))
        }
        (EntryKind::File, _) => Err(FileError::Refused(
            "only regular files and directories can be operated on",
        )),
        (EntryKind::Other, _) => Err(FileError::Refused("unsupported entry kind")),
    }
}

/// List one directory. Directories first, then files, each alphabetically —
/// stable ordering matters because the client renders this without re-sorting.
pub fn list_directory(
    root: &Path,
    relative: &str,
    limits: &FileLimits,
) -> Result<Listing, FileError> {
    let safe = resolve(root, relative)?;
    require_kind(&safe, EntryKind::Directory)?;

    let mut entries = Vec::new();
    let mut truncated = false;

    for item in std::fs::read_dir(safe.absolute()).map_err(FileError::io)? {
        let item = item.map_err(FileError::io)?;
        let name = item.file_name().to_string_lossy().to_string();

        // A name that would fail validation cannot be addressed by a later
        // request, so listing it as usable would be a lie. It is skipped: it
        // can only have arrived from outside the product.
        let child = match safe.join(root, &name) {
            Ok(child) => child,
            // A link whose target leaves the project is still shown. Hiding it
            // would leave an entry the user can neither inspect nor delete,
            // and `entry_from` stats the link rather than the target, so
            // nothing here follows it.
            Err(PathError::Escape) => match safe.join_no_follow(root, &name) {
                Ok(child) => child,
                Err(_) => continue,
            },
            Err(_) => continue,
        };

        if entries.len() >= limits.max_listing_entries {
            truncated = true;
            break;
        }

        match entry_from(child.relative(), &name, child.absolute()) {
            Ok(entry) => entries.push(entry),
            // Vanished between read_dir and stat. Routine in a live project.
            Err(FileError::Io(_)) => continue,
            Err(other) => return Err(other),
        }
    }

    entries.sort_by(|a, b| {
        let rank = |kind: EntryKind| match kind {
            EntryKind::Directory => 0,
            EntryKind::File => 1,
            EntryKind::Other => 2,
        };
        rank(a.kind)
            .cmp(&rank(b.kind))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });

    Ok(Listing {
        path: safe.relative().to_string(),
        entries,
        truncated,
    })
}

/// Metadata for a single entry.
pub fn stat(root: &Path, relative: &str) -> Result<FileEntry, FileError> {
    let safe = resolve(root, relative)?;
    if !safe.absolute().exists() && std::fs::symlink_metadata(safe.absolute()).is_err() {
        return Err(FileError::NotFound(safe.relative().to_string()));
    }
    entry_from(safe.relative(), &name_of(safe.relative()), safe.absolute())
}

/// A file is binary if it contains a NUL in its first block, or if the bytes
/// are not valid UTF-8.
///
/// Checking a prefix rather than the whole file is the standard heuristic and is
/// what `git` does; a text file with a NUL a megabyte in is pathological enough
/// that refusing to open it in an editor is the right outcome anyway.
pub fn looks_binary(bytes: &[u8]) -> bool {
    const SNIFF: usize = 8000;
    let head = if bytes.len() > SNIFF {
        bytes.get(..SNIFF).unwrap_or(bytes)
    } else {
        bytes
    };
    if head.contains(&0) {
        return true;
    }
    std::str::from_utf8(head).is_err() && std::str::from_utf8(bytes).is_err()
}

/// The editor's syntax hint. Unknown extensions get plain text rather than a
/// guess, because a wrong highlighter is worse than none.
pub fn language_for(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    match name.as_str() {
        "dockerfile" => return "dockerfile",
        "makefile" => return "makefile",
        ".env" | ".env.example" | ".env.local" => return "dotenv",
        _ => {}
    }
    match name.rsplit('.').next().unwrap_or("") {
        "js" | "cjs" | "mjs" => "javascript",
        "ts" | "cts" | "mts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "json" => "json",
        "py" | "pyi" => "python",
        "rs" => "rust",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "cs" => "csharp",
        "php" => "php",
        "sh" | "bash" | "zsh" => "shell",
        "ps1" => "powershell",
        "sql" => "sql",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "md" | "markdown" => "markdown",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "ini" | "cfg" | "conf" => "ini",
        "xml" | "svg" => "xml",
        "txt" | "log" => "plaintext",
        _ => "plaintext",
    }
}

/// Read a file as text for the editor.
///
/// The size is checked from metadata *before* the read, so an oversized file
/// costs a stat rather than a buffer.
pub fn read_text_file(
    root: &Path,
    relative: &str,
    limits: &FileLimits,
) -> Result<TextFile, FileError> {
    let safe = resolve(root, relative)?;
    let metadata = require_kind(&safe, EntryKind::File)?;

    if metadata.len() > limits.max_editable_bytes {
        return Err(FileError::TooLarge {
            path: safe.relative().to_string(),
            size: metadata.len(),
            limit: limits.max_editable_bytes,
        });
    }

    let bytes = std::fs::read(safe.absolute()).map_err(FileError::io)?;
    if looks_binary(&bytes) {
        return Err(FileError::Binary(safe.relative().to_string()));
    }
    let text =
        String::from_utf8(bytes).map_err(|_| FileError::Binary(safe.relative().to_string()))?;

    Ok(TextFile {
        path: safe.relative().to_string(),
        size_bytes: metadata.len(),
        language: language_for(safe.relative()),
        text,
    })
}

/// Write a file, replacing it if it exists.
///
/// Written to a sibling temporary file and renamed over the target, so an
/// interrupted write cannot leave a half-saved source file behind. The
/// temporary name is generated here and never derived from client input.
pub fn write_text_file(
    root: &Path,
    relative: &str,
    contents: &str,
    limits: &FileLimits,
) -> Result<FileEntry, FileError> {
    let safe = resolve(root, relative)?;

    if contents.len() as u64 > limits.max_editable_bytes {
        return Err(FileError::TooLarge {
            path: safe.relative().to_string(),
            size: contents.len() as u64,
            limit: limits.max_editable_bytes,
        });
    }

    // Refuse to replace a directory or a link with a file.
    if let Ok(metadata) = std::fs::symlink_metadata(safe.absolute()) {
        if metadata.is_symlink() {
            return Err(FileError::Refused(
                "symbolic links are not followed; delete the link instead",
            ));
        }
        if metadata.is_dir() {
            return Err(FileError::NotAFile(safe.relative().to_string()));
        }
    }

    write_atomically(safe.absolute(), contents.as_bytes())?;
    entry_from(safe.relative(), &name_of(safe.relative()), safe.absolute())
}

/// Write bytes from an upload. Same atomicity, a much higher ceiling, and no
/// text or UTF-8 assumption.
pub fn write_bytes(
    root: &Path,
    relative: &str,
    bytes: &[u8],
    limits: &FileLimits,
) -> Result<FileEntry, FileError> {
    let safe = resolve(root, relative)?;
    if bytes.len() as u64 > limits.max_upload_bytes {
        return Err(FileError::TooLarge {
            path: safe.relative().to_string(),
            size: bytes.len() as u64,
            limit: limits.max_upload_bytes,
        });
    }
    if let Ok(metadata) = std::fs::symlink_metadata(safe.absolute()) {
        if metadata.is_symlink() || metadata.is_dir() {
            return Err(FileError::Refused(
                "the destination exists and is not a regular file",
            ));
        }
    }
    write_atomically(safe.absolute(), bytes)?;
    entry_from(safe.relative(), &name_of(safe.relative()), safe.absolute())
}

/// Reserve a destination for a chunked upload.
///
/// The real destination must not exist, so dropping a file onto the explorer can
/// never silently replace a user's project file. Chunks land in a generated
/// sibling first and are only renamed into place by [`finish_upload`].
pub fn begin_upload(
    root: &Path,
    relative: &str,
    upload_id: &str,
    total_size: u64,
    limits: &FileLimits,
) -> Result<(), FileError> {
    validate_upload_id(upload_id)?;
    let destination = resolve(root, relative)?;
    validate_upload_destination(&destination, total_size, limits)?;
    let temporary = upload_temporary(root, destination.relative(), upload_id)?;

    if destination.relative() == temporary.relative() {
        return Err(FileError::Refused("that file name is reserved for uploads"));
    }
    if std::fs::symlink_metadata(destination.absolute()).is_ok() {
        return Err(FileError::AlreadyExists(destination.relative().to_string()));
    }
    if let Some(parent) = destination.absolute().parent() {
        std::fs::create_dir_all(parent).map_err(FileError::io)?;
    }

    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary.absolute())
        .map_err(FileError::io)?;
    Ok(())
}

/// Append one chunk to an upload that was reserved by [`begin_upload`].
pub fn append_upload(
    root: &Path,
    relative: &str,
    upload_id: &str,
    offset: u64,
    bytes: &[u8],
    limits: &FileLimits,
) -> Result<u64, FileError> {
    validate_upload_id(upload_id)?;
    let destination = resolve(root, relative)?;
    let next_offset = offset
        .checked_add(bytes.len() as u64)
        .ok_or(FileError::TooLarge {
            path: destination.relative().to_string(),
            size: u64::MAX,
            limit: limits.max_upload_bytes,
        })?;
    if next_offset > limits.max_upload_bytes {
        return Err(FileError::TooLarge {
            path: destination.relative().to_string(),
            size: next_offset,
            limit: limits.max_upload_bytes,
        });
    }

    let temporary = upload_temporary(root, destination.relative(), upload_id)?;
    let metadata = require_kind(&temporary, EntryKind::File)?;
    if metadata.len() != offset {
        return Err(FileError::Refused(
            "upload chunks arrived out of order; retry the upload",
        ));
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .open(temporary.absolute())
        .map_err(FileError::io)?;
    file.seek(SeekFrom::Start(offset)).map_err(FileError::io)?;
    file.write_all(bytes).map_err(FileError::io)?;
    Ok(next_offset)
}

/// Move a complete chunked upload into place if the destination is still free.
pub fn finish_upload(
    root: &Path,
    relative: &str,
    upload_id: &str,
    total_size: u64,
    limits: &FileLimits,
) -> Result<FileEntry, FileError> {
    validate_upload_id(upload_id)?;
    let destination = resolve(root, relative)?;
    validate_upload_destination(&destination, total_size, limits)?;
    let temporary = upload_temporary(root, destination.relative(), upload_id)?;

    let metadata = require_kind(&temporary, EntryKind::File)?;
    if metadata.len() != total_size {
        return Err(FileError::Refused("upload is incomplete; retry the upload"));
    }
    if std::fs::symlink_metadata(destination.absolute()).is_ok() {
        let _ = std::fs::remove_file(temporary.absolute());
        return Err(FileError::AlreadyExists(destination.relative().to_string()));
    }

    std::fs::rename(temporary.absolute(), destination.absolute()).map_err(FileError::io)?;
    entry_from(
        destination.relative(),
        &name_of(destination.relative()),
        destination.absolute(),
    )
}

/// Remove the temporary file for an upload that was cancelled or failed.
pub fn cancel_upload(root: &Path, relative: &str, upload_id: &str) -> Result<(), FileError> {
    validate_upload_id(upload_id)?;
    let destination = resolve(root, relative)?;
    let temporary = upload_temporary(root, destination.relative(), upload_id)?;

    match std::fs::symlink_metadata(temporary.absolute()) {
        Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {
            Err(FileError::Refused("upload temporary path is a directory"))
        }
        Ok(_) => std::fs::remove_file(temporary.absolute()).map_err(FileError::io),
        Err(_) => Ok(()),
    }
}

fn validate_upload_id(upload_id: &str) -> Result<(), FileError> {
    if !upload_id.is_empty()
        && upload_id.len() <= 80
        && upload_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Ok(());
    }
    Err(FileError::Refused("upload id is invalid"))
}

fn validate_upload_destination(
    destination: &SafePath,
    total_size: u64,
    limits: &FileLimits,
) -> Result<(), FileError> {
    if destination.relative().is_empty() {
        return Err(FileError::Refused(
            "the project root cannot be replaced by an upload",
        ));
    }
    if total_size > limits.max_upload_bytes {
        return Err(FileError::TooLarge {
            path: destination.relative().to_string(),
            size: total_size,
            limit: limits.max_upload_bytes,
        });
    }
    Ok(())
}

fn upload_temporary(root: &Path, relative: &str, upload_id: &str) -> Result<SafePath, FileError> {
    let name = format!(".project-host-upload-{upload_id}.tmp");
    let temporary = match relative.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => format!("{parent}/{name}"),
        _ => name,
    };
    resolve(root, &temporary)
}

fn write_atomically(target: &Path, bytes: &[u8]) -> Result<(), FileError> {
    let parent = target.parent().ok_or(FileError::Refused(
        "the destination has no parent directory",
    ))?;
    std::fs::create_dir_all(parent).map_err(FileError::io)?;

    let temporary = parent.join(format!(".project-host-{}.tmp", unique_suffix()));
    let result = (|| {
        std::fs::write(&temporary, bytes).map_err(FileError::io)?;
        // Rename over the target: on both platforms this replaces atomically
        // for files on the same volume, which a sibling always is.
        std::fs::rename(&temporary, target).map_err(FileError::io)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// A collision-resistant suffix without pulling in a UUID dependency here.
/// Monotonic time plus the thread id is enough for a file that lives for
/// microseconds inside a directory only this process writes to.
fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}-{:?}", std::thread::current().id())
        .replace(|c: char| !c.is_ascii_alphanumeric() && c != '-', "")
}

/// Create an empty file. Fails if anything is already there — an explorer's
/// "new file" must never silently truncate.
pub fn create_file(root: &Path, relative: &str) -> Result<FileEntry, FileError> {
    let safe = resolve(root, relative)?;
    if std::fs::symlink_metadata(safe.absolute()).is_ok() {
        return Err(FileError::AlreadyExists(safe.relative().to_string()));
    }
    if let Some(parent) = safe.absolute().parent() {
        std::fs::create_dir_all(parent).map_err(FileError::io)?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(safe.absolute())
        .map_err(FileError::io)?;
    entry_from(safe.relative(), &name_of(safe.relative()), safe.absolute())
}

pub fn create_directory(root: &Path, relative: &str) -> Result<FileEntry, FileError> {
    let safe = resolve(root, relative)?;
    if std::fs::symlink_metadata(safe.absolute()).is_ok() {
        return Err(FileError::AlreadyExists(safe.relative().to_string()));
    }
    std::fs::create_dir_all(safe.absolute()).map_err(FileError::io)?;
    entry_from(safe.relative(), &name_of(safe.relative()), safe.absolute())
}

/// Delete a file, a link, or a directory.
///
/// `recursive` is required for a non-empty directory: the caller has to state
/// that it means it, and the API layer turns that into a confirmation dialog.
pub fn delete(root: &Path, relative: &str, recursive: bool) -> Result<(), FileError> {
    let safe = match resolve(root, relative) {
        Ok(safe) => safe,
        // A link out of the project fails the containment check, but removing
        // the link never touches what it points at — and the listing shows it,
        // so refusing would leave an entry that cannot be got rid of. The
        // fallback is deliberately narrow: only a symlink takes it, and
        // anything else still escapes.
        Err(FileError::Path(PathError::Escape)) => {
            let candidate = SafePath::new_no_follow(root, relative)?;
            let is_link = std::fs::symlink_metadata(candidate.absolute())
                .map(|metadata| metadata.is_symlink())
                .unwrap_or(false);
            if !is_link {
                return Err(FileError::Path(PathError::Escape));
            }
            candidate
        }
        Err(other) => return Err(other),
    };

    if safe.relative().is_empty() {
        return Err(FileError::Refused(
            "the project root cannot be deleted from the file explorer",
        ));
    }

    let metadata = std::fs::symlink_metadata(safe.absolute())
        .map_err(|_| FileError::NotFound(safe.relative().to_string()))?;

    if metadata.is_symlink() {
        // Remove the link itself, never its target. `remove_file` is correct
        // for a file symlink on both platforms; a directory symlink or junction
        // on Windows needs `remove_dir`, which does not touch the target either.
        return std::fs::remove_file(safe.absolute())
            .or_else(|_| std::fs::remove_dir(safe.absolute()))
            .map_err(FileError::io);
    }

    if metadata.is_dir() {
        if recursive {
            return std::fs::remove_dir_all(safe.absolute()).map_err(FileError::io);
        }
        return match std::fs::remove_dir(safe.absolute()) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::Other => Err(FileError::NotEmpty),
            // Portable detection of "not empty" is unreliable across platforms
            // and Rust versions, so fall back to asking the directory.
            Err(error) => {
                let empty = std::fs::read_dir(safe.absolute())
                    .map(|mut items| items.next().is_none())
                    .unwrap_or(false);
                if empty {
                    Err(FileError::io(error))
                } else {
                    Err(FileError::NotEmpty)
                }
            }
        };
    }

    std::fs::remove_file(safe.absolute()).map_err(FileError::io)
}

/// Rename within the same parent directory.
///
/// `new_name` is a single component, validated by joining it to the parent
/// through [`SafePath`] — so `../elsewhere` is refused by the same rule that
/// refuses it everywhere else, rather than by an ad-hoc check here.
pub fn rename(root: &Path, relative: &str, new_name: &str) -> Result<FileEntry, FileError> {
    if new_name.contains('/') || new_name.contains('\\') {
        return Err(FileError::Refused(
            "a new name must be a single path component; use move instead",
        ));
    }
    let safe = resolve(root, relative)?;
    if safe.relative().is_empty() {
        return Err(FileError::Refused("the project root cannot be renamed"));
    }

    let parent = match safe.relative().rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    };
    let destination = if parent.is_empty() {
        resolve(root, new_name)?
    } else {
        resolve(root, &format!("{parent}/{new_name}"))?
    };

    move_resolved(safe, destination)
}

/// Move an entry anywhere inside the same project.
pub fn move_entry(root: &Path, from: &str, to: &str) -> Result<FileEntry, FileError> {
    let source = resolve(root, from)?;
    let destination = resolve(root, to)?;
    if source.relative().is_empty() {
        return Err(FileError::Refused("the project root cannot be moved"));
    }
    move_resolved(source, destination)
}

fn move_resolved(source: SafePath, destination: SafePath) -> Result<FileEntry, FileError> {
    if std::fs::symlink_metadata(source.absolute()).is_err() {
        return Err(FileError::NotFound(source.relative().to_string()));
    }
    if std::fs::symlink_metadata(destination.absolute()).is_ok() {
        return Err(FileError::AlreadyExists(destination.relative().to_string()));
    }
    if is_prefix_of(source.relative(), destination.relative()) {
        return Err(FileError::IntoItself);
    }
    if let Some(parent) = destination.absolute().parent() {
        std::fs::create_dir_all(parent).map_err(FileError::io)?;
    }
    std::fs::rename(source.absolute(), destination.absolute()).map_err(FileError::io)?;
    entry_from(
        destination.relative(),
        &name_of(destination.relative()),
        destination.absolute(),
    )
}

/// True when `parent` names an ancestor of `child`, compared component-wise so
/// `src` is not treated as an ancestor of `srcfile.js`.
fn is_prefix_of(parent: &str, child: &str) -> bool {
    if parent.is_empty() {
        return true;
    }
    if parent == child {
        return true;
    }
    child.starts_with(parent) && child.as_bytes().get(parent.len()) == Some(&b'/')
}

/// Copy a file or a whole directory tree.
///
/// The walk is bounded by `max_walk_entries` and refuses links rather than
/// following them, so copying cannot be tricked into pulling content in from
/// outside the project.
pub fn copy(
    root: &Path,
    from: &str,
    to: &str,
    limits: &FileLimits,
) -> Result<FileEntry, FileError> {
    let source = resolve(root, from)?;
    let destination = resolve(root, to)?;

    if source.relative().is_empty() {
        return Err(FileError::Refused("the project root cannot be copied"));
    }
    if std::fs::symlink_metadata(destination.absolute()).is_ok() {
        return Err(FileError::AlreadyExists(destination.relative().to_string()));
    }
    if is_prefix_of(source.relative(), destination.relative()) {
        return Err(FileError::IntoItself);
    }

    let metadata = require_kind(&source, EntryKind::Directory)
        .or_else(|_| require_kind(&source, EntryKind::File))?;

    if let Some(parent) = destination.absolute().parent() {
        std::fs::create_dir_all(parent).map_err(FileError::io)?;
    }

    if metadata.is_file() {
        std::fs::copy(source.absolute(), destination.absolute()).map_err(FileError::io)?;
    } else {
        copy_tree(source.absolute(), destination.absolute(), limits)?;
    }

    entry_from(
        destination.relative(),
        &name_of(destination.relative()),
        destination.absolute(),
    )
}

fn copy_tree(source: &Path, destination: &Path, limits: &FileLimits) -> Result<(), FileError> {
    let mut queue = VecDeque::from([(source.to_path_buf(), destination.to_path_buf())]);
    let mut visited = 0usize;

    while let Some((from, to)) = queue.pop_front() {
        visited += 1;
        if visited > limits.max_walk_entries {
            return Err(FileError::Refused(
                "the directory contains too many entries to copy",
            ));
        }

        std::fs::create_dir_all(&to).map_err(FileError::io)?;

        for item in std::fs::read_dir(&from).map_err(FileError::io)? {
            let item = item.map_err(FileError::io)?;
            let metadata = item.metadata().map_err(FileError::io)?;
            let child_to = to.join(item.file_name());

            if metadata.is_symlink() {
                // Copying a link would either duplicate an escape or silently
                // dereference it. Neither is acceptable, so the copy fails.
                return Err(FileError::Refused(
                    "the directory contains a symbolic link and cannot be copied",
                ));
            }
            if metadata.is_dir() {
                queue.push_back((item.path(), child_to));
            } else if metadata.is_file() {
                std::fs::copy(item.path(), &child_to).map_err(FileError::io)?;
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PlannedImportKind {
    Directory,
    File { size: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedImport {
    source: PathBuf,
    relative: String,
    kind: PlannedImportKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImportPlan {
    entries: Vec<PlannedImport>,
    top_level_destinations: Vec<String>,
    total_files: u64,
    total_bytes: u64,
    /// Links met while walking the source, which are not copied.
    skipped_symlinks: Vec<String>,
}

/// How many skipped links an import names before it stops listing them.
const SKIPPED_SYMLINK_EXAMPLES: usize = 10;

/// The prefix every import's staging directory is named with, at the project
/// root. Shared by the import itself and by the sweep that clears up after one
/// that never finished.
const IMPORT_STAGE_PREFIX: &str = ".project-host-import-";

fn import_stage_name(import_id: &str) -> String {
    format!("{IMPORT_STAGE_PREFIX}{import_id}")
}

/// Remove staging directories left behind by imports that never finished.
///
/// An import removes its own staging directory on every path out of
/// [`import_local_paths`], including cancellation and failure. What no amount of
/// care inside that function can cover is the process not living to run it: a
/// crash, a forced quit, or the machine losing power partway through a copy
/// leaves the directory behind, and the user sees it in the explorer as a
/// half-copied folder they did not create and cannot explain.
///
/// A *running* import owns its staging directory, so `is_active` is asked about
/// every candidate before it is removed. Sweeping one that is in flight would
/// delete the files out from under a copy that is still writing them.
pub fn remove_abandoned_import_staging<A>(
    root: &Path,
    is_active: A,
) -> Result<Vec<String>, FileError>
where
    A: Fn(&str) -> bool,
{
    let safe = SafePath::root(root)?;
    require_kind(&safe, EntryKind::Directory)?;
    let mut removed = Vec::new();

    for item in std::fs::read_dir(safe.absolute()).map_err(FileError::io)? {
        let item = item.map_err(FileError::io)?;
        let name = item.file_name().to_string_lossy().to_string();
        let Some(import_id) = name.strip_prefix(IMPORT_STAGE_PREFIX) else {
            continue;
        };
        if import_id.is_empty() || is_active(import_id) {
            continue;
        }

        // Anything here that is not a plain directory was not left by an
        // import. Deleting it would mean following a link out of the project,
        // which nothing in this module ever does.
        let metadata = std::fs::symlink_metadata(item.path()).map_err(FileError::io)?;
        if metadata.is_symlink() || !metadata.is_dir() {
            continue;
        }

        std::fs::remove_dir_all(item.path()).map_err(FileError::io)?;
        removed.push(name);
    }

    Ok(removed)
}

/// Import files or directories from the host filesystem into a project.
///
/// The import is staged inside the project first, then renamed into place only
/// after every source path has been copied. Existing destinations are refused
/// before staging begins, duplicate destination paths are refused before any
/// bytes are written, and a cancelled or failed import removes its staging
/// directory so half-copied files do not appear in the explorer.
/// One thing being imported, and whether its own folder comes with it.
///
/// `unwrap` exists for the case where the folder is a container the user has
/// already opened: dropping a project into a project should put the project's
/// files at the root, not produce `MyProject/MyProject/package.json`. It is
/// meaningless for a file and ignored there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSource {
    pub path: std::path::PathBuf,
    /// Copy the directory's *children* into the destination, not the directory.
    pub unwrap: bool,
    /// Land under this name instead of the source's own.
    ///
    /// What a "keep both" resolution needs: the incoming `config.json` becomes
    /// `config copy.json` without the caller having to copy it and rename it
    /// afterwards, which would leave the wrong name on disk in between.
    /// Ignored when `unwrap` is set, because then there is no single name.
    pub name: Option<String>,
}

impl ImportSource {
    /// The ordinary case: the folder is copied, name and all.
    pub fn nested(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            unwrap: false,
            name: None,
        }
    }

    /// The contents land in the destination and the folder itself does not.
    pub fn unwrapped(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            path: path.into(),
            unwrap: true,
            name: None,
        }
    }

    /// Land under a name of the caller's choosing.
    pub fn renamed(path: impl Into<std::path::PathBuf>, name: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            unwrap: false,
            name: Some(name.into()),
        }
    }
}

/// One thing an import would create, worked out without copying anything.
///
/// The window cannot compute these itself: unwrapping a folder lands its
/// children, and it has no way to read a directory. Returning the sizes at the
/// same time means one walk answers both "what will collide?" and "how much is
/// there?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDestination {
    /// The absolute path this comes from — a child, when a folder is unwrapped.
    pub source: std::path::PathBuf,
    /// Where it lands, relative to the project root.
    pub relative: String,
    pub is_directory: bool,
    pub total_files: u64,
    pub total_bytes: u64,
}

/// What an import would create, and how much it would copy.
///
/// Reads the sources but writes nothing. Used to analyse conflicts and to size
/// a progress bar before the first byte moves.
pub fn plan_import_destinations(
    root: &Path,
    destination_directory: &str,
    sources: &[ImportSource],
    limits: &FileLimits,
) -> Result<Vec<PlannedDestination>, FileError> {
    let plan = plan_local_import_unchecked(root, destination_directory, sources, limits)?;

    // The plan lists every entry in the tree; the caller wants the top level,
    // with each one's subtree rolled up into it.
    let mut destinations = Vec::new();
    for relative in &plan.top_level_destinations {
        let prefix = format!("{relative}/");
        let mut total_files = 0u64;
        let mut total_bytes = 0u64;
        let mut source = None;
        let mut is_directory = false;

        for entry in &plan.entries {
            if entry.relative != *relative && !entry.relative.starts_with(&prefix) {
                continue;
            }
            if entry.relative == *relative {
                source = Some(entry.source.clone());
                is_directory = matches!(entry.kind, PlannedImportKind::Directory);
            }
            if let PlannedImportKind::File { size } = entry.kind {
                total_files += 1;
                total_bytes = total_bytes.saturating_add(size);
            }
        }

        let Some(source) = source else { continue };
        destinations.push(PlannedDestination {
            source,
            relative: relative.clone(),
            is_directory,
            total_files,
            total_bytes,
        });
    }

    Ok(destinations)
}

/// Import paths from this machine, each keeping or shedding its own folder.
pub fn import_local_sources<F, C>(
    root: &Path,
    destination_directory: &str,
    sources: &[ImportSource],
    import_id: &str,
    limits: &FileLimits,
    progress: F,
    cancelled: C,
) -> Result<LocalImportReport, FileError>
where
    F: FnMut(LocalImportProgress),
    C: Fn() -> bool,
{
    import_planned(
        root,
        destination_directory,
        sources,
        import_id,
        limits,
        progress,
        cancelled,
    )
}

pub fn import_local_paths<P, F, C>(
    root: &Path,
    destination_directory: &str,
    source_paths: &[P],
    import_id: &str,
    limits: &FileLimits,
    progress: F,
    cancelled: C,
) -> Result<LocalImportReport, FileError>
where
    P: AsRef<Path>,
    F: FnMut(LocalImportProgress),
    C: Fn() -> bool,
{
    let sources: Vec<ImportSource> = source_paths
        .iter()
        .map(|path| ImportSource::nested(path.as_ref()))
        .collect();
    import_planned(
        root,
        destination_directory,
        &sources,
        import_id,
        limits,
        progress,
        cancelled,
    )
}

fn import_planned<F, C>(
    root: &Path,
    destination_directory: &str,
    sources: &[ImportSource],
    import_id: &str,
    limits: &FileLimits,
    mut progress: F,
    cancelled: C,
) -> Result<LocalImportReport, FileError>
where
    F: FnMut(LocalImportProgress),
    C: Fn() -> bool,
{
    validate_upload_id(import_id)?;
    let plan = plan_local_import(root, destination_directory, sources, limits)?;
    let stage = resolve(root, &import_stage_name(import_id))?;

    if std::fs::symlink_metadata(stage.absolute()).is_ok() {
        return Err(FileError::AlreadyExists(stage.relative().to_string()));
    }
    std::fs::create_dir(stage.absolute()).map_err(FileError::io)?;

    progress(LocalImportProgress {
        copied_bytes: 0,
        total_bytes: plan.total_bytes,
        copied_files: 0,
        total_files: plan.total_files,
        current_path: String::new(),
    });

    let result = copy_and_commit_import(root, stage.absolute(), &plan, &mut progress, &cancelled);
    // Once the commit has renamed everything into place the import *happened*.
    // A staging directory that will not go away is litter, not a failure: the
    // caller would report an error for files that are already in the project
    // and the obvious retry would then fail with `AlreadyExists`.
    let cleanup = std::fs::remove_dir_all(stage.absolute());

    import_outcome(result, cleanup, stage.relative()).map(|entries| LocalImportReport {
        entries,
        total_files: plan.total_files,
        total_bytes: plan.total_bytes,
        skipped_symlinks: plan.skipped_symlinks.len() as u64,
        skipped_symlink_examples: plan
            .skipped_symlinks
            .iter()
            .take(SKIPPED_SYMLINK_EXAMPLES)
            .cloned()
            .collect(),
    })
}

/// What an import reports, once the copy has finished and the staging directory
/// has been cleaned up.
///
/// Cleaning up is deliberately not part of the outcome. By the time it runs the
/// commit has already renamed every imported path into the project, so a
/// staging directory that will not go away is litter and not a failure:
/// reporting it would tell the user their import failed while the files sit
/// exactly where they asked for them, and the obvious retry would then fail
/// again with `AlreadyExists`. An import that genuinely failed reports why it
/// failed, which is never the cleanup error.
fn import_outcome<T>(
    copied: Result<T, FileError>,
    cleanup: std::io::Result<()>,
    stage: &str,
) -> Result<T, FileError> {
    if let Err(error) = cleanup {
        tracing::warn!(%error, stage, "could not remove the import staging directory");
    }
    copied
}

/// Plan the copy, and refuse anything that would land on an existing file.
fn plan_local_import(
    root: &Path,
    destination_directory: &str,
    sources: &[ImportSource],
    limits: &FileLimits,
) -> Result<ImportPlan, FileError> {
    let plan = plan_local_import_unchecked(root, destination_directory, sources, limits)?;

    let mut seen = HashSet::new();
    for entry in &plan.entries {
        if !seen.insert(destination_key(&entry.relative)) {
            return Err(FileError::AlreadyExists(entry.relative.clone()));
        }
        let destination = resolve(root, &entry.relative)?;
        let Ok(existing) = std::fs::symlink_metadata(destination.absolute()) else {
            continue;
        };

        // A folder that is already there is merged into, not refused. Dropping
        // a project with a `src/` into a project that has one should put the
        // incoming files beside the existing ones, which is what every file
        // manager does and what "preserve the folder structure" requires.
        //
        // A *file* that is already there is still refused, before anything is
        // copied. Merging cannot mean overwriting.
        let mergeable = matches!(entry.kind, PlannedImportKind::Directory)
            && existing.is_dir()
            && !existing.is_symlink();
        if !mergeable {
            return Err(FileError::AlreadyExists(destination.relative().to_string()));
        }
    }

    Ok(plan)
}

/// Work out what would be copied where, without asking whether it is free.
///
/// Split out so the destinations can be reported to the window for conflict
/// analysis — which needs to know about the collisions rather than be stopped
/// by them.
fn plan_local_import_unchecked(
    root: &Path,
    destination_directory: &str,
    sources: &[ImportSource],
    limits: &FileLimits,
) -> Result<ImportPlan, FileError> {
    if sources.is_empty() {
        return Err(FileError::Refused("no files or folders were provided"));
    }

    let destination = if destination_directory.is_empty() {
        SafePath::root(root)?
    } else {
        let destination = resolve(root, destination_directory)?;
        require_kind(&destination, EntryKind::Directory)?;
        destination
    };

    let mut entries = Vec::new();
    let mut top_level_destinations = Vec::new();
    let mut skipped_symlinks: Vec<String> = Vec::new();
    let mut visited = 0usize;
    let mut total_files = 0u64;
    let mut total_bytes = 0u64;

    for item in sources {
        let source = item.path.as_path();
        if !source.is_absolute() {
            return Err(FileError::Refused("import sources must be absolute paths"));
        }
        let metadata = std::fs::symlink_metadata(source).map_err(FileError::io)?;
        if metadata.is_symlink() {
            // Same rule as for a nested one, so that dropping a folder and
            // dropping a folder *and* a link beside it do not disagree about
            // what happens to the folder.
            skipped_symlinks.push(source.to_string_lossy().into_owned());
            continue;
        }

        // Unwrapping is only meaningful for a directory. Asking for it on a
        // file is a caller mistake worth naming rather than quietly ignoring,
        // because the file would otherwise land somewhere unexpected.
        if item.unwrap && !metadata.is_dir() {
            return Err(FileError::Refused(
                "only a folder's contents can be imported without the folder",
            ));
        }

        if item.unwrap {
            // The children are planned as though each had been dropped on its
            // own, so the folder contributes its contents and not its name.
            for child in std::fs::read_dir(source).map_err(FileError::io)? {
                let child = child.map_err(FileError::io)?;
                let name = child
                    .file_name()
                    .to_str()
                    .ok_or(FileError::Refused("source path contains invalid Unicode"))?
                    .to_string();
                let relative = join_relative(destination.relative(), &name);
                let child_metadata =
                    std::fs::symlink_metadata(child.path()).map_err(FileError::io)?;

                // Before the destination is registered, not after: a skipped
                // link stages nothing, so a destination recorded for it would
                // send the commit looking for a path that was never written —
                // and would report a conflict for a file that is not coming.
                if child_metadata.is_symlink() {
                    skipped_symlinks.push(relative);
                    continue;
                }

                top_level_destinations.push(resolve(root, &relative)?.relative().to_string());
                plan_import_entry(
                    root,
                    &child.path(),
                    &relative,
                    child_metadata,
                    limits,
                    &mut visited,
                    &mut total_files,
                    &mut total_bytes,
                    &mut entries,
                    &mut skipped_symlinks,
                )?;
            }
            continue;
        }

        let name = match item.name.as_deref() {
            Some(chosen) => {
                if chosen.contains('/') || chosen.contains('\\') {
                    return Err(FileError::Refused(
                        "an import name must be a single path component",
                    ));
                }
                chosen
            }
            None => source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(FileError::Refused("source path contains invalid Unicode"))?,
        };
        let relative = join_relative(destination.relative(), name);
        top_level_destinations.push(resolve(root, &relative)?.relative().to_string());
        plan_import_entry(
            root,
            source,
            &relative,
            metadata,
            limits,
            &mut visited,
            &mut total_files,
            &mut total_bytes,
            &mut entries,
            &mut skipped_symlinks,
        )?;
    }

    // Skipping a nested link keeps an import going; skipping *everything* the
    // user asked for and reporting success would be a silent no-op, which is a
    // worse answer than either copying or refusing.
    if entries.is_empty() && !skipped_symlinks.is_empty() {
        return Err(FileError::Refused(
            "there was nothing to import but links, which are not copied",
        ));
    }

    Ok(ImportPlan {
        entries,
        top_level_destinations,
        total_files,
        total_bytes,
        skipped_symlinks,
    })
}

#[allow(clippy::too_many_arguments)]
fn plan_import_entry(
    root: &Path,
    source: &Path,
    relative: &str,
    metadata: std::fs::Metadata,
    limits: &FileLimits,
    visited: &mut usize,
    total_files: &mut u64,
    total_bytes: &mut u64,
    entries: &mut Vec<PlannedImport>,
    skipped: &mut Vec<String>,
) -> Result<(), FileError> {
    *visited += 1;
    if *visited > limits.max_walk_entries {
        return Err(FileError::Refused(
            "the import contains too many entries to copy",
        ));
    }

    // Not copied, and not followed either — a copy is not the place the link
    // points at, and following one would pull in a tree from outside the
    // import. But it cannot veto the files beside it: any Node project has
    // links in `node_modules` (junctions on Windows, which `is_symlink` also
    // reports), and refusing the import over them refused every real project.
    // `git_clone::refuse_escaping_symlinks` settled this policy already.
    if metadata.is_symlink() {
        skipped.push(relative.to_string());
        return Ok(());
    }

    let destination = resolve(root, relative)?;
    if metadata.is_dir() {
        entries.push(PlannedImport {
            source: source.to_path_buf(),
            relative: destination.relative().to_string(),
            kind: PlannedImportKind::Directory,
        });

        for item in std::fs::read_dir(source).map_err(FileError::io)? {
            let item = item.map_err(FileError::io)?;
            let child_name = item
                .file_name()
                .to_str()
                .ok_or(FileError::Refused("source path contains invalid Unicode"))?
                .to_string();
            let child_relative = join_relative(destination.relative(), &child_name);
            let child_metadata = std::fs::symlink_metadata(item.path()).map_err(FileError::io)?;
            plan_import_entry(
                root,
                &item.path(),
                &child_relative,
                child_metadata,
                limits,
                visited,
                total_files,
                total_bytes,
                entries,
                skipped,
            )?;
        }
    } else if metadata.is_file() {
        if metadata.len() > limits.max_upload_bytes {
            return Err(FileError::TooLarge {
                path: destination.relative().to_string(),
                size: metadata.len(),
                limit: limits.max_upload_bytes,
            });
        }
        *total_files = total_files.saturating_add(1);
        *total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or(FileError::Refused("the import is too large"))?;
        entries.push(PlannedImport {
            source: source.to_path_buf(),
            relative: destination.relative().to_string(),
            kind: PlannedImportKind::File {
                size: metadata.len(),
            },
        });
    } else {
        return Err(FileError::Refused(
            "only regular files and directories can be imported",
        ));
    }

    Ok(())
}

fn copy_and_commit_import<F, C>(
    root: &Path,
    stage_root: &Path,
    plan: &ImportPlan,
    progress: &mut F,
    cancelled: &C,
) -> Result<Vec<FileEntry>, FileError>
where
    F: FnMut(LocalImportProgress),
    C: Fn() -> bool,
{
    let mut copied_bytes = 0u64;
    let mut copied_files = 0u64;

    for entry in &plan.entries {
        refuse_if_cancelled(cancelled)?;
        let stage_destination = stage_root.join(Path::new(&entry.relative));
        match entry.kind {
            PlannedImportKind::Directory => {
                std::fs::create_dir_all(&stage_destination).map_err(FileError::io)?;
            }
            PlannedImportKind::File { size } => {
                if let Some(parent) = stage_destination.parent() {
                    std::fs::create_dir_all(parent).map_err(FileError::io)?;
                }
                copy_file_with_progress(
                    &entry.source,
                    &stage_destination,
                    &entry.relative,
                    size,
                    &mut copied_bytes,
                    &mut copied_files,
                    plan,
                    progress,
                    cancelled,
                )?;
            }
        }
    }

    refuse_if_cancelled(cancelled)?;
    commit_staged_import(root, stage_root, plan)
}

#[allow(clippy::too_many_arguments)]
fn copy_file_with_progress<F, C>(
    source: &Path,
    destination: &Path,
    relative: &str,
    size: u64,
    copied_bytes: &mut u64,
    copied_files: &mut u64,
    plan: &ImportPlan,
    progress: &mut F,
    cancelled: &C,
) -> Result<(), FileError>
where
    F: FnMut(LocalImportProgress),
    C: Fn() -> bool,
{
    const COPY_BUFFER_BYTES: usize = 1024 * 1024;

    let mut from = std::fs::File::open(source).map_err(FileError::io)?;
    let mut to = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(FileError::io)?;
    let mut buffer = vec![0; COPY_BUFFER_BYTES];

    loop {
        refuse_if_cancelled(cancelled)?;
        let read = from.read(&mut buffer).map_err(FileError::io)?;
        if read == 0 {
            break;
        }
        let chunk = buffer
            .get(..read)
            .ok_or(FileError::Refused("the import chunk was invalid"))?;
        to.write_all(chunk).map_err(FileError::io)?;
        *copied_bytes = copied_bytes.saturating_add(read as u64);
        progress(LocalImportProgress {
            copied_bytes: *copied_bytes,
            total_bytes: plan.total_bytes,
            copied_files: *copied_files,
            total_files: plan.total_files,
            current_path: relative.to_string(),
        });
    }

    if size == 0 {
        progress(LocalImportProgress {
            copied_bytes: *copied_bytes,
            total_bytes: plan.total_bytes,
            copied_files: *copied_files,
            total_files: plan.total_files,
            current_path: relative.to_string(),
        });
    }

    *copied_files = copied_files.saturating_add(1);
    progress(LocalImportProgress {
        copied_bytes: *copied_bytes,
        total_bytes: plan.total_bytes,
        copied_files: *copied_files,
        total_files: plan.total_files,
        current_path: relative.to_string(),
    });
    Ok(())
}

fn commit_staged_import(
    root: &Path,
    stage_root: &Path,
    plan: &ImportPlan,
) -> Result<Vec<FileEntry>, FileError> {
    let mut moved = Vec::new();

    for relative in &plan.top_level_destinations {
        let source = stage_root.join(Path::new(relative));
        let destination = resolve(root, relative)?;
        if let Err(error) = merge_into_place(&source, destination.absolute(), &mut moved) {
            rollback_import(&moved);
            return Err(error);
        }
    }

    plan.top_level_destinations
        .iter()
        .map(|relative| {
            let destination = resolve(root, relative)?;
            entry_from(
                destination.relative(),
                &name_of(destination.relative()),
                destination.absolute(),
            )
        })
        .collect()
}

/// Move one staged path into the project, merging folders that already exist.
///
/// The fast path is a single rename of the whole subtree, which is what happens
/// when nothing is in the way. When a directory *is* in the way the two are
/// merged by recursing, because refusing would mean a project containing `src/`
/// could never be imported into a project containing `src/`.
///
/// Every path this creates is recorded in `moved`, and only those. That is what
/// makes the rollback safe: a failure halfway through a merge removes the files
/// this import added and never the ones that were already there.
fn merge_into_place(
    source: &Path,
    destination: &Path,
    moved: &mut Vec<PathBuf>,
) -> Result<(), FileError> {
    let Ok(existing) = std::fs::symlink_metadata(destination) else {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(FileError::io)?;
        }
        std::fs::rename(source, destination).map_err(FileError::io)?;
        moved.push(destination.to_path_buf());
        return Ok(());
    };

    let staged = std::fs::symlink_metadata(source).map_err(FileError::io)?;
    if !(existing.is_dir() && !existing.is_symlink() && staged.is_dir()) {
        // Planning refused this case before a byte was copied. Reaching it here
        // means the path appeared while the copy was running, and the file that
        // arrived first is the one that stays.
        return Err(FileError::AlreadyExists(
            destination.to_string_lossy().into_owned(),
        ));
    }

    for child in std::fs::read_dir(source).map_err(FileError::io)? {
        let child = child.map_err(FileError::io)?;
        merge_into_place(&child.path(), &destination.join(child.file_name()), moved)?;
    }
    Ok(())
}

fn rollback_import(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        let removal = std::fs::symlink_metadata(path)
            .map(|metadata| {
                if metadata.is_dir() && !metadata.is_symlink() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                }
            })
            .unwrap_or(Ok(()));
        let _ = removal;
    }
}

fn refuse_if_cancelled<C: Fn() -> bool>(cancelled: &C) -> Result<(), FileError> {
    if cancelled() {
        Err(FileError::Refused("import cancelled"))
    } else {
        Ok(())
    }
}

fn join_relative(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_string()
    } else {
        format!("{parent}/{child}")
    }
}

fn destination_key(relative: &str) -> String {
    relative.to_ascii_lowercase()
}

/// Search filenames beneath a directory.
///
/// Names only, not content: a content search over an unbounded project tree is
/// a denial-of-service primitive, and the editor's own search covers the file
/// the user is looking at.
pub fn search(
    root: &Path,
    within: &str,
    query: &str,
    limits: &FileLimits,
) -> Result<Vec<FileEntry>, FileError> {
    let start = resolve(root, within)?;
    require_kind(&start, EntryKind::Directory)?;

    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let mut queue = VecDeque::from([start]);
    let mut visited = 0usize;

    while let Some(directory) = queue.pop_front() {
        visited += 1;
        if visited > limits.max_walk_entries || results.len() >= limits.max_search_results {
            break;
        }

        let items = match std::fs::read_dir(directory.absolute()) {
            Ok(items) => items,
            // An unreadable subdirectory should not fail the whole search.
            Err(_) => continue,
        };

        for item in items.flatten() {
            let name = item.file_name().to_string_lossy().to_string();
            let Ok(child) = directory.join(root, &name) else {
                continue;
            };
            let Ok(metadata) = std::fs::symlink_metadata(child.absolute()) else {
                continue;
            };

            if name.to_lowercase().contains(&needle) && results.len() < limits.max_search_results {
                results.push(FileEntry {
                    name: name.clone(),
                    path: child.relative().to_string(),
                    kind: kind_of(&metadata),
                    size_bytes: if metadata.is_dir() { 0 } else { metadata.len() },
                    modified_unix_ms: modified_ms(&metadata),
                    is_symlink: metadata.is_symlink(),
                });
            }

            // Links are never descended into: that is how a search escapes.
            if metadata.is_dir() && !metadata.is_symlink() {
                queue.push_back(child);
            }
        }
    }

    results.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(results)
}

/// Total size of a directory tree, bounded by the same walk limit.
///
/// Used for the project's disk figure and to decide whether a backup will fit.
pub fn directory_size(root: &Path, relative: &str, limits: &FileLimits) -> Result<u64, FileError> {
    let start = resolve(root, relative)?;
    require_kind(&start, EntryKind::Directory)?;

    let mut total = 0u64;
    let mut queue = VecDeque::from([start.absolute().to_path_buf()]);
    let mut visited = 0usize;

    while let Some(directory) = queue.pop_front() {
        visited += 1;
        if visited > limits.max_walk_entries {
            break;
        }
        let Ok(items) = std::fs::read_dir(&directory) else {
            continue;
        };
        for item in items.flatten() {
            let Ok(metadata) = item.metadata() else {
                continue;
            };
            if metadata.is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                queue.push_back(item.path());
            } else {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Project {
        _dir: tempfile::TempDir,
        root: std::path::PathBuf,
    }

    fn project() -> Project {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().join("prj_test");
        std::fs::create_dir_all(root.join("src")).expect("create");
        std::fs::write(root.join("index.js"), "console.log('hi');\n").expect("write");
        std::fs::write(root.join("src/app.ts"), "export const a = 1;\n").expect("write");
        std::fs::write(root.join("README.md"), "# hello\n").expect("write");
        Project { _dir: dir, root }
    }

    fn limits() -> FileLimits {
        FileLimits::default()
    }

    #[test]
    fn the_root_listing_puts_directories_first() {
        let p = project();
        let listing = list_directory(&p.root, "", &limits()).expect("list");
        let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["src", "index.js", "README.md"]);
        assert!(!listing.truncated);
    }

    #[test]
    fn a_listing_reports_sizes_and_kinds() {
        let p = project();
        let listing = list_directory(&p.root, "", &limits()).expect("list");
        let index = listing
            .entries
            .iter()
            .find(|e| e.name == "index.js")
            .expect("index.js");
        assert_eq!(index.kind, EntryKind::File);
        assert_eq!(index.size_bytes, 19);
        assert_eq!(index.path, "index.js");
        assert!(index.modified_unix_ms.is_some());
    }

    #[test]
    fn traversal_is_refused_by_every_operation() {
        let p = project();
        let l = limits();
        assert!(matches!(
            list_directory(&p.root, "../", &l),
            Err(FileError::Path(_))
        ));
        assert!(matches!(
            read_text_file(&p.root, "../secrets.txt", &l),
            Err(FileError::Path(_))
        ));
        assert!(matches!(
            write_text_file(&p.root, "../evil.js", "x", &l),
            Err(FileError::Path(_))
        ));
        assert!(matches!(
            delete(&p.root, "../anything", true),
            Err(FileError::Path(_))
        ));
        assert!(matches!(
            create_directory(&p.root, "../outside"),
            Err(FileError::Path(_))
        ));
        assert!(matches!(
            move_entry(&p.root, "index.js", "../stolen.js"),
            Err(FileError::Path(_))
        ));
        assert!(matches!(
            copy(&p.root, "index.js", "../stolen.js", &l),
            Err(FileError::Path(_))
        ));
    }

    #[test]
    fn a_listing_truncates_rather_than_growing_without_bound() {
        let p = project();
        let bounded = FileLimits {
            max_listing_entries: 2,
            ..limits()
        };
        let listing = list_directory(&p.root, "", &bounded).expect("list");
        assert_eq!(listing.entries.len(), 2);
        assert!(listing.truncated);
    }

    #[test]
    fn reading_a_text_file_returns_its_content_and_language() {
        let p = project();
        let file = read_text_file(&p.root, "src/app.ts", &limits()).expect("read");
        assert_eq!(file.text, "export const a = 1;\n");
        assert_eq!(file.language, "typescript");
        assert_eq!(file.path, "src/app.ts");
    }

    #[test]
    fn reading_a_directory_as_a_file_is_refused() {
        let p = project();
        assert!(matches!(
            read_text_file(&p.root, "src", &limits()),
            Err(FileError::NotAFile(_))
        ));
    }

    #[test]
    fn an_oversized_file_is_refused_without_being_read() {
        let p = project();
        let tiny = FileLimits {
            max_editable_bytes: 4,
            ..limits()
        };
        assert!(matches!(
            read_text_file(&p.root, "index.js", &tiny),
            Err(FileError::TooLarge { .. })
        ));
    }

    #[test]
    fn a_binary_file_is_refused_by_the_editor() {
        let p = project();
        std::fs::write(p.root.join("app.bin"), [0x00, 0x01, 0x02, 0xff]).expect("write");
        assert!(matches!(
            read_text_file(&p.root, "app.bin", &limits()),
            Err(FileError::Binary(_))
        ));
    }

    #[test]
    fn utf8_text_with_accents_is_not_mistaken_for_binary() {
        let p = project();
        std::fs::write(p.root.join("notes.md"), "café — naïve ✅\n").expect("write");
        let file = read_text_file(&p.root, "notes.md", &limits()).expect("read");
        assert!(file.text.contains("café"));
    }

    #[test]
    fn writing_replaces_content_atomically_and_leaves_no_temporary() {
        let p = project();
        write_text_file(&p.root, "index.js", "// replaced\n", &limits()).expect("write");
        assert_eq!(
            std::fs::read_to_string(p.root.join("index.js")).expect("read"),
            "// replaced\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&p.root)
            .expect("read dir")
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".project-host-")
            })
            .collect();
        assert!(leftovers.is_empty(), "temporary file left behind");
    }

    #[test]
    fn writing_creates_missing_parent_directories() {
        let p = project();
        write_text_file(&p.root, "a/b/c/new.txt", "hi", &limits()).expect("write");
        assert!(p.root.join("a/b/c/new.txt").is_file());
    }

    #[test]
    fn writing_over_a_directory_is_refused() {
        let p = project();
        assert!(matches!(
            write_text_file(&p.root, "src", "x", &limits()),
            Err(FileError::NotAFile(_))
        ));
    }

    #[test]
    fn an_upload_lands_in_place_after_ordered_chunks() {
        let p = project();
        begin_upload(&p.root, "assets/image.bin", "upload-1", 11, &limits()).expect("begin upload");
        assert!(!p.root.join("assets/image.bin").exists());

        assert_eq!(
            append_upload(
                &p.root,
                "assets/image.bin",
                "upload-1",
                0,
                b"hello ",
                &limits()
            )
            .expect("append first"),
            6
        );
        assert_eq!(
            append_upload(
                &p.root,
                "assets/image.bin",
                "upload-1",
                6,
                b"world",
                &limits()
            )
            .expect("append second"),
            11
        );

        let entry = finish_upload(&p.root, "assets/image.bin", "upload-1", 11, &limits())
            .expect("finish upload");
        assert_eq!(entry.path, "assets/image.bin");
        assert_eq!(
            std::fs::read(p.root.join("assets/image.bin")).expect("read upload"),
            b"hello world"
        );
        assert!(!p
            .root
            .join("assets/.project-host-upload-upload-1.tmp")
            .exists());
    }

    #[test]
    fn an_upload_refuses_to_start_when_the_destination_exists() {
        let p = project();
        assert!(matches!(
            begin_upload(&p.root, "index.js", "upload-2", 3, &limits()),
            Err(FileError::AlreadyExists(_))
        ));
        assert_eq!(
            std::fs::read_to_string(p.root.join("index.js")).expect("read"),
            "console.log('hi');\n"
        );
    }

    #[test]
    fn a_cancelled_upload_removes_its_temporary_file() {
        let p = project();
        begin_upload(&p.root, "new.txt", "upload_3", 10, &limits()).expect("begin upload");
        append_upload(&p.root, "new.txt", "upload_3", 0, b"partial", &limits()).expect("append");

        cancel_upload(&p.root, "new.txt", "upload_3").expect("cancel");

        assert!(!p.root.join("new.txt").exists());
        assert!(!p.root.join(".project-host-upload-upload_3.tmp").exists());
    }

    #[test]
    fn finishing_an_upload_refuses_a_late_collision() {
        let p = project();
        begin_upload(&p.root, "fresh.txt", "upload-4", 3, &limits()).expect("begin upload");
        append_upload(&p.root, "fresh.txt", "upload-4", 0, b"new", &limits()).expect("append");
        std::fs::write(p.root.join("fresh.txt"), "old").expect("race write");

        assert!(matches!(
            finish_upload(&p.root, "fresh.txt", "upload-4", 3, &limits()),
            Err(FileError::AlreadyExists(_))
        ));
        assert_eq!(
            std::fs::read_to_string(p.root.join("fresh.txt")).expect("read"),
            "old"
        );
        assert!(!p.root.join(".project-host-upload-upload-4.tmp").exists());
    }

    #[test]
    fn creating_a_file_that_exists_is_refused_rather_than_truncating() {
        let p = project();
        assert!(matches!(
            create_file(&p.root, "index.js"),
            Err(FileError::AlreadyExists(_))
        ));
        assert_eq!(
            std::fs::read_to_string(p.root.join("index.js")).expect("read"),
            "console.log('hi');\n",
            "the existing file must be untouched"
        );
    }

    #[test]
    fn a_non_empty_directory_needs_the_recursive_flag() {
        let p = project();
        assert!(matches!(
            delete(&p.root, "src", false),
            Err(FileError::NotEmpty)
        ));
        assert!(p.root.join("src/app.ts").exists());
        delete(&p.root, "src", true).expect("recursive delete");
        assert!(!p.root.join("src").exists());
    }

    #[test]
    fn the_project_root_cannot_be_deleted() {
        let p = project();
        assert!(matches!(
            delete(&p.root, "", true),
            Err(FileError::Refused(_))
        ));
        assert!(p.root.exists());
    }

    #[test]
    fn renaming_moves_within_the_same_directory() {
        let p = project();
        let entry = rename(&p.root, "src/app.ts", "main.ts").expect("rename");
        assert_eq!(entry.path, "src/main.ts");
        assert!(p.root.join("src/main.ts").is_file());
        assert!(!p.root.join("src/app.ts").exists());
    }

    #[test]
    fn a_rename_cannot_smuggle_a_path_separator() {
        let p = project();
        for name in ["../escaped.ts", "sub/dir.ts", "..\\escaped.ts"] {
            assert!(
                matches!(
                    rename(&p.root, "src/app.ts", name),
                    Err(FileError::Refused(_))
                ),
                "{name} should be refused"
            );
        }
    }

    #[test]
    fn renaming_onto_an_existing_name_is_refused() {
        let p = project();
        assert!(matches!(
            rename(&p.root, "index.js", "README.md"),
            Err(FileError::AlreadyExists(_))
        ));
    }

    #[test]
    fn moving_a_directory_into_itself_is_refused() {
        let p = project();
        assert!(matches!(
            move_entry(&p.root, "src", "src/nested"),
            Err(FileError::IntoItself)
        ));
    }

    #[test]
    fn a_sibling_with_a_shared_prefix_is_not_treated_as_a_descendant() {
        // `src` must not look like an ancestor of `srcbackup`.
        let p = project();
        let entry = copy(&p.root, "src", "srcbackup", &limits()).expect("copy");
        assert_eq!(entry.path, "srcbackup");
        assert!(p.root.join("srcbackup/app.ts").is_file());
    }

    #[test]
    fn copying_a_tree_reproduces_it() {
        let p = project();
        std::fs::create_dir_all(p.root.join("src/deep")).expect("create");
        std::fs::write(p.root.join("src/deep/x.js"), "1").expect("write");
        copy(&p.root, "src", "copy", &limits()).expect("copy");
        assert!(p.root.join("copy/app.ts").is_file());
        assert!(p.root.join("copy/deep/x.js").is_file());
        assert!(p.root.join("src/app.ts").is_file(), "source must remain");
    }

    #[test]
    fn importing_a_local_directory_preserves_nested_files_and_empty_folders() {
        let p = project();
        let source_root = p.root.parent().expect("parent").join("My Build");
        std::fs::create_dir_all(source_root.join("dist/assets/empty folder")).expect("dirs");
        std::fs::write(source_root.join("dist/assets/café file.txt"), "hello").expect("write");
        std::fs::write(source_root.join("package.json"), "{}").expect("write");

        let mut progress = Vec::new();
        let report = import_local_paths(
            &p.root,
            "",
            std::slice::from_ref(&source_root),
            "import-1",
            &limits(),
            |event| progress.push(event),
            || false,
        )
        .expect("import");

        assert_eq!(report.entries.len(), 1);
        assert_eq!(report.entries[0].path, "My Build");
        assert_eq!(report.total_files, 2);
        assert_eq!(report.total_bytes, 7);
        assert!(p.root.join("My Build/dist/assets/empty folder").is_dir());
        assert_eq!(
            std::fs::read_to_string(p.root.join("My Build/dist/assets/café file.txt"))
                .expect("read"),
            "hello",
        );
        assert_eq!(progress.last().expect("progress").copied_files, 2);
        assert_eq!(progress.last().expect("progress").copied_bytes, 7);
    }

    /// Create a link the way this platform actually makes them, returning
    /// false when the session cannot make one at all.
    ///
    /// A junction rather than a symbolic link on Windows, tried second: it
    /// needs no privilege, and it is what `npm` and `pnpm` actually write into
    /// `node_modules`. That is the link a real import meets.
    #[cfg(windows)]
    fn make_link(target: &Path, link: &Path) -> bool {
        if std::os::windows::fs::symlink_dir(target, link).is_ok() {
            return true;
        }
        // `cmd` reads a forward slash as the start of a switch, so the paths
        // have to reach it in Windows form whatever the caller built them from.
        let windows_form = |path: &Path| path.to_string_lossy().replace('/', "\\");
        std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(windows_form(link))
            .arg(windows_form(target))
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn make_link(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    /// The defect a user met: importing a project reported "symbolic links
    /// cannot be imported" and copied nothing.
    ///
    /// Any Node project has links in `node_modules` — on Windows they are
    /// junctions, which `is_symlink` also reports — so a single one anywhere
    /// in the tree destroyed the whole import. `git_clone` had already settled
    /// the policy this should follow: links inside the tree "are ordinary in
    /// real repositories". They are not copied, because a copy is not the
    /// place they point at, but they cannot veto the files beside them.
    #[test]
    fn a_link_inside_an_imported_folder_is_skipped_rather_than_failing_the_import() {
        let p = project();
        let source_root = p.root.parent().expect("parent").join("RomiPlayoff");
        std::fs::create_dir_all(source_root.join("src")).expect("dirs");
        std::fs::create_dir_all(source_root.join("node_modules")).expect("dirs");
        std::fs::write(source_root.join("src/index.ts"), "hello").expect("write");
        std::fs::write(source_root.join("package.json"), "{}").expect("write");

        if !make_link(
            &source_root.join("src"),
            &source_root.join("node_modules/self"),
        ) {
            eprintln!("skipped: this session cannot create links of any kind");
            return;
        }

        let report = import_local_paths(
            &p.root,
            "",
            std::slice::from_ref(&source_root),
            "import-links",
            &limits(),
            |_| {},
            || false,
        )
        .expect("a project with links in node_modules must still import");

        assert_eq!(
            std::fs::read_to_string(p.root.join("RomiPlayoff/src/index.ts")).expect("read"),
            "hello",
            "the files beside the link must be copied"
        );
        assert!(p.root.join("RomiPlayoff/package.json").is_file());
        assert!(
            !p.root.join("RomiPlayoff/node_modules/self").exists(),
            "the link itself is not copied"
        );
        assert_eq!(
            report.skipped_symlinks, 1,
            "what was not copied has to be counted, or the import lies"
        );
        assert!(
            report
                .skipped_symlink_examples
                .iter()
                .any(|path| path.contains("self")),
            "a skipped link must be nameable, got {:?}",
            report.skipped_symlink_examples
        );
    }

    /// Skipping must not turn "import this link" into a silent no-op. The
    /// user asked for one thing; doing nothing and reporting success would be
    /// the worst of both answers.
    #[test]
    fn importing_nothing_but_a_link_is_refused_rather_than_silently_doing_nothing() {
        let p = project();
        let outside = p.root.parent().expect("parent").join("LinkOnly");
        std::fs::create_dir_all(&outside).expect("dirs");
        let target = outside.join("real");
        std::fs::create_dir_all(&target).expect("dirs");
        let link = outside.join("alias");

        if !make_link(&target, &link) {
            eprintln!("skipped: this session cannot create links of any kind");
            return;
        }

        let result = import_local_paths(
            &p.root,
            "",
            std::slice::from_ref(&link),
            "import-link-only",
            &limits(),
            |_| {},
            || false,
        );

        assert!(
            matches!(result, Err(FileError::Refused(_))),
            "got {result:?}"
        );
    }

    /// The bug this exists to prevent: a dropped project produced
    /// `Project/RomiPlayoff/package.json` — the folder the user had already
    /// opened, wrapped in itself.
    #[test]
    fn an_unwrapped_folder_lands_as_its_contents_not_as_itself() {
        let p = project();
        let source_root = p.root.parent().expect("parent").join("RomiPlayoff");
        std::fs::create_dir_all(source_root.join("src/commands")).expect("dirs");
        std::fs::create_dir_all(source_root.join("empty")).expect("dirs");
        std::fs::write(source_root.join("src/commands/ping.ts"), "ping").expect("write");
        std::fs::write(source_root.join("package.json"), "{}").expect("write");
        // A dotfile, because these are the ones an import quietly drops.
        std::fs::write(source_root.join(".env"), "TOKEN=1").expect("write");

        let report = import_local_sources(
            &p.root,
            "",
            &[ImportSource::unwrapped(&source_root)],
            "unwrap-1",
            &limits(),
            |_| {},
            || false,
        )
        .expect("import");

        assert!(p.root.join("package.json").is_file());
        assert!(p.root.join(".env").is_file(), "dotfiles must survive");
        assert!(p.root.join("src/commands/ping.ts").is_file());
        assert!(p.root.join("empty").is_dir(), "empty folders must survive");
        assert!(
            !p.root.join("RomiPlayoff").exists(),
            "the folder itself must not appear"
        );

        let mut top = report
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        top.sort_unstable();
        assert_eq!(top, vec![".env", "empty", "package.json", "src"]);
    }

    /// Importing a project into a project that already has a `src/` must put
    /// the incoming files beside the existing ones rather than refusing, and
    /// must not disturb what was already there.
    #[test]
    fn a_folder_that_already_exists_is_merged_into() {
        let p = project();
        let source_root = p.root.parent().expect("parent").join("incoming");
        std::fs::create_dir_all(source_root.join("src/deep")).expect("dirs");
        std::fs::write(source_root.join("src/added.ts"), "added").expect("write");
        std::fs::write(source_root.join("src/deep/nested.ts"), "nested").expect("write");

        import_local_sources(
            &p.root,
            "",
            &[ImportSource::unwrapped(&source_root)],
            "merge-1",
            &limits(),
            |_| {},
            || false,
        )
        .expect("import");

        assert!(p.root.join("src/added.ts").is_file(), "incoming file");
        assert!(p.root.join("src/deep/nested.ts").is_file(), "nested file");
        assert_eq!(
            std::fs::read_to_string(p.root.join("src/app.ts")).expect("read"),
            "export const a = 1;\n",
            "the file that was already there must be untouched",
        );
    }

    /// The dangerous half of merging: a failure partway through must not take
    /// the user's existing files with it.
    #[test]
    fn a_failed_merge_removes_only_what_it_added() {
        let p = project();
        let source_root = p.root.parent().expect("parent").join("half");
        std::fs::create_dir_all(&source_root).expect("dirs");
        std::fs::create_dir_all(source_root.join("src")).expect("dirs");
        std::fs::write(source_root.join("src/new.ts"), "new").expect("write");

        let mut moved = Vec::new();
        merge_into_place(&source_root.join("src"), &p.root.join("src"), &mut moved).expect("merge");

        // Only the file that did not exist before is recorded, so rolling back
        // cannot delete `src/app.ts` or the `src` directory itself.
        assert_eq!(moved, vec![p.root.join("src/new.ts")]);

        rollback_import(&moved);
        assert!(!p.root.join("src/new.ts").exists(), "added file removed");
        assert!(p.root.join("src/app.ts").is_file(), "existing file kept");
        assert!(p.root.join("src").is_dir(), "existing folder kept");
    }

    #[test]
    fn an_unwrapped_folder_can_land_inside_a_subdirectory() {
        let p = project();
        let source_root = p.root.parent().expect("parent").join("bundle");
        std::fs::create_dir_all(&source_root).expect("dirs");
        std::fs::write(source_root.join("a.txt"), "a").expect("write");

        import_local_sources(
            &p.root,
            "src",
            &[ImportSource::unwrapped(&source_root)],
            "unwrap-2",
            &limits(),
            |_| {},
            || false,
        )
        .expect("import");

        assert!(p.root.join("src/a.txt").is_file());
        assert!(!p.root.join("src/bundle").exists());
    }

    #[test]
    fn unwrapping_still_refuses_to_overwrite_what_is_already_there() {
        let p = project();
        let source_root = p.root.parent().expect("parent").join("clash");
        std::fs::create_dir_all(&source_root).expect("dirs");
        std::fs::write(source_root.join("index.js"), "replacement").expect("write");

        assert!(matches!(
            import_local_sources(
                &p.root,
                "",
                &[ImportSource::unwrapped(&source_root)],
                "unwrap-3",
                &limits(),
                |_| {},
                || false,
            ),
            Err(FileError::AlreadyExists(_))
        ));
        assert_eq!(
            std::fs::read_to_string(p.root.join("index.js")).expect("read"),
            "console.log('hi');\n",
            "the existing file must be untouched",
        );
    }

    #[test]
    fn a_file_cannot_be_unwrapped() {
        let p = project();
        let source = p.root.parent().expect("parent").join("loose.txt");
        std::fs::write(&source, "x").expect("write");

        assert!(matches!(
            import_local_sources(
                &p.root,
                "",
                &[ImportSource::unwrapped(&source)],
                "unwrap-4",
                &limits(),
                |_| {},
                || false,
            ),
            Err(FileError::Refused(_))
        ));
    }

    #[test]
    fn importing_onto_an_existing_path_is_refused_before_copying() {
        let p = project();
        let source = p.root.parent().expect("parent").join("index.js");
        std::fs::write(&source, "replacement").expect("write");

        assert!(matches!(
            import_local_paths(
                &p.root,
                "",
                &[source],
                "import-2",
                &limits(),
                |_| {},
                || false
            ),
            Err(FileError::AlreadyExists(_))
        ));
        assert_eq!(
            std::fs::read_to_string(p.root.join("index.js")).expect("read"),
            "console.log('hi');\n",
        );
        assert!(!p.root.join(".project-host-import-import-2").exists());
    }

    /// A crash partway through an import leaves its staging directory in the
    /// project, where the user sees it as a half-copied folder they did not
    /// create. The next import clears it up.
    #[test]
    fn staging_left_behind_by_a_crashed_import_is_swept_away() {
        let p = project();
        let abandoned = p.root.join(".project-host-import-import-dead");
        std::fs::create_dir_all(abandoned.join("half/copied")).expect("dirs");
        std::fs::write(abandoned.join("half/copied/part.bin"), "partial").expect("write");

        let removed = remove_abandoned_import_staging(&p.root, |_| false).expect("sweep");

        assert_eq!(
            removed,
            vec![".project-host-import-import-dead".to_string()]
        );
        assert!(!abandoned.exists());
        assert!(
            p.root.join("index.js").is_file(),
            "the project is untouched"
        );
    }

    /// The dangerous case: an import that is still copying owns its staging
    /// directory, and sweeping it would delete the files out from under a copy
    /// that is still writing them.
    #[test]
    fn staging_belonging_to_a_running_import_is_left_alone() {
        let p = project();
        let running = p.root.join(".project-host-import-import-live");
        let abandoned = p.root.join(".project-host-import-import-dead");
        std::fs::create_dir(&running).expect("create");
        std::fs::create_dir(&abandoned).expect("create");

        let removed =
            remove_abandoned_import_staging(&p.root, |id| id == "import-live").expect("sweep");

        assert_eq!(
            removed,
            vec![".project-host-import-import-dead".to_string()]
        );
        assert!(
            running.is_dir(),
            "a running import lost its staging directory"
        );
    }

    /// The sweep runs at the project root, where the user's own files live.
    #[test]
    fn the_sweep_only_touches_import_staging_directories() {
        let p = project();
        std::fs::create_dir(p.root.join(".github")).expect("create");
        std::fs::write(p.root.join(".project-host-import-not-a-directory"), "x").expect("write");
        std::fs::create_dir(p.root.join(".project-host-upload-abc.tmp")).expect("create");

        let removed = remove_abandoned_import_staging(&p.root, |_| false).expect("sweep");

        assert!(removed.is_empty(), "removed {removed:?}");
        assert!(p.root.join(".github").is_dir());
        assert!(p.root.join("src/app.ts").is_file());
        assert!(p
            .root
            .join(".project-host-import-not-a-directory")
            .is_file());
    }

    fn cleanup_failed() -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "the staging directory is in use",
        ))
    }

    /// The files are already in the project by the time the staging directory
    /// is cleaned up. Reporting the cleanup as a failure tells the user their
    /// import failed while the files sit exactly where they asked for them, and
    /// the obvious retry then fails again with `AlreadyExists`.
    #[test]
    fn a_staging_directory_that_will_not_delete_does_not_fail_a_finished_import() {
        assert_eq!(
            import_outcome(Ok("imported"), cleanup_failed(), ".stage"),
            Ok("imported")
        );
    }

    /// And the ordinary case still reports what was imported.
    #[test]
    fn a_clean_import_reports_what_it_imported() {
        assert_eq!(
            import_outcome(Ok("imported"), Ok(()), ".stage"),
            Ok("imported")
        );
    }

    /// An import that really did fail has to say why it failed. Letting the
    /// cleanup error win would replace "that file already exists" — which the
    /// user can act on — with a stray permissions error about a path they have
    /// never seen.
    #[test]
    fn a_failed_import_reports_its_own_error_rather_than_the_cleanup_error() {
        let refused: Result<&str, FileError> = Err(FileError::Refused("import cancelled"));

        assert_eq!(
            import_outcome(refused, cleanup_failed(), ".stage"),
            Err(FileError::Refused("import cancelled")),
        );
    }

    #[test]
    fn cancelling_a_local_import_removes_staged_files() {
        use std::cell::Cell;
        use std::rc::Rc;

        let p = project();
        let source = p.root.parent().expect("parent").join("large.bin");
        std::fs::write(&source, vec![7u8; 2 * 1024 * 1024]).expect("write");
        let saw_progress = Rc::new(Cell::new(false));
        let progress_flag = Rc::clone(&saw_progress);
        let cancel_flag = Rc::clone(&saw_progress);

        assert!(matches!(
            import_local_paths(
                &p.root,
                "",
                &[source],
                "import-3",
                &limits(),
                move |event| {
                    if event.copied_bytes > 0 {
                        progress_flag.set(true);
                    }
                },
                move || cancel_flag.get(),
            ),
            Err(FileError::Refused("import cancelled"))
        ));
        assert!(!p.root.join("large.bin").exists());
        assert!(!p.root.join(".project-host-import-import-3").exists());
    }

    #[test]
    fn copying_onto_an_existing_path_is_refused() {
        let p = project();
        assert!(matches!(
            copy(&p.root, "index.js", "README.md", &limits()),
            Err(FileError::AlreadyExists(_))
        ));
    }

    #[test]
    fn search_matches_names_case_insensitively() {
        let p = project();
        let hits = search(&p.root, "", "APP", &limits()).expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "src/app.ts");
    }

    #[test]
    fn search_is_capped_and_an_empty_query_matches_nothing() {
        let p = project();
        assert!(search(&p.root, "", "   ", &limits())
            .expect("search")
            .is_empty());

        for i in 0..10 {
            std::fs::write(p.root.join(format!("match{i}.txt")), "x").expect("write");
        }
        let capped = FileLimits {
            max_search_results: 3,
            ..limits()
        };
        assert_eq!(
            search(&p.root, "", "match", &capped).expect("search").len(),
            3
        );
    }

    #[test]
    fn directory_size_sums_the_tree() {
        let p = project();
        let total = directory_size(&p.root, "", &limits()).expect("size");
        assert_eq!(total, 19 + 20 + 8);
    }

    #[test]
    fn stat_reports_a_missing_file_as_missing() {
        let p = project();
        assert!(matches!(
            stat(&p.root, "nope.js"),
            Err(FileError::NotFound(_))
        ));
    }

    #[test]
    fn language_detection_covers_names_without_extensions() {
        assert_eq!(language_for("Dockerfile"), "dockerfile");
        assert_eq!(language_for("a/b/.env"), "dotenv");
        assert_eq!(language_for("src/main.py"), "python");
        assert_eq!(language_for("weird.qqq"), "plaintext");
        assert_eq!(language_for("noextension"), "plaintext");
    }

    #[test]
    fn binary_sniffing_uses_the_prefix() {
        assert!(looks_binary(&[b'a', 0, b'b']));
        assert!(!looks_binary(b"plain text"));
        assert!(!looks_binary("caf\u{e9}".as_bytes()));
        // Invalid UTF-8 without a NUL is still binary.
        assert!(looks_binary(&[0xff, 0xfe, 0xfd]));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_escaping_the_project_is_refused() {
        use std::os::unix::fs::symlink;
        let p = project();
        let outside = p.root.parent().expect("parent").join("outside.txt");
        std::fs::write(&outside, "secret").expect("write");
        symlink(&outside, p.root.join("link.txt")).expect("symlink");

        assert!(matches!(
            read_text_file(&p.root, "link.txt", &limits()),
            Err(FileError::Path(_)) | Err(FileError::Refused(_))
        ));
        // It is still listed, so the user can see and remove it.
        let listing = list_directory(&p.root, "", &limits()).expect("list");
        let link = listing
            .entries
            .iter()
            .find(|e| e.name == "link.txt")
            .expect("link listed");
        assert!(link.is_symlink);
        assert_eq!(link.kind, EntryKind::Other);

        // Deleting removes the link, not the target.
        delete(&p.root, "link.txt", false).expect("delete link");
        assert!(outside.exists(), "the target must survive");
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_containing_a_symlink_cannot_be_copied() {
        use std::os::unix::fs::symlink;
        let p = project();
        symlink("/etc/passwd", p.root.join("src/link")).expect("symlink");
        assert!(matches!(
            copy(&p.root, "src", "copy", &limits()),
            Err(FileError::Refused(_))
        ));
    }
}
