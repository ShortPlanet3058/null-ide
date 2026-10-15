//! The test at the caret, or a file's tests, as the command that runs them: `cargo test`,
//! `go test`, pytest, vitest or jest, read off the syntax tree and the project's files.

use std::path::{Path, PathBuf};
use tree_sitter::{Node, Tree};

/// What to run, and what to call it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestRun {
    pub command: String,
    /// "conflicts_are_visited_in_turn", "the tests in conflicts.rs".
    pub name: String,
}

/// A test found in a file: its name as runs report it, its line (from 0), and what runs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    /// What runs report it by: its full name ("cart::tests::adds", "tests/test_x.py::test_y",
    /// "cart adds items"), so two tests with one name are told apart.
    pub id: String,
    pub line: usize,
    pub command: String,
}

/// A file's tests, and what runs them all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileTests {
    pub path: PathBuf,
    pub command: Option<String>,
    /// What runs every test of its kind there (its crate's, module's, package's).
    pub every: Option<String>,
    pub tests: Vec<Found>,
}

/// What runs every test of `language` where `path` is: `cargo test` for its crate, `go test
/// ./...` for its module, pytest for the project, the package's JS runner.
pub fn every_test(language: &str, root: &Path, path: &Path) -> Option<String> {
    let dir = path.parent()?;
    match language {
        "Rust" => {
            let krate = ancestor_with(dir, "Cargo.toml")?;
            let manifest = in_dir(root, &krate, |d| format!(" --manifest-path {}", quote(&format!("{d}/Cargo.toml"))));
            Some(format!("cargo test{manifest}"))
        }
        "Go" => {
            let module = ancestor_with(dir, "go.mod")?;
            Some(format!("go test {}-v ./...", in_dir(root, &module, |d| format!("-C {} ", quote(d)))))
        }
        // A package with its own environment (a monorepo's): its tests, with its Python.
        "Python" => {
            let python = crate::python_env::python_for(root, path);
            let package = crate::python_env::owner_for(root, path).map(|(dir, _)| dir);
            let text = std::fs::read_to_string(path).unwrap_or_default();
            if !uses_pytest(root, path, &text) {
                return Some(match package.filter(|p| p != root) {
                    Some(package) => {
                        format!("{python} -m unittest discover -v -s {}", quote(&relative(root, &package)))
                    }
                    None => format!("{python} -m unittest discover -v"),
                });
            }
            Some(match package.filter(|p| p != root) {
                Some(package) => format!("{python} -m pytest {}", quote(&relative(root, &package))),
                None => format!("{python} -m pytest"),
            })
        }
        "JavaScript" | "TypeScript" | "TSX" => {
            let project = ancestor_with(dir, "package.json")?;
            // Each test's own line, as runs of many files don't print by themselves.
            let command = match js_runner(&project) {
                JsRunner::Vitest(x) => format!("{x} vitest run --reporter=verbose"),
                JsRunner::Jest(x) => format!("{x} jest --verbose"),
                JsRunner::Node => "node --test".into(),
            };
            Some(in_package(root, &project, command))
        }
        _ => None,
    }
}

/// `command` run in the package at `project`: in a subshell, so the terminal stays where it
/// was (a `cd` typed into it would last).
fn in_package(root: &Path, project: &Path, command: String) -> String {
    if project == root {
        command
    } else {
        crate::shell::in_folder(&relative(root, project), &command, crate::shell::kind())
    }
}

/// What runs every test found, each kind once: one command, the parts after each other
/// (one failing doesn't keep the others from running).
pub fn run_everything(files: &[FileTests]) -> Option<String> {
    let mut commands: Vec<String> = Vec::new();
    for every in files.iter().filter_map(|f| f.every.as_deref()) {
        if !commands.iter().any(|c| c == every) {
            commands.push(every.to_string());
        }
    }
    // Packages with their own environment run their tests with it: not again with the
    // project's, or a package's they're inside (which would find them too).
    let packages: Vec<String> =
        commands.iter().filter_map(|c| c.split_once(" -m pytest ").map(|(_, package)| package.to_string())).collect();
    let bare = |package: &str| package.trim_matches('\'').to_string();
    for command in commands.iter_mut() {
        let within = match command.split_once(" -m pytest ") {
            Some((_, package)) => Some(bare(package)),
            None if command.ends_with(" -m pytest") => None,
            None => continue,
        };
        // (Inside it as paths go: `api/plugins` in `api`, with `\` on Windows.)
        let inside = |p: &str, w: &str| p != w && Path::new(p).starts_with(w);
        let inner = packages.iter().filter(|p| within.as_ref().is_none_or(|w| inside(&bare(p), w)));
        let ignores: String = inner.map(|p| format!(" --ignore={p}")).collect();
        command.push_str(&ignores);
    }
    (!commands.is_empty()).then(|| commands.join("; "))
}

/// How many files are looked through, at most, and how big one can be.
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 1 << 20;

/// Whether a file's name says it may hold tests (Rust's: any file, read to see).
pub fn may_hold_tests(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let in_tests_folder = path.components().any(|c| c.as_os_str() == "__tests__");
    match path.extension().and_then(|x| x.to_str()) {
        Some("rs") => true,
        Some("go") => name.ends_with("_test.go"),
        Some("py") => name.starts_with("test_") || name.ends_with("_test.py"),
        Some("js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts") => {
            name.contains(".test.") || name.contains(".spec.") || in_tests_folder
        }
        _ => false,
    }
}

