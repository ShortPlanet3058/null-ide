//! Folders on SSH servers. Such a project is at `/ssh:me@box:/home/me/proj` (Emacs's form),
//! and Null reaches it with the person's own `ssh`: their keys, agent and `~/.ssh/config`.
//! Files go through a long-lived `sh` on the server (a few per server, for work at once),
//! and the programs Null runs for the folder (git, language servers, the terminal) run
//! there. Nothing is installed on the server: sh and the usual tools are what's used (cat,
//! stat, base64, mkdir…). On a Mac and Linux, ssh's connections to a server are shared
//! (a master connection), so a program starts without signing in again.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// A path on a server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePath {
    /// As given to ssh (`me@box`, a `~/.ssh/config` name), with `#2222` for a port.
    pub host: String,
    /// As the server has it: "/home/me/proj".
    pub path: String,
}

const PREFIX: &str = "/ssh:";

/// `path` as a path on a server, when it's one: `/ssh:HOST:/path` (`/ssh:HOST:` is its
/// root); separators either way (Windows joins with `\`).
pub fn of(path: &Path) -> Option<RemotePath> {
    let text = path.to_str()?.replace('\\', "/");
    let rest = text.strip_prefix(PREFIX)?;
    // (An address in brackets has colons of its own: `me@[::1]:/home`.)
    let after_host = if rest.starts_with('[') || rest.contains("@[") { rest.find(']')? } else { 0 };
    let colon = after_host + rest[after_host..].find(':')?;
    let (host, inside) = (&rest[..colon], &rest[colon + 1..]);
    if host.is_empty() {
        return None;
    }
    Some(RemotePath { host: host.to_string(), path: normal(inside) })
}

/// Whether `path` is on a server.
pub fn is_remote(path: &Path) -> bool {
    of(path).is_some()
}

/// The path to `inside` (a server's path) on `host`.
pub fn path(host: &str, inside: &str) -> PathBuf {
    PathBuf::from(format!("{PREFIX}{host}:{}", normal(inside)))
}

/// A path on a server as people write it: `me@box:/home/me/proj`; others as they are.
pub fn shown(path: &Path) -> String {
    match of(path) {
        Some(remote) => format!("{}:{}", remote.host, remote.path),
        None => path.display().to_string(),
    }
}

/// `inside` with one `/` between names, none at the end, `.` and `..` worked out.
fn normal(inside: &str) -> String {
    let mut names: Vec<&str> = Vec::new();
    for name in inside.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                names.pop();
            }
            name => names.push(name),
        }
    }
    format!("/{}", names.join("/"))
}

/// `text` in single quotes for sh: as it is, whatever's in it.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// A host as ssh takes it, and its port: `box#2222` is `box` on port 2222.
fn destination(host: &str) -> (&str, Option<&str>) {
    match host.rsplit_once('#') {
        Some((dest, port)) if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => (dest, Some(port)),
        _ => (host, None),
    }
}

/// Where ssh keeps the shared connections Null opens (Mac and Linux: Windows' ssh can't
/// share them). Short: a socket's path has a length limit.
#[cfg(unix)]
fn control_dir() -> Option<PathBuf> {
    // SAFETY: getuid can't fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    let dir = PathBuf::from(format!("/tmp/null-ssh-{uid}"));
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    // (Made as its owner's from the start: no moment another could reach it.)
    std::fs::DirBuilder::new().mode(0o700).create(&dir).ok();
    // Only one of ours, as made: in a /tmp others write to, one put there before by
    // someone else (or a link to elsewhere) would hold the connection open to them.
    let meta = std::fs::symlink_metadata(&dir).ok()?;
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
        return None;
    }
    Some(dir)
}

/// `ssh` to `host`, with what every connection of Null's has: its port, a check that the
/// server's still there (a dead connection ends instead of hanging), and the shared connection.
fn ssh(host: &str) -> std::process::Command {
    let (_, port) = destination(host);
    let mut command = crate::tools::command("ssh");
    if let Some(port) = port {
        command.args(["-p", port]);
    }
    command.args(["-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3", "-o", "ConnectTimeout=20"]);
    #[cfg(unix)]
    if let Some(dir) = control_dir() {
        command.args(["-o", "ControlMaster=auto", "-o", "ControlPersist=300"]);
        command.arg("-o").arg(format!("ControlPath={}/%C", dir.display()));
    }
    command
}

/// Null's own folders and the usual ones on a server's PATH, before its own: where Null
/// installs language servers there.
pub fn path_inside() -> &'static str {
    "$HOME/.local/share/null/servers/node_modules/.bin:$HOME/.local/share/null/servers/bin:\
     $HOME/.local/share/null/node/bin:$HOME/.local/bin:$HOME/.cargo/bin:$HOME/go/bin:$PATH"
}

