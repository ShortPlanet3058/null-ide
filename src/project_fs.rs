//! A project's files, wherever they are: on this computer (std::fs), or on a server
//! (`/ssh:host:/path`, through `remote`). What Null does with a project's files goes
//! through here; its own files (settings, sessions, caches) don't need to.
//!
//! A server's answers about files (there, its kind, size, last change) are kept a moment,
//! and a folder's listing answers for what's in it: walking up folders, or asking during
//! painting, doesn't wait on the network each time. What Null changes there is forgotten
//! at once, and what the watcher reports too.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use crate::remote::{self, RemotePath};

/// What's known of a file: its kind, size, last change, and whether it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
    dir: bool,
    len: u64,
    modified: Option<SystemTime>,
    executable: bool,
}

impl Meta {
    pub fn is_dir(&self) -> bool {
        self.dir
    }

    pub fn is_file(&self) -> bool {
        !self.dir
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn modified(&self) -> io::Result<SystemTime> {
        self.modified.ok_or_else(|| io::Error::other("no time of change"))
    }

    pub fn executable(&self) -> bool {
        self.executable
    }

    fn of_local(meta: &std::fs::Metadata) -> Meta {
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            meta.is_file() && meta.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        Meta {
            dir: meta.is_dir(),
            len: meta.len(),
            modified: meta.modified().ok(),
            executable,
        }
    }

    fn of_remote(meta: remote::Meta) -> Meta {
        Meta { dir: meta.dir, len: meta.len, modified: meta.modified, executable: meta.executable }
    }
}

/// How long a server's answer is taken as still true.
const FRESH: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Known {
    /// A path's metadata, or that it isn't there.
    files: HashMap<PathBuf, (Option<Meta>, Instant)>,
    /// A folder's names, when it was listed.
    folders: HashMap<PathBuf, (HashSet<String>, Instant)>,
}

static KNOWN: Mutex<Option<Known>> = Mutex::new(None);

fn with_known<T>(f: impl FnOnce(&mut Known) -> T) -> T {
    let mut known = KNOWN.lock().unwrap_or_else(|e| e.into_inner());
    f(known.get_or_insert_with(Known::default))
}

/// One form for a server's path, whatever the separators.
fn key(at: &RemotePath) -> PathBuf {
    remote::path(&at.host, &at.path)
}

/// What's known of a server's path without asking: Some(None) when it's known not to be there.
fn known(at: &RemotePath) -> Option<Option<Meta>> {
    let path = key(at);
    with_known(|known| {
        if let Some((meta, when)) = known.files.get(&path)
            && when.elapsed() < FRESH
        {
            return Some(*meta);
        }
        // (Not in its folder's fresh listing: not there.)
        let parent = path.parent()?;
        let name = path.file_name()?.to_str()?;
        match known.folders.get(parent) {
            Some((names, when)) if when.elapsed() < FRESH && !names.contains(name) => Some(None),
            _ => None,
        }
    })
}

fn remember(at: &RemotePath, meta: Option<Meta>) {
    let path = key(at);
    with_known(|known| known.files.insert(path, (meta, Instant::now())));
}

/// Forgets what's known of `path` and its folder's listing (it changed).
pub fn forget(path: &Path) {
    let Some(at) = remote::of(path) else { return };
    let path = key(&at);
    with_known(|known| {
        known.files.remove(&path);
        if let Some(parent) = path.parent() {
            known.folders.remove(parent);
        }
        known.folders.remove(&path);
    });
}

pub fn metadata(path: &Path) -> io::Result<Meta> {
    let Some(at) = remote::of(path) else {
        return std::fs::metadata(path).map(|m| Meta::of_local(&m));
    };
    match known(&at) {
        Some(Some(meta)) => return Ok(meta),
        Some(None) => return Err(io::Error::new(io::ErrorKind::NotFound, "not found")),
        None => {}
    }
    match remote::metadata(&at) {
        Ok(meta) => {
            let meta = Meta::of_remote(meta);
            remember(&at, Some(meta));
            Ok(meta)
        }
        Err(error) => {
            if error.kind() == io::ErrorKind::NotFound {
                remember(&at, None);
            }
            Err(error)
        }
    }
}

pub fn exists(path: &Path) -> bool {
    metadata(path).is_ok()
}

pub fn is_dir(path: &Path) -> bool {
    metadata(path).is_ok_and(|m| m.is_dir())
}

/// Whether `path` is there, without waiting on a server: what's known of it, else `assume`.
pub fn exists_now(path: &Path, assume: bool) -> bool {
    match remote::of(path) {
        None => path.exists(),
        Some(at) => known(&at).map_or(assume, |meta| meta.is_some()),
    }
}

pub fn read(path: &Path) -> io::Result<Vec<u8>> {
    match remote::of(path) {
        None => std::fs::read(path),
        Some(at) => remote::read(&at),
    }
}

pub fn read_to_string(path: &Path) -> io::Result<String> {
    String::from_utf8(read(path)?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// The first `limit` bytes of a file (what's there, when it's shorter).
pub fn read_head(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    match remote::of(path) {
        None => {
            use std::io::Read;
            let mut head = Vec::new();
            std::fs::File::open(path)?.take(limit as u64).read_to_end(&mut head)?;
            Ok(head)
        }
        Some(at) => {
            let output = remote::run(&at.host, &format!("head -c {limit} -- {}", remote::quote(&at.path)))?;
            if output.success() { Ok(output.stdout) } else { remote::read(&at) }
        }
    }
}

/// Writes `bytes` to `path` as it is (no temporary file: see `fs_ops::write_file` for saving).
pub fn write(path: &Path, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    match remote::of(path) {
        None => std::fs::write(path, bytes),
        Some(at) => {
            forget(path);
            remote::write(&at, bytes.as_ref())
        }
    }
}

/// What's in a folder: each path, with what's known of it (links followed).
pub fn read_dir(path: &Path) -> io::Result<Vec<(PathBuf, Meta)>> {
    let Some(at) = remote::of(path) else {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            // (A link: what it points to; a broken one as a file.)
            let meta = std::fs::metadata(&path).or_else(|_| entry.metadata());
            if let Ok(meta) = meta {
                entries.push((path, Meta::of_local(&meta)));
            }
        }
        return Ok(entries);
    };
    let listed = remote::read_dir(&at)?;
    let folder = key(&at);
    let now = Instant::now();
    let entries: Vec<(PathBuf, Meta)> =
        listed.into_iter().map(|(name, meta)| (folder.join(&name), Meta::of_remote(meta))).collect();
    with_known(|known| {
        let names = entries.iter().filter_map(|(p, _)| p.file_name()?.to_str().map(str::to_string)).collect();
        known.folders.insert(folder.clone(), (names, now));
        for (path, meta) in &entries {
            known.files.insert(path.clone(), (Some(*meta), now));
        }
    });
    Ok(entries)
}

pub fn create_dir_all(path: &Path) -> io::Result<()> {
    match remote::of(path) {
        None => std::fs::create_dir_all(path),
        Some(at) => {
            forget(path);
            remote::create_dir_all(&at)
        }
    }
}

/// Moves `from` to `to`; both on this computer, or on the same server.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    match (remote::of(from), remote::of(to)) {
        (None, None) => std::fs::rename(from, to),
        (Some(a), Some(b)) if a.host == b.host => {
            forget(from);
            forget(to);
            remote::rename(&a, &b)
        }
        _ => Err(io::Error::new(io::ErrorKind::Unsupported, "can't move between this computer and a server")),
    }
}