/// Every test under `root` (as .gitignore leaves it), file by file. Slow on a big project:
/// off the main thread.
pub fn discover(root: &Path) -> Vec<FileTests> {
    let files = ignore::WalkBuilder::new(root)
        // .gitignore read outside a git repository too; packages and builds never looked in.
        .require_git(false)
        .filter_entry(|e| !matches!(e.file_name().to_str(), Some("node_modules" | "target" | ".venv" | "venv")))
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()) && may_hold_tests(e.path()))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_BYTES))
        .take(MAX_FILES);
    // (Each folder's Python environment looked for once, not once a test.)
    let mut files: Vec<FileTests> = crate::python_env::remembering(|| {
        files
            .filter_map(|e| {
                let text = crate::encoding::read(e.path()).ok()?.0;
                tests_in_file(root, e.path(), &text)
            })
            .collect()
    });
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// The tests in one file (None: none, or not a kind of file Null runs tests of).
pub fn tests_in_file(root: &Path, path: &Path, text: &str) -> Option<FileTests> {
    // Rust files without a test attribute: not even parsed.
    if !may_hold_tests(path) || (path.extension().is_some_and(|x| x == "rs") && !text.contains("test")) {
        return None;
    }
    let language = crate::languages::for_path(path)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language.grammar()).ok()?;
    let tree = parser.parse(text, None)?;
    let mut places: Vec<(String, usize)> = Vec::new();
    match language.name {
        "Go" => places.extend(go_tests(text, &tree).into_iter().map(|(name, r)| (name, r.start))),
        "Rust" | "Python" | "JavaScript" | "TypeScript" | "TSX" => {
            let mut cursor = tree.walk();
            let mut seen = 0;
            'walk: loop {
                let node = cursor.node();
                seen += 1;
                if let Some(found) = test_named(language.name, node, text) {
                    places.push(found);
                }
                if seen < 200_000 && cursor.goto_first_child() {
                    continue;
                }
                while !cursor.goto_next_sibling() {
                    if !cursor.goto_parent() {
                        break 'walk;
                    }
                }
            }
        }
        _ => return None,
    }
    if places.is_empty() {
        return None;
    }
    let tests: Vec<Found> = places
        .into_iter()
        .filter_map(|(name, byte)| {
            let run = find(language.name, root, path, text, &tree, Some(byte))?;
            let line = text[..byte.min(text.len())].matches('\n').count();
            let id = match language.name {
                // The path `--exact` is given: what cargo prints.
                "Rust" => run.command.rsplit_once("--exact ").map_or(name.clone(), |(_, id)| id.to_string()),
                "Python" => format!("{}::{name}", relative(root, path)),
                "JavaScript" | "TypeScript" | "TSX" => js_test(text, &tree, byte).unwrap_or_else(|| name.clone()),
                _ => name.clone(),
            };
            Some(Found { name, id, line, command: run.command })
        })
        .collect();
    if tests.is_empty() {
        return None;
    }
    let command = find(language.name, root, path, text, &tree, None).map(|run| run.command);
    let every = every_test(language.name, root, path);
    Some(FileTests { path: path.to_path_buf(), command, every, tests })
}

/// A test at `node`: its name as runs report it, and a byte inside it.
fn test_named(language: &str, node: Node, text: &str) -> Option<(String, usize)> {
    match language {
        "Rust" if node.kind() == "function_item" && rust_is_test(node, text) => {
            let name = node.child_by_field_name("name")?;
            Some((node_text(name, text).to_string(), name.start_byte()))
        }
        "Python" if node.kind() == "function_definition" => {
            let name = node.child_by_field_name("name")?;
            // test_x, in no class or in Test classes (or unittest's TestCase ones), as pytest
            // collects them; not one inside another function.
            let in_tests = ancestors(node).skip(1).filter(|n| n.kind() == "class_definition").all(|class| {
                class.child_by_field_name("name").is_some_and(|n| node_text(n, text).starts_with("Test"))
                    || class
                        .child_by_field_name("superclasses")
                        .is_some_and(|b| node_text(b, text).contains("TestCase"))
            });
            let nested = ancestors(node).skip(1).any(|n| n.kind() == "function_definition");
            if !node_text(name, text).starts_with("test") || !in_tests || nested {
                return None;
            }
            // pytest's id: the classes around it, then its name ("TestCart::test_total").
            let mut id: Vec<String> = ancestors(node)
                .filter(|n| n.kind() == "class_definition")
                .filter_map(|n| n.child_by_field_name("name").map(|c| node_text(c, text).to_string()))
                .collect();
            id.reverse();
            id.push(node_text(name, text).to_string());
            Some((id.join("::"), name.start_byte()))
        }
        "JavaScript" | "TypeScript" | "TSX" if node.kind() == "call_expression" => {
            let function = node.child_by_field_name("function")?;
            let callee = match function.kind() {
                "member_expression" => function.child_by_field_name("object")?,
                _ => function,
            };
            if !matches!(node_text(callee, text), "test" | "it") {
                return None;
            }
            let first = node.child_by_field_name("arguments")?.named_child(0)?;
            let title = js_title(first, text)?;
            Some((title, first.start_byte()))
        }
        _ => None,
    }
}

