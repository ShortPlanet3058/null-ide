//! Newer versions of Null: whether one is out (once a day when the person asked to be told,
//! or when they ask), and, when they click Update, getting it and putting it in place.
//!
//! What's out is read from the latest release's `update.json`: its version and each
//! package's SHA-256, signed with the project's own key (Ed25519, its private half only in
//! the release workflow). An answer that isn't signed with it is ignored, and a package
//! whose digest differs is never run. One request at a time, nothing sent about the person
//! or the computer beyond what any web request carries.
//!
//! A package is installed the way Null was: Windows' installer run again quietly, the Mac
//! app replaced, Linux's binary replaced or its .deb installed again (the system asks for
//! the password). Any other way (the Windows zip, a build from source) is told where to
//! get it. Installing waits for Null to quit; Restart to Update quits and starts it again.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The latest release's files.
const LATEST: &str = "https://github.com/ShortPlanet3058/null-ide/releases/latest/download";
/// A release's files, by its tag.
const RELEASES: &str = "https://github.com/ShortPlanet3058/null-ide/releases/download";
/// Where to get it by hand.
pub const DOWNLOAD: &str = "https://null-ide.com/#download";
/// Not asked again sooner than this.
const EVERY_SECS: u64 = 20 * 3600;
/// The public half of the key releases are signed with.
const KEY: [u8; 32] = [
    0x48, 0x42, 0xc1, 0xed, 0xaa, 0xba, 0x08, 0xe9, 0x29, 0x01, 0xfc, 0xf7, 0x21, 0x06, 0x27, 0x34, 0x91, 0x76, 0x15,
    0xd6, 0x99, 0x54, 0x7f, 0xad, 0x50, 0x30, 0x05, 0xca, 0x7f, 0x98, 0x1f, 0x49,
];

/// What the last check found, kept in Null's folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Kept {
    /// When (seconds since 1970).
    checked: u64,
    /// The latest release's version ("0.2.0").
    latest: Option<String>,
}

/// A release, as its signed `update.json` has it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct Manifest {
    version: String,
    /// Each package's name, and its SHA-256 (hex).
    files: BTreeMap<String, String>,
}

/// Where a newer version is at, this run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Nothing,
    /// This version is out.
    Out(String),
    /// Being downloaded: how much of it, in percent.
    Getting(String, u8),
    /// Downloaded and checked: installed when Null quits.
    Ready(String),
}

static STATE: Mutex<State> = Mutex::new(State::Nothing);
/// The package downloaded, and how it's installed.
static READY: Mutex<Option<(Install, PathBuf)>> = Mutex::new(None);
/// Started again once the update is in place.
static RESTART: AtomicBool = AtomicBool::new(false);

/// How this Null was installed, so how a newer one replaces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// Windows' installer (setup.exe): run again, quietly. Null's own path.
    Setup(PathBuf),
    /// The Mac app (from the disk image): replaced. The app's path.
    App(PathBuf),
    /// Linux's binary (the .tar.gz, install.sh's ~/.local/bin/null): replaced.
    Binary(PathBuf),
    /// Linux's .deb: installed again, the system asking for the password.
    Deb(PathBuf),
}

impl Install {
    /// The release's package for it.
    fn package(&self) -> &'static str {
        match self {
            Install::Setup(_) => "null-windows-x86_64-setup.exe",
            Install::App(_) => "Null-macos.dmg",
            Install::Binary(_) => "null-linux-x86_64.tar.gz",
            Install::Deb(_) => "null-linux-amd64.deb",
        }
    }
}

/// This build's version.
pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Whether `latest` ("v0.2.0", "0.2.0") is newer than `current` ("0.1.1").
pub fn newer(current: &str, latest: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> {
        v.trim().trim_start_matches('v').split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (a, b) = (parts(current), parts(latest));
    (0..3).map(|i| b.get(i).copied().unwrap_or(0).cmp(&a.get(i).copied().unwrap_or(0))).find(|o| o.is_ne())
        == Some(std::cmp::Ordering::Greater)
}

fn file() -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("update.json"))
}