/// `program args…` run on `host`, in `folder` (a server's path; the home folder without),
/// with Null's folders on its PATH, through a login shell (what the person's profile sets up
/// is there). Its input and output are the program's, through ssh; no password is asked
/// (keys or an agent), so it fails instead of waiting.
pub fn command(host: &str, folder: Option<&str>, program: &str, args: &[&str]) -> std::process::Command {
    let script = format!(
        "{}PATH=\"{}\"; export PATH; exec \"$0\" \"$@\"",
        folder.map(|f| format!("cd {} || exit 1; ", quote(f))).unwrap_or_default(),
        path_inside()
    );
    let mut line = format!("exec sh -lc {} {}", quote(&script), quote(program));
    for arg in args {
        line.push(' ');
        line.push_str(&quote(arg));
    }
    let mut command = ssh(host);
    asking(&mut command);
    command.args(["-T", "--", destination(host).0, &line]);
    command
}

/// What ssh asks (a password, a passphrase, whether to trust a server), asked in Null's
/// window (see `askpass`); where it can't be, nothing asked: it fails at once instead.
fn asking(command: &mut std::process::Command) {
    // (From the window's own thread: what was answered already, nothing asked: the window
    // couldn't show the question while it waits.)
    let environment =
        if on_main_thread() { crate::askpass::environment_known_only() } else { crate::askpass::environment() };
    match environment {
        Some(environment) => {
            command.envs(environment);
        }
        None => {
            command.args(["-o", "BatchMode=yes"]);
        }
    }
}

/// The program, arguments and environment for a terminal on `host`, in `folder`: the
/// person's own shell there. What ssh asks is asked as for the rest (in the window, a
/// password typed once this run answering again); where it can't be, in the terminal.
pub fn terminal(host: &str, folder: &str) -> (PathBuf, Vec<String>, Vec<(&'static str, String)>) {
    let command = ssh(host);
    let program = PathBuf::from(command.get_program());
    let mut args: Vec<String> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
    args.extend(["-t".into(), "--".into(), destination(host).0.into()]);
    args.push(format!("cd {} 2>/dev/null; exec \"${{SHELL:-sh}}\" -l", quote(folder)));
    (program, args, crate::askpass::environment().unwrap_or_default())
}

/// What a script on a server printed, and how it ended.
#[derive(Debug, Default)]
pub struct Output {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Output {
    pub fn success(&self) -> bool {
        self.status == 0
    }

    /// Its error output as an error, with the kind it reads as.
    fn error(&self) -> io::Error {
        let text = String::from_utf8_lossy(&self.stderr).trim().to_string();
        let kind = if text.contains("No such file") {
            io::ErrorKind::NotFound
        } else if text.contains("Permission denied") {
            io::ErrorKind::PermissionDenied
        } else if text.contains("File exists") {
            io::ErrorKind::AlreadyExists
        } else {
            io::ErrorKind::Other
        };
        // (cat: /x: No such file or directory → "No such file or directory".)
        let message = text.lines().last().map(|l| l.rsplit(": ").next().unwrap_or(l)).unwrap_or("failed");
        io::Error::new(kind, message.to_string())
    }
}

/// A `sh` on a server, its input Null's: a script is sent (run in a subshell: its `exit` or
/// `cd` is its own), and its output comes back in one piece, framed: `KEY status length-out length-err`, then those bytes.
struct Session {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    /// The frame's mark: what the script printed can't be taken for it.
    key: String,
    /// What ssh said on its error output lately (why it couldn't connect).
    said: Arc<Mutex<String>>,
}

/// A program that's a server for tests: `sh` here.
#[cfg(test)]
pub const TEST_HOST: &str = "null-test-host";

impl Session {
    fn start(host: &str) -> io::Result<Session> {
        #[cfg(test)]
        let mut command = if host == TEST_HOST {
            std::process::Command::new("sh")
        } else {
            let mut command = ssh(host);
            asking(&mut command);
            command.args(["-T", "--", destination(host).0, "sh"]);
            command
        };
        #[cfg(not(test))]
        let mut command = {
            let mut command = ssh(host);
            asking(&mut command);
            command.args(["-T", "--", destination(host).0, "sh"]);
            command
        };
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
        let said = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            let said = said.clone();
            std::thread::Builder::new().name("ssh-stderr".into()).spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Ok(mut said) = said.lock() {
                        if said.len() > 4000 {
                            said.clear();
                        }
                        said.push_str(&line);
                        said.push('\n');
                    }
                }
            })?;
        }
        let input = child.stdin.take().ok_or_else(|| io::Error::other("no input"))?;
        let output = BufReader::new(child.stdout.take().ok_or_else(|| io::Error::other("no output"))?);
        let key = format!(
            "NULL{:x}{:x}",
            std::process::id(),
            SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
        );
        let mut session = Session { child, input, output, key, said };
        // Where each script's output goes before it's sent, `st` for what stat says (GNU's
        // and BusyBox's `-c`, or a Mac's `-f`), and a word that it's ready (whatever a login
        // prints before is skipped).
        let ready = format!(
            "K={key}; o=$(mktemp) || exit 1; e=$(mktemp) || exit 1; trap 'rm -f \"$o\" \"$e\"' EXIT; \
             if stat -c %f / >/dev/null 2>&1; then st() {{ stat -L -c '%f %s %Y %n' -- \"$@\"; }}; \
             else st() {{ stat -L -f '%Xp %z %m %N' -- \"$@\"; }}; fi; \
             printf '%s ready\\n' \"$K\"\n",
            key = session.key
        );
        session.input.write_all(ready.as_bytes())?;
        session.input.flush()?;
        let expected = format!("{} ready", session.key);
        let mut line = String::new();
        loop {
            line.clear();
            if session.output.read_line(&mut line)? == 0 {
                // (Give ssh a moment to say why.)
                let _ = session.child.wait();
                std::thread::sleep(Duration::from_millis(50));
                let said = session.said.lock().map(|s| s.trim().to_string()).unwrap_or_default();
                return Err(io::Error::new(io::ErrorKind::ConnectionRefused, connect_error(host, &said)));
            }
            if line.trim_end() == expected {
                return Ok(session);
            }
        }
    }

    fn run(&mut self, script: &str) -> io::Result<Output> {
        let script = if script.trim().is_empty() { ":" } else { script };
        let sent = format!(
            "( {script}\n) >\"$o\" 2>\"$e\" </dev/null; s=$?; \
             printf '%s %s %s %s\\n' \"$K\" \"$s\" $(wc -c <\"$o\") $(wc -c <\"$e\"); cat \"$o\" \"$e\"\n"
        );
        self.input.write_all(sent.as_bytes())?;
        self.input.flush()?;
        let mut line = String::new();
        if self.output.read_line(&mut line)? == 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the connection to the server ended"));
        }
        let mut fields = line.split_whitespace();
        if fields.next() != Some(self.key.as_str()) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the server's answer couldn't be read"));
        }
        let mut number = || -> io::Result<i64> {
            fields
                .next()
                .and_then(|f| f.parse().ok())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "the server's answer couldn't be read"))
        };
        let (status, out, err) = (number()?, number()?, number()?);
        let mut stdout = vec![0; out.max(0) as usize];
        self.output.read_exact(&mut stdout)?;
        let mut stderr = vec![0; err.max(0) as usize];
        self.output.read_exact(&mut stderr)?;
        Ok(Output { status: status as i32, stdout, stderr })
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Why ssh couldn't connect, in a sentence.
fn connect_error(host: &str, said: &str) -> String {
    let last = said.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim();
    if said.contains("Permission denied") || said.contains("Too many authentication failures") {
        // (What was typed didn't sign in: asked again next time.)
        crate::askpass::forget();
        format!(
            "Couldn't sign in to {host}: check the password, or add your key to the server \
             (ssh-copy-id {host}) or to your agent (ssh-add)."
        )
    } else if said.contains("Host key verification failed") {
        format!("{host} isn't a known server yet: connect once with ssh {host} in a terminal to check and accept it.")
    } else if last.is_empty() {
        format!("Couldn't connect to {host}.")
    } else {
        format!("Couldn't connect to {host}: {last}")
    }
}

