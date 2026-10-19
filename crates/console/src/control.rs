//! The console from outside the process, for a dedicated server that has no
//! terminal: with `IW4L_CONSOLE_SOCKET=<path>` the game listens on a Unix
//! socket there, mode 0600 (its own user only, and nothing on the network).
//! A connection sends one command line and gets back what the command
//! printed on the console, then the socket closes it. `iw4l
//! dedicated-console "<command>"` is the client.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::prelude::*;

pub const SOCKET_ENV: &str = "IW4L_CONSOLE_SOCKET";
/// Where a client looks when `IW4L_CONSOLE_SOCKET` is not set.
pub const DEFAULT_SOCKET: &str = "/tmp/iw4l-console.sock";
/// The longest command line taken.
pub const MAX_COMMAND_BYTES: usize = 1024;
/// Frames with nothing left to run before the answer is sent: a reply some
/// commands print a frame or two after they ran (`admin transfer`).
const SETTLE_FRAMES: u32 = 30;
/// The longest a command may take to answer (one waiting on the world).
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Where a client connects: `IW4L_CONSOLE_SOCKET`, or the default.
pub fn socket_path() -> PathBuf {
    std::env::var_os(SOCKET_ENV)
        .filter(|path| !path.is_empty())
        .map_or_else(|| PathBuf::from(DEFAULT_SOCKET), PathBuf::from)
}

/// One command line, refused when it is empty, too long or more than a line.
pub fn command_line(text: &str) -> Result<&str, String> {
    let line = text.trim();
    if line.is_empty() {
        return Err("no command".into());
    }
    if line.len() > MAX_COMMAND_BYTES {
        return Err(format!("a command is at most {MAX_COMMAND_BYTES} bytes"));
    }
    if line.contains(['\n', '\r', '\0']) {
        return Err("one line only".into());
    }
    Ok(line)
}

/// The answer to an asked command: its console lines, or a note that it
/// printed none.
pub fn answer_text(lines: &[String]) -> String {
    if lines.is_empty() {
        return "(no output)\n".to_owned();
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Sends `command` to the console listening at `path` and returns its answer.
#[cfg(unix)]
pub fn ask(path: &Path, command: &str) -> std::io::Result<String> {
    use std::io::{Read, Write};
    let command = command_line(command)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.set_read_timeout(Some(ANSWER_TIMEOUT + IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    stream.write_all(command.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer)?;
    Ok(answer)
}

#[cfg(not(unix))]
pub fn ask(_path: &Path, _command: &str) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the dedicated server's console socket is a Unix socket",
    ))
}

/// A command asked through the socket, and where its answer goes.
pub struct Request {
    pub line: String,
    pub answer: std::sync::mpsc::SyncSender<String>,
}

#[derive(Resource)]
pub(crate) struct ControlRequests(std::sync::Mutex<std::sync::mpsc::Receiver<Request>>);

/// The command being answered.
#[derive(Default)]
pub(crate) struct InFlight {
    answer: Option<std::sync::mpsc::SyncSender<String>>,
    started: Option<std::time::Instant>,
    settled: u32,
}

/// Listens on `path` (a socket left there by a server that stopped is
/// replaced) and hands each connection's command to `requests`; one
/// connection at a time, each answered or timed out before the next.
#[cfg(unix)]
pub fn listen(path: &Path, requests: std::sync::mpsc::SyncSender<Request>) -> std::io::Result<()> {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} exists and is not a socket", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    std::thread::Builder::new()
        .name("console-socket".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if let Err(e) = serve(stream, &requests)
                    && e.kind() == std::io::ErrorKind::NotConnected
                {
                    break;
                }
            }
        })?;
    Ok(())
}

#[cfg(unix)]
fn serve(
    mut stream: std::os::unix::net::UnixStream,
    requests: &std::sync::mpsc::SyncSender<Request>,
) -> std::io::Result<()> {
    use std::io::{Read, Write};
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let mut text = Vec::new();
    (&mut stream)
        .take(MAX_COMMAND_BYTES as u64 + 2)
        .read_to_end(&mut text)?;
    let text = String::from_utf8_lossy(&text);
    let reply = match command_line(text.strip_suffix('\n').unwrap_or(&text)) {
        Ok(line) => {
            let (answer, answered) = std::sync::mpsc::sync_channel(1);
            requests
                .send(Request {
                    line: line.to_owned(),
                    answer,
                })
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::NotConnected))?;
            answered
                .recv_timeout(ANSWER_TIMEOUT + IO_TIMEOUT)
                .unwrap_or_else(|_| "(no answer in time)\n".to_owned())
        }
        Err(e) => format!("refused: {e}\n"),
    };
    stream.write_all(reply.as_bytes())
}

