//! Two games through a master of the run's own: an `iw4l-master` on the
//! loopback with a certificate minted for the run, a host that opens a lobby
//! and a client that joins it, each driven through its console on stdin
//! (`IW4L_CONSOLE_STDIN`) and read from its stdout.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::runner::{Event, parse};

/// The master's name in its certificate, which the games check.
pub const SERVER_NAME: &str = "iw4l-dev";

/// A process killed when dropped.
struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub struct Master {
    _child: Killed,
    pub addr: String,
    pub ca: PathBuf,
}

/// A CA, and a server certificate it signs for `SERVER_NAME`, as PEM files in
/// `dir`: (ca, cert, key).
fn mint_certificates(dir: &Path) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair};
    let error = |e: rcgen::Error| format!("certificate: {e}");
    let ca_key = KeyPair::generate().map_err(error)?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).map_err(error)?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "approved_tests run CA");
    let ca = ca_params.self_signed(&ca_key).map_err(error)?;
    let key = KeyPair::generate().map_err(error)?;
    let params = CertificateParams::new(vec![SERVER_NAME.to_owned()]).map_err(error)?;
    let cert = params.signed_by(&key, &ca, &ca_key).map_err(error)?;
    let paths = (
        dir.join("ca.pem"),
        dir.join("master.pem"),
        dir.join("master.key"),
    );
    for (path, text) in [
        (&paths.0, ca.pem()),
        (&paths.1, cert.pem()),
        (&paths.2, key.serialize_pem()),
    ] {
        std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    Ok(paths)
}

impl Master {
    /// Starts `bin` on a free loopback port with certificates minted in `dir`.
    pub fn start(bin: &Path, dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let (ca, cert, key) = mint_certificates(dir)?;
        let port = std::net::UdpSocket::bind("127.0.0.1:0")
            .and_then(|socket| socket.local_addr())
            .map_err(|e| format!("free port: {e}"))?
            .port();
        let addr = format!("127.0.0.1:{port}");
        let log = |name: &str| {
            std::fs::File::create(dir.join(name)).map_err(|e| format!("master log: {e}"))
        };
        let child = Command::new(bin)
            .args(["serve", "--bind", &addr, "--cert"])
            .arg(&cert)
            .arg("--key")
            .arg(&key)
            .stdout(log("master_stdout.txt")?)
            .stderr(log("master_stderr.txt")?)
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
        let mut child = Killed(child);
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(Some(status)) = child.0.try_wait() {
            return Err(format!(
                "iw4l-master exited at once: {status}; see {}",
                dir.display()
            ));
        }
        Ok(Self {
            _child: child,
            addr,
            ca,
        })
    }
}

/// One game: its console, its stdout lines, and its master status file.
pub struct Player {
    pub name: &'static str,
    pub dir: PathBuf,
    child: Killed,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<(u128, String)>,
    log: std::fs::File,
    started: Instant,
    /// Every `minecraft-state:`, `minecraft-block:` and `minecraft-mobs:`
    /// line, by label.
    pub seen: BTreeMap<String, Event>,
    /// A dedicated server's `dedicated-players:` lines, in order.
    pub rosters: Vec<Event>,
    /// Its `minecraft-mobnet:` lines (a host's mob bytes to each client).
    pub mobnet: Vec<Event>,
    /// Peak resident memory sampled, in KiB.
    pub peak_rss_kib: u64,
}

impl Player {
    pub fn launch(
        bin: &Path,
        dir: &Path,
        name: &'static str,
        session: u32,
        env: &[(&str, String)],
    ) -> Result<Self, String> {
        Self::launch_with(bin, dir, name, session, &["menu"], env)
    }

