#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Cursor, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::Arc;

use rustls::pki_types::{CertificateDer, ServerName};
use updater::{
    GAME_NAME, MANIFEST_NAME, Manifest, ManifestFile, bytes_sha256, normalized_hash,
    require_file_name,
};

const EMBEDDED_CA_PEM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/iw4l-ca.pem"));
/// The update URL a build names for itself: this fork's release workflow
/// builds with its GitHub releases (`IW4L_DEFAULT_UPDATE_URL`), so its
/// players update from them with no `.env` setting. `IW4L_UPDATE_URL`
/// still wins.
const BUILT_IN_UPDATE_URL: Option<&str> = option_env!("IW4L_DEFAULT_UPDATE_URL");

/// `BUILT_IN_UPDATE_URL`, unless the build set it empty (a pull request's).
fn built_in_update_url() -> Option<&'static str> {
    BUILT_IN_UPDATE_URL.filter(|url| !url.trim().is_empty())
}
const MAX_DOWNLOAD_BYTES: usize = 512 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const SIDECAR_MASTER_ENV: [&str; 3] = [
    "IW4L_MASTER_ADDR",
    "IW4L_MASTER_SERVER_NAME",
    "IW4L_MASTER_CA_CERT",
];

struct HttpsTarget {
    host: String,
    port: u16,
    path: String,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[iw4launcher] {error}");
            #[cfg(windows)]
            {
                eprintln!("[iw4launcher] press Enter to exit");
                let _ = std::io::stdin().read_line(&mut String::new());
            }
            ExitCode::FAILURE
        }
    }
}

/// A release archive ships `iw4l.exe`, so the folder runs offline. Upstream's
/// builds fetch an update only when asked for: `iw4launcher update`, or
/// `IW4L_UPDATE=1` for a shortcut that cannot pass an argument. A build that
/// names its own update URL (this fork's releases) checks on every start
/// unless `IW4L_UPDATE=0`, and plays the build it has when the check fails.
fn run() -> Result<(), String> {
    let install_dir = install_dir()?;
    let from_sidecar = load_sidecar_env(&install_dir)?;
    println!("[iw4launcher] install {}", install_dir.display());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let asked = args.iter().any(|arg| arg == "update" || arg == "--update");
    let update_env = std::env::var("IW4L_UPDATE").ok();
    let wanted = updater::update_wanted(
        &args,
        update_env.as_deref(),
        built_in_update_url().is_some(),
    );

    let release_env = if wanted {
        match update(&install_dir) {
            Ok(env) => env,
            // Offline, or GitHub unreachable: the build already here plays.
            Err(error)
                if !asked && update_env.is_none() && install_dir.join(GAME_NAME).is_file() =>
            {
                eprintln!(
                    "[iw4launcher] update check failed ({error}); starting the {GAME_NAME} already here"
                );
                BTreeMap::new()
            }
            Err(error) => return Err(error),
        }
    } else {
        if !install_dir.join(GAME_NAME).is_file() {
            return Err(format!(
                "{GAME_NAME} is not in {}. A release archive carries it; to fetch \
                 one instead, run `iw4launcher update` with IW4L_UPDATE_URL set in \
                 the portable .env",
                install_dir.display()
            ));
        }
        println!(
            "[iw4launcher] starting the {GAME_NAME} already here (pass `update` to fetch a new one)"
        );
        BTreeMap::new()
    };
    start_game(
        &install_dir,
        &release_env,
        &updater::game_args(&args),
        &from_sidecar,
    )
}

/// Whether this launcher trusts only the CA it was built with (upstream's
/// bundles, `make launcher windows`), rather than the public web PKI.
fn own_ca() -> bool {
    embedded_ca_certs().is_ok_and(|certs| !certs.is_empty())
}

/// Fetch the manifest, install what it names, and hand back its env defaults.
fn update(install_dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let update_base = update_base()?;
    println!("[iw4launcher] channel {update_base}");

    let manifest_bytes = https_get(&format!("{update_base}/{MANIFEST_NAME}"))?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("manifest parse: {error}"))?;
    manifest.validate(own_ca())?;

    for entry in &manifest.files {
        install_entry(&update_base, install_dir, entry)?;
    }
    install_default_sidecar(install_dir, &manifest.env)?;
    println!(
        "[iw4launcher] updated to channel={} protocol={} git={}{}",
        manifest.channel,
        manifest.protocol,
        manifest.git,
        manifest
            .build
            .map(|build| format!(" build={build}"))
            .unwrap_or_default()
    );
    Ok(manifest.env)
}