pub(crate) fn install(app: &mut App) {
    let Some(path) = std::env::var_os(SOCKET_ENV).filter(|path| !path.is_empty()) else {
        return;
    };
    let path = PathBuf::from(path);
    let (sender, receiver) = std::sync::mpsc::sync_channel(4);
    #[cfg(unix)]
    let listening = listen(&path, sender);
    #[cfg(not(unix))]
    let listening: std::io::Result<()> = {
        drop(sender);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Unix sockets only",
        ))
    };
    match listening {
        Ok(()) => {
            diag::info!(Launch, "console socket: {}", path.display());
            println!("console-socket: {}", path.display());
            app.insert_resource(ControlRequests(std::sync::Mutex::new(receiver)));
        }
        Err(e) => diag::warn!(Launch, "console socket {}: {e}", path.display()),
    }
}

/// Runs an asked command and, once nothing is left to run and its lines
/// have settled, sends back what the console printed meanwhile.
pub(crate) fn serve_requests(
    requests: Option<Res<ControlRequests>>,
    mut queue: ResMut<crate::ConsoleQueue>,
    commands: Res<crate::ConsoleCommandQueue>,
    mut in_flight: Local<InFlight>,
) {
    let Some(requests) = requests else { return };
    if in_flight.answer.is_some() {
        let idle = queue.is_empty() && commands.0.is_empty();
        in_flight.settled = if idle { in_flight.settled + 1 } else { 0 };
        let late = in_flight
            .started
            .is_some_and(|at| at.elapsed() > ANSWER_TIMEOUT);
        if in_flight.settled < SETTLE_FRAMES && !late {
            return;
        }
        if let Some(answer) = std::mem::take(&mut *in_flight).answer {
            let _ = answer.try_send(answer_text(&diag::end_tap()));
        }
    }
    let Ok(receiver) = requests.0.lock() else {
        return;
    };
    let Some(request) = receiver.try_iter().find(|r| !r.line.is_empty()) else {
        return;
    };
    diag::info!(Launch, "console socket: {}", request.line);
    diag::begin_tap();
    queue.push_line(&request.line);
    *in_flight = InFlight {
        answer: Some(request.answer),
        started: Some(std::time::Instant::now()),
        settled: 0,
    };
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("iw4l-console-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// Stands in for the game: answers each request with `reply(line)`.
    fn console(reply: fn(&str) -> String) -> std::sync::mpsc::SyncSender<Request> {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Request>(4);
        std::thread::spawn(move || {
            for request in receiver {
                let _ = request.answer.send(reply(&request.line));
            }
        });
        sender
    }

    #[test]
    fn a_command_sent_to_the_socket_gets_its_console_lines_back() {
        let dir = scratch("ask");
        let path = dir.join("console.sock");
        listen(
            &path,
            console(|line| answer_text(&[format!("ran: {line}")])),
        )
        .expect("listen");
        assert_eq!(ask(&path, "whoami").expect("ask"), "ran: whoami\n");
        assert_eq!(
            ask(&path, "  admins set - label  ").expect("ask"),
            "ran: admins set - label\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_socket_is_its_owner_s_only_and_replaces_a_stale_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("mode");
        let path = dir.join("console.sock");
        // A socket a stopped server left behind.
        drop(std::os::unix::net::UnixListener::bind(&path).expect("stale socket"));
        listen(&path, console(|_| answer_text(&[]))).expect("listen over a stale socket");
        let mode = std::fs::metadata(&path)
            .expect("socket")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(ask(&path, "admins").expect("ask"), "(no output)\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_that_is_not_a_socket_is_left_alone() {
        let dir = scratch("file");
        let path = dir.join("console.sock");
        std::fs::write(&path, "keep").expect("file");
        assert!(listen(&path, console(|_| String::new())).is_err());
        assert_eq!(std::fs::read_to_string(&path).expect("file"), "keep");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_one_line_of_a_bounded_length_is_a_command() {
        assert_eq!(command_line(" whoami \n"), Ok("whoami"));
        assert!(command_line("").is_err());
        assert!(command_line("whoami\nadmins").is_err());
        assert!(command_line(&"x".repeat(MAX_COMMAND_BYTES + 1)).is_err());
        let dir = scratch("refuse");
        let path = dir.join("console.sock");
        listen(&path, console(|line| format!("ran: {line}\n"))).expect("listen");
        let mut stream = std::os::unix::net::UnixStream::connect(&path).expect("connect");
        use std::io::{Read, Write};
        stream.write_all(b"whoami\nadmins set -\n").expect("write");
        stream
            .shutdown(std::net::Shutdown::Write)
            .expect("shutdown");
        let mut answer = String::new();
        stream.read_to_string(&mut answer).expect("read");
        assert!(answer.starts_with("refused: "), "{answer}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