    pub fn launch_with(
        bin: &Path,
        dir: &Path,
        name: &'static str,
        session: u32,
        args: &[&str],
        env: &[(&str, String)],
    ) -> Result<Self, String> {
        let file = |file: &str| {
            std::fs::File::create(dir.join(file)).map_err(|e| format!("create {file}: {e}"))
        };
        // A status left by an earlier session would name its room.
        let _ = std::fs::remove_file(dir.join("master.status"));
        let mut command = Command::new(bin);
        command
            .args(args)
            .current_dir(dir)
            .env("IW4L_SETTINGS_PATH", dir.join("settings.cfg"))
            .env("IW4L_MASTER_STATUS_FILE", dir.join("master.status"))
            .env("IW4L_CONSOLE_STDIN", "1")
            .env_remove("IW4L_CMDS")
            .env_remove("IW4L_MASTER_JOIN")
            .env_remove("IW4L_MASTER_HOST_NAME")
            .env_remove("IW4L_MINECRAFT_SEED")
            .env_remove("IW4L_MINECRAFT_WORLD")
            .env_remove("IW4L_MINECRAFT_WORLDS")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(file(&format!("child_stderr_{session}.txt"))?);
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", bin.display()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout is piped");
        let started = Instant::now();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send((started.elapsed().as_millis(), line)).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            name,
            dir: dir.to_path_buf(),
            child: Killed(child),
            stdin,
            lines,
            log: file(&format!("child_stdout_{session}.txt"))?,
            started,
            seen: BTreeMap::new(),
            rosters: Vec::new(),
            mobnet: Vec::new(),
            peak_rss_kib: 0,
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.0.id()
    }

    pub fn send(&mut self, script: &str) -> Result<(), String> {
        println!(
            "[{:>7.1}s] {} <- {script}",
            self.started.elapsed().as_secs_f64(),
            self.name
        );
        let stdin = self.stdin.as_mut().ok_or("console closed")?;
        writeln!(stdin, "{script}").map_err(|e| format!("{}: console: {e}", self.name))
    }

    pub fn exited(&mut self) -> Option<std::process::ExitStatus> {
        self.child.0.try_wait().ok().flatten()
    }

    /// Asks the process to stop as a supervisor does: SIGTERM, or the
    /// console's `quit` where there is none.
    pub fn terminate(&mut self) -> Result<(), String> {
        println!(
            "[{:>7.1}s] {} <- SIGTERM",
            self.started.elapsed().as_secs_f64(),
            self.name
        );
        if cfg!(unix) {
            let status = Command::new("kill")
                .args(["-TERM", &self.pid().to_string()])
                .status()
                .map_err(|e| format!("kill: {e}"))?;
            if !status.success() {
                return Err(format!("kill -TERM {}: {status}", self.pid()));
            }
            Ok(())
        } else {
            self.send("quit")
        }
    }

    /// Takes the lines printed so far.
    pub fn pump(&mut self) {
        while let Ok((at_ms, line)) = self.lines.try_recv() {
            let _ = writeln!(self.log, "{at_ms:>9} {line}");
            if let Some(event) = parse(&line, "dedicated-players:", at_ms) {
                println!("[{:>7.1}s] {} -> {line}", at_ms as f64 / 1000.0, self.name);
                self.rosters.push(event);
            }
            if let Some(event) = parse(&line, "minecraft-mobnet:", at_ms) {
                println!("[{:>7.1}s] {} -> {line}", at_ms as f64 / 1000.0, self.name);
                self.mobnet.push(event);
            }
            for prefix in ["minecraft-state:", "minecraft-block:", "minecraft-mobs:"] {
                if let Some(event) = parse(&line, prefix, at_ms) {
                    println!("[{:>7.1}s] {} -> {line}", at_ms as f64 / 1000.0, self.name);
                    if let Some(label) = event.get("label") {
                        self.seen.insert(label.to_owned(), event);
                    }
                }
            }
        }
    }

    pub fn status(&self) -> String {
        std::fs::read_to_string(self.dir.join("master.status")).unwrap_or_else(|_| String::new())
    }
}

/// `key=value` from a master status file.
pub fn field<'a>(status: &'a str, key: &str) -> Option<&'a str> {
    status
        .lines()
        .filter_map(|line| line.split_once('='))
        .find_map(|(k, v)| (k == key).then_some(v))
}