/// A test's title as written in a string (its quotes off, its escapes read); None for one
/// made as it runs (`${n}`).
fn js_title(node: Node, text: &str) -> Option<String> {
    let raw = node_text(node, text);
    let inside = match node.kind() {
        "string" => raw.get(1..raw.len().saturating_sub(1))?,
        "template_string" if !raw.contains("${") => raw.get(1..raw.len().saturating_sub(1))?,
        _ => return None,
    };
    let mut out = String::with_capacity(inside.len());
    let mut chars = inside.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            },
            _ => out.push(c),
        }
    }
    Some(out)
}

/// What a test run printed, read: each test's name as `Found` has it, and whether it passed.
/// cargo test, go test -v, pytest -v, and jest, vitest or node's runner.
pub fn results(output: &str) -> Vec<(String, bool)> {
    let mut found: Vec<(String, bool)> = Vec::new();
    // jest's and node's suites, by how far in they're written: what a test's title follows.
    let mut suites: Vec<(usize, String)> = Vec::new();
    let mut in_jest_file = false;
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();
        // cargo: "test editor::tests::adds_up ... ok" ("- should panic" after it, sometimes)
        if let Some(rest) = trimmed.strip_prefix("test ")
            && let Some((path, outcome)) = rest.split_once(" ... ")
        {
            let id = path.split(" - ").next().unwrap_or(path).trim().to_string();
            match outcome.trim() {
                "ok" => found.push((id, true)),
                o if o.starts_with("FAILED") => found.push((id, false)),
                _ => {}
            }
            continue;
        }
        // unittest -v: "test_total (tests.test_cart.TestCart.test_total) ... ok" (before
        // Python 3.11 without the test's name in the brackets), as pytest's id.
        if let Some((head, outcome)) = trimmed.split_once(" ... ")
            && let Some((name, dotted)) = head.split_once(" (")
            && let Some(dotted) = dotted.strip_suffix(')')
            && !name.contains(' ')
            && dotted.contains('.')
        {
            let mut parts: Vec<&str> = dotted.split('.').collect();
            if parts.last() == Some(&name) {
                parts.pop();
            }
            // The classes, then the module (classes start with a capital).
            let classes = parts.iter().rev().take_while(|p| p.starts_with(char::is_uppercase)).count();
            let (module, classes) = parts.split_at(parts.len() - classes);
            let mut id = format!("{}.py", module.join(std::path::MAIN_SEPARATOR_STR));
            for part in classes.iter().chain([&name]) {
                id.push_str("::");
                id.push_str(part);
            }
            match outcome.trim() {
                "ok" => found.push((id, true)),
                "FAIL" | "ERROR" => found.push((id, false)),
                _ => {}
            }
            continue;
        }
        // go -v: "--- PASS: TestTotal (0.00s)"
        if let Some(rest) = trimmed.strip_prefix("--- PASS: ").or_else(|| trimmed.strip_prefix("--- FAIL: ")) {
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            found.push((name, trimmed.starts_with("--- PASS")));
            continue;
        }
        // pytest -v: "tests/test_cart.py::TestCart::test_total PASSED [ 50%]" (test_x[1-2]
        // for each of a parametrized test's cases: the test, failed if any case did)
        if let Some((id, outcome)) = trimmed.split_once(' ')
            && id.contains("::")
            && (outcome.starts_with("PASSED") || outcome.starts_with("FAILED"))
        {
            let id = match id.find('[') {
                Some(open) if id.ends_with(']') => &id[..open],
                _ => id,
            };
            found.push((id.to_string(), outcome.starts_with("PASSED")));
            continue;
        }
        // jest: "PASS src/cart.test.js" starts a file's tests; a blank line ends them.
        if trimmed.starts_with("PASS ") || trimmed.starts_with("FAIL ") {
            in_jest_file = true;
            suites.clear();
            continue;
        }
        if trimmed.is_empty() {
            in_jest_file = false;
            suites.clear();
            continue;
        }
        let mut chars = trimmed.chars();
        let passed = match chars.next() {
            Some('✓' | '✔' | '√') => true,
            Some('✕' | '✖' | '×') => false,
            // node: "▶ cart" opens a suite; jest: a line in a file's tests that isn't one.
            Some('▶') => {
                suites.retain(|(at, _)| *at < indent);
                suites.push((indent, without_time(chars.as_str().trim()).to_string()));
                continue;
            }
            _ if in_jest_file && indent >= 2 => {
                suites.retain(|(at, _)| *at < indent);
                suites.push((indent, trimmed.to_string()));
                continue;
            }
            _ => continue,
        };
        let title = without_time(chars.as_str().trim());
        // vitest: "src/a.test.ts > cart > adds": the file, then the suites and the test.
        let id = if title.contains(" > ") {
            let mut parts: Vec<&str> = title.split(" > ").collect();
            if parts.first().is_some_and(|f| f.contains(".test.") || f.contains(".spec.")) {
                parts.remove(0);
            }
            parts.join(" ")
        } else {
            suites.retain(|(at, _)| *at < indent);
            suites.iter().map(|(_, s)| s.as_str()).chain(std::iter::once(title)).collect::<Vec<_>>().join(" ")
        };
        if !id.trim().is_empty() {
            found.push((id.trim().to_string(), passed));
        }
    }
    found
}

/// A test's title without how long it took: "(3 ms)", "(0.5ms)", or vitest's "2ms".
fn without_time(title: &str) -> &str {
    if let Some(open) = title.rfind(" (")
        && title.ends_with("ms)")
    {
        return &title[..open];
    }
    if let Some((rest, last)) = title.rsplit_once(' ')
        && last.strip_suffix("ms").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit() || c == '.'))
    {
        return rest;
    }
    title
}

