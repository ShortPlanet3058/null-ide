//! Creating, renaming, duplicating and deleting files for the file tree.
//! Errors are sentences meant to be shown as they are.

use std::path::{Component, Path, PathBuf};

/// Checks a name typed in the tree. `nested` allows `a/b/c.rs` to create folders on the way.
fn check_name(name: &str, nested: bool) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a name".into());
    }
    if !nested && name.contains(['/', '\\']) {
        return Err("A name can't contain / or \\".into());
    }
    if Path::new(name).components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("That name isn't allowed".into());
    }
    Ok(())
}

pub fn create_file(dir: &Path, name: &str) -> Result<PathBuf, String> {
    check_name(name, true)?;
    let path = dir.join(name.trim());
    if path.exists() {
        return Err(format!("{} already exists", name.trim()));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Couldn't create the folder: {e}"))?;
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| format!("Couldn't create the file: {e}"))?;
    Ok(path)
}

pub fn create_dir(dir: &Path, name: &str) -> Result<PathBuf, String> {
    check_name(name, true)?;
    let path = dir.join(name.trim());
    if path.exists() {
        return Err(format!("{} already exists", name.trim()));
    }
    std::fs::create_dir_all(&path).map_err(|e| format!("Couldn't create the folder: {e}"))?;
    Ok(path)
}

pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let target = rename_target(path, new_name)?;
    if target == path {
        return Ok(target);
    }
    std::fs::rename(path, &target).map_err(|e| format!("Couldn't rename: {e}"))?;
    Ok(target)
}

/// Where moving `path` into folder `dir` would put it, if that's allowed: not into
/// itself, and not over something already there.
pub fn move_target(path: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().ok_or("Nothing to move")?;
    let target = dir.join(name);
    if dir.starts_with(path) {
        return Err("A folder can't go inside itself".into());
    }
    if target != path && target.exists() {
        return Err(format!("{} already has a {}", folder_name(dir), name.to_string_lossy()));
    }
    Ok(target)
}

fn folder_name(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| dir.display().to_string())
}

/// Moves `path` to `target` (as [`move_target`] or [`rename_target`] worked out).
pub fn move_path(path: &Path, target: &Path) -> Result<PathBuf, String> {
    if target == path {
        return Ok(target.to_path_buf());
    }
    if path.parent() == target.parent() {
        let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return rename(path, &name);
    }
    let dir = target.parent().ok_or("Nowhere to move it")?;
    let checked = move_target(path, dir)?;
    std::fs::rename(path, &checked).map_err(|e| format!("Couldn't move: {e}"))?;
    Ok(checked)
}

/// Where renaming `path` to `new_name` would put it, if that's allowed.
pub fn rename_target(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    check_name(new_name, false)?;
    let target = path.with_file_name(new_name.trim());
    // On case-insensitive disks "readme" and "README" are the same file: changing case is
    // fine. On case-sensitive ones they can be two files, and the other one must stay.
    if target != path && target.exists() && !same_file(path, &target) {
        return Err(format!("{} already exists", new_name.trim()));
    }
    Ok(target)
}

/// Copies a file or folder next to itself as "name copy.ext", "name copy 2.ext", ...
pub fn duplicate(path: &Path) -> Result<PathBuf, String> {
    let dir = path.parent().ok_or("Nowhere to copy it")?;
    let target = copy_name(path, dir)?;
    copy_recursively(path, &target).map_err(|e| format!("Couldn't duplicate: {e}"))?;
    Ok(target)
}

/// Copies a file or folder into folder `dir`, under its own name, or "name copy.ext" when
/// that's taken: nothing there is written over.
pub fn copy_into(path: &Path, dir: &Path) -> Result<PathBuf, String> {
    if dir.starts_with(path) {
        return Err("A folder can't go inside itself".into());
    }
    let name = path.file_name().ok_or("Nothing to copy")?;
    let target = dir.join(name);
    let target = if target.exists() { copy_name(path, dir)? } else { target };
    copy_recursively(path, &target).map_err(|e| format!("Couldn't copy {}: {e}", name.to_string_lossy()))?;
    Ok(target)
}

/// A name in `dir` for a copy of `path` nothing has yet: "name copy.ext", "name copy 2.ext"...
fn copy_name(path: &Path, dir: &Path) -> Result<PathBuf, String> {
    // A folder's whole name (`my.app`), a file's without its extension.
    let whole = |p: &Path| p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = if path.is_dir() {
        whole(path)
    } else {
        path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    };
    let ext = if path.is_dir() { None } else { path.extension().map(|e| e.to_string_lossy().into_owned()) };
    let name = |n: usize| {
        let copy = if n == 1 { format!("{stem} copy") } else { format!("{stem} copy {n}") };
        match &ext {
            Some(ext) => format!("{copy}.{ext}"),
            None => copy,
        }
    };
    Ok((1..1000).map(|n| dir.join(name(n))).find(|p| !p.exists()).ok_or("Too many copies")?)
}

/// Whether two paths name the same file on disk (one file under two spellings).
pub fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        // Windows disks don't tell case apart.
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    }
}

