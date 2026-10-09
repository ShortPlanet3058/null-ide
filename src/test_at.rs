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
            Some(format!("go test {}./...", in_dir(root, &module, |d| format!("-C {} ", quote(d)))))
        }
        "Python" => Some("python3 -m pytest".into()),
        "JavaScript" | "TypeScript" | "TSX" => {
            let project = ancestor_with(dir, "package.json")?;
            let chdir = in_dir(root, &project, |d| format!("cd {} && ", quote(d)));
            let command = match js_runner(&project) {
                JsRunner::Vitest(x) => format!("{x} vitest run"),
                JsRunner::Jest(x) => format!("{x} jest"),
                JsRunner::Node => "node --test".into(),
            };
            Some(format!("{chdir}{command}"))
        }
        _ => None,
    }
}

/// What runs every test found, each kind once: one command, the parts after each other
/// (one failing doesn't keep the others from running).
pub fn run_everything(files: &[FileTests]) -> Option<String> {
    let mut commands: Vec<&str> = Vec::new();
    for every in files.iter().filter_map(|f| f.every.as_deref()) {
        if !commands.contains(&every) {
            commands.push(every);
        }
    }
    (!commands.is_empty()).then(|| commands.join("; "))
}

/// How many files are looked through, at most, and how big one can be.
const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 1 << 20;

