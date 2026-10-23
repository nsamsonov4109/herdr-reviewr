//! The reply channel: the agent's `herdr-reviewr --reply <n> …` or `--comment <path>:<line> …`
//! reaches the pane over a socket named for its process, and lands in its threads.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the agent sent the pane.
#[derive(Debug)]
pub enum Incoming {
    /// An answer for the thread numbered `number`.
    Reply { number: u64, text: String, agent: Option<String> },
    /// The agent's own comment on `path`'s lines `start..=end`; `verdict` takes its number, or why not.
    Comment {
        path: PathBuf,
        start: u32,
        end: u32,
        text: String,
        agent: Option<String>,
        verdict: std::sync::mpsc::Sender<Result<u64, String>>,
    },
}

/// The most one request carries: one answer, never a stream.
const MAX_REQUEST: u64 = 1 << 20;

/// How long a comment waits for the pane to place it: a frame, unless an editor holds the pane.
#[cfg(unix)]
const VERDICT_WAIT: Duration = Duration::from_secs(4);

/// Where a pane reviews and sits, as its sidecar says: what a command finds it by without `--to`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct Whereabouts {
    pub root: PathBuf,
    /// The herdr session's socket: tab and workspace ids repeat from one session to the next.
    #[serde(default)]
    pub socket: Option<String>,
    pub workspace: Option<String>,
    pub tab: Option<String>,
}

/// Where the sockets live: private to the user, beside the search cache.
#[cfg(unix)]
fn dir() -> PathBuf {
    crate::search::cache_dir().join("replies")
}

/// The socket of the pane listening at `token`.
#[cfg(unix)]
fn socket_path(token: &str) -> PathBuf {
    dir().join(format!("{token}.sock"))
}

/// The sidecar of the pane listening at `token`.
#[cfg(unix)]
fn sidecar_path(token: &str) -> PathBuf {
    dir().join(format!("{token}.json"))
}

/// The pane's end of the channel: its socket and sidecar, removed when it drops.
#[derive(Debug)]
pub struct Listener {
    token: String,
    path: PathBuf,
    sidecar: PathBuf,
    told: Option<Whereabouts>,
}

impl Listener {
    /// What the agent names this pane by.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Say where the pane is, once that changed; a failed write leaves only `--to` to find it.
    pub fn tell(&mut self, whereabouts: &Whereabouts) {
        if self.told.as_ref() == Some(whereabouts) {
            return;
        }
        let Ok(json) = serde_json::to_vec(whereabouts) else { return };
        // Whole or not at all: a reader never sees half a sidecar.
        let fresh = self.sidecar.with_extension("json.new");
        if std::fs::write(&fresh, json)
            .and_then(|()| std::fs::rename(&fresh, &self.sidecar))
            .is_ok()
        {
            self.told = Some(whereabouts.clone());
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.sidecar);
    }
}

/// Listen for answers to the threads `open` names and for the agent's own comments, through `tx`.
#[cfg(unix)]
pub fn listen(
    open: Arc<Mutex<Vec<u64>>>,
    whereabouts: &Whereabouts,
    tx: crate::wake::Sender<Incoming>,
) -> std::io::Result<Listener> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    static LISTENING: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = dir();
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir)?;
    // A folder an earlier umask made stays private to this user all the same.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    // The process id names the pane; a second listener in one process gets its own.
    let nth = LISTENING.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let pid = std::process::id();
    let token = if nth == 0 { format!("{pid:x}") } else { format!("{pid:x}-{nth}") };
    let path = socket_path(&token);
    // A process that held this id before is gone, and its socket is stale.
    let _ = std::fs::remove_file(&path);
    let listener = std::os::unix::net::UnixListener::bind(&path)?;
    std::thread::Builder::new()
        .name("replies".into())
        .spawn(move || serve(&listener, &open, &tx))?;
    let mut listener = Listener { sidecar: sidecar_path(&token), token, path, told: None };
    listener.tell(whereabouts);
    Ok(listener)
}

/// Without unix sockets nothing reaches the pane; the send then asks for no answer.
#[cfg(not(unix))]
pub fn listen(
    _open: Arc<Mutex<Vec<u64>>>,
    _whereabouts: &Whereabouts,
    _tx: crate::wake::Sender<Incoming>,
) -> std::io::Result<Listener> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "replies need macOS or Linux"))
}