/// Resident memory of a process now, in KiB (Linux; 0 elsewhere).
pub fn rss_kib(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("VmRSS:"))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|kib| kib.parse().ok())
        })
        .unwrap_or(0)
}

/// CPU time a process has used so far, in seconds (Linux; 0 elsewhere):
/// user and system clock ticks from `/proc/<pid>/stat`, at the kernel's
/// usual 100 a second.
pub fn cpu_secs(pid: u32) -> f64 {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            // Fields after the parenthesised command, which may hold spaces.
            let rest = stat.rsplit_once(')')?.1;
            let fields: Vec<&str> = rest.split_whitespace().collect();
            let utime: u64 = fields.get(11)?.parse().ok()?;
            let stime: u64 = fields.get(12)?.parse().ok()?;
            Some((utime + stime) as f64 / 100.0)
        })
        .unwrap_or(0.0)
}

/// Both games, sampled for memory as they are pumped.
pub struct Pair {
    pub host: Player,
    pub client: Option<Player>,
    /// The most both held at once, in KiB.
    pub peak_combined_kib: u64,
    /// The most the client held, in KiB, kept after it exits.
    pub peak_client_kib: u64,
}

impl Pair {
    /// Pumps both games and samples their memory until `done` holds or
    /// `timeout` passes; the error names what was awaited.
    pub fn until(
        &mut self,
        what: &str,
        timeout: Duration,
        mut done: impl FnMut(&mut Pair) -> bool,
    ) -> Result<(), String> {
        let start = Instant::now();
        loop {
            self.sample();
            if done(self) {
                return Ok(());
            }
            if let Some(status) = self.host.exited() {
                return Err(format!("host exited ({status}) waiting for {what}"));
            }
            if let Some(client) = self.client.as_mut()
                && let Some(status) = client.exited()
            {
                return Err(format!("client exited ({status}) waiting for {what}"));
            }
            if start.elapsed() > timeout {
                return Err(format!(
                    "timed out after {}s waiting for {what}",
                    timeout.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn sample(&mut self) {
        self.host.pump();
        let mut combined = rss_kib(self.host.pid());
        self.host.peak_rss_kib = self.host.peak_rss_kib.max(combined);
        if let Some(client) = self.client.as_mut() {
            client.pump();
            let rss = rss_kib(client.pid());
            client.peak_rss_kib = client.peak_rss_kib.max(rss);
            self.peak_client_kib = self.peak_client_kib.max(rss);
            combined += rss;
        }
        self.peak_combined_kib = self.peak_combined_kib.max(combined);
    }

    /// Asks `who` for a labelled line (`script` prints it) and waits for it.
    pub fn ask(
        &mut self,
        client: bool,
        script: &str,
        label: &str,
        timeout: Duration,
    ) -> Result<Event, String> {
        let player = if client {
            self.client.as_mut().ok_or("no client")?
        } else {
            &mut self.host
        };
        player.seen.remove(label);
        player.send(script)?;
        let mut found = None;
        self.until(&format!("`{label}`"), timeout, |pair| {
            let player = if client {
                pair.client.as_mut()
            } else {
                Some(&mut pair.host)
            };
            found = player.and_then(|p| p.seen.get(label).cloned());
            found.is_some()
        })?;
        Ok(found.expect("found"))
    }

    /// Asks for a block until it is `want` or `timeout` passes; the last
    /// answer either way.
    pub fn poll_block(
        &mut self,
        client: bool,
        label: &str,
        at: [i32; 3],
        want: &str,
        timeout: Duration,
    ) -> Result<String, String> {
        let start = Instant::now();
        loop {
            let script = format!("mc block {label} {} {} {}", at[0], at[1], at[2]);
            let event = self.ask(client, &script, label, Duration::from_secs(30))?;
            let block = event.get("block").unwrap_or("").to_owned();
            if block == want || start.elapsed() > timeout {
                return Ok(block);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}