/// Copies files and folders; a link is copied as a link, never followed, so a link to a
/// folder above it can't make the copy go on forever.
fn copy_recursively(from: &Path, to: &Path) -> std::io::Result<()> {
    let kind = std::fs::symlink_metadata(from)?.file_type();
    if kind.is_symlink() {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(std::fs::read_link(from)?, to);
        #[cfg(not(unix))]
        return std::fs::copy(from, to).map(|_| ());
    }
    if kind.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Writes a file whole or not at all: the bytes go to a file beside it first, then take
/// its place, so a crash or a full disk mid-write leaves the old file as it was. A link
/// is written through to its file; the file keeps its permissions, tags and extended
/// attributes. A read-only file isn't written. A file with other hard links, someone
/// else's file, or one in a folder Null can't add to is written in place instead. What
/// the file held before is kept in its local history.
pub fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SAVES: AtomicU64 = AtomicU64::new(0);
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let existing = std::fs::metadata(&target).ok();
    if existing.as_ref().is_some_and(|m| m.permissions().readonly()) {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "it's read-only"));
    }
    #[cfg(unix)]
    let in_place = existing.as_ref().is_some_and(|m| {
        use std::os::unix::fs::MetadataExt;
        // Swapping in a new file would make it this user's.
        m.nlink() > 1 || !owned_by_me(m.uid())
    });
    #[cfg(not(unix))]
    let in_place = false;
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else { return std::fs::write(path, bytes) };
    // What it held is kept a while, to go back to (Show File History).
    crate::local_history::keep_before_writing(&target, bytes);
    if in_place {
        return std::fs::write(&target, bytes);
    }
    // Its own name, so two saves at once (an AI task, auto-save) never share one.
    let n = SAVES.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(".{}.null-saving-{}-{n}", name.to_string_lossy(), std::process::id()));
    let written = std::fs::File::options().write(true).create_new(true).open(&temp).and_then(|mut file| {
        file.write_all(bytes)?;
        if let Some(meta) = &existing {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()
    });
    if written.is_ok() && existing.is_some() {
        copy_metadata(&target, &temp);
    }
    match written.and_then(|()| std::fs::rename(&temp, &target)) {
        Ok(()) => Ok(()),
        Err(error) => {
            std::fs::remove_file(&temp).ok();
            // Couldn't make the file beside it (a folder only the file itself can be
            // written in): in place, as before.
            if error.kind() == std::io::ErrorKind::PermissionDenied {
                std::fs::write(&target, bytes)
            } else {
                Err(error)
            }
        }
    }
}

/// Whether the file's owner is the user Null runs as.
#[cfg(unix)]
fn owned_by_me(uid: u32) -> bool {
    #[cfg(target_os = "macos")]
    {
        // Safety: no arguments, can't fail.
        uid == unsafe { libc::geteuid() }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = uid;
        true
    }
}

/// The file's tags, extended attributes and access lists, onto the new one.
#[cfg(target_os = "macos")]
fn copy_metadata(from: &Path, to: &Path) {
    use std::os::unix::ffi::OsStrExt;
    let (Ok(from), Ok(to)) =
        (std::ffi::CString::new(from.as_os_str().as_bytes()), std::ffi::CString::new(to.as_os_str().as_bytes()))
    else {
        return;
    };
    // Safety: two valid C strings; no state.
    unsafe { libc::copyfile(from.as_ptr(), to.as_ptr(), std::ptr::null_mut(), libc::COPYFILE_METADATA) };
}

#[cfg(not(target_os = "macos"))]
fn copy_metadata(_: &Path, _: &Path) {}

/// Moves to the Trash (or the platform's recycle bin): never deletes outright.
#[cfg(not(test))]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut context = trash::TrashContext::default();
    // On macOS, ask the system directly instead of scripting Finder: faster, and no
    // "Null wants to control Finder" permission prompt.
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        context.set_delete_method(DeleteMethod::NsFileManager);
    }
    context.delete(path).map_err(|e| format!("Couldn't move to the Trash: {e}"))
}

/// In tests, a folder of their own stands in for the Trash: nothing of theirs lands in
/// the real one.
#[cfg(test)]
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    let trash = test_trash();
    std::fs::create_dir_all(&trash).map_err(|e| e.to_string())?;
    let name = path.file_name().ok_or("Nothing to move")?;
    let mut to = trash.join(name);
    let mut n = 1;
    while to.symlink_metadata().is_ok() {
        n += 1;
        to = trash.join(format!("{}-{n}", name.to_string_lossy()));
    }
    std::fs::rename(path, &to).map_err(|e| format!("Couldn't move to the Trash: {e}"))
}

/// Where tests' "Trash" is.
#[cfg(test)]
pub fn test_trash() -> std::path::PathBuf {
    crate::tools::test_dir("test-trash")
}

#[cfg(test)]
mod tests {