/// The sessions a server has (a few: a file is read while git runs).
struct Server {
    sessions: Vec<Mutex<Option<Session>>>,
}

const SESSIONS: usize = 3;

static SERVERS: Mutex<Vec<(String, Arc<Server>)>> = Mutex::new(Vec::new());

/// The thread the app's windows run on (see `note_main_thread`).
static MAIN_THREAD: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();

/// Says this is the thread the windows run on (called once, as Null starts).
pub fn note_main_thread() {
    let _ = MAIN_THREAD.set(std::thread::current().id());
}

fn on_main_thread() -> bool {
    MAIN_THREAD.get().is_some_and(|main| *main == std::thread::current().id())
}

/// Whether Null has a connection to `host` open (a folder there can be read at once).
pub fn is_connected(host: &str) -> bool {
    let Some(server) = SERVERS.lock().ok().and_then(|s| s.iter().find(|(h, _)| h == host).map(|(_, s)| s.clone()))
    else {
        return false;
    };
    // (One busy is one open.)
    server.sessions.iter().any(|s| s.try_lock().map_or(true, |s| s.is_some()))
}

/// A server's path in one form, however it was joined (Windows joins with `\`); others as
/// they are.
pub fn plain(path: PathBuf) -> PathBuf {
    match of(&path) {
        Some(remote) => self::path(&remote.host, &remote.path),
        None => path,
    }
}

fn server(host: &str) -> Arc<Server> {
    let mut servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, server)) = servers.iter().find(|(h, _)| h == host) {
        return server.clone();
    }
    let server = Arc::new(Server { sessions: (0..SESSIONS).map(|_| Mutex::new(None)).collect() });
    servers.push((host.to_string(), server.clone()));
    server
}

