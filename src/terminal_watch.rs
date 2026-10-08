//! What runs in a terminal: the program in the foreground of its shell, so a long one
//! finishing while you're elsewhere can say so (the Dock's icon jumps once, a line in the
//! status bar tells what finished and how long it took).

use std::time::{Duration, Instant};

/// A command running at least this long says when it's done.
pub const LONG: Duration = Duration::from_secs(10);

/// Looks at the shell's terminal for the program it's running.
#[cfg_attr(not(unix), allow(dead_code))]
pub struct Foreground {
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
    #[cfg(unix)]
    shell: i32,
}

impl Foreground {
    /// From the terminal's own end of the PTY (kept open on a copy) and the shell's process.
    #[cfg(unix)]
    pub fn new(pty: &std::fs::File, shell: u32) -> Option<Self> {
        use std::os::fd::AsFd;
        let fd = pty.as_fd().try_clone_to_owned().ok()?;
        Some(Self { fd, shell: shell as i32 })
    }

    /// The program in the foreground, if it isn't the shell waiting at its prompt: its
    /// process group and its name.
    pub fn program(&self) -> Option<(i32, String)> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: the descriptor is ours and open for as long as `self` is.
            let group = unsafe { libc::tcgetpgrp(self.fd.as_raw_fd()) };
            if group <= 0 || group == self.shell {
                return None;
            }
            Some((group, name_of(group).unwrap_or_else(|| "A command".into())))
        }
        #[cfg(not(unix))]
        None
    }
}

#[cfg(target_os = "macos")]
fn name_of(pid: i32) -> Option<String> {
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is as long as said.
    let len = unsafe { libc::proc_name(pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
    (len > 0).then(|| String::from_utf8_lossy(&buffer[..len as usize]).into_owned())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn name_of(pid: i32) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(name.trim().to_string()).filter(|n| !n.is_empty())
}

/// A program running in a terminal, since when.
pub struct Running {
    group: i32,
    pub name: String,
    since: Instant,
}

/// Whether any process of `group` is still there: a job suspended with ⌃Z is, one that
/// finished isn't.
pub fn group_alive(group: i32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 only checks; nothing is sent.
        unsafe { libc::kill(-group, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        let _ = group;
        false
    }
}

/// The user's own shell, from the account, when `$SHELL` doesn't say.
pub fn account_shell() -> Option<String> {
    #[cfg(unix)]
    {
        // SAFETY: getpwuid's record is read at once, before any other call could reuse it.
        unsafe {
            let entry = libc::getpwuid(libc::getuid());
            if entry.is_null() || (*entry).pw_shell.is_null() {
                return None;
            }
            let shell = std::ffi::CStr::from_ptr((*entry).pw_shell).to_string_lossy().into_owned();
            (!shell.is_empty()).then_some(shell)
        }
    }
    #[cfg(not(unix))]
    None
}

/// What changed since last look: a long command that just finished, its name and time.
/// `alive` says whether a process group is still there (a suspended job hasn't finished).
pub fn step(
    running: &mut Option<Running>,
    now: Option<(i32, String)>,
    at: Instant,
    alive: impl Fn(i32) -> bool,
) -> Option<(String, Duration)> {
    match (running.as_mut(), now) {
        (None, Some((group, name))) => {
            *running = Some(Running { group, name, since: at });
            None
        }
        // `make && make test`: the next command of the same line carries on the run.
        (Some(r), Some((group, _))) => {
            r.group = group;
            None
        }
        (Some(_), None) => {
            let r = running.take()?;
            let took = at.saturating_duration_since(r.since);
            (took >= LONG && !alive(r.group)).then_some((r.name, took))
        }
        (None, None) => None,
    }
}

/// `2 min 13 s`, `45 s`, `1 h 05 min`.
pub fn took_words(took: Duration) -> String {
    let s = took.as_secs();
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min {:02} s", s / 60, s % 60),
        _ => format!("{} h {:02} min", s / 3600, s / 60 % 60),
    }
}

/// The Dock's icon jumps once, to say something's done while Null is in the background.
pub fn bounce_dock() {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSApplication, NSRequestUserAttentionType};
        if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm)
                .requestUserAttention(NSRequestUserAttentionType::InformationalRequest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_command_says_when_it_s_done() {
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);
        let mut running = None;
        let gone = |_| false;
        assert_eq!(step(&mut running, Some((50, "cargo".into())), at(0), gone), None);
        // The next command of the line: still one run, named after the first.
        assert_eq!(step(&mut running, Some((51, "cargo".into())), at(30), gone), None);
        assert_eq!(step(&mut running, None, at(75), gone), Some(("cargo".into(), Duration::from_secs(75))));
        assert!(running.is_none());
        // A short one: nothing to say.
        step(&mut running, Some((60, "ls".into())), at(80), gone);
        assert_eq!(step(&mut running, None, at(81), gone), None);
        // Suspended with ⌃Z: still there, not finished.
        step(&mut running, Some((70, "vim".into())), at(100), gone);
        assert_eq!(step(&mut running, None, at(200), |group| group == 70), None);
        assert!(running.is_none());
    }

    #[test]
    fn times_read_shortly() {
        assert_eq!(took_words(Duration::from_secs(45)), "45 s");
        assert_eq!(took_words(Duration::from_secs(133)), "2 min 13 s");
        assert_eq!(took_words(Duration::from_secs(3900)), "1 h 05 min");
    }

    /// A real shell: its prompt is no program, `sleep` is.
    #[cfg(unix)]
    #[test]
    fn the_foreground_program_is_seen() {
        let dir = crate::tools::test_dir("terminal-watch");
        std::fs::create_dir_all(&dir).unwrap();
        let options = alacritty_terminal::tty::Options {
            shell: Some(alacritty_terminal::tty::Shell::new("/bin/sh".into(), vec![])),
            working_directory: Some(dir.clone()),
            drain_on_exit: false,
            env: Default::default(),
        };
        let size =
            alacritty_terminal::event::WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 };
        let pty = alacritty_terminal::tty::new(&options, size, 0).unwrap();
        let watch = Foreground::new(pty.file(), pty.child().id()).unwrap();
        // Its output read away, as the terminal does: a shell can't exit with output unread.
        let mut output = pty.file().try_clone().unwrap();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match std::io::Read::read(&mut output, &mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20))
                    }
                    Err(_) => break,
                }
            }
        });
        let wait_for = |want: bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while watch.program().is_some() != want && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            watch.program()
        };
        assert_eq!(wait_for(false), None, "at the prompt");
        use std::io::Write;
        pty.file().write_all(b"sleep 2\n").unwrap();
        assert_eq!(wait_for(true).map(|(_, name)| name).as_deref(), Some("sleep"));
        assert_eq!(wait_for(false), None, "back at the prompt");
        drop(pty);
        std::fs::remove_dir_all(&dir).ok();
    }
}
