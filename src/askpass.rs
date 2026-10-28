//! When ssh asks something (a password, a key's passphrase, whether to trust a server it
//! hasn't met), Null asks it in its window. ssh runs its "askpass" program for that, with
//! the question: Null itself (`SSH_ASKPASS`), which, started so, asks the Null that's running
//! over a local port (with a secret only that Null and its ssh know), prints the answer for
//! ssh, and ends. Answers are kept only in memory, for this run: Windows' ssh can't share a
//! connection, so each program it starts would ask again.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Set on ssh for its askpass program (Null started so): where to ask, and the secret.
const ENV: &str = "NULL_ASKPASS";

#[derive(Serialize, Deserialize)]
struct Question {
    secret: String,
    prompt: String,
    /// Answered only from this run's answers, never asked in the window: an ssh started
    /// by Null's window's own thread, which can't show a question while it waits for ssh.
    #[serde(default)]
    known_only: bool,
    /// The ssh asking (its process; 0 where that isn't known): asking the same again means
    /// the answer it was given didn't do.
    #[serde(default)]
    asker: u32,
    /// The server ssh is reaching: an answer is given again only to the same one (a server
    /// can write its own question, the same words as another's).
    #[serde(default)]
    host: String,
}

/// Set on an ssh whose questions are answered only from this run's answers.
const KNOWN_ONLY: &str = "NULL_ASKPASS_KNOWN_ONLY";

/// Set on ssh: the server it reaches (see `Question::host`).
const HOST: &str = "NULL_ASKPASS_HOST";

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

/// What the person answered this run, by server and question (a password asked again by
/// the next connection to it is answered at once).
static ANSWERED: Mutex<Vec<((String, String), String)>> = Mutex::new(Vec::new());

/// Who asked what this run (an ssh's process, the question): the same ssh asking the same
/// again was given a wrong answer.
static ASKED: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

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
    // (A question is a line, and short: no more is read.)
    let mut reader = BufReader::new(&stream).take(64 * 1024);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let Ok(question) = serde_json::from_str::<Question>(&line) else { return };
    if question.secret != expected {
        return;
    }
    // Asked again by the same ssh: the answer kept was wrong (a typo). Not given again (three
    // tries at once, a server shutting the door): asked in the window.
    if question.asker != 0 {
        let mut asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
        let again = (question.asker, question.prompt.clone());
        if asked.contains(&again) {
            let key = (question.host.clone(), question.prompt.clone());
            ANSWERED.lock().unwrap_or_else(|e| e.into_inner()).retain(|(k, _)| *k != key);
        } else {
            asked.push(again);
        }
    }
    let key = (question.host.clone(), question.prompt.clone());
    let answer = known(&key).or_else(|| (!question.known_only).then(|| ask(&question.prompt)).flatten());
    if let Some(answer) = &answer
        && kept(&question.prompt)
    {
        let mut answered = ANSWERED.lock().unwrap_or_else(|e| e.into_inner());
        answered.retain(|(k, _)| *k != key);
        answered.push((key, answer.clone()));
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

fn known(key: &(String, String)) -> Option<String> {
    ANSWERED.lock().ok()?.iter().find(|(k, _)| k == key).map(|(_, a)| a.clone())
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

/// The same, for an ssh that mustn't ask in the window (started by the window's own
/// thread: it would wait for itself): this run's answers only.
pub fn environment_known_only(host: &str) -> Option<Vec<(&'static str, String)>> {
    let mut environment = environment(host)?;
    environment.push((KNOWN_ONLY, "1".into()));
    Some(environment)
}

/// What ssh reaching `host` is given so it asks Null (None: it can't, and asks nothing).
pub fn environment(host: &str) -> Option<Vec<(&'static str, String)>> {
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
        (HOST, host.to_string()),
    ])
}

/// Null started as ssh's askpass: asks the Null that started that ssh, prints the answer.
/// True when it was (and has answered); the exit code says whether there was an answer.
pub fn answered_for_ssh() -> Option<i32> {
    let address = std::env::var(ENV).ok()?;
    let prompt = std::env::args().nth(1).unwrap_or_default();
    let (port, secret) = address.split_once(' ')?;
    let port: u16 = port.parse().ok()?;
    let known_only = std::env::var_os(KNOWN_ONLY).is_some();
    let host = std::env::var(HOST).unwrap_or_default();
    // (The ssh that started this one: on Windows not known, 0.)
    #[cfg(unix)]
    let asker = std::os::unix::process::parent_id();
    #[cfg(not(unix))]
    let asker = 0;
    Some(match ask_running(port, secret, &prompt, known_only, asker, &host) {
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
fn ask_running(port: u16, secret: &str, prompt: &str, known_only: bool, asker: u32, host: &str) -> Option<String> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).ok()?;
    let question = serde_json::to_string(&Question {
        secret: secret.to_string(),
        prompt: prompt.to_string(),
        known_only,
        asker,
        host: host.to_string(),
    })
    .ok()?;
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
        let password = "me@box's password: ";
        let window = std::thread::spawn(move || {
            for typed in ["typo", "right"] {
                let ask = futures::executor::block_on(questions.next()).unwrap();
                assert_eq!(ask.prompt, "me@box's password: ");
                ask.answer.send(Some(typed.into())).unwrap();
            }
        });
        assert_eq!(ask_running(port, &secret, password, false, 101, "box").as_deref(), Some("typo"));
        // (Kept for this run: another ssh is answered at once, no window asked.)
        assert_eq!(ask_running(port, &secret, password, false, 102, "box").as_deref(), Some("typo"));
        // That ssh asking again: the answer didn't do. Asked in the window, not given again.
        assert_eq!(ask_running(port, &secret, password, false, 102, "box").as_deref(), Some("right"));
        window.join().unwrap();
        assert_eq!(ask_running(port, "wrong", password, false, 103, "box"), None);
        // (Answers only from this run's: one never given isn't asked in the window.)
        assert_eq!(ask_running(port, &secret, "other@box's password: ", true, 104, "box"), None);
        // Another server asking the same words isn't given the answer.
        assert_eq!(ask_running(port, &secret, password, true, 105, "evil"), None);
        assert_eq!(ask_running(port, &secret, password, true, 106, "box").as_deref(), Some("right"));
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