/// Answer each connection: an answer to an open thread or a placed comment lands, the rest refused.
#[cfg(unix)]
fn serve(
    listener: &std::os::unix::net::UnixListener,
    open: &Mutex<Vec<u64>>,
    tx: &crate::wake::Sender<Incoming>,
) {
    use std::io::{BufRead, BufReader, Read, Write};
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        // A client that never finishes its line cannot hold the channel.
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let mut line = String::new();
        let read = BufReader::new((&stream).take(MAX_REQUEST)).read_line(&mut line);
        let verdict = match read.map_err(|e| e.to_string()).and_then(|_| parse_request(&line)) {
            Ok(request) => match answer(request, open, tx) {
                Some(verdict) => verdict,
                None => return,
            },
            Err(e) => format!("error: {e}"),
        };
        let _ = writeln!(stream, "{verdict}");
    }
}

/// The pane's word on one request, `ok #n` or `error: …`; `None` once the pane is gone.
#[cfg(unix)]
fn answer(
    request: Request,
    open: &Mutex<Vec<u64>>,
    tx: &crate::wake::Sender<Incoming>,
) -> Option<String> {
    let Request { number, text, comment, agent } = request;
    if let Some(Spot { path, start, end }) = comment {
        let (verdict, heard) = std::sync::mpsc::channel();
        tx.send(Incoming::Comment { path, start, end, text, agent, verdict }).ok()?;
        return Some(match heard.recv_timeout(VERDICT_WAIT) {
            Ok(Ok(number)) => format!("ok #{number}"),
            Ok(Err(why)) => format!("error: {why}"),
            Err(_) => "error: the review pane is busy; try again".to_string(),
        });
    }
    let Some(number) = number else { return Some("error: a reply needs a number".to_string()) };
    let open = open.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    if !open.contains(&number) {
        return Some(refusal(number, &open));
    }
    tx.send(Incoming::Reply { number, text, agent }).ok()?;
    Some(format!("ok #{number}"))
}

/// Why `number` takes no answer, naming the threads that do.
#[cfg(unix)]
fn refusal(number: u64, open: &[u64]) -> String {
    if open.is_empty() {
        return format!("error: no comment #{number} is open, and none is");
    }
    let open: Vec<String> = open.iter().map(|n| format!("#{n}")).collect();
    format!("error: no comment #{number} is open; open: {}", open.join(", "))
}

/// One message on the wire. A reply is `{number, text}`, as panes before comments read it.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct Request {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    number: Option<u64>,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comment: Option<Spot>,
    /// The agent's name as herdr knows its pane, which its words carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
}

/// Where a comment goes: a file by its absolute path, and its lines.
#[derive(Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
struct Spot {
    path: PathBuf,
    start: u32,
    end: u32,
}

#[cfg(unix)]
fn parse_request(line: &str) -> Result<Request, String> {
    serde_json::from_str(line.trim_end()).map_err(|e| format!("unreadable request: {e}"))
}

/// Deliver one `--reply` or `--comment`, its text inline or on stdin; the exit code says if it landed.
pub fn run(args: &[OsString]) -> i32 {
    match deliver(args) {
        Ok(said) => {
            println!("{said}");
            0
        }
        Err(e) => {
            eprintln!("reviewr: {e}");
            1
        }
    }
}

/// What a command sends: an answer to a thread, or a comment on lines.
#[derive(Debug, PartialEq, Eq)]
enum What {
    Reply(u64),
    Comment { path: PathBuf, start: u32, end: u32 },
}

/// What the arguments ask for: the message, the pane if named, and the text if given inline.
fn parse_args(args: &[OsString]) -> Result<(What, Option<String>, String), String> {
    const USAGE: &str = "usage: herdr-reviewr --reply <number> [--to <token>] [answer]\n       \
                         herdr-reviewr --comment <path>:<line>[-<end>] [--to <token>] [comment]";
    let mut what = None;
    let mut token = None;
    let mut words = Vec::new();
    let mut it = args.iter().map(|a| a.to_string_lossy().into_owned());
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--reply" => {
                let value = it.next().ok_or(USAGE)?;
                let value = value.trim_start_matches('#');
                let number =
                    value.parse::<u64>().map_err(|_| format!("not a comment number: {value}"))?;
                what = Some(What::Reply(number));
            }
            "--comment" => what = Some(parse_spot(&it.next().ok_or(USAGE)?)?),
            "--to" => token = Some(it.next().ok_or(USAGE)?),
            _ => words.push(arg),
        }
    }
    let what = what.ok_or(USAGE)?;
    Ok((what, token, words.join(" ")))
}

/// `<path>:<line>` or `<path>:<start>-<end>`, lines from 1.
fn parse_spot(value: &str) -> Result<What, String> {
    let bad = || format!("not a <path>:<line> or <path>:<start>-<end>: {value}");
    let (path, lines) = value.rsplit_once(':').ok_or_else(bad)?;
    let (start, end) = lines.split_once('-').unwrap_or((lines, lines));
    let line = |s: &str| s.trim().parse::<u32>().ok().filter(|&n| n > 0);
    match (line(start), line(end)) {
        (Some(start), Some(end)) if end >= start && !path.is_empty() => {
            Ok(What::Comment { path: PathBuf::from(path), start, end })
        }
        _ => Err(bad()),
    }
}

