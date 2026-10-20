//! The update manifest `iw4launcher` reads, the rules it holds a manifest
//! to, and how this fork's release workflow writes one for GitHub Releases
//! (`iw4l-update-manifest`).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

pub mod release;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const GAME_NAME: &str = "iw4l.exe";
pub const LAUNCHER_NAME: &str = "iw4launcher.exe";
pub const CA_NAME: &str = "iw4l-ca.pem";
/// The manifest's name beside the files it lists.
pub const MANIFEST_NAME: &str = "manifest.json";
/// The channel a GitHub release's manifest names.
pub const GITHUB_CHANNEL: &str = "github";

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct Manifest {
    pub channel: String,
    pub protocol: u16,
    pub git: String,
    /// The release's build number, when it has one (a GitHub release's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<u64>,
    pub env: BTreeMap<String, String>,
    pub files: Vec<ManifestFile>,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct ManifestFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_size: Option<u64>,
}

impl Manifest {
    /// What every manifest must hold before anything it names is installed:
    /// an identity, the game, and (for a launcher that trusts its own CA)
    /// that CA; file names only; well-formed hashes; `IW4L_` settings with
    /// one-line values.
    pub fn validate(&self, needs_ca: bool) -> Result<(), String> {
        if self.channel.trim().is_empty() || self.git.trim().is_empty() || self.protocol == 0 {
            return Err("manifest identity is incomplete".to_owned());
        }
        let required: &[&str] = if needs_ca {
            &[GAME_NAME, CA_NAME]
        } else {
            &[GAME_NAME]
        };
        for required in required {
            if !self.files.iter().any(|entry| entry.path == *required) {
                return Err(format!("manifest has no {required}"));
            }
        }
        for entry in &self.files {
            require_file_name(&entry.path)?;
            if let Some(download) = &entry.download {
                require_file_name(download)?;
            }
            normalized_hash(&entry.sha256)?;
            if let Some(hash) = &entry.download_sha256 {
                normalized_hash(hash)?;
            }
        }
        for (name, value) in &self.env {
            if !name.starts_with("IW4L_") || name.contains('=') || name.contains('\0') {
                return Err(format!("manifest contains invalid env name `{name}`"));
            }
            if value.contains(['\r', '\n', '\0']) {
                return Err(format!("manifest contains invalid env value for `{name}`"));
            }
        }
        Ok(())
    }
}

/// One file of a release as the manifest lists it: installed as `path`,
/// fetched as `download` (zstd) when it is compressed.
pub struct ReleaseFile<'a> {
    pub path: &'a str,
    pub bytes: &'a [u8],
    pub download: Option<(&'a str, &'a [u8])>,
}

impl ReleaseFile<'_> {
    fn entry(&self) -> ManifestFile {
        ManifestFile {
            path: self.path.to_owned(),
            sha256: bytes_sha256(self.bytes),
            size: self.bytes.len() as u64,
            download: self.download.map(|(name, _)| name.to_owned()),
            download_sha256: self.download.map(|(_, blob)| bytes_sha256(blob)),
            download_size: self.download.map(|(_, blob)| blob.len() as u64),
        }
    }
}

/// A GitHub release's manifest: no master settings (the game's first launch
/// writes `.env`) and no CA (GitHub is reached over the public web PKI);
/// each file by its SHA-256, which the launcher checks before installing.
pub fn github_manifest(git: &str, build: u64, files: &[ReleaseFile<'_>]) -> Manifest {
    Manifest {
        channel: GITHUB_CHANNEL.to_owned(),
        protocol: master_protocol::PROTOCOL_VERSION,
        git: git.to_owned(),
        build: Some(build),
        env: BTreeMap::new(),
        files: files.iter().map(ReleaseFile::entry).collect(),
    }
}

/// Where a GitHub repository's newest release's assets are:
/// `https://github.com/<owner>/<repo>/releases/latest/download`.
pub fn github_latest_download(repository: &str) -> Result<String, String> {
    let mut parts = repository.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(format!("not owner/repo: `{repository}`"));
    };
    let fine = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    };
    if !fine(owner) || !fine(repo) {
        return Err(format!("not owner/repo: `{repository}`"));
    }
    Ok(format!(
        "https://github.com/{owner}/{repo}/releases/latest/download"
    ))
}

/// Whether a redirect from `from` to `to` (hosts) may be followed: only
/// within GitHub, whose release downloads redirect to its asset hosts.
pub fn github_redirect_allowed(from: &str, to: &str) -> bool {
    let github = |host: &str| {
        let host = host.to_ascii_lowercase();
        host == "github.com" || host.ends_with(".githubusercontent.com")
    };
    github(from) && github(to)
}

/// Whether to fetch an update before starting the game: asked for
/// (`update`, `IW4L_UPDATE=1`), or by default (`updates_by_default`), unless
/// `IW4L_UPDATE` says no.
pub fn update_wanted(args: &[String], update_env: Option<&str>, by_default: bool) -> bool {
    if args.iter().any(|arg| arg == "update" || arg == "--update") {
        return true;
    }
    match update_env {
        Some(value) => !switched_off(value),
        None => by_default,
    }
}