    /// A save takes the old file's place whole, through a link, keeping its permissions.
    #[test]
    #[cfg(unix)]
    fn saves_replace_files_whole() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::tools::test_dir("write-file");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("run.sh");
        std::fs::write(&file, "old\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        super::write_file(&file, b"new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o755);
        // Through a link: the file changes, the link stays a link.
        let link = dir.join("link.sh");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        super::write_file(&link, b"linked\n").unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "linked\n");
        // A new file, and nothing left beside them.
        super::write_file(&dir.join("new.txt"), b"x").unwrap();
        let mut names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["link.sh", "new.txt", "run.sh"]);
        // A read-only file isn't written over.
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(super::write_file(&file, b"over\n").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "linked\n");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn moves_into_another_folder_but_not_into_itself_or_over_something() {
        let dir = crate::tools::test_dir("move");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/inner")).unwrap();
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join("notes.md"), "n").unwrap();
        std::fs::write(dir.join("docs/notes.md"), "other").unwrap();
        std::fs::write(dir.join("todo.md"), "t").unwrap();
        // Into a folder: there, with its name.
        assert_eq!(move_path(&dir.join("todo.md"), &dir.join("src/todo.md")).unwrap(), dir.join("src/todo.md"));
        assert!(dir.join("src/todo.md").is_file() && !dir.join("todo.md").exists());
        // Not into itself, nor over a file of the same name.
        assert!(move_target(&dir.join("src"), &dir.join("src/inner")).is_err());
        assert_eq!(move_target(&dir.join("notes.md"), &dir.join("docs")).unwrap_err(), "docs already has a notes.md");
        std::fs::remove_dir_all(&dir).ok();
    }

    use super::*;

    #[test]
    fn copies_dropped_files_without_writing_over_anything() {
        let root = crate::tools::test_dir("copy-into");
        let _ = std::fs::remove_dir_all(&root);
        let (from, into) = (root.join("outside"), root.join("project/src"));
        std::fs::create_dir_all(from.join("assets")).unwrap();
        std::fs::create_dir_all(&into).unwrap();
        std::fs::write(from.join("a.txt"), "new").unwrap();
        std::fs::write(from.join("assets/logo.svg"), "<svg/>").unwrap();
        std::fs::write(into.join("a.txt"), "old").unwrap();
        // Taken: a copy beside it, the old one kept.
        assert_eq!(copy_into(&from.join("a.txt"), &into), Ok(into.join("a copy.txt")));
        assert_eq!(std::fs::read_to_string(into.join("a.txt")).unwrap(), "old");
        // A folder, with what's in it.
        assert_eq!(copy_into(&from.join("assets"), &into), Ok(into.join("assets")));
        assert!(into.join("assets/logo.svg").is_file());
        assert!(copy_into(&root.join("project"), &into).is_err());
        // A folder with a dot keeps its whole name.
        std::fs::create_dir_all(into.join("my.app")).unwrap();
        assert_eq!(duplicate(&into.join("my.app")), Ok(into.join("my.app copy")));
        std::fs::remove_dir_all(&root).ok();
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = crate::tools::test_dir(&format!("fs-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creates_files_and_nested_folders() {
        let dir = temp_dir("create");
        let file = create_file(&dir, "a/b/main.rs").unwrap();
        assert!(file.is_file() && dir.join("a/b").is_dir());
        assert_eq!(create_file(&dir, "a/b/main.rs").unwrap_err(), "a/b/main.rs already exists");
        assert!(create_file(&dir, "  ").is_err());
        assert!(create_file(&dir, "../escape.rs").is_err());
        assert!(create_dir(&dir, "src").unwrap().is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn renames_and_duplicates() {
        let dir = temp_dir("rename");
        let file = create_file(&dir, "notes.txt").unwrap();
        assert!(rename(&file, "x/y.txt").is_err());
        let renamed = rename(&file, "todo.txt").unwrap();
        assert!(renamed.is_file() && !file.exists());
        assert_eq!(duplicate(&renamed).unwrap().file_name().unwrap(), "todo copy.txt");
        assert_eq!(duplicate(&renamed).unwrap().file_name().unwrap(), "todo copy 2.txt");
        let folder = create_dir(&dir, "src").unwrap();
        create_file(&folder, "lib.rs").unwrap();
        assert!(duplicate(&folder).unwrap().join("lib.rs").is_file());
        // A link back to a folder above is copied as a link, not followed forever.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, folder.join("up")).unwrap();
            let copy = duplicate(&folder).unwrap();
            assert!(std::fs::symlink_metadata(copy.join("up")).unwrap().file_type().is_symlink());
        }
        // Changing only the case renames; a different file with that name is never replaced.
        let lower = create_file(&dir, "notes.md").unwrap();
        let upper = dir.join("NOTES.md");
        if upper.exists() {
            // Case-insensitive disk: the same file.
            assert_eq!(rename(&lower, "NOTES.md").unwrap(), upper);
        } else {
            std::fs::write(&upper, "keep me").unwrap();
            assert!(rename(&lower, "NOTES.md").is_err());
            assert_eq!(std::fs::read_to_string(&upper).unwrap(), "keep me");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