/// Send one message to its pane, and read the pane's verdict.
fn deliver(args: &[OsString]) -> Result<String, String> {
    let (what, token, mut text) = parse_args(args)?;
    if text.trim().is_empty() {
        use std::io::Read;
        std::io::stdin()
            .take(MAX_REQUEST)
            .read_to_string(&mut text)
            .map_err(|e| format!("reading the message: {e}"))?;
    }
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("a message needs text, inline or on stdin".to_string());
    }
    let agent =
        std::env::var("HERDR_PANE_ID").ok().and_then(|pane| crate::herdr::agent_name(&pane));
    let here =
        std::env::current_dir().map_err(|e| format!("reading the working directory: {e}"))?;
    let (request, near, done) = match &what {
        What::Reply(number) => {
            let request = Request { number: Some(*number), text, comment: None, agent };
            (request, here, "answered")
        }
        What::Comment { path, start, end } => {
            let path = std::fs::canonicalize(here.join(path))
                .map_err(|_| format!("no file at {}", path.display()))?;
            let spot = Spot { path: path.clone(), start: *start, end: *end };
            (Request { number: None, text, comment: Some(spot), agent }, path, "commented")
        }
    };
    let token = match token {
        Some(token) => token,
        None => discover(&near)?,
    };
    let verdict = exchange(&token, &request)?;
    match verdict.strip_prefix("error: ") {
        // A pane from before comments reads a comment as a reply missing its number.
        Some(why) if why.contains("missing field `number`") => {
            Err("that review pane is older than this command: reopen it".to_string())
        }
        Some(why) => Err(why.to_string()),
        None => match verdict.strip_prefix("ok ") {
            Some(number) => Ok(format!("{done} {number}")),
            None => Err(format!("the pane answered {verdict:?}")),
        },
    }
}

/// The pane without `--to`: the one in the agent's tab, else its workspace, else the deepest at `near`.
#[cfg(unix)]
fn discover(near: &Path) -> Result<String, String> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    // Inside herdr, only this session's panes: another session reuses the same ids.
    let socket = var("HERDR_SOCKET_PATH");
    let panes: Vec<(String, Whereabouts)> = listening()
        .into_iter()
        .filter(|(_, w)| socket.is_none() || w.socket.is_none() || w.socket == socket)
        .collect();
    let only = |keep: &dyn Fn(&Whereabouts) -> bool| -> Option<String> {
        let mut found = panes.iter().filter(|(_, w)| keep(w));
        match (found.next(), found.next()) {
            (Some((token, _)), None) => Some(token.clone()),
            _ => None,
        }
    };
    let by_tab =
        var("HERDR_TAB_ID").and_then(|tab| only(&|w| w.tab.as_deref() == Some(tab.as_str())));
    let by_workspace = || {
        var("HERDR_WORKSPACE_ID")
            .and_then(|ws| only(&|w| w.workspace.as_deref() == Some(ws.as_str())))
    };
    let by_root = || {
        // An empty root would sit above every path.
        let mut under: Vec<&(String, Whereabouts)> = panes
            .iter()
            .filter(|(_, w)| !w.root.as_os_str().is_empty() && near.starts_with(&w.root))
            .collect();
        let depth = |w: &Whereabouts| w.root.components().count();
        under.sort_by_key(|(_, w)| std::cmp::Reverse(depth(w)));
        match under.as_slice() {
            [deepest] => Some(deepest.0.clone()),
            [deepest, next, ..] if depth(&deepest.1) > depth(&next.1) => Some(deepest.0.clone()),
            _ => None,
        }
    };
    by_tab.or_else(by_workspace).or_else(by_root).ok_or_else(|| {
        if panes.is_empty() {
            return "no review pane is open".to_string();
        }
        let tokens: Vec<&str> = panes.iter().map(|(t, _)| t.as_str()).collect();
        format!("several review panes are open: pass --to with one of {}", tokens.join(", "))
    })
}

#[cfg(not(unix))]
fn discover(_near: &Path) -> Result<String, String> {
    Err("replies need macOS or Linux for now".to_string())
}

/// Every pane whose socket answers, with what its sidecar says.
#[cfg(unix)]
fn listening() -> Vec<(String, Whereabouts)> {
    let Ok(entries) = std::fs::read_dir(dir()) else { return Vec::new() };
    let mut panes: Vec<(String, Whereabouts)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                return None;
            }
            let token = path.file_stem()?.to_str()?.to_string();
            let whereabouts = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            // A sidecar a crashed pane left behind answers nothing.
            std::os::unix::net::UnixStream::connect(socket_path(&token)).ok()?;
            Some((token, whereabouts))
        })
        .collect();
    panes.sort_by(|a, b| a.0.cmp(&b.0));
    panes
}

