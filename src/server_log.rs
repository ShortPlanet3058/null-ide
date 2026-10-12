//! What language servers say about themselves: what they print on their error output,
//! their log messages, and whether they started. Kept in memory (the last few thousand
//! lines), shown on asking ("Show Language Server Log"), for finding out why one fails.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Lines kept of each server (a chatty one doesn't push out another's), the oldest going
/// first; and how long a line can be.
const KEPT: usize = 2_000;
const LONGEST: usize = 2_000;

/// Each server's lines, with the order they came in.
struct Log {
    next: u64,
    servers: HashMap<String, VecDeque<(u64, String)>>,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// A line from `server`.
pub fn append(server: &str, line: &str) {
    let line = line.trim_end();
    if line.is_empty() {
        return;
    }
    let line = match line.char_indices().nth(LONGEST) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    };
    let mut log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let log = log.get_or_insert_with(|| Log { next: 0, servers: HashMap::new() });
    let seq = log.next;
    log.next += 1;
    let lines = log.servers.entry(server.to_string()).or_default();
    lines.push_back((seq, format!("[{server}] {line}")));
    while lines.len() > KEPT {
        lines.pop_front();
    }
}

/// Everything kept, oldest first.
pub fn text() -> String {
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    let Some(log) = log.as_ref() else { return String::new() };
    let mut lines: Vec<&(u64, String)> = log.servers.values().flatten().collect();
    lines.sort_by_key(|(seq, _)| *seq);
    lines.iter().map(|(_, line)| line.as_str()).collect::<Vec<_>>().join("\n")
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
        // A very long line: cut.
        append("test-long", &"x".repeat(10_000));
        assert!(text().lines().any(|l| l.starts_with("[test-long] xxx") && l.ends_with('…') && l.len() < 2_100));
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