/// The test the caret (`byte`) is in, or with `byte` None, all of the file's tests.
pub fn find(language: &str, root: &Path, path: &Path, text: &str, tree: &Tree, byte: Option<usize>) -> Option<TestRun> {
    let file = path.file_name()?.to_string_lossy().into_owned();
    let all = |command: String| TestRun { command, name: format!("the tests in {file}") };
    match language {
        "Rust" => rust(root, path, text, tree, byte, &file),
        "Go" => {
            let names = go_tests(text, tree);
            let dir = path.parent()?;
            let module = ancestor_with(dir, "go.mod")?;
            let package = match dir.strip_prefix(&module).ok()?.to_string_lossy().into_owned() {
                p if p.is_empty() => "./".to_string(),
                p => format!("./{p}"),
            };
            let chdir = in_dir(root, &module, |d| format!("-C {} ", quote(d)));
            match byte {
                Some(byte) => {
                    let name = names.into_iter().find(|(_, r)| r.contains(&byte))?.0;
                    let command = format!("go test {chdir}-v -run {} {package}", quote(&format!("^{name}$")));
                    Some(TestRun { command, name })
                }
                None if names.is_empty() => None,
                None => {
                    let list: Vec<String> = names.into_iter().map(|(n, _)| n).collect();
                    let pattern = format!("^({})$", list.join("|"));
                    Some(all(format!("go test {chdir}-v -run {} {package}", quote(&pattern))))
                }
            }
        }
        "Python" => {
            let relative = relative(root, path);
            let python = crate::python_env::python_for(root, path);
            let pytest = uses_pytest(root, path, text);
            match byte {
                Some(byte) => {
                    let (id, name) = python_test(text, tree, byte)?;
                    let command = if pytest {
                        format!("{python} -m pytest {}", quote(&format!("{relative}::{id}")))
                    } else {
                        let test = format!("{}.{}", unittest_module(&relative), id.replace("::", "."));
                        format!("{python} -m unittest -v {}", quote(&test))
                    };
                    Some(TestRun { command, name })
                }
                None if pytest => Some(all(format!("{python} -m pytest {}", quote(&relative)))),
                None => Some(all(format!("{python} -m unittest -v {}", quote(&unittest_module(&relative))))),
            }
        }
        "JavaScript" | "TypeScript" | "TSX" => {
            let project = ancestor_with(path.parent()?, "package.json")?;
            let runner = js_runner(&project);
            let relative = relative(&project, path);
            let pattern = match byte {
                Some(byte) => Some(js_test(text, tree, byte)?),
                None => None,
            };
            let command = match (runner, &pattern) {
                (JsRunner::Vitest(x), Some(p)) => {
                    format!("{x} vitest run --reporter=verbose {} -t {}", quote(&relative), quote(&regex_escape(p)))
                }
                (JsRunner::Vitest(x), None) => format!("{x} vitest run --reporter=verbose {}", quote(&relative)),
                (JsRunner::Jest(x), Some(p)) => format!("{x} jest {} -t {}", quote(&relative), quote(&regex_escape(p))),
                (JsRunner::Jest(x), None) => format!("{x} jest {}", quote(&relative)),
                (JsRunner::Node, Some(p)) => {
                    format!("node --test --test-name-pattern={} {}", quote(&regex_escape(p)), quote(&relative))
                }
                (JsRunner::Node, None) => format!("node --test {}", quote(&relative)),
            };
            let name = pattern.unwrap_or_else(|| format!("the tests in {file}"));
            Some(TestRun { command: in_package(root, &project, command), name })
        }
        _ => None,
    }
}

/// A Rust test, to debug: its crate's folder, what picks its target for cargo (`--bin x`,
/// `--test y`, or nothing), and its full name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RustTest {
    pub krate: PathBuf,
    pub target: Vec<String>,
    /// "cart::tests::adds".
    pub id: String,
    pub name: String,
}

/// The Rust test the caret (`byte`) is in.
pub fn rust_test_at(path: &Path, text: &str, tree: &Tree, byte: usize) -> Option<RustTest> {
    let (krate, target, mut module) = rust_place(path)?;
    let node = tree.root_node().descendant_for_byte_range(byte, byte)?;
    let function = ancestors(node).find(|n| n.kind() == "function_item" && rust_is_test(*n, text))?;
    let name = node_text(function.child_by_field_name("name")?, text).to_string();
    module.extend(rust_modules(function, text));
    module.push(name.clone());
    Some(RustTest { krate, target, id: module.join("::"), name })
}

/// The crate a Rust file is in, the target it belongs to (as cargo's arguments), and its
/// module path within it.
fn rust_place(path: &Path) -> Option<(PathBuf, Vec<String>, Vec<String>)> {
    let krate = ancestor_with(path.parent()?, "Cargo.toml")?;
    let inside = path.strip_prefix(&krate).ok()?;
    let mut parts: Vec<String> = inside.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let mut target = Vec::new();
    let mut module: Vec<String> = Vec::new();
    match parts.first().map(String::as_str) {
        Some("src") if parts.len() == 3 && parts[1] == "bin" => {
            target = vec!["--bin".to_string(), parts[2].trim_end_matches(".rs").to_string()];
        }
        Some("src") => {
            parts.remove(0);
            let last = parts.pop()?;
            module.extend(parts);
            match last.as_str() {
                "main.rs" | "lib.rs" | "mod.rs" => {}
                other => module.push(other.trim_end_matches(".rs").to_string()),
            }
        }
        Some("tests") if parts.len() == 2 => {
            target = vec!["--test".to_string(), parts[1].trim_end_matches(".rs").to_string()];
        }
        _ => return None,
    }
    Some((krate, target, module))
}

