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
fn same_file(a: &Path, b: &Path) -> bool {
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

/// Moves to the Trash (or the platform's recycle bin): never deletes outright.
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

#[cfg(test)]
mod tests {

    #[test]
    fn moves_into_another_folder_but_not_into_itself_or_over_something() {
        let dir = std::env::temp_dir().join(format!("null-move-{}", std::process::id()));
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
        let root = std::env::temp_dir().join(format!("null-copy-into-{}", std::process::id()));
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
        let dir = std::env::temp_dir().join(format!("null-fs-{name}-{}", std::process::id()));
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
