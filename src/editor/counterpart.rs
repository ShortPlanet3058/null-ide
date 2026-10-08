//! ⌃⌘↑ in C, C++ and Objective-C: from a source file to its header and back (`app.cpp` ↔
//! `app.h`), as Xcode's "counterpart". clangd finds it anywhere in the project; without it,
//! it's looked for beside the file.

use super::{Editor, EditorEvent};
use gpui::{App, Context, KeyBinding, Window, actions};
use std::path::{Path, PathBuf};

actions!(editor, [SwitchSourceHeader]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-secondary-up", SwitchSourceHeader, Some("Editor"))]);
}

/// How long clangd is waited for before looking beside the file.
const SERVER_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

const SOURCES: &[&str] = &["c", "cc", "cpp", "cxx", "c++", "m", "mm"];
const HEADERS: &[&str] = &["h", "hh", "hpp", "hxx", "h++", "inl", "ipp"];

fn extension(path: &Path) -> Option<String> {
    path.extension().and_then(|e| e.to_str()).map(str::to_lowercase)
}

/// A C, C++ or Objective-C file, source or header.
pub fn has_counterpart(path: &Path) -> bool {
    extension(path).is_some_and(|e| SOURCES.contains(&e.as_str()) || HEADERS.contains(&e.as_str()))
}

/// The counterpart beside `path`, among the files `exists` says are there (by their exact
/// name: the Mac's disk would say `app.H` is there for `app.h`): for a source, a header of
/// the same name; for a header, a source.
fn beside(path: &Path, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let ext = extension(path)?;
    let wanted = if SOURCES.contains(&ext.as_str()) { HEADERS } else { SOURCES };
    let original = path.extension()?.to_str()?;
    // `.H`, `.CPP`: the same case as the file's own extension, then lower case.
    let upper = original.chars().all(|c| !c.is_ascii_lowercase());
    wanted.iter().find_map(|e| {
        let cased = if upper { e.to_uppercase() } else { e.to_string() };
        [path.with_extension(&cased), path.with_extension(e)].into_iter().find(|p| exists(p))
    })
}

impl Editor {
    pub(super) fn switch_source_header(&mut self, _: &SwitchSourceHeader, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.selection.head;
        let Some(path) = self.path.clone().filter(|p| has_counterpart(p)) else {
            return self.show_notice(at, "Header and source are for C, C++ and Objective-C files.".into(), cx);
        };
        let asked = self.served(cx).then(|| self.lsp.as_ref().map(|lsp| lsp.read(cx).counterpart(&path))).flatten();
        self.counterpart_task = Some(cx.spawn(async move |this, cx| {
            // A server that doesn't answer soon: what's beside the file.
            let found = match asked {
                Some(request) => {
                    let timeout = cx.background_executor().timer(SERVER_WAIT);
                    futures::select_biased! {
                        found = futures::FutureExt::fuse(request) => found,
                        _ = futures::FutureExt::fuse(timeout) => None,
                    }
                }
                None => None,
            };
            let names: std::collections::HashSet<std::ffi::OsString> = path
                .parent()
                .and_then(|dir| std::fs::read_dir(dir).ok())
                .map(|read| read.filter_map(Result::ok).map(|e| e.file_name()).collect())
                .unwrap_or_default();
            let listed = |p: &Path| p.file_name().is_some_and(|n| names.contains(n));
            let found = found.filter(|p| p.exists()).or_else(|| beside(&path, listed));
            this.update(cx, |this, cx| match found {
                Some(other) => cx.emit(EditorEvent::Open(other)),
                None => this.show_notice(at, "No header or source found for this file.".into(), cx),
            })
            .ok();
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_s_header_and_back() {
        let files = [
            "src/app.cpp",
            "src/app.hpp",
            "src/util.c",
            "src/util.h",
            "src/View.m",
            "src/View.h",
            "x/LOUD.CPP",
            "x/LOUD.H",
        ];
        let exists = |p: &Path| files.iter().any(|f| Path::new(f) == p);
        let find = |f: &str| beside(Path::new(f), exists).map(|p| p.display().to_string());
        assert_eq!(find("src/app.cpp").as_deref(), Some("src/app.hpp"));
        assert_eq!(find("src/app.hpp").as_deref(), Some("src/app.cpp"));
        assert_eq!(find("src/util.h").as_deref(), Some("src/util.c"));
        assert_eq!(find("src/View.h").as_deref(), Some("src/View.m"));
        assert_eq!(find("x/LOUD.CPP").as_deref(), Some("x/LOUD.H"));
        assert_eq!(find("src/main.c"), None);
        assert!(has_counterpart(Path::new("a.mm")) && !has_counterpart(Path::new("a.rs")));
    }

    /// Against the real clangd: `cargo test clangd_finds_a_header_elsewhere -- --ignored`.
    #[test]
    #[ignore]
    fn clangd_finds_a_header_elsewhere() {
        use crate::lsp::{LanguageServer, ServerMessage, path_for, uri_for};
        use futures::{FutureExt, StreamExt};
        use lsp_types::notification::{DidOpenTextDocument, Initialized};
        use lsp_types::request::Initialize;
        use lsp_types::*;
        let dir = crate::tools::test_dir("counterpart-clangd");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join("include")).unwrap();
        std::fs::write(dir.join("include/app.h"), "int run();\n").unwrap();
        let source = "#include \"app.h\"\nint run() { return 1; }\n";
        std::fs::write(dir.join("src/app.cpp"), source).unwrap();
        let commands = format!(
            r#"[{{"directory":"{0}","file":"{0}/src/app.cpp","arguments":["clang++","-I{0}/include","-c","{0}/src/app.cpp"]}}]"#,
            dir.display()
        );
        std::fs::write(dir.join("compile_commands.json"), commands).unwrap();
        let file = dir.join("src/app.cpp");
        assert_eq!(beside(&file, Path::exists), None, "not beside it: only clangd can find it");
        let (server, mut messages) = LanguageServer::spawn(Path::new("/usr/bin/clangd"), &[], &dir).unwrap();
        let found = futures::executor::block_on(async {
            #[allow(deprecated)]
            let init = InitializeParams { root_uri: uri_for(&dir), ..Default::default() };
            server.request::<Initialize>(init).await.unwrap();
            server.notify::<Initialized>(InitializedParams {});
            server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: uri_for(&file).unwrap(),
                    language_id: "cpp".into(),
                    version: 0,
                    text: source.into(),
                },
            });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
            loop {
                while let Some(Some(message)) = messages.next().now_or_never() {
                    if let ServerMessage::Request { id, .. } = message {
                        server.respond(id, serde_json::Value::Null);
                    }
                }
                let asked = TextDocumentIdentifier { uri: uri_for(&file).unwrap() };
                let found = server.request::<crate::lsp_store::SwitchSourceHeader>(asked).await.ok().flatten();
                if found.is_some() || std::time::Instant::now() > deadline {
                    return found.and_then(|uri| path_for(&uri));
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
        assert_eq!(found.map(|p| p.canonicalize().unwrap()), Some(dir.join("include/app.h").canonicalize().unwrap()));
        std::fs::remove_dir_all(&dir).ok();
    }
}
