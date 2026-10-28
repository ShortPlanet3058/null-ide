//! Ollama, started when it's needed: with the AI set to Ollama on this Mac and nothing
//! answering there, Null runs `ollama serve` itself (if it's installed), and stops it when
//! Null quits. One already running, or one on another computer, is left as it is.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::process::Child;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The `ollama serve` Null started, to stop when it quits.
static STARTED: Mutex<Option<Child>> = Mutex::new(None);

/// When starting it last failed (it stopped at once): not tried again for a minute, so
/// requests don't each wait for one that won't come.
static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// How long a starting Ollama is waited for.
const START_WAIT: Duration = Duration::from_secs(8);

/// The address Ollama would listen on for `base` (`http://127.0.0.1:11434/v1`), when it's
/// this computer's: another computer's isn't Null's to start.
/// (`localhost` as 127.0.0.1, where Ollama listens: the Mac also names it ::1.)
fn local_address(base: &str) -> Option<(String, SocketAddr)> {
    let url = url::Url::parse(base).ok()?;
    let port = url.port_or_known_default()?;
    let host = match url.host_str()? {
        "127.0.0.1" | "localhost" => "127.0.0.1".to_string(),
        "::1" | "[::1]" => "[::1]".to_string(),
        _ => return None,
    };
    let address = format!("{host}:{port}").to_socket_addrs().ok()?.next()?;
    Some((format!("{host}:{port}"), address))
}

fn answers(address: &SocketAddr) -> bool {
    TcpStream::connect_timeout(address, Duration::from_millis(300)).is_ok()
}

/// The models Ollama at `base` has installed (its `/api/tags`), by name: empty when it
/// doesn't answer. Asks over the network: off the main thread.
pub fn installed(base: &str) -> Vec<String> {
    if cfg!(test) || crate::system_clipboard::kept_apart() {
        return Vec::new();
    }
    let root = base.trim_end_matches('/').trim_end_matches("/v1");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(3))).build().into();
    let Ok(mut response) = agent.get(&format!("{root}/api/tags")).call() else { return Vec::new() };
    let Ok(json) = response.body_mut().read_json::<serde_json::Value>() else { return Vec::new() };
    names_of(&json)
}

/// The names in Ollama's list of models.
fn names_of(json: &serde_json::Value) -> Vec<String> {
    json["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str().map(str::to_string)).collect()
}

/// Before asking Ollama at `base`: started if it's this computer's, isn't running, and is
/// installed. Waits a few seconds for it to answer. (Runs off the main thread.)
pub fn ensure_running(base: &str) {
    // (Tests and screenshot runs keep to themselves: nothing of yours is started, and
    // their stand-in servers aren't knocked on.)
    if cfg!(test) || crate::system_clipboard::kept_apart() {
        return;
    }
    let Some((host, address)) = local_address(base) else { return };
    if !answers(&address) {
        let failed_lately = FAILED_AT.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|at| at.elapsed() < RETRY);
        if !failed_lately {
            start(&host, &address);
        }
    }
}

/// How long after a start that failed it isn't tried again.
const RETRY: Duration = Duration::from_secs(60);

/// Runs `ollama serve` on `host` (if it's installed and not started already), and waits for
/// it at `address`.
fn start(host: &str, address: &SocketAddr) {
    let Some(program) = crate::tools::find("ollama") else { return };
    let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
    // Started already, and still starting: wait for that one.
    let running = started.as_mut().is_some_and(|child| child.try_wait().is_ok_and(|done| done.is_none()));
    if !running {
        let child = crate::tools::command(program)
            .arg("serve")
            .env("OLLAMA_HOST", host)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(child) => *started = Some(child),
            Err(err) => return eprintln!("null: couldn't start ollama: {err}"),
        }
    }
    drop(started);
    let since = Instant::now();
    while since.elapsed() < START_WAIT && !answers(address) {
        // Stopped already (it couldn't start): no use waiting.
        let mut started = STARTED.lock().unwrap_or_else(|e| e.into_inner());
        if started.as_mut().is_none_or(|child| child.try_wait().is_ok_and(|done| done.is_some())) {
            started.take();
            *FAILED_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
            return;
        }
        drop(started);
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Null quitting: the Ollama it started stops too.
/// (Asked to stop first, so it stops the model it runs too; killed if it doesn't.)
pub fn stop() {
    let Some(mut child) = STARTED.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    #[cfg(target_os = "macos")]
    {
        // SAFETY: a signal to the process Null started, still its child.
        unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
        let since = Instant::now();
        while since.elapsed() < Duration::from_secs(2) {
            if child.try_wait().is_ok_and(|done| done.is_some()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    child.kill().ok();
    child.wait().ok();
}

#[cfg(test)]
mod tests {

    #[test]
    fn reads_the_models_installed() {
        let json = serde_json::json!({ "models": [{ "name": "qwen2.5-coder:1.5b" }, { "name": "llama3.2:3b" }] });
        assert_eq!(super::names_of(&json), ["qwen2.5-coder:1.5b", "llama3.2:3b"]);
        assert!(super::names_of(&serde_json::json!({})).is_empty());
    }

    use super::*;

    #[test]
    fn only_this_computer_s_ollama_is_started() {
        let (host, address) = local_address("http://127.0.0.1:11434/v1").unwrap();
        assert_eq!((host.as_str(), address.port()), ("127.0.0.1:11434", 11434));
        // localhost: where Ollama listens, not the ::1 the Mac may give first.
        assert_eq!(local_address("http://localhost:11434").unwrap().0, "127.0.0.1:11434");
        assert_eq!(local_address("http://[::1]:11434").unwrap().0, "[::1]:11434");
        assert!(local_address("http://192.168.1.20:11434/v1").is_none(), "another computer's");
        assert!(local_address("https://api.example.com/v1").is_none());
        assert!(local_address("not a url").is_none());
    }

    /// Needs Ollama installed (on the PATH): `cargo test ollama_starts_and_stops -- --ignored`
    #[test]
    #[ignore]
    fn ollama_starts_and_stops() {
        // A port of its own, not the one an Ollama of yours may be on.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let (host, address) = local_address(&format!("http://127.0.0.1:{port}/v1")).unwrap();
        assert!(!answers(&address));
        start(&host, &address);
        assert!(answers(&address), "started");
        stop();
        std::thread::sleep(Duration::from_millis(500));
        assert!(!answers(&address), "stopped with Null");
    }
}
