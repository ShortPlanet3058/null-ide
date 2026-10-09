//! What language servers say about themselves: what they print on their error output,
//! their log messages, and whether they started. Kept in memory (the last few thousand
//! lines), shown on asking ("Show Language Server Log"), for finding out why one fails.

use std::collections::VecDeque;
use std::sync::Mutex;

/// Lines kept, all servers together; the oldest go first.
const KEPT: usize = 5_000;

static LOG: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// A line from `server`.
pub fn append(server: &str, line: &str) {
    let line = line.trim_end();
    if line.is_empty() {
        return;
    }
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.push_back(format!("[{server}] {line}"));
    while log.len() > KEPT {
        log.pop_front();
    }
}

/// Everything kept, oldest first.
pub fn text() -> String {
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    log.iter().map(String::as_str).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_kept_with_their_server() {
        append("test-server", "  starting\n");
        append("test-server", "   ");
        assert!(text().lines().any(|l| l == "[test-server]   starting"));
        assert!(!text().lines().any(|l| l == "[test-server]"), "blank lines aren't kept");
    }

    /// A server's error output, and its stopping, are kept as its own.
    #[cfg(unix)]
    #[test]
    fn a_server_s_error_output_is_kept() {
        let dir = std::env::temp_dir();
        let started = crate::lsp::LanguageServer::spawn_named(
            std::path::Path::new("/bin/sh"),
            &["-c", "echo 'cannot load workspace' >&2"],
            &dir,
            "fake-server",
        );
        assert!(started.is_ok());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !(text().contains("[fake-server] cannot load workspace") && text().contains("[fake-server] (stopped)")) {
            assert!(std::time::Instant::now() < deadline, "{}", text());
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}