fn start_game(
    install_dir: &Path,
    release_env: &BTreeMap<String, String>,
    args: &[String],
    from_sidecar: &[String],
) -> Result<(), String> {
    println!("[iw4launcher] starting {GAME_NAME} {}", args.join(" "));
    let mut game = Command::new(install_dir.join(GAME_NAME));
    game.current_dir(install_dir).args(args);
    if own_ca() {
        if std::env::var_os("IW4L_GAMES").is_none() {
            game.env("IW4L_GAMES", install_dir);
        }
    } else {
        // The game reads `.env` itself, and its first launch may rewrite
        // it: what this launcher took from it is not passed on stale.
        for name in from_sidecar {
            game.env_remove(name);
        }
    }
    for (name, value) in release_env {
        if std::env::var_os(name).is_none() {
            game.env(name, value);
        }
    }
    // The game hands a start to this launcher unless it came from it.
    game.env("IW4L_LAUNCHER", "1");
    game.spawn()
        .map_err(|error| format!("spawn {GAME_NAME}: {error}"))?;
    Ok(())
}

/// Loads `.env` beside the launcher without overriding what is set, and
/// returns the names it set.
fn load_sidecar_env(install_dir: &Path) -> Result<Vec<String>, String> {
    let path = install_dir.join(".env");
    if !path.exists() {
        return Ok(Vec::new());
    }
    if !path.is_file() {
        return Err(format!("{} exists but is not a file", path.display()));
    }
    let mut unset = Vec::new();
    for item in dotenvy::from_path_iter(&path)
        .map_err(|error| format!("load {}: {error}", path.display()))?
    {
        let (name, _) = item.map_err(|error| format!("load {}: {error}", path.display()))?;
        if std::env::var_os(&name).is_none() {
            unset.push(name);
        }
    }
    dotenvy::from_path(&path)
        .map(|_| unset)
        .map_err(|error| format!("load {}: {error}", path.display()))
}

fn install_default_sidecar(
    install_dir: &Path,
    release_env: &BTreeMap<String, String>,
) -> Result<(), String> {
    // A release with no settings (a GitHub release) leaves `.env` to the
    // game's first launch.
    if release_env.is_empty() {
        return Ok(());
    }
    let path = install_dir.join(".env");
    if path.is_file() {
        return Ok(());
    }
    if path.exists() {
        return Err(format!("{} exists but is not a file", path.display()));
    }
    let mut contents = String::from(
        "# iw4l portable runtime configuration; local values override release defaults.\n",
    );
    for name in SIDECAR_MASTER_ENV {
        let value = release_env
            .get(name)
            .ok_or_else(|| format!("manifest has no {name}"))?;
        contents.push_str(name);
        contents.push('=');
        contents.push_str(value);
        contents.push('\n');
    }
    if let Ok(url) = std::env::var("IW4L_UPDATE_URL")
        && !url.trim().is_empty()
        && !url.contains(['\r', '\n', '\0'])
    {
        contents.push_str("IW4L_UPDATE_URL=");
        contents.push_str(url.trim());
        contents.push('\n');
    }
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(mut file) => file
            .write_all(contents.as_bytes())
            .map_err(|error| format!("write {}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(format!("create {}: {error}", path.display())),
    }
}

fn update_base() -> Result<String, String> {
    parse_update_url(
        std::env::var("IW4L_UPDATE_URL")
            .ok()
            .filter(|url| !url.trim().is_empty())
            .or_else(|| built_in_update_url().map(str::to_owned)),
    )
}

fn parse_update_url(raw: Option<String>) -> Result<String, String> {
    let raw = raw.ok_or_else(|| {
        "IW4L_UPDATE_URL is unset; set it in the portable .env before the first HTTP request"
            .to_owned()
    })?;
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("IW4L_UPDATE_URL is empty".to_owned());
    }
    if trimmed.contains(['\r', '\n', '\0']) {
        return Err("IW4L_UPDATE_URL contains invalid characters".to_owned());
    }
    Ok(trimmed.to_owned())
}