fn rust(root: &Path, path: &Path, text: &str, tree: &Tree, byte: Option<usize>, file: &str) -> Option<TestRun> {
    let (krate, target, mut module) = rust_place(path)?;
    // Which target the file belongs to, and its module path within it.
    let target: String = target.iter().map(|a| format!(" {a}")).collect();
    let manifest = in_dir(root, &krate, |d| format!(" --manifest-path {}", quote(&format!("{d}/Cargo.toml"))));
    match byte {
        Some(byte) => {
            let node = tree.root_node().descendant_for_byte_range(byte, byte)?;
            let function = ancestors(node).find(|n| n.kind() == "function_item" && rust_is_test(*n, text))?;
            let name = node_text(function.child_by_field_name("name")?, text).to_string();
            module.extend(rust_modules(function, text));
            module.push(name.clone());
            let command = format!("cargo test{manifest}{target} -- --exact {}", module.join("::"));
            Some(TestRun { command, name })
        }
        None => {
            let filter = if module.is_empty() { String::new() } else { format!(" -- {}::", module.join("::")) };
            Some(TestRun {
                command: format!("cargo test{manifest}{target}{filter}"),
                name: format!("the tests in {file}"),
            })
        }
    }
}

/// `#[test]`, `#[tokio::test]`, `#[rstest]`… among the attributes just above it.
fn rust_is_test(function: Node, text: &str) -> bool {
    let mut previous = function.prev_named_sibling();
    while let Some(node) = previous {
        match node.kind() {
            "attribute_item" => {
                let attribute = node_text(node, text).trim_start_matches("#[").trim_end_matches(']').trim();
                let path = attribute.split(['(', ' ']).next().unwrap_or("");
                if path == "test" || path.ends_with("::test") || path == "rstest" {
                    return true;
                }
            }
            "line_comment" | "block_comment" => {}
            _ => return false,
        }
        previous = node.prev_named_sibling();
    }
    false
}

/// The inline `mod` blocks around a node, outermost first.
fn rust_modules(node: Node, text: &str) -> Vec<String> {
    let mut names: Vec<String> = ancestors(node)
        .filter(|n| n.kind() == "mod_item")
        .filter_map(|n| n.child_by_field_name("name").map(|name| node_text(name, text).to_string()))
        .collect();
    names.reverse();
    names
}

/// The file's `func TestX(t *testing.T)`, with the bytes each covers.
fn go_tests(text: &str, tree: &Tree) -> Vec<(String, std::ops::Range<usize>)> {
    let root = tree.root_node();
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter(|n| n.kind() == "function_declaration")
        .filter_map(|n| {
            let name = node_text(n.child_by_field_name("name")?, text);
            let rest = name.strip_prefix("Test").or_else(|| name.strip_prefix("Fuzz"))?;
            // TestX, not Testify: the next letter isn't lowercase.
            (!rest.starts_with(|c: char| c.is_lowercase())).then(|| (name.to_string(), n.start_byte()..n.end_byte()))
        })
        .collect()
}

/// Whether Python tests run with pytest here: the project says so (a conftest.py, its
/// settings, its requirements), the file imports it, or its tests are bare functions (only
/// pytest runs those). Else a file of unittest's TestCases runs with unittest, which comes
/// with Python: pytest may not be installed at all.
fn uses_pytest(root: &Path, path: &Path, text: &str) -> bool {
    let imports = |module: &str| {
        text.lines().any(|l| {
            let l = l.trim_start();
            l.starts_with(&format!("import {module}")) || l.starts_with(&format!("from {module}"))
        })
    };
    if imports("pytest") || !imports("unittest") {
        return true;
    }
    let says_pytest = |file: PathBuf| std::fs::read_to_string(file).is_ok_and(|t| t.contains("pytest"));
    let mut dir = path.parent();
    while let Some(d) = dir.filter(|d| d.starts_with(root)) {
        if d.join("conftest.py").is_file() || d.join("pytest.ini").is_file() {
            return true;
        }
        let settings = ["pyproject.toml", "setup.cfg", "tox.ini", "requirements.txt", "requirements-dev.txt"];
        if settings.iter().any(|f| says_pytest(d.join(f))) {
            return true;
        }
        dir = d.parent();
    }
    false
}

/// unittest's name for a test file: `tests/test_cart.py` is `tests.test_cart`.
fn unittest_module(relative: &str) -> String {
    let module = relative.strip_suffix(".py").unwrap_or(relative);
    module.replace(['/', '\\'], ".")
}

