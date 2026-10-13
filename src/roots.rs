//! The folders a window has open: one, as usual, or several (each a heading in the files).
//! The first is the primary one: where a new terminal starts with no file open, whose
//! project settings are used, after which the window is named.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectRoots {
    /// Never empty; the first is the primary folder.
    roots: Vec<PathBuf>,
}

impl ProjectRoots {
    pub fn new(primary: PathBuf) -> Self {
        Self { roots: vec![primary] }
    }

    pub fn primary(&self) -> &Path {
        &self.roots[0]
    }

    pub fn all(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether the window has several folders.
    pub fn several(&self) -> bool {
        self.roots.len() > 1
    }

    /// The folder `path` is in, if it's in one (the innermost, should one hold another).
    pub fn root_of(&self, path: &Path) -> Option<&Path> {
        self.roots.iter().filter(|r| path.starts_with(r)).max_by_key(|r| r.as_os_str().len()).map(PathBuf::as_path)
    }

    /// The folder `path` is in, else the primary one.
    pub fn root_for(&self, path: &Path) -> &Path {
        self.root_of(path).unwrap_or_else(|| self.primary())
    }

    /// Whether `path` is in one of the folders.
    pub fn contains(&self, path: &Path) -> bool {
        self.root_of(path).is_some()
    }

    /// `path` as shown: inside its folder (`src/main.rs`), with the folder's name first when
    /// the window has several (`api/src/main.rs`); outside them all, whole.
    pub fn display(&self, path: &Path) -> String {
        let Some(root) = self.root_of(path) else { return path.display().to_string() };
        let inside = path.strip_prefix(root).unwrap_or(path);
        match (self.several(), root.file_name()) {
            (true, Some(_)) => Path::new(&self.label(root)).join(inside).display().to_string(),
            _ => inside.display().to_string(),
        }
    }

    /// A folder's name, for headings and titles.
    pub fn name(root: &Path) -> String {
        root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned())
    }

    /// `root`'s name among the window's folders: with its parent's when another has the
    /// same name (`a/app`, `b/app`, with the system's separator), its whole path when even that's the same.
    pub fn label(&self, root: &Path) -> String {
        Self::label_among(&self.roots, root)
    }

    /// [`Self::label`] among `all`.
    pub fn label_among(all: &[PathBuf], root: &Path) -> String {
        let name = Self::name(root);
        let others: Vec<&PathBuf> = all.iter().filter(|r| r.as_path() != root && Self::name(r) == name).collect();
        if others.is_empty() {
            return name;
        }
        let parent = |p: &Path| p.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
        match parent(root) {
            Some(up) if others.iter().all(|r| parent(r).as_deref() != Some(up.as_str())) => {
                Path::new(&up).join(&name).display().to_string()
            }
            _ => root.display().to_string(),
        }
    }

    /// Adds `root` (a folder inside one already there, or that one, changes nothing; nor
    /// one holding the primary, which stays what the window's session is kept by). Whether it
    /// was added.
    pub fn add(&mut self, root: PathBuf) -> bool {
        if self.roots.iter().any(|r| root.starts_with(r)) || self.primary().starts_with(&root) {
            return false;
        }
        self.roots.retain(|r| !r.starts_with(&root));
        self.roots.push(root);
        true
    }

    /// Removes `root`, unless it's the primary one (what the window's session and unsaved
    /// work are kept by). Whether it was removed.
    pub fn remove(&mut self, root: &Path) -> bool {
        if root == self.primary() || !self.roots.iter().any(|r| r == root) {
            return false;
        }
        self.roots.retain(|r| r != root);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_are_found_shown_and_kept_apart() {
        let p = PathBuf::from;
        let one = ProjectRoots::new(p("/code/api"));
        assert_eq!(one.display(Path::new("/code/api/src/a.rs")), Path::new("src").join("a.rs").display().to_string());
        assert_eq!(one.root_for(Path::new("/elsewhere/x")), Path::new("/code/api"));
        let mut two = ProjectRoots::new(p("/code/api"));
        assert!(two.add(p("/code/web")) && !two.add(p("/code/api/sub")), "a folder inside another isn't one more");
        assert_eq!(two.all(), [p("/code/api"), p("/code/web")]);
        assert_eq!(two.root_of(Path::new("/code/web/app/page.tsx")), Some(Path::new("/code/web")));
        assert_eq!(two.display(Path::new("/code/web/app.tsx")), Path::new("web").join("app.tsx").display().to_string());
        assert_eq!(two.display(Path::new("/tmp/x")), Path::new("/tmp/x").display().to_string());
        assert!(!two.add(p("/code/web/inner")));
        assert!(!two.add(p("/code")), "nor one holding the primary");
        assert!(!two.remove(Path::new("/code/api")), "the primary stays");
        assert!(two.remove(Path::new("/code/web")));
        assert_eq!(two.all(), [p("/code/api")]);
    }

    /// Two folders of the same name: each shown with its parent, or its whole path.
    #[test]
    fn folders_of_the_same_name_are_told_apart() {
        let p = PathBuf::from;
        let mut roots = ProjectRoots::new(p("/work/client/app"));
        roots.add(p("/work/server/app"));
        roots.add(p("/work/docs"));
        assert_eq!(roots.label(Path::new("/work/client/app")), Path::new("client").join("app").display().to_string());
        assert_eq!(roots.label(Path::new("/work/server/app")), Path::new("server").join("app").display().to_string());
        assert_eq!(roots.label(Path::new("/work/docs")), "docs");
        assert_eq!(
            roots.display(Path::new("/work/server/app/main.rs")),
            Path::new("server").join("app").join("main.rs").display().to_string()
        );
        let same = [p("/a/x/app"), p("/b/x/app")];
        assert_eq!(
            ProjectRoots::label_among(&same, Path::new("/b/x/app")),
            Path::new("/b/x/app").display().to_string()
        );
    }
}
