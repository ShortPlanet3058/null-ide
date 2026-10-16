//! When ssh asks something (a password, a key's passphrase, whether to trust a server it
//! hasn't met), Null asks it in its window. ssh runs its "askpass" program for that, with
//! the question: Null itself (`SSH_ASKPASS`), which, started so, asks the Null that's running
//! over a local port (with a secret only that Null and its ssh know), prints the answer for
//! ssh, and ends. Answers are kept only in memory, for this run: Windows' ssh can't share a
//! connection, so each program it starts would ask again.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Set on ssh for its askpass program (Null started so): where to ask, and the secret.
const ENV: &str = "NULL_ASKPASS";

#[derive(Serialize, Deserialize)]
struct Question {
    secret: String,
    prompt: String,
}

#[derive(Serialize, Deserialize)]
struct Answer {
    answer: Option<String>,
}

/// A question for the window, and where its answer goes (None: not answered).
pub struct Ask {
    pub prompt: String,
    pub answer: futures::channel::oneshot::Sender<Option<String>>,
}

/// Where the questions go: the app takes them (see `serve`).
static ASKS: Mutex<Option<futures::channel::mpsc::UnboundedSender<Ask>>> = Mutex::new(None);

/// What the person answered this run, by question (a password asked again by the next
/// connection is answered at once).
static ANSWERED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// The port and secret of this Null's listener, started the first time it's needed.
fn listener() -> Option<&'static (u16, String)> {
    static LISTENING: OnceLock<Option<(u16, String)>> = OnceLock::new();
    LISTENING
        .get_or_init(|| {
            let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).ok()?;
            let port = listener.local_addr().ok()?.port();
            let secret = secret();
            let expected = secret.clone();
            std::thread::Builder::new()
                .name("askpass".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let expected = expected.clone();
                        std::thread::spawn(move || answer(stream, &expected));
                    }
                })
                .ok()?;
            Some((port, secret))
        })
        .as_ref()
}

/// A secret no other program can guess: the system's randomness (each `RandomState` is
/// seeded from it), with the time and the process.
fn secret() -> String {
    use sha2::Digest;
    use std::hash::{BuildHasher, Hasher};
    let mut seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
    for _ in 0..4 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write(seed.as_bytes());
        seed.push_str(&format!("{:x}", hasher.finish()));
    }
    sha2::Sha256::digest(seed.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// One question from Null's askpass, answered (by the window, or from this run's answers).
fn answer(stream: TcpStream, expected: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let Ok(question) = serde_json::from_str::<Question>(&line) else { return };
    if question.secret != expected {
        return;
    }
    let answer = known(&question.prompt).or_else(|| ask(&question.prompt));
    if let Some(answer) = &answer
        && kept(&question.prompt)
    {
        let mut answered = ANSWERED.lock().unwrap_or_else(|e| e.into_inner());
        answered.retain(|(p, _)| *p != question.prompt);
        answered.push((question.prompt.clone(), answer.clone()));
    }
    if let Ok(reply) = serde_json::to_string(&Answer { answer }) {
        let _ = (&stream).write_all(format!("{reply}\n").as_bytes());
    }
}

/// Whether an answer is kept for the next time it's asked: a password or a passphrase, not
/// whether to trust a server (ssh remembers that itself).
fn kept(prompt: &str) -> bool {
    !is_trust_question(prompt)
}

/// ssh asking whether to trust a server it hasn't met (`… (yes/no/[fingerprint])?`).
pub fn is_trust_question(prompt: &str) -> bool {
    prompt.contains("(yes/no")
}

fn known(prompt: &str) -> Option<String> {
    ANSWERED.lock().ok()?.iter().find(|(p, _)| p == prompt).map(|(_, a)| a.clone())
}

/// Forgets this run's answers (they didn't sign in: asked again next time).
pub fn forget() {
    ANSWERED.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The question, to the window (waiting for the answer).
fn ask(prompt: &str) -> Option<String> {
    let (tx, rx) = futures::channel::oneshot::channel();
    ASKS.lock().ok()?.as_ref()?.unbounded_send(Ask { prompt: prompt.to_string(), answer: tx }).ok()?;
    futures::executor::block_on(rx).ok().flatten()
}

/// Where the app takes ssh's questions from.
pub fn questions() -> futures::channel::mpsc::UnboundedReceiver<Ask> {
    let (tx, rx) = futures::channel::mpsc::unbounded();
    *ASKS.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    rx
}

/// What ssh is given so it asks Null (None: it can't, and asks nothing).
pub fn environment() -> Option<Vec<(&'static str, String)>> {
    // (Tests' ssh asks nothing.)
    if cfg!(test) || crate::system_clipboard::kept_apart() {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let (port, secret) = listener()?;
    Some(vec![
        ("SSH_ASKPASS", exe.to_string_lossy().into_owned()),
        ("SSH_ASKPASS_REQUIRE", "force".into()),
        // (Older ssh asks its askpass only with a display named.)
        ("DISPLAY", std::env::var("DISPLAY").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| ":0".into())),
        (ENV, format!("{port} {secret}")),
    ])
}

/// Null started as ssh's askpass: asks the Null that started that ssh, prints the answer.
/// True when it was (and has answered); the exit code says whether there was an answer.
pub fn answered_for_ssh() -> Option<i32> {
    let address = std::env::var(ENV).ok()?;
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let (port, secret) = address.split_once(' ')?;
    let port: u16 = port.parse().ok()?;
    Some(match ask_running(port, secret, &prompt) {
        Some(answer) => {
            let mut out = std::io::stdout();
            let _ = writeln!(out, "{answer}");
            let _ = out.flush();
            0
        }
        None => 1,
    })
}

/// `prompt` asked of the Null listening on `port`, with its secret.
fn ask_running(port: u16, secret: &str, prompt: &str) -> Option<String> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).ok()?;
    let question = serde_json::to_string(&Question { secret: secret.to_string(), prompt: prompt.to_string() }).ok()?;
    stream.write_all(format!("{question}\n").as_bytes()).ok()?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).ok()?;
    serde_json::from_str::<Answer>(&line).ok()?.answer
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A question over the port: to the window, its answer back; asked again, answered
    /// at once; with the wrong secret, nothing.
    #[test]
    fn a_question_goes_to_the_window_and_its_answer_back() {
        use futures::StreamExt;
        let mut questions = questions();
        let (port, secret) = listener().unwrap().clone();
        let window = std::thread::spawn(move || {
            let ask = futures::executor::block_on(questions.next()).unwrap();
            assert_eq!(ask.prompt, "me@box's password: ");
            ask.answer.send(Some("typed".into())).unwrap();
        });
        assert_eq!(ask_running(port, &secret, "me@box's password: ").as_deref(), Some("typed"));
        window.join().unwrap();
        // (Kept for this run: no window asked.)
        assert_eq!(ask_running(port, &secret, "me@box's password: ").as_deref(), Some("typed"));
        assert_eq!(ask_running(port, "wrong", "me@box's password: "), None);
        forget();
    }

    #[test]
    fn tells_a_trust_question_from_a_password() {
        assert!(is_trust_question(
            "The authenticity of host 'box (10.0.0.2)' can't be established.\nED25519 key fingerprint is SHA256:x.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? "
        ));
        assert!(!is_trust_question("me@box's password: "));
        assert!(!is_trust_question("Enter passphrase for key '/home/me/.ssh/id_ed25519': "));
    }
}