/// pytest's id for the test at `byte` ("TestCart::test_total"), and its name.
fn python_test(text: &str, tree: &Tree, byte: usize) -> Option<(String, String)> {
    let node = tree.root_node().descendant_for_byte_range(byte, byte)?;
    let function = ancestors(node).find(|n| {
        n.kind() == "function_definition"
            && n.child_by_field_name("name").is_some_and(|name| node_text(name, text).starts_with("test"))
    })?;
    let name = node_text(function.child_by_field_name("name")?, text).to_string();
    let mut id: Vec<String> = ancestors(function)
        .filter(|n| n.kind() == "class_definition")
        .filter_map(|n| n.child_by_field_name("name").map(|c| node_text(c, text).to_string()))
        .collect();
    id.reverse();
    id.push(name.clone());
    Some((id.join("::"), name))
}

/// The full name of the `test(…)`/`it(…)` (or `describe(…)`) at `byte`, with the
/// `describe` blocks around it: what `-t` matches.
fn js_test(text: &str, tree: &Tree, byte: usize) -> Option<String> {
    let node = tree.root_node().descendant_for_byte_range(byte, byte)?;
    let mut names: Vec<String> = ancestors(node)
        .filter(|n| n.kind() == "call_expression")
        .filter_map(|call| {
            let function = call.child_by_field_name("function")?;
            // test(…), it(…), describe(…), and test.only(…) / it.skip(…).
            let callee = match function.kind() {
                "member_expression" => function.child_by_field_name("object")?,
                _ => function,
            };
            if !matches!(node_text(callee, text), "test" | "it" | "describe") {
                return None;
            }
            let arguments = call.child_by_field_name("arguments")?;
            let first = arguments.named_child(0)?;
            js_title(first, text)
        })
        .collect();
    names.reverse();
    (!names.is_empty()).then(|| names.join(" "))
}

enum JsRunner {
    /// With the command that runs a package's binary: npx, pnpm exec…
    Vitest(&'static str),
    Jest(&'static str),
    Node,
}

fn js_runner(project: &Path) -> JsRunner {
    let exec = if project.join("pnpm-lock.yaml").is_file() {
        "pnpm exec"
    } else if project.join("yarn.lock").is_file() {
        "yarn"
    } else if project.join("bun.lockb").is_file() || project.join("bun.lock").is_file() {
        "bunx"
    } else {
        "npx"
    };
    let manifest = std::fs::read_to_string(project.join("package.json")).unwrap_or_default();
    let json: serde_json::Value = serde_json::from_str(&manifest).unwrap_or_default();
    let has = |name: &str| {
        ["dependencies", "devDependencies"].iter().any(|key| json.get(key).and_then(|d| d.get(name)).is_some())
    };
    if has("vitest") {
        JsRunner::Vitest(exec)
    } else if has("jest") {
        JsRunner::Jest(exec)
    } else {
        JsRunner::Node
    }
}

fn ancestors(node: Node) -> impl Iterator<Item = Node> {
    std::iter::successors(Some(node), |n| n.parent())
}

fn node_text<'a>(node: Node, text: &'a str) -> &'a str {
    &text[node.start_byte()..node.end_byte()]
}

/// The nearest folder from `dir` up that holds `file`.
fn ancestor_with(dir: &Path, file: &str) -> Option<PathBuf> {
    dir.ancestors().find(|d| d.join(file).is_file()).map(Path::to_path_buf)
}

fn relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base).unwrap_or(path).to_string_lossy().into_owned()
}

/// `make(dir)` when `dir` isn't the project's root (where the terminal starts), else nothing.
fn in_dir(root: &Path, dir: &Path, make: impl Fn(&str) -> String) -> String {
    if dir == root { String::new() } else { make(&relative(root, dir)) }
}

/// A shell word: as is when it's plain, else in single quotes.
fn quote(word: &str) -> String {
    crate::shell::quote(word, crate::shell::kind())
}

