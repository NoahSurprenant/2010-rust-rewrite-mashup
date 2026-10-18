//! A GitHub release's update: the manifest written from the files in a
//! folder, and the check that a folder holds what a manifest names.
use std::path::Path;

use crate::{
    GAME_NAME, LAUNCHER_NAME, Manifest, ReleaseFile, bytes_sha256, github_manifest, normalized_hash,
};

/// The zstd-packed game a release publishes, which the launcher downloads.
pub const GAME_BLOB: &str = "iw4l.exe.zst";

fn read(path: &Path) -> Result<Vec<u8>, String> {
    std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))
}

fn zstd_decoded(blob: &[u8]) -> Result<Vec<u8>, String> {
    zstd::decode_all(blob).map_err(|e| format!("decode {GAME_BLOB}: {e}"))
}

/// Writes the manifest for `iw4l.exe`, `iw4l.exe.zst` and `iw4launcher.exe`
/// in `dir` to `out`.
pub fn write(dir: &Path, git: &str, build: u64, out: &Path) -> Result<Manifest, String> {
    let game = read(&dir.join(GAME_NAME))?;
    let blob = read(&dir.join(GAME_BLOB))?;
    let launcher = read(&dir.join(LAUNCHER_NAME))?;
    if zstd_decoded(&blob)? != game {
        return Err(format!("{GAME_BLOB} does not decode to {GAME_NAME}"));
    }
    let manifest = github_manifest(
        git,
        build,
        &[
            ReleaseFile {
                path: GAME_NAME,
                bytes: &game,
                download: Some((GAME_BLOB, &blob)),
            },
            ReleaseFile {
                path: LAUNCHER_NAME,
                bytes: &launcher,
                download: None,
            },
        ],
    );
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(out, text + "\n").map_err(|e| format!("write {}: {e}", out.display()))?;
    Ok(manifest)
}

/// The manifest at `path` as the launcher holds it, and each file it names
/// (the download, when there is one) in `dir` with its hash and size, and
/// a download that decodes to the file.
pub fn check(dir: &Path, path: &Path) -> Result<Manifest, String> {
    let manifest: Manifest =
        serde_json::from_slice(&read(path)?).map_err(|e| format!("manifest parse: {e}"))?;
    manifest.validate(false)?;
    for entry in &manifest.files {
        let fetched = entry.download.as_deref().unwrap_or(&entry.path);
        let bytes = read(&dir.join(fetched))?;
        let (want_hash, want_size) = match (&entry.download_sha256, entry.download_size) {
            (Some(hash), Some(size)) if entry.download.is_some() => (hash.as_str(), size),
            _ => (entry.sha256.as_str(), entry.size),
        };
        if bytes.len() as u64 != want_size || bytes_sha256(&bytes) != normalized_hash(want_hash)? {
            return Err(format!("{fetched} does not match the manifest"));
        }
        if entry.download.is_some() {
            let installed = zstd_decoded(&bytes)?;
            if installed.len() as u64 != entry.size
                || bytes_sha256(&installed) != normalized_hash(&entry.sha256)?
            {
                return Err(format!("{fetched} does not decode to {}", entry.path));
            }
        }
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(test: &str, game: &[u8], blob_of: &[u8], launcher: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "iw4l-update-manifest-{}-{test}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(GAME_NAME), game).unwrap();
        std::fs::write(dir.join(GAME_BLOB), zstd::encode_all(blob_of, 3).unwrap()).unwrap();
        std::fs::write(dir.join(LAUNCHER_NAME), launcher).unwrap();
        dir
    }

    #[test]
    fn a_written_manifest_checks_and_names_every_file() {
        let dir = folder("written", b"the game", b"the game", b"the launcher");
        let out = dir.join("manifest.json");
        let written = write(&dir, "0123abcd", 37, &out).unwrap();
        let checked = check(&dir, &out).unwrap();
        assert_eq!(written, checked);
        assert_eq!(checked.build, Some(37));
        assert_eq!(checked.files.len(), 2);
        assert_eq!(checked.files[0].download.as_deref(), Some(GAME_BLOB));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_blob_of_another_game_is_refused() {
        let dir = folder("older", b"the game", b"an older game", b"the launcher");
        let out = dir.join("manifest.json");
        assert!(
            write(&dir, "0123abcd", 37, &out)
                .unwrap_err()
                .contains("does not decode")
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_changed_after_the_manifest_is_refused() {
        let dir = folder("changed", b"the game", b"the game", b"the launcher");
        let out = dir.join("manifest.json");
        write(&dir, "0123abcd", 37, &out).unwrap();
        std::fs::write(dir.join(LAUNCHER_NAME), b"another launcher").unwrap();
        assert!(
            check(&dir, &out)
                .unwrap_err()
                .contains("iw4launcher.exe does not match")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
