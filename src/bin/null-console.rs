//! `null` typed in a Command Prompt or PowerShell on Windows. Null is a windowed program,
//! which a Command Prompt doesn't wait for: what it said (`null --version`, an option it
//! doesn't know) came after the next prompt, or not at all, and `null --wait` returned at
//! once. Installed beside `null.exe` as `null.com` (found first, as Visual Studio's
//! `devenv.com`), this runs it and passes on what it says until it's done: it has answered,
//! handed the folder to the Null running, or opened its window (and lets go of the pipes
//! then, see `let_go_of_pipes` in main.rs), or for `--wait`, the file's tab is closed.

#[cfg(windows)]
fn main() {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};

    fn pass_on(mut from: impl Read, mut to: impl Write) {
        let mut buffer = [0u8; 8192];
        while let Ok(n) = from.read(&mut buffer) {
            if n == 0 || to.write_all(&buffer[..n]).and_then(|()| to.flush()).is_err() {
                break;
            }
        }
    }

    let null = std::env::current_exe().map(|exe| exe.with_file_name("null.exe")).unwrap_or_else(|_| "null.exe".into());
    let started = Command::new(&null)
        .args(std::env::args_os().skip(1))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut null_exe = match started {
        Ok(child) => child,
        Err(e) => {
            eprintln!("Couldn't start {}: {e}", null.display());
            std::process::exit(1);
        }
    };
    let (out, err) = (null_exe.stdout.take(), null_exe.stderr.take());
    let errors = std::thread::spawn(move || err.map(|err| pass_on(err, std::io::stderr())));
    if let Some(out) = out {
        pass_on(out, std::io::stdout());
    }
    errors.join().ok();
    // Ended, its code (it ends a moment after its pipes close); still on, its window open.
    for _ in 0..50 {
        if let Ok(Some(status)) = null_exe.try_wait() {
            std::process::exit(status.code().unwrap_or(1));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(not(windows))]
fn main() {}
