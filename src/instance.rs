//! One Null at a time on Windows and Linux. On the Mac the system hands a running Null what
//! `null x` or a double-click opens (see `on_open_urls`); elsewhere every launch is a new
//! process. So the first Null listens on a local port, written with a secret into its data
//! folder, and a later one hands it what to open, then goes: one window per project stays
//! one, and its backups of unsaved work stay its own.

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::Duration;

/// What a launch asks the running Null to open.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Request {
    pub root: PathBuf,
    pub file: Option<PathBuf>,
    pub place: Option<(u32, u32)>,
}

#[derive(Serialize, Deserialize)]
struct Message {
    secret: String,
    request: Request,
}

/// Where the running Null says where it listens: its port, then its secret.
fn address_file() -> Option<PathBuf> {
    Some(crate::tools::data_dir()?.join("instance"))
}

/// Whether launches are handed over here at all: not on the Mac (the system does it), and
/// not for QA runs, which must never reach the Null someone's working in.
fn hands_over() -> bool {
    !cfg!(target_os = "macos") && std::env::var_os("NULL_QA").is_none()
}

/// Hands `request` to the Null already running, if there is one and it takes it.
pub fn hand_over(request: &Request) -> bool {
    if !hands_over() {
        return false;
    }
    let Some(text) = address_file().and_then(|f| std::fs::read_to_string(f).ok()) else { return false };
    let mut lines = text.lines();
    let (Some(port), Some(secret)) = (lines.next().and_then(|p| p.trim().parse::<u16>().ok()), lines.next()) else {
        return false;
    };
    send(port, secret.trim(), request).is_some()
}

fn send(port: u16, secret: &str, request: &Request) -> Option<()> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    // (Nothing there, or something else: quickly given up, and this Null starts itself.)
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_millis(300)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(800))).ok()?;
    let message = Message { secret: secret.to_string(), request: request.clone() };
    let mut line = serde_json::to_string(&message).ok()?;
    line.push('\n');
    stream.write_all(line.as_bytes()).ok()?;
    let mut answer = String::new();
    BufReader::new(stream).take(16).read_line(&mut answer).ok()?;
    (answer.trim() == "ok").then_some(())
}

/// Listens for later launches, as the running Null: what they ask arrives on the receiver.
pub fn listen() -> Option<futures::channel::mpsc::UnboundedReceiver<Request>> {
    if !hands_over() {
        return None;
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let secret = new_secret();
    let file = address_file()?;
    std::fs::create_dir_all(file.parent()?).ok()?;
    write_private(&file, &format!("{port}\n{secret}\n")).ok()?;
    let (tx, rx) = futures::channel::mpsc::unbounded();
    std::thread::Builder::new()
        .name("instance".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Some(request) = receive(stream, &secret)
                    && tx.unbounded_send(request).is_err()
                {
                    break;
                }
            }
        })
        .ok()?;
    Some(rx)
}

fn receive(stream: TcpStream, secret: &str) -> Option<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut line = String::new();
    // (A request is a line of paths: far under this.)
    BufReader::new(&stream).take(64 * 1024).read_line(&mut line).ok()?;
    let message: Message = serde_json::from_str(&line).ok()?;
    if !same(message.secret.as_bytes(), secret.as_bytes()) {
        return None;
    }
    (&stream).write_all(b"ok\n").ok()?;
    Some(message.request)
}

/// Compared in full whatever differs, so timing says nothing of the secret.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// A secret no other program can guess: the standard library's hash keys come from the
/// system's randomness, so what they make of a few values can't be worked out from outside.
fn new_secret() -> String {
    use sha2::Digest;
    use std::hash::{BuildHasher, Hasher};
    let keys = std::collections::hash_map::RandomState::new();
    let mut hasher = sha2::Sha256::new();
    for part in 0u64..4 {
        let mut keyed = keys.build_hasher();
        keyed.write_u64(part);
        keyed.write_u32(std::process::id());
        hasher.update(keyed.finish().to_le_bytes());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    hasher.update(now.as_nanos().to_le_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Written readable by its owner alone (Windows' own folder for each person already is).
fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)?;
        file.write_all(text.as_bytes())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A request goes over with the right secret, and not with another.
    #[test]
    fn a_launch_is_handed_over_only_with_the_secret() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut got = Vec::new();
            for stream in listener.incoming().take(2).flatten() {
                got.push(receive(stream, "s3cret"));
            }
            got
        });
        let request = Request { root: PathBuf::from("/p"), file: Some(PathBuf::from("/p/a.rs")), place: Some((4, 2)) };
        assert!(send(port, "wrong", &request).is_none());
        assert!(send(port, "s3cret", &request).is_some());
        assert_eq!(server.join().unwrap(), [None, Some(request)]);
        assert!(same(b"abc", b"abc") && !same(b"abc", b"abd") && !same(b"abc", b"ab"));
        assert_eq!(new_secret().len(), 64);
    }
}