/// Servers that couldn't be reached lately, and why: asked again only after a while, so
/// a server that's down doesn't hold up everything that asks it.
static UNREACHABLE: Mutex<Vec<(String, std::time::Instant, String)>> = Mutex::new(Vec::new());

const RETRY_AFTER: Duration = Duration::from_secs(20);

fn unreachable(host: &str) -> Option<String> {
    let mut failed = UNREACHABLE.lock().unwrap_or_else(|e| e.into_inner());
    failed.retain(|(_, when, _)| when.elapsed() < RETRY_AFTER);
    failed.iter().find(|(h, ..)| h == host).map(|(.., why)| why.clone())
}

/// Connects to `host` now (asked for by the person: tried again even if it just failed),
/// and says its home folder.
pub fn connect(host: &str) -> io::Result<String> {
    UNREACHABLE.lock().unwrap_or_else(|e| e.into_inner()).retain(|(h, ..)| h != host);
    home(host)
}

/// Runs `script` (sh) on `host`, in the home folder, its input empty. Slow (the network):
/// off the main thread when it can be. A connection that ended is made again, once.
pub fn run(host: &str, script: &str) -> io::Result<Output> {
    if let Some(why) = unreachable(host) {
        return Err(io::Error::new(io::ErrorKind::NotConnected, why));
    }
    let result = run_now(host, script);
    if let Err(error) = &result {
        crate::trace::mark(&format!("ssh {host}: {error}"));
    }
    if let Err(error) = &result
        && matches!(error.kind(), io::ErrorKind::ConnectionRefused | io::ErrorKind::BrokenPipe)
    {
        UNREACHABLE.lock().unwrap_or_else(|e| e.into_inner()).push((
            host.to_string(),
            std::time::Instant::now(),
            error.to_string(),
        ));
    }
    result
}

fn run_now(host: &str, script: &str) -> io::Result<Output> {
    let server = server(host);
    // A free session, else the first when it's free.
    let mut session = match server.sessions.iter().find_map(|s| s.try_lock().ok()) {
        Some(session) => session,
        None => server.sessions[0].lock().unwrap_or_else(|e| e.into_inner()),
    };
    let fresh = session.is_none();
    if fresh {
        *session = Some(Session::start(host)?);
    }
    match session.as_mut().map(|s| s.run(script)) {
        Some(Ok(output)) => Ok(output),
        Some(Err(_)) if !fresh => {
            *session = Some(Session::start(host)?);
            session.as_mut().map_or_else(|| Err(io::Error::other("no session")), |s| s.run(script))
        }
        Some(Err(error)) => {
            *session = None;
            Err(error)
        }
        None => Err(io::Error::other("no session")),
    }
}

/// `script` run on `host`, as an error when it fails.
fn run_ok(host: &str, script: &str) -> io::Result<Output> {
    let output = run(host, script)?;
    if output.success() { Ok(output) } else { Err(output.error()) }
}

/// The servers to offer for a folder: those used before (most recent first), then those
/// named in `~/.ssh/config` (not its patterns).
pub fn hosts_to_offer() -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for project in crate::session::recent_projects() {
        if let Some(remote) = of(&project)
            && !hosts.contains(&remote.host)
        {
            hosts.push(remote.host);
        }
    }
    let config = crate::tools::home_dir().and_then(|h| std::fs::read_to_string(h.join(".ssh").join("config")).ok());
    for host in config.as_deref().map(hosts_in_config).unwrap_or_default() {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
}

/// The names `Host` lines give in an ssh config, but patterns (`*`, `?`, `!`).
fn hosts_in_config(config: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in config.lines() {
        let line = line.trim();
        let Some((word, rest)) = line.split_once(|c: char| c.is_whitespace() || c == '=') else { continue };
        if !word.eq_ignore_ascii_case("host") {
            continue;
        }
        for name in rest.split_whitespace() {
            if !name.contains(['*', '?', '!']) && !hosts.iter().any(|h| h == name) {
                hosts.push(name.to_string());
            }
        }
    }
    hosts
}

/// What changed in `folders` on `host` since the last look with this `stamp` (a file on
/// the server it keeps): files and folders written since, and git's own (its HEAD, index
/// and refs: a commit, a checkout). Nothing the first time: that look sets the stamp.
/// Dependencies and build output aren't gone through (node_modules, target…): too many.
pub fn changed_since(host: &str, folders: &[String], stamp: &str) -> io::Result<Vec<String>> {
    let quoted: Vec<String> = folders.iter().map(|f| quote(f)).collect();
    let script = format!(
        "s={stamp}; n=\"$s.next\"; : > \"$n\" || exit 1; \
         if [ -e \"$s\" ]; then for r in {folders}; do \
           find \"$r\" \\( -name .git -o -name node_modules -o -name target -o -name .venv -o -name __pycache__ \\) -prune -o -newer \"$s\" -print; \
           if [ -d \"$r/.git\" ]; then find \"$r/.git\" -maxdepth 1 -newer \"$s\" -print; \
             [ \"$r/.git/logs/HEAD\" -nt \"$s\" ] && printf '%s\\n' \"$r/.git/logs/HEAD\"; fi; \
         done 2>/dev/null; fi; mv -f \"$n\" \"$s\"; exit 0",
        stamp = quote(stamp),
        folders = quoted.join(" ")
    );
    let output = run_ok(host, &script)?;
    Ok(String::from_utf8_lossy(&output.stdout).lines().filter(|l| l.starts_with('/')).map(str::to_string).collect())
}