/// Whether a file's name says it may hold tests (Rust's: any file, read to see).
fn may_hold_tests(path: &Path) -> bool {
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
    let mut files: Vec<FileTests> = ignore::WalkBuilder::new(root)
        .build()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()) && may_hold_tests(e.path()))
        .filter(|e| e.metadata().is_ok_and(|m| m.len() <= MAX_BYTES))
        .take(MAX_FILES)
        .filter_map(|e| {
            let text = crate::encoding::read(e.path()).ok()?.0;
            tests_in_file(root, e.path(), &text)
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

/// The tests in one file (None: none, or not a kind of file Null runs tests of).
pub fn tests_in_file(root: &Path, path: &Path, text: &str) -> Option<FileTests> {
    // Rust files without a test attribute: not even parsed.
    if path.extension().is_some_and(|x| x == "rs") && !text.contains("test") {
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
            Some(Found { name, line, command: run.command })
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
            // test_x, in no class or in Test classes, as pytest collects them.
            let in_tests = ancestors(node).skip(1).filter(|n| n.kind() == "class_definition").all(|class| {
                class.child_by_field_name("name").is_some_and(|n| node_text(n, text).starts_with("Test"))
            });
            if !node_text(name, text).starts_with("test") || !in_tests {
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
            matches!(first.kind(), "string" | "template_string")
                .then(|| (node_text(first, text).trim_matches(['"', '\'', '`']).to_string(), first.start_byte()))
        }
        _ => None,
    }
}

/// What a test run printed, read: each test's name as `Found` has it, and whether it passed.
/// cargo test, go test -v, pytest -v, and jest, vitest or node's runner.
pub fn results(output: &str) -> Vec<(String, bool)> {
    let mut found = Vec::new();
    for line in output.lines() {
        let line = line.trim_end_matches('\r');
        let trimmed = line.trim();
        // cargo: "test editor::tests::adds_up ... ok"
        if let Some(rest) = trimmed.strip_prefix("test ")
            && let Some((path, outcome)) = rest.split_once(" ... ")
        {
            let name = path.rsplit("::").next().unwrap_or(path).to_string();
            match outcome.trim() {
                "ok" => found.push((name, true)),
                o if o.starts_with("FAILED") => found.push((name, false)),
                _ => {}
            }
            continue;
        }
        // go: "--- PASS: TestTotal (0.00s)"
        if let Some(rest) = trimmed.strip_prefix("--- PASS: ").or_else(|| trimmed.strip_prefix("--- FAIL: ")) {
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            found.push((name, trimmed.starts_with("--- PASS")));
            continue;
        }
        // pytest -v: "tests/test_cart.py::TestCart::test_total PASSED [ 50%]"
        if let Some((id, outcome)) = trimmed.split_once(' ')
            && id.contains("::")
            && (outcome.starts_with("PASSED") || outcome.starts_with("FAILED"))
        {
            let name = id.split_once("::").map_or(id, |(_, rest)| rest).to_string();
            found.push((name, outcome.starts_with("PASSED")));
            continue;
        }
        // jest, vitest, node: "✓ adds two items (3 ms)", "✕ …", "× …"
        let mut chars = trimmed.chars();
        let passed = match chars.next() {
            Some('✓' | '✔' | '√') => true,
            Some('✕' | '✖' | '×') => false,
            _ => continue,
        };
        let mut name = chars.as_str().trim();
        if let Some(open) = name.rfind(" (")
            && name.ends_with("ms)")
        {
            name = &name[..open];
        }
        // (vitest's "2ms", unbracketed)
        if let Some((rest, last)) = name.rsplit_once(' ')
            && last.strip_suffix("ms").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        {
            name = rest;
        }
        // vitest's "file > describe > test": the test's own title.
        let name = name.rsplit(" > ").next().unwrap_or(name).trim();
        if !name.is_empty() {
            found.push((name.to_string(), passed));
        }
    }
    found
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
                    let command = format!("go test {chdir}-run {} {package}", quote(&format!("^{name}$")));
                    Some(TestRun { command, name })
                }
                None if names.is_empty() => None,
                None => {
                    let list: Vec<String> = names.into_iter().map(|(n, _)| n).collect();
                    let pattern = format!("^({})$", list.join("|"));
                    Some(all(format!("go test {chdir}-run {} {package}", quote(&pattern))))
                }
            }
        }
        "Python" => {
            let relative = relative(root, path);
            match byte {
                Some(byte) => {
                    let (id, name) = python_test(text, tree, byte)?;
                    let command = format!("python3 -m pytest {}", quote(&format!("{relative}::{id}")));
                    Some(TestRun { command, name })
                }
                None => Some(all(format!("python3 -m pytest {}", quote(&relative)))),
            }
        }
        "JavaScript" | "TypeScript" | "TSX" => {
            let project = ancestor_with(path.parent()?, "package.json")?;
            let runner = js_runner(&project);
            let relative = relative(&project, path);
            let chdir = in_dir(root, &project, |d| format!("cd {} && ", quote(d)));
            let pattern = match byte {
                Some(byte) => Some(js_test(text, tree, byte)?),
                None => None,
            };
            let command = match (runner, &pattern) {
                (JsRunner::Vitest(x), Some(p)) => {
                    format!("{x} vitest run {} -t {}", quote(&relative), quote(&regex_escape(p)))
                }
                (JsRunner::Vitest(x), None) => format!("{x} vitest run {}", quote(&relative)),
                (JsRunner::Jest(x), Some(p)) => format!("{x} jest {} -t {}", quote(&relative), quote(&regex_escape(p))),
                (JsRunner::Jest(x), None) => format!("{x} jest {}", quote(&relative)),
                (JsRunner::Node, Some(p)) => {
                    format!("node --test --test-name-pattern={} {}", quote(&regex_escape(p)), quote(&relative))
                }
                (JsRunner::Node, None) => format!("node --test {}", quote(&relative)),
            };
            let name = pattern.unwrap_or_else(|| format!("the tests in {file}"));
            Some(TestRun { command: format!("{chdir}{command}"), name })
        }
        _ => None,
    }
}