fn kept() -> Kept {
    file().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether it's time to ask again.
pub fn due() -> bool {
    now().saturating_sub(kept().checked) >= EVERY_SECS
}

pub fn state() -> State {
    STATE.lock().map(|s| s.clone()).unwrap_or_default()
}

fn set_state(state: State) {
    if let Ok(mut s) = STATE.lock() {
        *s = state;
    }
}

/// What the last check found, known again (a newer version found yesterday is still out).
pub fn remember() {
    let found = kept().latest.filter(|l| newer(current(), l));
    if let (Some(found), Ok(mut state)) = (found, STATE.lock())
        && *state == State::Nothing
    {
        *state = State::Out(found);
    }
}

/// True the first time it's asked in this run: the check is made once, however many
/// windows there are.
pub fn first_this_run() -> bool {
    static ASKED: AtomicBool = AtomicBool::new(false);
    !ASKED.swap(true, Ordering::Relaxed)
}

/// Where the releases are read from: GitHub, or for trying an update out, a folder served
/// elsewhere (`NULL_UPDATE_FROM=http://…`), with its own `update.json` and packages.
fn served_from() -> Option<String> {
    std::env::var("NULL_UPDATE_FROM").ok().filter(|f| !f.is_empty()).map(|f| f.trim_end_matches('/').to_string())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(15)))
        .user_agent(format!("Null/{}", current()))
        .build()
        .into()
}

/// A GET, asked again once on a connection the other end had closed (kept for reuse).
fn call(agent: &ureq::Agent, url: &str) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    match agent.get(url).call() {
        Err(ureq::Error::Io(_)) => agent.get(url).call(),
        answer => answer,
    }
}

/// A small file's bytes; None when it isn't there.
fn fetch(agent: &ureq::Agent, url: &str) -> Result<Option<Vec<u8>>, String> {
    match call(agent, url) {
        Ok(response) => response
            .into_body()
            .read_to_vec()
            .map(Some)
            .map_err(|e| format!("The update's details couldn't be read: {e}")),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(e) => Err(format!("Couldn't reach GitHub: {e}")),
    }
}

/// `json` read, if `signature` is the release key's signature of it.
fn verified(json: &[u8], signature: &[u8], key: &[u8]) -> Result<Manifest, String> {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
        .verify(json, signature)
        .map_err(|_| "The update isn't signed with Null's key: it's left alone.".to_string())?;
    serde_json::from_slice(json).map_err(|e| format!("The update's details couldn't be read: {e}"))
}

/// The latest release, signed. None when it has no `update.json`.
fn latest(agent: &ureq::Agent) -> Result<Option<Manifest>, String> {
    let base = served_from().unwrap_or_else(|| LATEST.to_string());
    let Some(json) = fetch(agent, &format!("{base}/update.json"))? else { return Ok(None) };
    let signature =
        fetch(agent, &format!("{base}/update.json.sig"))?.ok_or("The update has no signature: it's left alone.")?;
    verified(&json, &signature, &KEY).map(Some)
}

/// Asks for the latest release. Slow (a request): off the main thread. Ok(Some) when it's
/// newer than this one.
pub fn check() -> Result<Option<String>, String> {
    // (A QA run asks nothing of the network.)
    if crate::system_clipboard::kept_apart() {
        return Ok(None);
    }
    let latest = latest(&agent())?.map(|m| m.version);
    let kept = Kept { checked: now(), latest: latest.clone() };
    if let (Some(path), Ok(json)) = (file(), serde_json::to_string(&kept)) {
        // (Null's folder may not be there yet, on a first launch.)
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, json);
    }
    let found = latest.filter(|l| newer(current(), l));
    if let Ok(mut state) = STATE.lock() {
        match (&found, &*state) {
            // (Already on its way.)
            (Some(v), State::Getting(on, _) | State::Ready(on)) if v == on => {}
            (Some(v), _) => *state = State::Out(v.clone()),
            (None, State::Out(_)) => *state = State::Nothing,
            (None, _) => {}
        }
    }
    Ok(found)
}

/// How this Null was installed; None when Null can't replace itself (the Windows zip, a
/// build from source, a folder it can't write to).
pub fn how() -> Option<Install> {
    if !cfg!(target_arch = "x86_64") && !cfg!(target_os = "macos") {
        return None;
    }
    how_for(&std::env::current_exe().ok()?, std::env::consts::OS)
}

fn how_for(exe: &Path, os: &str) -> Option<Install> {
    // (A build from source: cargo's folder.)
    if exe.components().any(|c| c.as_os_str() == "target") {
        return None;
    }
    let dir = exe.parent()?;
    match os {
        // (Inno Setup leaves its uninstaller beside Null; the zip has none.)
        "windows" => {
            let installed = std::fs::read_dir(dir).ok()?.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                name.starts_with("unins") && name.ends_with(".exe")
            });
            installed.then(|| Install::Setup(exe.to_path_buf()))
        }
        "macos" => {
            let app = dir.parent()?.parent()?;
            let bundled = dir.ends_with("Contents/MacOS") && app.extension().is_some_and(|e| e == "app");
            (bundled && writable(app.parent()?)).then(|| Install::App(app.to_path_buf()))
        }
        "linux" if exe.starts_with("/usr") => {
            let packaged = Path::new("/var/lib/dpkg/info/null-editor.list").exists();
            (packaged && Path::new("/usr/bin/pkexec").exists()).then(|| Install::Deb(exe.to_path_buf()))
        }
        "linux" => writable(dir).then(|| Install::Binary(exe.to_path_buf())),
        _ => None,
    }
}