fn regex_escape(text: &str) -> String {
    text.chars().fold(String::new(), |mut out, c| {
        if "\\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::highlight::Highlighter;

    fn tree(file: &str, text: &str) -> Tree {
        let language = crate::languages::for_path(Path::new(file)).unwrap();
        let mut highlighter = Highlighter::new(language).unwrap();
        highlighter.sync(&Buffer::from_text(text));
        highlighter.tree().unwrap().clone()
    }

    fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = crate::tools::test_dir(&format!("test-at-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in files {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn at(language: &str, root: &Path, file: &str, text: &str, marker: &str) -> Option<TestRun> {
        let byte = text.find(marker).unwrap();
        find(language, root, &root.join(file), text, &tree(file, text), Some(byte))
    }

    #[test]
    fn rust_tests_by_their_module_path() {
        let text = "fn helper() {}\n\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds_up() {\n        assert!(true);\n    }\n\n    #[tokio::test]\n    async fn waits() {}\n}\n";
        let root = project("rust", &[("Cargo.toml", "[package]\n"), ("src/editor/conflicts.rs", text)]);
        let run = at("Rust", &root, "src/editor/conflicts.rs", text, "assert").unwrap();
        assert_eq!(run.command, "cargo test -- --exact editor::conflicts::tests::adds_up");
        assert_eq!(run.name, "adds_up");
        assert_eq!(at("Rust", &root, "src/editor/conflicts.rs", text, "{}\n}").unwrap().name, "waits");
        // Not in a test.
        assert_eq!(at("Rust", &root, "src/editor/conflicts.rs", text, "helper"), None);
        let file = find("Rust", &root, &root.join("src/editor/conflicts.rs"), text, &tree("a.rs", text), None);
        assert_eq!(file.unwrap().command, "cargo test -- editor::conflicts::");
        // An integration test, in a crate of a workspace.
        let root2 = project("rust2", &[("crates/core/Cargo.toml", ""), ("crates/core/tests/api.rs", text)]);
        let run = at("Rust", &root2, "crates/core/tests/api.rs", text, "assert").unwrap();
        assert_eq!(
            run.command,
            "cargo test --manifest-path crates/core/Cargo.toml --test api -- --exact tests::adds_up"
        );
        // To debug: the same test, as its parts.
        let byte = text.find("assert").unwrap();
        let test = rust_test_at(&root2.join("crates/core/tests/api.rs"), text, &tree("a.rs", text), byte).unwrap();
        assert_eq!(test.krate, root2.join("crates/core"));
        assert_eq!((test.target.join(" "), test.id.as_str()), ("--test api".to_string(), "tests::adds_up"));
        let test = rust_test_at(&root.join("src/editor/conflicts.rs"), text, &tree("a.rs", text), byte).unwrap();
        assert!(test.target.is_empty() && test.id == "editor::conflicts::tests::adds_up", "{test:?}");
        assert_eq!(rust_test_at(&root.join("src/editor/conflicts.rs"), text, &tree("a.rs", text), 3), None);
    }

    /// A project's tests, found file by file, each with what runs it.
    #[test]
    #[cfg(unix)] // (POSIX command lines: PowerShell's are tested in `shell`.)
    fn tests_are_found_in_a_project() {
        let rust = "fn helper() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds_up() {}\n}\n";
        let py = "class TestCart:\n    def test_total(self):\n        pass\n\nclass Helper:\n    def test_not(self):\n        pass\n\ndef test_free():\n    pass\n";
        let js = "describe('cart', () => {\n  it('adds items', () => {});\n  test.skip('later', () => {});\n});\n";
        let root = project(
            "discover",
            &[
                ("Cargo.toml", "[package]\n"),
                ("src/cart.rs", rust),
                ("src/plain.rs", "fn main() {}\n"),
                ("tests/test_cart.py", py),
                ("web/package.json", r#"{"devDependencies":{"jest":"1"}}"#),
                ("web/cart.test.js", js),
                ("web/cart.js", "it('not a test file', () => {})\n"),
            ],
        );
        let files = discover(&root);
        let names: Vec<(String, Vec<String>)> = files
            .iter()
            .map(|f| {
                let path = f.path.strip_prefix(&root).unwrap().to_string_lossy().into_owned();
                (path, f.tests.iter().map(|t| t.name.clone()).collect())
            })
            .collect();
        assert_eq!(
            names,
            [
                ("src/cart.rs".to_string(), vec!["adds_up".to_string()]),
                ("tests/test_cart.py".to_string(), vec!["TestCart::test_total".to_string(), "test_free".to_string()]),
                ("web/cart.test.js".to_string(), vec!["adds items".to_string(), "later".to_string()]),
            ]
        );
        let rust = &files[0];
        assert_eq!(rust.tests[0].line, 4);
        assert_eq!(rust.tests[0].command, "cargo test -- --exact cart::tests::adds_up");
        assert_eq!(rust.command.as_deref(), Some("cargo test -- cart::"));
        assert_eq!(files[2].tests[0].command, "(cd web && npx jest cart.test.js -t 'cart adds items')");
        // Each told apart by its full name.
        assert_eq!(rust.tests[0].id, "cart::tests::adds_up");
        assert_eq!(files[1].tests[0].id, "tests/test_cart.py::TestCart::test_total");
        assert_eq!(files[2].tests[0].id, "cart adds items");
        // Everything: each kind once.
        assert_eq!(
            run_everything(&files).as_deref(),
            Some("cargo test; python3 -m pytest; (cd web && npx jest --verbose)")
        );
    }

    #[test]
    fn results_are_read_from_what_runs_print() {
        let output = "running 3 tests\ntest editor::tests::adds_up ... ok\ntest tests::fails ... FAILED\n\
            test tests::panics - should panic ... ok\n\
            --- PASS: TestTotal (0.00s)\n    --- FAIL: TestSub (0.01s)\n\
            tests/test_cart.py::TestCart::test_total PASSED   [ 50%]\ntests/test_cart.py::test_free FAILED  [100%]\n\
            tests/test_cart.py::test_sum[1-2] PASSED\n\n\
            PASS web/cart.test.js\n  cart\n    ✓ adds items (3 ms)\n    ✕ times out after 100ms (105 ms)\n\n\
            ✓ src/a.test.ts > cart > counts 2ms\n\
            ▶ basket\n  ✔ empties (0.5ms)\n";
        let results = results(output);
        let has = |id: &str, ok: bool| results.contains(&(id.to_string(), ok));
        assert!(has("editor::tests::adds_up", true) && has("tests::fails", false) && has("tests::panics", true));
        assert!(has("TestTotal", true) && has("TestSub", false));
        assert!(has("tests/test_cart.py::TestCart::test_total", true) && has("tests/test_cart.py::test_free", false));
        assert!(has("tests/test_cart.py::test_sum", true), "a parametrized case, as its test");
        assert!(has("cart adds items", true) && has("cart times out after 100ms", false), "{results:?}");
        assert!(has("cart counts", true) && has("basket empties", true), "{results:?}");
    }

    /// unittest -v's lines, as pytest's ids (how the tests list knows them).
    #[test]
    fn unittest_results_are_read_as_pytest_ids() {
        let output = "test_total (tests.test_cart.TestCart.test_total) ... ok\n\
            test_empty (tests.test_cart.TestCart.test_empty) ... FAIL\n\
            test_old (test_cart.TestCart) ... ERROR\n\
            test_skip (test_cart.TestCart.test_skip) ... skipped 'later'\n\
            Ran 4 tests in 0.001s\n";
        let sep = std::path::MAIN_SEPARATOR_STR;
        assert_eq!(
            results(output),
            [
                (format!("tests{sep}test_cart.py::TestCart::test_total"), true),
                (format!("tests{sep}test_cart.py::TestCart::test_empty"), false),
                ("test_cart.py::TestCart::test_old".to_string(), false),
            ]
        );
    }

    /// A title as written, its quotes and escapes read; one made as it runs isn't a test to find.
    #[test]
    fn js_titles_are_read_as_they_run() {
        let js = "it('it\\'s \"x\"', () => {});\ntest(`adds ${n}`, () => {});\ntest(`plain`, () => {});\n";
        let root = project("js-titles", &[("package.json", "{}"), ("a.test.js", js)]);
        let names: Vec<String> =
            tests_in_file(&root, &root.join("a.test.js"), js).unwrap().tests.into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["it's \"x\"", "plain"]);
    }

    /// A monorepo package with its own environment: its tests run with its Python, and
    /// not again with the project's.
    #[test]
    #[cfg(unix)] // (POSIX command lines: PowerShell's are tested in `shell`.)
    fn a_package_s_tests_run_with_its_environment() {
        let test = "def test_one():\n    pass\n";
        let root = project(
            "nested-env",
            &[
                ("tests/test_a.py", test),
                ("api/.venv/bin/python", ""),
                ("api/tests/test_b.py", test),
                ("api/plugins/.venv/bin/python", ""),
                ("api/plugins/test_c.py", test),
            ],
        );
        let files = discover(&root);
        let b = files.iter().find(|f| f.path.ends_with("api/tests/test_b.py")).unwrap();
        assert_eq!(b.tests[0].command, "api/.venv/bin/python -m pytest api/tests/test_b.py::test_one");
        assert_eq!(b.every.as_deref(), Some("api/.venv/bin/python -m pytest api"));
        assert_eq!(
            run_everything(&files).as_deref(),
            Some(
                "api/plugins/.venv/bin/python -m pytest api/plugins; api/.venv/bin/python -m pytest api --ignore=api/plugins; \
                 python3 -m pytest --ignore=api/plugins --ignore=api"
            )
        );
    }

    #[test]
    #[cfg(unix)] // (POSIX command lines and paths: PowerShell's are tested in `shell`.)
    fn go_python_and_js_tests() {
        let go = "package cart\n\nfunc TestTotal(t *testing.T) {\n\tt.Log(1)\n}\n\nfunc Testify() {}\n";
        let root = project("go", &[("go.mod", "module x\n"), ("cart/cart_test.go", go)]);
        let run = at("Go", &root, "cart/cart_test.go", go, "t.Log").unwrap();
        assert_eq!(run.command, "go test -v -run '^TestTotal$' ./cart");
        let file = find("Go", &root, &root.join("cart/cart_test.go"), go, &tree("a.go", go), None).unwrap();
        assert_eq!(file.command, "go test -v -run '^(TestTotal)$' ./cart");

        let py = "class TestCart:\n    def test_total(self):\n        assert 1\n\ndef test_free():\n    pass\n";
        let root = project("py", &[("tests/test_cart.py", py)]);
        let run = at("Python", &root, "tests/test_cart.py", py, "assert").unwrap();
        assert_eq!(run.command, "python3 -m pytest tests/test_cart.py::TestCart::test_total");
        assert_eq!(at("Python", &root, "tests/test_cart.py", py, "pass").unwrap().name, "test_free");

        // unittest's TestCases, with no sign of pytest: unittest, which comes with Python.
        let unit = "import unittest\n\nclass TestCart(unittest.TestCase):\n    def test_total(self):\n        self.assertTrue(1)\n";
        let root = project("py-unit", &[("tests/test_cart.py", unit)]);
        let run = at("Python", &root, "tests/test_cart.py", unit, "assertTrue").unwrap();
        assert_eq!(run.command, "python3 -m unittest -v tests.test_cart.TestCart.test_total");
        let every = every_test("Python", &root, &root.join("tests/test_cart.py")).unwrap();
        assert_eq!(every, "python3 -m unittest discover -v");
        // The same file in a project that uses pytest: pytest.
        let root = project(
            "py-unit-pytest",
            &[("tests/test_cart.py", unit), ("pyproject.toml", "[tool.pytest.ini_options]\n")],
        );
        let run = at("Python", &root, "tests/test_cart.py", unit, "assertTrue").unwrap();
        assert_eq!(run.command, "python3 -m pytest tests/test_cart.py::TestCart::test_total");

        let js = "describe('cart', () => {\n  it('adds (two) items', () => {\n    expect(1).toBe(1);\n  });\n});\n";
        let root =
            project("js", &[("package.json", r#"{"devDependencies":{"vitest":"1"}}"#), ("src/cart.test.ts", js)]);
        let run = at("TypeScript", &root, "src/cart.test.ts", js, "expect").unwrap();
        assert_eq!(run.command, r"npx vitest run --reporter=verbose src/cart.test.ts -t 'cart adds \(two\) items'");
        assert_eq!(run.name, "cart adds (two) items");
    }
}