#[cfg(unix)]
fn exchange(token: &str, request: &Request) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    // A token is what the answer line names, so it never spells a path of its own.
    if token.is_empty() || !token.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(format!("not a review pane's token: {token}"));
    }
    let mut stream = std::os::unix::net::UnixStream::connect(socket_path(token))
        .map_err(|_| format!("no review pane answers at {token}: it closed"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let line = serde_json::to_string(request).map_err(|e| e.to_string())?;
    writeln!(stream, "{line}").map_err(|e| format!("sending the message: {e}"))?;
    let mut verdict = String::new();
    BufReader::new(&stream)
        .read_line(&mut verdict)
        .map_err(|e| format!("the pane did not answer: {e}"))?;
    Ok(verdict.trim_end().to_string())
}

#[cfg(not(unix))]
fn exchange(_token: &str, _request: &Request) -> Result<String, String> {
    Err("replies need macOS or Linux for now".to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use super::{Incoming, What, Whereabouts, listen, parse_args};
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_arguments_name_the_message_the_pane_and_the_text() {
        let parsed = parse_args(&args(&["--reply", "#3", "--to", "1f", "because", "it", "is"]));
        assert_eq!(parsed, Ok((What::Reply(3), Some("1f".to_string()), "because it is".into())));
        assert!(parse_args(&args(&["--reply", "three", "--to", "1f"])).is_err());
        assert_eq!(parse_args(&args(&["--reply", "3"])), Ok((What::Reply(3), None, String::new())));
        let comment = parse_args(&args(&["--comment", "src/a.rs:12-14", "look", "here"]));
        let spot = What::Comment { path: PathBuf::from("src/a.rs"), start: 12, end: 14 };
        assert_eq!(comment, Ok((spot, None, "look here".to_string())));
        let one = parse_args(&args(&["--comment", "a.rs:7"])).unwrap().0;
        assert_eq!(one, What::Comment { path: PathBuf::from("a.rs"), start: 7, end: 7 });
        for bad in ["a.rs", "a.rs:0", "a.rs:9-3", ":4", "a.rs:x"] {
            assert!(parse_args(&args(&["--comment", bad])).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_answer_to_an_open_thread_lands_and_any_other_is_refused() {
        let open = Arc::new(Mutex::new(vec![2, 5]));
        let (tx, rx) = crate::wake::channel::<Incoming>(&crate::wake::Waker::detached());
        let listener = listen(Arc::clone(&open), &Whereabouts::default(), tx).unwrap();
        let token = listener.token().to_string();
        let send = |number: &str, text: &str| {
            super::deliver(&args(&["--reply", number, "--to", &token, text]))
        };
        assert_eq!(send("5", "it guards the restart"), Ok("answered #5".to_string()));
        match rx.recv().unwrap() {
            Incoming::Reply { number, text, .. } => {
                assert_eq!((number, text.as_str()), (5, "it guards the restart"));
            }
            other @ Incoming::Comment { .. } => panic!("not a reply: {other:?}"),
        }
        let refused = send("9", "lost").unwrap_err();
        assert!(refused.contains("no comment #9") && refused.contains("#2, #5"), "{refused}");
        drop(listener);
        let gone = send("2", "late").unwrap_err();
        assert!(gone.contains("closed"), "{gone}");
    }

    #[test]
    fn a_comment_lands_as_the_number_the_pane_gives_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        let (tx, rx) = crate::wake::channel::<Incoming>(&crate::wake::Waker::detached());
        let listener =
            listen(Arc::new(Mutex::new(Vec::new())), &Whereabouts::default(), tx).unwrap();
        let token = listener.token().to_string();
        let pane = std::thread::spawn(move || match rx.recv().unwrap() {
            Incoming::Comment { path, start, end, text, verdict, .. } => {
                verdict.send(Ok(7)).unwrap();
                (path, start, end, text)
            }
            other @ Incoming::Reply { .. } => panic!("not a comment: {other:?}"),
        });
        let spot = format!("{}:2", file.display());
        let said = super::deliver(&args(&["--comment", &spot, "--to", &token, "see", "this"]));
        assert_eq!(said, Ok("commented #7".to_string()));
        let (path, start, end, text) = pane.join().unwrap();
        assert_eq!(path, std::fs::canonicalize(&file).unwrap());
        assert_eq!((start, end, text.as_str()), (2, 2, "see this"));
        let missing =
            super::deliver(&args(&["--comment", "/no/such/file.rs:1", "--to", &token, "x"]));
        assert!(missing.unwrap_err().contains("no file at"));
    }
}