fn rust(root: &Path, path: &Path, text: &str, tree: &Tree, byte: Option<usize>, file: &str) -> Option<TestRun> {
    let krate = ancestor_with(path.parent()?, "Cargo.toml")?;
    let inside = path.strip_prefix(&krate).ok()?;
    let mut parts: Vec<String> = inside.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    // Which target the file belongs to, and its module path within it.
    let mut target = String::new();
    let mut module: Vec<String> = Vec::new();
    match parts.first().map(String::as_str) {
        Some("src") if parts.len() == 3 && parts[1] == "bin" => {
            target = format!(" --bin {}", parts[2].trim_end_matches(".rs"));
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
        Some("tests") if parts.len() == 2 => target = format!(" --test {}", parts[1].trim_end_matches(".rs")),
        _ => return None,
    }
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
            matches!(first.kind(), "string" | "template_string")
                .then(|| node_text(first, text).trim_matches(['"', '\'', '`']).to_string())
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
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '=' | '@' | '+'));
    if plain { word.to_string() } else { format!("'{}'", word.replace('\'', r"'\''")) }
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
    }

    /// A project's tests, found file by file, each with what runs it.
    #[test]
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
        assert_eq!(files[2].tests[0].command, "cd web && npx jest cart.test.js -t 'cart adds items'");
        // Everything: each kind once.
        assert_eq!(run_everything(&files).as_deref(), Some("cargo test; python3 -m pytest; cd web && npx jest"));
    }

    #[test]
    fn results_are_read_from_what_runs_print() {
        let output = "running 2 tests\ntest editor::tests::adds_up ... ok\ntest tests::fails ... FAILED\n\
            --- PASS: TestTotal (0.00s)\n    --- FAIL: TestSub (0.01s)\n\
            tests/test_cart.py::TestCart::test_total PASSED   [ 50%]\ntests/test_cart.py::test_free FAILED  [100%]\n\
            \u{2003} ✓ adds items (3 ms)\n  ✕ later\n ✓ src/a.test.ts > cart > counts 2ms\n";
        let results = results(output);
        assert!(results.contains(&("adds_up".into(), true)) && results.contains(&("fails".into(), false)));
        assert!(results.contains(&("TestTotal".into(), true)) && results.contains(&("TestSub".into(), false)));
        assert!(results.contains(&("TestCart::test_total".into(), true)) && results.contains(&("test_free".into(), false)));
        assert!(results.contains(&("adds items".into(), true)) && results.contains(&("later".into(), false)));
        assert!(results.contains(&("counts".into(), true)));
    }

    #[test]
    fn go_python_and_js_tests() {
        let go = "package cart\n\nfunc TestTotal(t *testing.T) {\n\tt.Log(1)\n}\n\nfunc Testify() {}\n";
        let root = project("go", &[("go.mod", "module x\n"), ("cart/cart_test.go", go)]);
        let run = at("Go", &root, "cart/cart_test.go", go, "t.Log").unwrap();
        assert_eq!(run.command, "go test -run '^TestTotal$' ./cart");
        let file = find("Go", &root, &root.join("cart/cart_test.go"), go, &tree("a.go", go), None).unwrap();
        assert_eq!(file.command, "go test -run '^(TestTotal)$' ./cart");

        let py = "class TestCart:\n    def test_total(self):\n        assert 1\n\ndef test_free():\n    pass\n";
        let root = project("py", &[("tests/test_cart.py", py)]);
        let run = at("Python", &root, "tests/test_cart.py", py, "assert").unwrap();
        assert_eq!(run.command, "python3 -m pytest tests/test_cart.py::TestCart::test_total");
        assert_eq!(at("Python", &root, "tests/test_cart.py", py, "pass").unwrap().name, "test_free");

        let js = "describe('cart', () => {\n  it('adds (two) items', () => {\n    expect(1).toBe(1);\n  });\n});\n";
        let root =
            project("js", &[("package.json", r#"{"devDependencies":{"vitest":"1"}}"#), ("src/cart.test.ts", js)]);
        let run = at("TypeScript", &root, "src/cart.test.ts", js, "expect").unwrap();
        assert_eq!(run.command, r"npx vitest run src/cart.test.ts -t 'cart adds \(two\) items'");
        assert_eq!(run.name, "cart adds (two) items");
    }
}