/// Every file in `folder` on `host` (paths there), at most `limit`: git's list (what it
/// tracks and what's new, not what it ignores) in a repository, else all of them but
/// dependencies and build output. Names as they are (`café.rs`, not git's quoted
/// `"caf\303\251.rs"`), in a folder whatever its name (`R&D`).
pub fn list_files(host: &str, folder: &str, limit: usize) -> io::Result<Vec<String>> {
    let script = format!(
        "r={q}; cd -- \"$r\" || exit 1; \
         if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then \
           git -c core.quotePath=false ls-files -co --exclude-standard 2>/dev/null \
             | P=\"$PWD\" awk '{{ print ENVIRON[\"P\"] \"/\" $0 }}'; \
         else find \"$PWD\" \\( -name .git -o -name node_modules -o -name target -o -name .venv -o -name __pycache__ \\) -prune -o -type f -print; \
         fi | head -n {limit}",
        q = quote(folder)
    );
    let output = run_ok(host, &script)?;
    Ok(String::from_utf8_lossy(&output.stdout).lines().filter(|l| l.starts_with('/')).map(str::to_string).collect())
}

/// Those of `files` on `host` that have `text` in them (with `ignore_case`, in any case),
/// found there: what a search then reads of a server is only those.
pub fn files_containing(host: &str, files: &[String], text: &str, ignore_case: bool) -> io::Result<Vec<String>> {
    if files.is_empty() || text.is_empty() {
        return Ok(files.to_vec());
    }
    let mut list = files.join("\n");
    list.push('\n');
    let script = format!(
        "tr '\\n' '\\000' <<'NULL_END' | xargs -0 grep -lF {case}-e {q} -- 2>/dev/null; exit 0\n{list}NULL_END",
        case = if ignore_case { "-i " } else { "" },
        q = quote(text)
    );
    let output = run_ok(host, &script)?;
    Ok(String::from_utf8_lossy(&output.stdout).lines().filter(|l| l.starts_with('/')).map(str::to_string).collect())
}

/// What `stat` says of a file: its kind, size, last change and whether it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meta {
    pub dir: bool,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub executable: bool,
}

/// `st`'s line: the mode in hexadecimal, the size, seconds since 1970 (then the name).
fn meta_of(fields: &[&str]) -> Option<Meta> {
    let mode = u32::from_str_radix(fields.first()?, 16).ok()?;
    let len = fields.get(1)?.parse().ok()?;
    let seconds: u64 = fields.get(2)?.parse().ok()?;
    Some(Meta {
        dir: mode & 0o170000 == 0o040000,
        len,
        modified: SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds)),
        executable: mode & 0o170000 == 0o100000 && mode & 0o111 != 0,
    })
}

pub fn read(at: &RemotePath) -> io::Result<Vec<u8>> {
    let bytes = run_ok(&at.host, &format!("cat -- {}", quote(&at.path)))?.stdout;
    // (What it held, for its local history at the next save.)
    crate::local_history::seen_on_server(&path(&at.host, &at.path), &bytes);
    Ok(bytes)
}

pub fn write(at: &RemotePath, bytes: &[u8]) -> io::Result<()> {
    run_ok(&at.host, &format!("base64 -d > {} <<'NULL_END'\n{}NULL_END", quote(&at.path), encoded_lines(bytes)))?;
    Ok(())
}

/// A new empty file, only if nothing has that name there yet (the shell's noclobber: made
/// there and then, not after a look that's out of date by then).
pub fn create_new(at: &RemotePath) -> io::Result<()> {
    let output = run(&at.host, &format!("set -C; : > {}", quote(&at.path)))?;
    match output.status {
        0 => Ok(()),
        _ if exists_there(at) => Err(io::Error::new(io::ErrorKind::AlreadyExists, "it already exists")),
        _ => Err(output.error()),
    }
}

fn exists_there(at: &RemotePath) -> bool {
    run(&at.host, &format!("[ -e {} ] || [ -L {} ]", quote(&at.path), quote(&at.path))).is_ok_and(|o| o.status == 0)
}

