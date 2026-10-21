//! Reading a game file the process parses once and keeps in its own memory.

use std::io::Read;
use std::path::Path;

/// The whole file, as `std::fs::read`, and then (on Linux) the kernel told
/// its cached pages are not needed again: a zone is read once and kept
/// parsed, so its file in the page cache would only be counted twice
/// against the machine's (or a container's) memory.
pub fn read_once(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata().map(|m| m.len() as usize).unwrap_or(0);
    let mut bytes = Vec::with_capacity(size);
    file.read_to_end(&mut bytes)?;
    forget_cached(&file);
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn forget_cached(file: &std::fs::File) {
    use std::os::fd::AsRawFd;
    // SAFETY: an advisory call on a descriptor this function owns.
    unsafe {
        libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

#[cfg(not(target_os = "linux"))]
fn forget_cached(_: &std::fs::File) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_whole_file() {
        let dir = std::env::temp_dir().join(format!("iw4l-read-once-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("zone.bin");
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        assert_eq!(read_once(&path).unwrap(), data);
        assert_eq!(
            read_once(&path).unwrap(),
            data,
            "read again after the advice"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_an_error() {
        assert!(read_once(Path::new("/nonexistent/iw4l/zone.ff")).is_err());
    }
}