/// A build that names its own update URL (this fork's releases) checks on
/// every start, and so does any build started with `IW4L_UPDATE_URL` in its
/// environment (not from `.env`, where upstream's bundles keep theirs).
pub fn updates_by_default(built_in_url: bool, environment_url: Option<&str>) -> bool {
    built_in_url || environment_url.is_some_and(|url| !url.trim().is_empty())
}

/// Whether `iw4l.exe` hands its start to `iw4launcher.exe`: it updates by
/// default, the launcher did not start it, and `IW4L_UPDATE` does not say no.
pub fn hands_off(by_default: bool, from_launcher: bool, update_env: Option<&str>) -> bool {
    by_default && !from_launcher && !update_env.is_some_and(switched_off)
}

fn switched_off(value: &str) -> bool {
    let value = value.trim();
    value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false")
}

/// The folder a `file://` update URL names (`file:///D:/updates`,
/// `file:///srv/updates`), read with the same checks as a download; `None`
/// for any other URL.
pub fn local_source(url: &str) -> Result<Option<PathBuf>, String> {
    let Some(rest) = url.strip_prefix("file://") else {
        return Ok(None);
    };
    let Some(after_root) = rest.strip_prefix('/') else {
        return Err(format!("file URL must be absolute (file:///...): `{url}`"));
    };
    if rest.contains('%') {
        return Err(format!(
            "percent-encoded file URLs are unsupported: `{url}`"
        ));
    }
    let drive = after_root.as_bytes();
    let path = if drive.len() >= 3
        && drive[0].is_ascii_alphabetic()
        && drive[1] == b':'
        && matches!(drive[2], b'/' | b'\\')
    {
        after_root
    } else {
        rest
    };
    Ok(Some(PathBuf::from(path)))
}

/// The launcher's arguments the game gets: all but `update`, or `menu`.
pub fn game_args(args: &[String]) -> Vec<String> {
    let rest: Vec<String> = args
        .iter()
        .filter(|arg| *arg != "update" && *arg != "--update")
        .cloned()
        .collect();
    if rest.is_empty() {
        vec!["menu".to_owned()]
    } else {
        rest
    }
}

pub fn require_file_name(name: &str) -> Result<&str, String> {
    if std::path::Path::new(name)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        == Some(name)
    {
        Ok(name)
    } else {
        Err(format!("manifest name must be a file name: `{name}`"))
    }
}

pub fn normalized_hash(hash: &str) -> Result<String, String> {
    let hash = hash.trim().to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid sha256 `{hash}`"));
    }
    Ok(hash)
}