/// Writes a file whole or not at all: beside it first (with its permissions), then in its
/// place; a link written through. In place only when nothing can be made beside it (a
/// folder it can't write in): a full disk, or a read-only file, is a failed save, never a
/// file half written over.
pub fn write_whole(at: &RemotePath, bytes: &[u8]) -> io::Result<()> {
    let script = format!(
        "f={q}; g=$(readlink -f -- \"$f\" 2>/dev/null) && [ -n \"$g\" ] && f=$g; \
         if [ -e \"$f\" ] && [ ! -w \"$f\" ]; then echo \"$f is read-only\" >&2; exit 1; fi; \
         t=\"${{f%/*}}/.${{f##*/}}.null-saving-$$\"; \
         ( : > \"$t\" ) 2>/dev/null || exit 75; \
         base64 -d > \"$t\" <<'NULL_END' || {{ rm -f -- \"$t\"; exit 1; }}\n{data}NULL_END\n\
         if [ -e \"$f\" ]; then m=$(stat -c %a -- \"$f\" 2>/dev/null || stat -f %Lp -- \"$f\" 2>/dev/null) && chmod \"$m\" \"$t\"; fi; \
         mv -f -- \"$t\" \"$f\"",
        q = quote(&at.path),
        data = encoded_lines(bytes)
    );
    let output = run(&at.host, &script)?;
    match output.status {
        0 => Ok(()),
        // (Nothing could be made beside it: written in place.)
        75 => write(at, bytes),
        _ => Err(output.error()),
    }
}

#[cfg_attr(test, allow(dead_code))]
/// To the server's Trash (the freedesktop.org one file managers there show): a name of
/// its own in it, and where it was, to put it back.
pub fn trash(at: &RemotePath) -> io::Result<()> {
    let script = format!(
        "f={q}; T=\"${{XDG_DATA_HOME:-$HOME/.local/share}}/Trash\"; mkdir -p \"$T/files\" \"$T/info\" || exit 1; \
         n=${{f##*/}}; d=$n; i=1; \
         while [ -e \"$T/files/$d\" ] || [ -e \"$T/info/$d.trashinfo\" ]; do i=$((i+1)); d=\"$n.$i\"; done; \
         printf '[Trash Info]\\nPath=%s\\nDeletionDate=%s\\n' \"$f\" \"$(date +%Y-%m-%dT%H:%M:%S)\" > \"$T/info/$d.trashinfo\" && \
         mv -- \"$f\" \"$T/files/$d\" || {{ rm -f -- \"$T/info/$d.trashinfo\"; exit 1; }}",
        q = quote(&at.path)
    );
    run_ok(&at.host, &script).map(drop)
}

/// `bytes` in base64, in lines (for a here-document).
pub fn encoded_lines(bytes: &[u8]) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut lines = String::with_capacity(encoded.len() + encoded.len() / 76 + 1);
    for chunk in encoded.as_bytes().chunks(76) {
        lines.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        lines.push('\n');
    }
    lines
}

pub fn metadata(at: &RemotePath) -> io::Result<Meta> {
    let output = run_ok(&at.host, &format!("st {}", quote(&at.path)))?;
    let text = String::from_utf8_lossy(&output.stdout);
    meta_of(&text.split_whitespace().collect::<Vec<_>>())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "stat's answer couldn't be read"))
}

/// What's in a folder (but `.` and `..`), each with what `stat` says; links followed, broken
/// ones left out.
pub fn read_dir(at: &RemotePath) -> io::Result<Vec<(String, Meta)>> {
    let script = format!("cd -- {} || exit 1; set -- * .[!.]* ..?*; st \"$@\" 2>/dev/null; exit 0", quote(&at.path));
    let output = run_ok(&at.host, &script)?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, ' ');
            let fields: Vec<&str> = (0..3).filter_map(|_| parts.next()).collect();
            let name = parts.next()?;
            Some((name.to_string(), meta_of(&fields)?))
        })
        .collect())
}

pub fn create_dir_all(at: &RemotePath) -> io::Result<()> {
    run_ok(&at.host, &format!("mkdir -p -- {}", quote(&at.path))).map(drop)
}

/// Moves `from` to `to`, on the same server.
pub fn rename(from: &RemotePath, to: &RemotePath) -> io::Result<()> {
    run_ok(&from.host, &format!("mv -f -- {} {}", quote(&from.path), quote(&to.path))).map(drop)
}

/// Copies a file (or a folder, with what's in it), on the same server.
pub fn copy(from: &RemotePath, to: &RemotePath) -> io::Result<()> {
    run_ok(&from.host, &format!("cp -pR -- {} {}", quote(&from.path), quote(&to.path))).map(drop)
}

/// The path with links followed (it must be there).
pub fn canonicalize(at: &RemotePath) -> io::Result<RemotePath> {
    let output = run_ok(
        &at.host,
        &format!("cd -- {} 2>/dev/null && pwd -P || readlink -f -- {}", quote(&at.path), quote(&at.path)),
    )?;
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !path.starts_with('/') {
        return Err(io::Error::new(io::ErrorKind::NotFound, "not found"));
    }
    Ok(RemotePath { host: at.host.clone(), path })
}

