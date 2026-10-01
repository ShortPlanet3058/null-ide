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
    check_name(new_name, false)?;
    let target = path.with_file_name(new_name.trim());
    if target == path {
        return Ok(target);
    }
    // On case-insensitive disks "readme" and "README" are the same file: allow changing case.
    let same_file_other_case = target.to_string_lossy().to_lowercase() == path.to_string_lossy().to_lowercase();
    if target.exists() && !same_file_other_case {
        return Err(format!("{} already exists", new_name.trim()));
    }
    std::fs::rename(path, &target).map_err(|e| format!("Couldn't rename: {e}"))?;
    Ok(target)
}

/// Copies a file or folder next to itself as "name copy.ext", "name copy 2.ext", ...
pub fn duplicate(path: &Path) -> Result<PathBuf, String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = if path.is_dir() { None } else { path.extension().map(|e| e.to_string_lossy().into_owned()) };
    let name = |n: usize| {
        let copy = if n == 1 { format!("{stem} copy") } else { format!("{stem} copy {n}") };
        match &ext {
            Some(ext) => format!("{copy}.{ext}"),
            None => copy,
        }
    };
    let target = (1..1000).map(|n| path.with_file_name(name(n))).find(|p| !p.exists()).ok_or("Too many copies")?;
    copy_recursively(path, &target).map_err(|e| format!("Couldn't duplicate: {e}"))?;
    Ok(target)
}

fn copy_recursively(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
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
    trash::delete(path).map_err(|e| format!("Couldn't move to the Trash: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
