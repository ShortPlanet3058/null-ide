//! When Null crashes: what happened is written down (`crash.log`, in Null's own folder),
//! and the next start says so, once. A crash otherwise leaves no trace for an app started
//! from the Dock (stderr goes nowhere).

use std::io::Write;
use std::path::PathBuf;

fn log_file() -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("crash.log"))
}

/// Where the last crash was noted as told, so it's told once.
fn told_file() -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("crash.told"))
}

/// From now on, a crash is written down (as well as printed, as before).
pub fn record() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        before(info);
        let Some(file) = log_file() else { return };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let when = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let entry =
            format!("\n== {when} · Null {} · thread {thread}\n{info}\n{backtrace}\n", env!("CARGO_PKG_VERSION"));
        // (Kept to a size: past 1 MB, the older half goes.)
        if std::fs::metadata(&file).is_ok_and(|m| m.len() > 1_000_000)
            && let Ok(text) = std::fs::read_to_string(&file)
        {
            // (On a character's edge: a cut inside one would panic in here, and abort.)
            let half = text.floor_char_boundary(text.len() / 2);
            let cut = text[half..].find("\n== ").map_or(half, |i| half + i);
            std::fs::write(&file, &text[cut..]).ok();
        }
        if let Ok(mut log) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
            log.write_all(entry.as_bytes()).ok();
        }
    }));
}

/// A crash written down since it was last told: what to say about it, once.
pub fn untold() -> Option<String> {
    let log = log_file()?;
    let crashed = std::fs::metadata(&log).and_then(|m| m.modified()).ok()?;
    let told = told_file()?;
    let told_at = std::fs::metadata(&told).and_then(|m| m.modified()).ok();
    if told_at.is_some_and(|at| at >= crashed) {
        return None;
    }
    std::fs::write(&told, "").ok();
    Some(format!("Null closed unexpectedly last time: what happened is in {}", log.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crash written down is told once.
    #[test]
    fn a_crash_is_told_once() {
        let (Some(log), Some(told)) = (log_file(), told_file()) else { return };
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::remove_file(&told).ok();
        std::fs::write(&log, "== a crash\n").unwrap();
        assert!(untold().is_some_and(|m| m.contains("crash.log")));
        assert!(untold().is_none(), "once");
        std::fs::remove_file(&log).ok();
        std::fs::remove_file(&told).ok();
    }
}