/// The home folder on `host` (connecting, if it isn't yet: the first thing asked of a
/// server); asked once.
pub fn home(host: &str) -> io::Result<String> {
    static HOMES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
    if let Some((_, home)) = HOMES.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(h, _)| h == host) {
        return Ok(home.clone());
    }
    let output = run_ok(host, "printf '%s' \"$HOME\"")?;
    let home = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let home = if home.starts_with('/') { home } else { "/".into() };
    HOMES.lock().unwrap_or_else(|e| e.into_inner()).push((host.to_string(), home.clone()));
    Ok(home)
}

/// The folder a shell on `host` says it's in, in its title (bash's `me@host: ~/proj`).
pub fn folder_in_title(host: &str, title: &str) -> Option<PathBuf> {
    let shown = title.rsplit_once(": ").map_or(title, |(_, f)| f).trim();
    let inside = match shown.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", home(host).ok()?),
        None if shown.starts_with('/') => shown.to_string(),
        None => return None,
    };
    Some(path(host, &inside))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_server_s_path() {
        let at = |h: &str, p: &str| Some(RemotePath { host: h.into(), path: p.into() });
        assert_eq!(of(Path::new("/ssh:me@box:/home/me/proj")), at("me@box", "/home/me/proj"));
        assert_eq!(of(Path::new("/ssh:box#2222:/srv/")), at("box#2222", "/srv"));
        assert_eq!(of(Path::new("/ssh:box:")), at("box", "/"));
        assert_eq!(of(Path::new(r"/ssh:box:/home\me\a.rs")), at("box", "/home/me/a.rs"));
        assert_eq!(of(Path::new(r"\ssh:box:\home\me")), at("box", "/home/me"));
        assert_eq!(of(Path::new("/ssh:me@[::1]:/x")), at("me@[::1]", "/x"));
        assert_eq!(of(Path::new("/ssh:box:/a/./b/../c")), at("box", "/a/c"));
        assert_eq!(of(Path::new("/home/me")), None);
        assert_eq!(of(Path::new("/ssh:")), None);
        assert_eq!(of(Path::new(r"C:\x")), None);
    }

    #[test]
    fn paths_join_and_go_up_as_paths_do() {
        let proj = path("box", "/home/me/proj");
        assert_eq!(proj, PathBuf::from("/ssh:box:/home/me/proj"));
        let file = proj.join("src").join("main.rs");
        assert_eq!(of(&file).unwrap().path, "/home/me/proj/src/main.rs");
        assert_eq!(file.file_name().unwrap(), "main.rs");
        assert_eq!(file.extension().unwrap(), "rs");
        assert!(file.starts_with(&proj));
        assert_eq!(file.strip_prefix(&proj).unwrap(), Path::new("src/main.rs"));
        assert_eq!(of(proj.parent().unwrap().parent().unwrap()).unwrap().path, "/home");
        assert_eq!(of(Path::new("/ssh:box:/home").parent().unwrap()).unwrap().path, "/");
        assert_eq!(shown(&proj), "box:/home/me/proj");
        assert_eq!(plain(PathBuf::from(r"/ssh:box:/home/me\proj")), proj);
        assert_eq!(destination("box#2222"), ("box", Some("2222")));
        assert_eq!(destination("me@box"), ("me@box", None));
    }

    #[test]
    fn quotes_anything_for_sh() {
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote("it's"), r"'it'\''s'");
        let output = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("printf '%s' {}", quote("$HOME `x` it's \"q\"")))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&output.stdout), "$HOME `x` it's \"q\"");
    }

    #[test]
    fn offers_the_hosts_an_ssh_config_names() {
        let config = "Host box dev-*\n  HostName 10.0.0.2\nhost=build\nMatch all\nHost *\n  User me\nHost box\n";
        assert_eq!(hosts_in_config(config), ["box", "build"]);
    }

    #[test]
    fn reads_stat_s_answer() {
        let meta = meta_of(&["81ed", "12", "1700000000"]).unwrap();
        assert!(!meta.dir && meta.executable && meta.len == 12);
        assert_eq!(meta.modified, SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(1_700_000_000)));
        assert!(meta_of(&["41ed", "4096", "1"]).unwrap().dir);
        assert!(!meta_of(&["81a4", "1", "1"]).unwrap().executable);
    }

    /// A repository's files, listed there: accented names as they are, in a folder named
    /// with characters sed would have taken for its own.
    #[test]
    #[cfg(unix)]
    fn a_server_s_repository_lists_its_files_as_named() {
        let dir = std::env::temp_dir().join(format!("null-remote-git-{}", std::process::id())).join("R&D|x");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(std::process::Command::new("git").arg("init").arg("-q").current_dir(&dir).status().unwrap().success());
        std::fs::write(dir.join("café.rs"), "").unwrap();
        let files = list_files(TEST_HOST, &dir.to_string_lossy(), 100).unwrap();
        assert_eq!(files, [format!("{}/café.rs", dir.display())]);
        std::fs::remove_dir_all(dir.parent().unwrap()).ok();
    }

    /// The whole way, with `sh` here for a server.
    #[test]
    #[cfg(unix)]
    fn files_on_a_server_are_read_written_and_listed() {
        let dir = std::env::temp_dir().join(format!("null-remote-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let at = |p: &str| RemotePath { host: TEST_HOST.into(), path: format!("{}/{p}", dir.display()) };
        create_dir_all(&at("a b/c")).unwrap();
        let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        write(&at("a b/c/it's.bin"), &bytes).unwrap();
        assert_eq!(read(&at("a b/c/it's.bin")).unwrap(), bytes);
        write(&at(".hidden"), b"").unwrap();
        let mut listed = read_dir(&at("")).unwrap();
        listed.sort_by(|a, b| a.0.cmp(&b.0));
        let names: Vec<&str> = listed.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, [".hidden", "a b"]);
        assert!(listed[1].1.dir);
        assert_eq!(metadata(&at("a b/c/it's.bin")).unwrap().len, 5000);
        assert_eq!(read(&at("missing")).unwrap_err().kind(), io::ErrorKind::NotFound);
        rename(&at(".hidden"), &at("shown")).unwrap();
        // Made only if nothing's there by that name: the file there is left as it is.
        assert_eq!(create_new(&at("shown")).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert!(create_new(&at("fresh")).is_ok());
        assert_eq!(read(&at("fresh")).unwrap(), b"");
        std::fs::remove_file(dir.join("fresh")).unwrap();
        // A save: whole, its permissions kept, through a link.
        write_whole(&at("shown"), b"saved").unwrap();
        assert_eq!(read(&at("shown")).unwrap(), b"saved");
        std::os::unix::fs::symlink(dir.join("shown"), dir.join("link")).unwrap();
        write_whole(&at("link"), b"through").unwrap();
        assert_eq!(std::fs::read(dir.join("shown")).unwrap(), b"through");
        assert!(std::fs::symlink_metadata(dir.join("link")).unwrap().file_type().is_symlink());
        assert!(metadata(&at("shown")).is_ok());
        // A read-only file isn't written over: the save fails, the file's as it was.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("shown"), std::fs::Permissions::from_mode(0o444)).unwrap();
        assert!(write_whole(&at("shown"), b"over").is_err());
        assert_eq!(std::fs::read(dir.join("shown")).unwrap(), b"through");
        std::fs::set_permissions(dir.join("shown"), std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_dir_all(dir.join("a b")).unwrap();
        assert!(metadata(&at("a b")).is_err());
        // Every file, as the finder lists them (no repository here: all of them).
        let mut files = list_files(TEST_HOST, &dir.to_string_lossy(), 100).unwrap();
        files.sort();
        assert_eq!(files, [format!("{}/shown", dir.display())]);
        // Those with a text in them, looked for there.
        write(&at("other"), b"nothing here").unwrap();
        let both = [format!("{}/shown", dir.display()), format!("{}/other", dir.display())];
        assert_eq!(files_containing(TEST_HOST, &both, "THROUGH", true).unwrap(), [both[0].clone()]);
        assert!(files_containing(TEST_HOST, &both, "THROUGH", false).unwrap().is_empty());
        std::fs::remove_file(dir.join("other")).unwrap();
        // What changed since the last look (the first only sets its stamp).
        let stamp = format!("{}/.stamp", dir.display());
        let folders = [dir.to_string_lossy().into_owned()];
        assert!(changed_since(TEST_HOST, &folders, &stamp).unwrap().is_empty());
        std::thread::sleep(Duration::from_millis(1100));
        write(&at("new.txt"), b"x").unwrap();
        let changed = changed_since(TEST_HOST, &folders, &stamp).unwrap();
        assert!(changed.contains(&format!("{}/new.txt", dir.display())), "{changed:?}");
        assert!(changed_since(TEST_HOST, &folders, &stamp).unwrap().iter().all(|p| !p.ends_with("new.txt")));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[cfg(unix)]
    fn a_session_answers_in_frames_whatever_is_printed() {
        let output = run(TEST_HOST, "printf 'one\\ntwo'; printf 'oops' >&2; exit 3").unwrap();
        assert_eq!(
            (output.status, output.stdout.as_slice(), output.stderr.as_slice()),
            (3, &b"one\ntwo"[..], &b"oops"[..])
        );
        // (A script printing the frame's form isn't taken for a frame.)
        let output = run(TEST_HOST, "echo 'NULL 0 0 0'; true").unwrap();
        assert_eq!(output.stdout, b"NULL 0 0 0\n");
        assert!(run(TEST_HOST, "").unwrap().success());
    }
}