fn install_entry(base: &str, dir: &Path, entry: &ManifestFile) -> Result<(), String> {
    require_file_name(&entry.path)?;
    let destination = dir.join(&entry.path);
    let expected = normalized_hash(&entry.sha256)?;
    if destination.is_file()
        && fs::metadata(&destination).map(|meta| meta.len()).ok() == Some(entry.size)
        && file_sha256(&destination)? == expected
    {
        println!("[iw4launcher] {} up to date", entry.path);
        return Ok(());
    }
    let remote = require_file_name(entry.download.as_deref().unwrap_or(&entry.path))?;
    println!("[iw4launcher] downloading {remote}");
    let blob = https_get(&format!("{base}/{remote}"))?;
    if let Some(size) = entry.download_size
        && blob.len() as u64 != size
    {
        return Err(format!("{remote} size {} != {size}", blob.len()));
    }
    if let Some(hash) = &entry.download_sha256
        && bytes_sha256(&blob) != normalized_hash(hash)?
    {
        return Err(format!("{remote} download sha256 mismatch"));
    }
    let bytes = if remote.ends_with(".zst") {
        zstd::decode_all(blob.as_slice()).map_err(|error| format!("decode {remote}: {error}"))?
    } else {
        blob
    };
    if bytes.len() as u64 != entry.size {
        return Err(format!(
            "{} size {} != {}",
            entry.path,
            bytes.len(),
            entry.size
        ));
    }
    if bytes_sha256(&bytes) != expected {
        return Err(format!("{} sha256 mismatch", entry.path));
    }
    atomic_install(&destination, &bytes)?;
    println!("[iw4launcher] installed {}", entry.path);
    Ok(())
}

fn embedded_ca_certs() -> Result<Vec<CertificateDer<'static>>, String> {
    let mut reader = BufReader::new(Cursor::new(EMBEDDED_CA_PEM));
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<_, _>>()
        .map_err(|error| format!("embedded CA parse: {error}"))
}

/// The roots TLS trusts: the CA this launcher was built with, or, built
/// without one, the public web PKI's (GitHub's releases).
fn root_store() -> Result<rustls::RootCertStore, String> {
    let mut roots = rustls::RootCertStore::empty();
    let certs = embedded_ca_certs()?;
    if certs.is_empty() {
        let (added, _) = roots
            .add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
        if added == 0 {
            return Err("no usable public root certificates".to_owned());
        }
        return Ok(roots);
    }
    for cert in certs {
        roots
            .add(cert)
            .map_err(|error| format!("embedded CA: {error}"))?;
    }
    Ok(roots)
}

/// A GET, following redirects only within GitHub (a release download
/// redirects to its asset host).
fn https_get(url: &str) -> Result<Vec<u8>, String> {
    let mut url = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        match https_get_once(&url)? {
            Response::Body(body) => return Ok(body),
            Response::Redirect(location) => {
                let from = parse_https_url(&url)?.host;
                let to = parse_https_url(&location)?.host;
                if !updater::github_redirect_allowed(&from, &to) {
                    return Err(format!("GET {url}: redirect to {to} refused"));
                }
                url = location;
            }
        }
    }
    Err(format!("GET {url}: too many redirects"))
}

enum Response {
    Body(Vec<u8>),
    Redirect(String),
}

fn https_get_once(url: &str) -> Result<Response, String> {
    let target = parse_https_url(url)?;
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store()?)
        .with_no_client_auth();
    let server_name = ServerName::try_from(target.host.clone())
        .map_err(|_| format!("invalid TLS server name `{}`", target.host))?;
    let connection = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|error| format!("TLS init: {error}"))?;
    let socket = TcpStream::connect((target.host.as_str(), target.port))
        .map_err(|error| format!("connect {}:{}: {error}", target.host, target.port))?;
    let mut stream = rustls::StreamOwned::new(connection, socket);
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: iw4launcher/0.1\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        target.path, target.host
    )
    .map_err(|error| format!("GET {url}: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("GET {url}: {error}"))?;
    let mut response = Vec::new();
    // A server may close without TLS close_notify once the body is sent: the
    // body is then only taken when its Content-Length says it is whole.
    let unterminated = match stream
        .take((MAX_DOWNLOAD_BYTES + 64 * 1024 + 1) as u64)
        .read_to_end(&mut response)
    {
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => true,
        Err(error) => return Err(format!("GET {url}: {error}")),
    };
    if response.len() > MAX_DOWNLOAD_BYTES + 64 * 1024 {
        return Err(format!("GET {url}: response exceeds limit"));
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| format!("GET {url}: malformed HTTP response"))?;
    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|_| format!("GET {url}: non-UTF8 HTTP headers"))?;
    let status = headers
        .lines()
        .next()
        .ok_or_else(|| format!("GET {url}: missing HTTP status"))?;
    let header = |name: &str| {
        headers.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
    };
    let code = status.split_whitespace().nth(1).unwrap_or("");
    if matches!(code, "301" | "302" | "303" | "307" | "308") {
        let location =
            header("location").ok_or_else(|| format!("GET {url}: {status} with no Location"))?;
        return Ok(Response::Redirect(location));
    }
    if !status.starts_with("HTTP/1.1 2") && !status.starts_with("HTTP/1.0 2") {
        return Err(format!("GET {url}: {status}"));
    }
    if header("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        return Err(format!("GET {url}: chunked responses are unsupported"));
    }
    let content_length = header("content-length").and_then(|value| value.parse::<usize>().ok());
    if unterminated && content_length.is_none() {
        return Err(format!("GET {url}: connection closed early"));
    }
    let body = response.split_off(header_end + 4);
    if let Some(length) = content_length
        && body.len() != length
    {
        return Err(format!("GET {url}: body length {} != {length}", body.len()));
    }
    Ok(Response::Body(body))
}

