//! Problems in what a command printed: a build's errors and warnings, a test's failures, a
//! Python traceback. Each with the file and line it names, so they can join the Problems
//! list (⌘⇧M) and F8 like the language servers' own.

use regex::Regex;
use std::sync::LazyLock;

/// A problem a command reported: the file as written (relative to where it ran, or not),
/// its line and column from 1, whether it's an error, and what it says.
#[derive(Clone, Debug, PartialEq)]
pub struct Reported {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub error: bool,
    pub message: String,
}

/// `path:line:col: error: message`: clang, gcc, swiftc, Go, ESLint's compact form…
static COLON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([^\s:][^:]*\.[A-Za-z0-9]+):(\d+)(?::(\d+))?:\s*(?:(fatal error|error|warning|note|remark)\s*(?:\[[^\]]*\])?:\s*)?(.*)$")
        .unwrap()
});
/// TypeScript: `src/a.ts(3,5): error TS2322: message`.
static PARENS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*([^\s(][^(]*\.[A-Za-z0-9]+)\((\d+),(\d+)\):\s*(error|warning)\s*(.*)$").unwrap());
/// Rust: `error[E0425]: message`, then `  --> src/main.rs:4:5`.
static RUST_HEAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(error|warning)(?:\[\w+\])?: (.+)$").unwrap());
static RUST_PLACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*--> ([^:]+):(\d+):(\d+)").unwrap());
/// Python: `  File "x.py", line 3, in f`.
static PY_FRAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^\s*File "([^"]+)", line (\d+)"#).unwrap());
static PY_ERROR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z_][\w.]*(?:Error|Exception|Exit|Interrupt)\b.*)$").unwrap());

/// The problems in `output`, in order, each once.
pub fn problems(output: &str) -> Vec<Reported> {
    let mut found: Vec<Reported> = Vec::new();
    let lines: Vec<&str> = output.lines().collect();
    // Rust's message waits for its place, a line or two below.
    let mut rust: Option<(bool, String)> = None;
    // A traceback's innermost frame waits for the error line that ends it.
    let mut frame: Option<(String, u32)> = None;
    let push = |found: &mut Vec<Reported>, problem: Reported| {
        if !found.iter().any(|f| f.file == problem.file && f.line == problem.line && f.message == problem.message) {
            found.push(problem);
        }
    };
    for line in lines {
        if let Some(c) = RUST_HEAD.captures(line) {
            rust = Some((&c[1] == "error", c[2].trim().to_string()));
            continue;
        }
        if let Some(c) = RUST_PLACE.captures(line) {
            if let Some((error, message)) = rust.take() {
                let at = |i: usize| c[i].parse().unwrap_or(1);
                push(&mut found, Reported { file: c[1].to_string(), line: at(2), column: at(3), error, message });
            }
            continue;
        }
        if let Some(c) = PY_FRAME.captures(line) {
            frame = Some((c[1].to_string(), c[2].parse().unwrap_or(1)));
            continue;
        }
        if let (Some(c), Some((file, at))) = (PY_ERROR.captures(line), &frame) {
            let message = c[1].trim().to_string();
            push(&mut found, Reported { file: file.clone(), line: *at, column: 1, error: true, message });
            frame = None;
            continue;
        }
        if let Some(c) = PARENS.captures(line) {
            let at = |i: usize| c[i].parse().unwrap_or(1);
            let message = c[5].trim().to_string();
            push(
                &mut found,
                Reported {
                    file: c[1].trim().to_string(),
                    line: at(2),
                    column: at(3),
                    error: &c[4] == "error",
                    message,
                },
            );
            continue;
        }
        if let Some(c) = COLON.captures(line) {
            let kind = c.get(4).map(|m| m.as_str());
            // Notes explain the problem above them; they aren't problems.
            if matches!(kind, Some("note" | "remark")) {
                continue;
            }
            let message = c[5].trim().to_string();
            if message.is_empty() {
                continue;
            }
            let column = c.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(1);
            let error = kind != Some("warning");
            push(
                &mut found,
                Reported { file: c[1].trim().to_string(), line: c[2].parse().unwrap_or(1), column, error, message },
            );
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short(output: &str) -> Vec<(String, u32, u32, bool, String)> {
        problems(output).into_iter().map(|p| (p.file, p.line, p.column, p.error, p.message)).collect()
    }

    #[test]
    fn compilers_errors_and_warnings() {
        let clang = "main.c:4:5: error: use of undeclared identifier 'x'\n    x = 1;\n    ^\nmain.c:2:10: warning: unused variable 'y' [-Wunused-variable]\nmain.c:1:1: note: in this function\n1 error generated.\n";
        assert_eq!(
            short(clang),
            [
                ("main.c".into(), 4, 5, true, "use of undeclared identifier 'x'".into()),
                ("main.c".into(), 2, 10, false, "unused variable 'y' [-Wunused-variable]".into()),
            ]
        );
        assert_eq!(
            short("./main.go:5:2: undefined: total\n"),
            [("./main.go".into(), 5, 2, true, "undefined: total".into())]
        );
        assert_eq!(
            short("src/a.ts(3,5): error TS2322: Type 'string' is not assignable to type 'number'.\n"),
            [("src/a.ts".into(), 3, 5, true, "TS2322: Type 'string' is not assignable to type 'number'.".into())]
        );
        assert_eq!(
            short("Sources/App/main.swift:7:13: error: cannot find 'foo' in scope\n"),
            [("Sources/App/main.swift".into(), 7, 13, true, "cannot find 'foo' in scope".into())]
        );
    }

    #[test]
    fn rust_says_where_below() {
        let cargo = "   Compiling t v0.1.0\nerror[E0425]: cannot find value `x` in this scope\n --> src/main.rs:4:13\n  |\n4 |     let y = x;\n  |             ^ not found\n\nwarning: unused variable: `y`\n --> src/main.rs:4:9\nerror: could not compile `t` due to 1 previous error\n";
        assert_eq!(
            short(cargo),
            [
                ("src/main.rs".into(), 4, 13, true, "cannot find value `x` in this scope".into()),
                ("src/main.rs".into(), 4, 9, false, "unused variable: `y`".into()),
            ]
        );
    }

    #[test]
    fn tests_and_tracebacks() {
        let python = "Traceback (most recent call last):\n  File \"/p/app.py\", line 10, in <module>\n    main()\n  File \"/p/lib.py\", line 3, in main\n    return 1 / 0\nZeroDivisionError: division by zero\n";
        assert_eq!(short(python), [("/p/lib.py".into(), 3, 1, true, "ZeroDivisionError: division by zero".into())]);
        assert_eq!(
            short("tests/test_a.py:12: AssertionError\n"),
            [("tests/test_a.py".into(), 12, 1, true, "AssertionError".into())]
        );
    }

    #[test]
    fn plain_output_is_no_problem() {
        let quiet = "   Compiling null v0.1.0\n    Finished dev in 2.1s\nhttp://localhost:3000\n12:30:01 server ready\nok: 3 passed\n";
        assert!(short(quiet).is_empty(), "{:?}", short(quiet));
    }
}