pub fn bytes_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn a_github_manifest_lists_each_file_by_its_hash() {
        let game = b"game bytes".as_slice();
        let blob = b"zstd bytes".as_slice();
        let launcher = b"launcher bytes".as_slice();
        let manifest = github_manifest(
            "0123abcd",
            42,
            &[
                ReleaseFile {
                    path: GAME_NAME,
                    bytes: game,
                    download: Some(("iw4l.exe.zst", blob)),
                },
                ReleaseFile {
                    path: LAUNCHER_NAME,
                    bytes: launcher,
                    download: None,
                },
            ],
        );
        manifest.validate(false).unwrap();
        // A launcher with its own CA wants that CA too.
        assert_eq!(
            manifest.validate(true),
            Err("manifest has no iw4l-ca.pem".to_owned())
        );
        assert_eq!(manifest.channel, "github");
        assert_eq!(manifest.protocol, master_protocol::PROTOCOL_VERSION);
        assert_eq!(manifest.build, Some(42));
        assert!(manifest.env.is_empty());
        let game_entry = &manifest.files[0];
        assert_eq!(game_entry.sha256, bytes_sha256(game));
        assert_eq!(game_entry.size, game.len() as u64);
        assert_eq!(game_entry.download.as_deref(), Some("iw4l.exe.zst"));
        assert_eq!(
            game_entry.download_sha256.as_deref(),
            Some(bytes_sha256(blob).as_str())
        );
        assert_eq!(game_entry.download_size, Some(blob.len() as u64));
        let launcher_entry = &manifest.files[1];
        assert_eq!(launcher_entry.download, None);
        // What the launcher reads back is what was written.
        let text = serde_json::to_string_pretty(&manifest).unwrap();
        let read: Manifest = serde_json::from_str(&text).unwrap();
        assert_eq!(read, manifest);
        assert!(!text.contains("download_sha256\": null"));
    }

    #[test]
    fn an_upstream_manifest_still_reads() {
        let text = r#"{"id":"abc","channel":"prod","protocol":10,"git":"deadbeef",
            "env":{"IW4L_MASTER_ADDR":"host:4433","IW4L_MASTER_SERVER_NAME":"iw4l-prod","IW4L_MASTER_CA_CERT":"iw4l-ca.pem"},
            "files":[{"path":"iw4l.exe","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":3,
                      "download":"iw4l-x.exe.zst","download_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","download_size":2},
                     {"path":"iw4l-ca.pem","sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","size":1}]}"#;
        let manifest: Manifest = serde_json::from_str(text).unwrap();
        manifest.validate(true).unwrap();
        assert_eq!(manifest.build, None);
    }

    #[test]
    fn a_manifest_that_names_a_path_or_a_bad_hash_is_refused() {
        let file = |path: &str, sha256: &str| ManifestFile {
            path: path.to_owned(),
            sha256: sha256.to_owned(),
            size: 1,
            download: None,
            download_sha256: None,
            download_size: None,
        };
        let good = "a".repeat(64);
        let mut manifest = github_manifest("abc", 1, &[]);
        manifest.files = vec![file(GAME_NAME, &good), file("../evil.exe", &good)];
        assert!(manifest.validate(false).unwrap_err().contains("file name"));
        manifest.files = vec![file(GAME_NAME, "nothex")];
        assert!(
            manifest
                .validate(false)
                .unwrap_err()
                .contains("invalid sha256")
        );
        manifest.files = vec![file(GAME_NAME, &good)];
        manifest.env.insert("PATH".into(), "x".into());
        assert!(manifest.validate(false).unwrap_err().contains("env name"));
        manifest.env.clear();
        manifest.git = " ".into();
        assert!(manifest.validate(false).unwrap_err().contains("identity"));
    }

    #[test]
    fn the_update_url_is_the_repository_s_latest_release() {
        assert_eq!(
            github_latest_download("NoahSurprenant/2010-rust-rewrite-mashup").unwrap(),
            "https://github.com/NoahSurprenant/2010-rust-rewrite-mashup/releases/latest/download"
        );
        assert!(github_latest_download("a/b/c").is_err());
        assert!(github_latest_download("a b/c").is_err());
        assert!(github_latest_download("/c").is_err());
    }

    #[test]
    fn redirects_stay_within_github() {
        assert!(github_redirect_allowed(
            "github.com",
            "release-assets.githubusercontent.com"
        ));
        assert!(github_redirect_allowed(
            "github.com",
            "objects.githubusercontent.com"
        ));
        assert!(github_redirect_allowed("GitHub.com", "github.com"));
        assert!(!github_redirect_allowed("github.com", "evil.example"));
        assert!(!github_redirect_allowed(
            "github.com",
            "githubusercontent.com.evil.example"
        ));
        assert!(!github_redirect_allowed("updates.example", "github.com"));
    }

    #[test]
    fn a_release_build_updates_unless_told_not_to() {
        assert!(update_wanted(&[], None, true));
        assert!(!update_wanted(&[], Some("0"), true));
        assert!(!update_wanted(&[], Some("false"), true));
        assert!(update_wanted(&args(&["map", "x"]), Some("1"), false));
        // Upstream's builds: only when asked.
        assert!(!update_wanted(&[], None, false));
        assert!(update_wanted(&args(&["update"]), None, false));
    }

    #[test]
    fn an_update_url_in_the_environment_updates_a_build_without_one() {
        assert!(updates_by_default(true, None));
        assert!(updates_by_default(false, Some("file:///D:/update-test")));
        assert!(!updates_by_default(false, Some(" ")));
        assert!(!updates_by_default(false, None));
        assert!(update_wanted(
            &[],
            None,
            updates_by_default(false, Some("https://x.test"))
        ));
    }

    #[test]
    fn the_game_hands_off_unless_the_launcher_started_it_or_updates_are_off() {
        assert!(hands_off(true, false, None));
        assert!(hands_off(true, false, Some("1")));
        assert!(!hands_off(true, true, None));
        assert!(!hands_off(true, false, Some("0")));
        assert!(!hands_off(true, false, Some(" FALSE ")));
        assert!(!hands_off(true, false, Some("")));
        assert!(!hands_off(false, false, Some("1")));
    }

    #[test]
    fn a_file_url_names_a_local_folder() {
        assert_eq!(
            local_source("file:///D:/a/repo/update-test").unwrap(),
            Some(PathBuf::from("D:/a/repo/update-test"))
        );
        assert_eq!(
            local_source(r"file:///c:\updates").unwrap(),
            Some(PathBuf::from(r"c:\updates"))
        );
        assert_eq!(
            local_source("file:///srv/updates/manifest.json").unwrap(),
            Some(PathBuf::from("/srv/updates/manifest.json"))
        );
        assert_eq!(local_source("https://github.com/x").unwrap(), None);
        assert!(local_source("file://host/share").is_err());
        assert!(local_source("file:///D:/with%20space").is_err());
    }

    #[test]
    fn the_game_gets_the_launcher_s_arguments_or_the_menu() {
        assert_eq!(game_args(&[]), args(&["menu"]));
        assert_eq!(game_args(&args(&["update"])), args(&["menu"]));
        assert_eq!(
            game_args(&args(&["map", "minecraft:overworld"])),
            args(&["map", "minecraft:overworld"])
        );
    }
}