/// Copies a file; both on this computer, or on the same server.
pub fn copy(from: &Path, to: &Path) -> io::Result<()> {
    match (remote::of(from), remote::of(to)) {
        (None, None) => std::fs::copy(from, to).map(drop),
        (Some(a), Some(b)) if a.host == b.host => {
            forget(to);
            remote::copy(&a, &b)
        }
        // (Across: through Null.)
        _ => write(to, read(from)?),
    }
}

/// The path with links followed (`dunce`'s on Windows: no `\\?\`); a server's asked.
pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
    match remote::of(path) {
        None => dunce::canonicalize(path),
        Some(at) => remote::canonicalize(&at).map(|c| key(&c)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_files_as_std_has_them() {
        let dir = std::env::temp_dir().join(format!("null-project-fs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create_dir_all(&dir.join("a")).unwrap();
        write(&dir.join("a/x.txt"), "hello").unwrap();
        assert_eq!(read_to_string(&dir.join("a/x.txt")).unwrap(), "hello");
        assert_eq!(read_head(&dir.join("a/x.txt"), 2).unwrap(), b"he");
        assert_eq!(metadata(&dir.join("a/x.txt")).unwrap().len(), 5);
        assert!(is_dir(&dir.join("a")) && !is_dir(&dir.join("a/x.txt")) && !exists(&dir.join("b")));
        let listed = read_dir(&dir.join("a")).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, dir.join("a/x.txt"));
        rename(&dir.join("a/x.txt"), &dir.join("a/y.txt")).unwrap();
        assert!(exists_now(&dir.join("a/y.txt"), false));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[cfg(unix)]
    fn a_server_s_listing_answers_for_what_s_in_it() {
        // (Links followed: a Mac's temporary folder is one.)
        let dir = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("null-project-fs-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        let root = remote::path(remote::TEST_HOST, &dir.to_string_lossy());
        assert!(!exists_now(&root.join("src/main.rs"), false), "nothing known yet");
        let listed = read_dir(&root.join("src")).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(exists_now(&root.join("src/main.rs"), false), "known from the listing");
        assert!(!exists_now(&root.join("src/other.rs"), true), "not in the listing");
        assert_eq!(read_to_string(&root.join("src/main.rs")).unwrap(), "fn main() {}");
        write(&root.join("src/new.rs"), "x").unwrap();
        assert!(exists(&root.join("src/new.rs")), "a change forgets the listing");
        assert_eq!(canonicalize(&root.join("src/../src")).unwrap(), root.join("src"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