/// Whether a file can be made in `dir` (permissions don't say, on every system).
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".null-update-{}", std::process::id()));
    let made = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    made
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Downloads the newer version and checks it, `progress` told how far it is (percent).
/// Slow: off the main thread. Once it's done, it's installed when Null quits.
pub fn get(progress: impl Fn(u8)) -> Result<String, String> {
    use sha2::Digest;
    use std::io::{Read, Write};
    let how = how().ok_or("Null can't update itself here: get the new version at null-ide.com.")?;
    let agent = agent();
    let manifest = latest(&agent)?.ok_or("No update is published.")?;
    let version = manifest.version.clone();
    if !newer(current(), &version) {
        return Err(format!("Null {} is the latest version.", current()));
    }
    let name = how.package();
    let digest = manifest.files.get(name).ok_or("The new version has no package for this system.")?.to_lowercase();
    let dir = crate::tools::data_dir().ok_or("Null's folder isn't known.")?.join("updates");
    // (An earlier download goes: one at a time.)
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
    let url = match served_from() {
        Some(base) => format!("{base}/{name}"),
        None => format!("{RELEASES}/v{version}/{name}"),
    };
    set_state(State::Getting(version.clone(), 0));
    let failed = |message: String| {
        set_state(State::Out(version.clone()));
        message
    };
    let response = call(&agent, &url).map_err(|e| failed(format!("Couldn't download the update: {e}")))?;
    let size = response.body().content_length();
    let mut reader = response.into_body().into_with_config().limit(u64::MAX).reader();
    let path = dir.join(name);
    let part = dir.join(format!("{name}.part"));
    let mut out = std::fs::File::create(&part).map_err(|e| failed(format!("Couldn't save the update: {e}")))?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let (mut read, mut shown) = (0u64, 0u8);
    loop {
        let n = reader.read(&mut buffer).map_err(|e| failed(format!("The download stopped: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        out.write_all(&buffer[..n]).map_err(|e| failed(format!("Couldn't save the update: {e}")))?;
        read += n as u64;
        if let Some(size) = size.filter(|s| *s > 0) {
            let percent = (read * 100 / size).min(100) as u8;
            if percent != shown {
                shown = percent;
                set_state(State::Getting(version.clone(), percent));
                progress(percent);
            }
        }
    }
    drop(out);
    if hex(&hasher.finalize()) != digest {
        let _ = std::fs::remove_file(&part);
        return Err(failed("The download isn't the release's package (its SHA-256 differs): it's left alone.".into()));
    }
    std::fs::rename(&part, &path).map_err(|e| failed(format!("Couldn't save the update: {e}")))?;
    if let Ok(mut ready) = READY.lock() {
        *ready = Some((how, path));
    }
    set_state(State::Ready(version.clone()));
    Ok(version)
}

/// An update installed (or given up on) at the last quit: its files go, once whatever ran
/// it has ended. Off the main thread.
pub fn tidy() {
    let Some(dir) = crate::tools::data_dir().map(|d| d.join("updates")) else { return };
    if dir.exists() {
        std::thread::sleep(std::time::Duration::from_secs(20));
        // (Downloaded meanwhile, this run: kept.)
        if READY.lock().is_ok_and(|r| r.is_none()) && !matches!(state(), State::Getting(..)) {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Null starts again once the update is in place (Restart to Update).
pub fn restart_after() {
    RESTART.store(true, Ordering::Relaxed);
}

/// Quitting was called off: a later quit only installs.
pub fn restart_called_off() {
    RESTART.store(false, Ordering::Relaxed);
}

fn reopen_file() -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("reopen.json"))
}

/// Restarting to update: each window's folders, opened again when Null starts (it's
/// started with nothing to open).
pub fn keep_windows(windows: Vec<Vec<PathBuf>>) {
    if !RESTART.load(Ordering::Relaxed) || READY.lock().is_ok_and(|r| r.is_none()) || windows.is_empty() {
        return;
    }
    if let (Some(file), Ok(json)) = (reopen_file(), serde_json::to_string(&windows)) {
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(file, json);
    }
}

/// The windows open when Null restarted to update (gone once read; a few minutes old at
/// most, else it's another start).
pub fn windows_to_reopen() -> Vec<Vec<PathBuf>> {
    let Some(file) = reopen_file() else { return Vec::new() };
    let fresh =
        std::fs::metadata(&file).and_then(|m| m.modified()).is_ok_and(|t| t.elapsed().is_ok_and(|e| e.as_secs() < 300));
    let windows = std::fs::read_to_string(&file).ok().and_then(|s| serde_json::from_str(&s).ok());
    let _ = std::fs::remove_file(&file);
    let windows: Vec<Vec<PathBuf>> = if fresh { windows.unwrap_or_default() } else { Vec::new() };
    windows.into_iter().filter(|w| !w.is_empty()).collect()
}

/// Null is quitting: a downloaded update is put in place once it has, by a process of its
/// own (Null's files can't be replaced while it runs, on Windows).
pub fn install_on_quit() {
    let Some((how, package)) = READY.lock().ok().and_then(|mut r| r.take()) else { return };
    let restart = if RESTART.load(Ordering::Relaxed) { "1" } else { "0" };
    let pid = std::process::id().to_string();
    if let Err(error) = start_installing(&how, &package, &pid, restart) {
        eprintln!("null: couldn't install the update: {error}");
    }
}

#[cfg(windows)]
fn start_installing(how: &Install, package: &Path, pid: &str, restart: &str) -> std::io::Result<()> {
    let Install::Setup(exe) = how else { return Err(std::io::Error::other("not an installed Null")) };
    // (A copy runs the installer: Null's own file is replaced.)
    let helper = package.with_file_name("null-update.exe");
    std::fs::copy(exe, &helper)?;
    std::process::Command::new(helper)
        .arg("--finish-update")
        .args([pid, restart])
        .arg(package)
        .arg(exe)
        .spawn()
        .map(drop)
}

/// Waits for Null to quit, runs the installer quietly (it keeps the choices made the first
/// time) and starts Null again if asked. Returns the exit code. (`null --finish-update`.)
#[cfg(windows)]
pub fn finish(args: &[String]) -> i32 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn WaitForSingleObject(handle: isize, millis: u32) -> u32;
        fn CloseHandle(handle: isize) -> i32;
    }
    let [pid, restart, setup, null] = args else { return 2 };
    const SYNCHRONIZE: u32 = 0x0010_0000;
    // SAFETY: a handle opened here, waited on and closed; no pointers.
    unsafe {
        let process = OpenProcess(SYNCHRONIZE, 0, pid.parse().unwrap_or(0));
        if process != 0 {
            WaitForSingleObject(process, 120_000);
            CloseHandle(process);
        }
    }
    let installed = std::process::Command::new(setup)
        .args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/NOCANCEL", "/SP-"])
        .status()
        .is_ok_and(|s| s.success());
    if restart == "1" {
        let _ = std::process::Command::new(null).spawn();
    }
    if installed { 0 } else { 1 }
}

#[cfg(unix)]
fn start_installing(how: &Install, package: &Path, pid: &str, restart: &str) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    // Each waits for Null to quit ($1), puts the package ($3) in place of the app ($4), and
    // starts it again if asked ($2). What's there stays when anything fails.
    const WAIT: &str = "while kill -0 \"$1\" 2>/dev/null; do sleep 0.2; done\n";
    let (script, target) = match how {
        Install::App(app) => (
            r#"m=$(mktemp -d) || exit 1
if hdiutil attach -nobrowse -readonly -noautoopen -mountpoint "$m" "$3" >/dev/null; then
    new=$(ls -d "$m"/*.app | head -n 1)
    if [ -n "$new" ] && rm -rf "$4.old" && mv "$4" "$4.old"; then
        if ditto "$new" "$4"; then rm -rf "$4.old"; else rm -rf "$4"; mv "$4.old" "$4"; fi
    fi
    hdiutil detach "$m" -quiet
fi
rmdir "$m"
[ "$2" = 1 ] && open "$4"
"#,
            app,
        ),
        Install::Binary(bin) => (
            r#"d=$(mktemp -d) || exit 1
if tar -xzf "$3" -C "$d"; then
    new=$(find "$d" -type f -name null | head -n 1)
    [ -n "$new" ] && cp "$new" "$4.new" && chmod 755 "$4.new" && mv -f "$4.new" "$4"
fi
rm -rf "$d"
[ "$2" = 1 ] && nohup "$4" >/dev/null 2>&1 &
"#,
            bin,
        ),
        Install::Deb(bin) => (
            r#"pkexec dpkg -i "$3"
[ "$2" = 1 ] && nohup "$4" >/dev/null 2>&1 &
"#,
            bin,
        ),
        Install::Setup(_) => return Err(std::io::Error::other("not Windows")),
    };
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{WAIT}{script}"))
        .arg("sh")
        .args([pid, restart])
        .arg(package)
        .arg(target)
        // (Its own group: a terminal Null was started from doesn't stop it.)
        .process_group(0)
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_a_newer_version() {
        assert!(newer("0.1.1", "v0.2.0"));
        assert!(newer("0.1.1", "0.1.2"));
        assert!(newer("0.9.9", "1.0.0"));
        assert!(!newer("0.2.0", "v0.2.0"));
        assert!(!newer("0.2.0", "0.1.9"));
        assert!(!newer("0.2.0", "0.2"));
        assert!(newer("0.2", "0.2.1"));
    }

    #[test]
    fn only_a_signed_release_is_read() {
        use ring::signature::KeyPair;
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let key = pair.public_key().as_ref();
        let json = br#"{"version":"0.2.0","files":{"Null-macos.dmg":"ab12"}}"#;
        let signature = pair.sign(json);
        let manifest = verified(json, signature.as_ref(), key).unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.files["Null-macos.dmg"], "ab12");
        // Changed, or signed with another key: not read.
        let changed = br#"{"version":"9.9.9","files":{"Null-macos.dmg":"ab12"}}"#;
        assert!(verified(changed, signature.as_ref(), key).is_err());
        assert!(verified(json, signature.as_ref(), &KEY).is_err());
    }

    /// The release workflow signs with openssl (`pkeyutl -sign -rawin`): what it makes is
    /// what Null reads, with the key it has.
    #[test]
    fn reads_what_the_release_workflow_signs() {
        let signature: Vec<u8> =
            (0..64).map(|i| u8::from_str_radix(&SIGNED_SAMPLE[i * 2..i * 2 + 2], 16).unwrap()).collect();
        let manifest = verified(br#"{"version":"0.0.1","files":{}}"#, &signature, &KEY).unwrap();
        assert_eq!(manifest.version, "0.0.1");
    }

    const SIGNED_SAMPLE: &str = "37d1c99634a4545e31f3343358e28f16de09df3e2818f91de86d3647946c40cc\
        1d6a63afca3a2ea14981d57fffe1163183be517c875eed99faba163f8aee9006";

    #[test]
    fn tells_how_null_was_installed() {
        let dir = crate::tools::test_dir("updates-how");
        let _ = std::fs::remove_dir_all(&dir);
        // Windows: the installer's uninstaller beside it; the zip has none.
        let setup = dir.join("Programs/Null");
        std::fs::create_dir_all(&setup).unwrap();
        std::fs::write(setup.join("null.exe"), "").unwrap();
        assert_eq!(how_for(&setup.join("null.exe"), "windows"), None);
        std::fs::write(setup.join("unins000.exe"), "").unwrap();
        assert_eq!(how_for(&setup.join("null.exe"), "windows"), Some(Install::Setup(setup.join("null.exe"))));
        // The Mac: inside an app.
        let app = dir.join("Applications/Null.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        assert_eq!(how_for(&app.join("Contents/MacOS/Null"), "macos"), Some(Install::App(app.clone())));
        assert_eq!(how_for(&dir.join("bin/null"), "macos"), None);
        // Linux: a binary where it can be replaced.
        let bin = dir.join("home/.local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        assert_eq!(how_for(&bin.join("null"), "linux"), Some(Install::Binary(bin.join("null"))));
        // Built from source: never.
        assert_eq!(how_for(&dir.join("target/release/null"), "linux"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restarting_to_update_opens_the_same_windows() {
        let windows = vec![vec![PathBuf::from("/a"), PathBuf::from("/b")], vec![PathBuf::from("/ssh:box:/app")]];
        // Not restarting (or nothing to install): nothing kept.
        keep_windows(windows.clone());
        assert!(windows_to_reopen().is_empty());
        *READY.lock().unwrap() = Some((Install::Binary(PathBuf::from("/x/null")), PathBuf::from("/x/u.tar.gz")));
        restart_after();
        keep_windows(windows.clone());
        restart_called_off();
        READY.lock().unwrap().take();
        assert_eq!(windows_to_reopen(), windows);
        assert!(windows_to_reopen().is_empty(), "read once");
    }

    /// Asks GitHub for real: `cargo test asks_github -- --ignored`.
    #[test]
    #[ignore]
    fn asks_github_for_its_latest_release() {
        let found = check().unwrap();
        assert!(!due(), "the check is kept");
        assert_eq!(found.is_some(), state() != State::Nothing);
        println!("latest newer than {}: {found:?}", current());
    }
}