fn parse_https_url(url: &str) -> Result<HttpsTarget, String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| format!("update URL must use https: `{url}`"))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority
        .rsplit_once(':')
        .map_or((authority, 443), |(host, port)| {
            (host, port.parse::<u16>().unwrap_or(0))
        });
    if host.is_empty() || port == 0 {
        return Err(format!("invalid update URL `{url}`"));
    }
    Ok(HttpsTarget {
        host: host.to_owned(),
        port,
        path: format!("/{path}"),
    })
}

fn install_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| "cannot determine updater directory".to_owned())
}

fn file_sha256(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Replaces `destination`, keeping the old file until the new one is in
/// place. A running executable (the game that handed its start over, or
/// this launcher) can be renamed on Windows but not removed: its backup is
/// removed by a later update.
fn atomic_install(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let part = destination.with_extension("part");
    let backup = destination.with_extension("bak");
    {
        let mut file =
            File::create(&part).map_err(|error| format!("create {}: {error}", part.display()))?;
        file.write_all(bytes)
            .map_err(|error| format!("write {}: {error}", part.display()))?;
        file.sync_all()
            .map_err(|error| format!("sync {}: {error}", part.display()))?;
    }
    if destination.exists() {
        let _ = fs::remove_file(&backup);
        rename_retrying(destination, &backup)
            .map_err(|error| format!("rename {} to backup: {error}", destination.display()))?;
    }
    if let Err(error) = fs::rename(&part, destination) {
        if backup.exists() {
            let _ = fs::rename(&backup, destination);
        }
        return Err(format!("install {}: {error}", destination.display()));
    }
    let _ = fs::remove_file(backup);
    Ok(())
}

/// A rename, tried for a few seconds: the game that started this launcher
/// may still be closing.
fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut last = None;
    for _ in 0..20 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("rename not tried")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launcher_built_without_a_ca_trusts_the_public_web_pki() {
        // Tests build without IW4L_UPDATER_CA_CERT, as the fork's releases do.
        assert!(!own_ca());
        assert!(!root_store().unwrap().is_empty());
    }

    #[test]
    fn urls_parse_as_the_launcher_fetches_them() {
        let target = parse_https_url(
            "https://github.com/NoahSurprenant/2010-rust-rewrite-mashup/releases/latest/download/manifest.json",
        )
        .unwrap();
        assert_eq!(target.host, "github.com");
        assert_eq!(target.port, 443);
        assert_eq!(
            target.path,
            "/NoahSurprenant/2010-rust-rewrite-mashup/releases/latest/download/manifest.json"
        );
        assert!(parse_https_url("http://github.com/x").is_err());
        assert_eq!(
            parse_update_url(Some(" https://example.test/prod/ ".into())).unwrap(),
            "https://example.test/prod"
        );
    }

    /// A release asset fetched from GitHub through its redirect to the asset
    /// host (`IW4L_GITHUB_ASSET_URL`, needs the network).
    #[test]
    #[ignore]
    fn fetches_a_github_release_asset() {
        let Ok(url) = std::env::var("IW4L_GITHUB_ASSET_URL") else {
            return;
        };
        let body = https_get(&url).unwrap();
        println!(
            "{url}: {} bytes, sha256 {}",
            body.len(),
            bytes_sha256(&body)
        );
        assert!(!body.is_empty());
    }
}
