//! Persistent artwork cache. Files hold the server's encoded image bytes, are
//! named by a hash of the server origin and artwork path (never the token),
//! and are trimmed oldest-first when the folder grows past its limit.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::SystemTime,
};

const LIMIT_BYTES: u64 = 512 * 1024 * 1024;
const TRIM_TO_BYTES: u64 = 384 * 1024 * 1024;

/// `%LOCALAPPDATA%\Perfuse\Artwork` on Windows, the XDG cache elsewhere.
/// Tests never touch the user's cache.
pub fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Perfuse"))
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .map(|path| path.join("perfuse"))
    };
    base.map(|path| path.join("Artwork"))
}

fn file(dir: &Path, origin: &str, path: &str) -> PathBuf {
    // FNV-1a keeps names stable across Rust releases, unlike std's hasher.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in origin.bytes().chain([0]).chain(path.bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    dir.join(format!("{hash:016x}.img"))
}

pub fn read(dir: &Path, origin: &str, path: &str) -> Option<Vec<u8>> {
    fs::read(file(dir, origin, path)).ok()
}

pub fn write(dir: &Path, origin: &str, path: &str, bytes: &[u8]) {
    let target = file(dir, origin, path);
    let partial = target.with_extension(format!("{}.part", std::process::id()));
    // Write then rename so a reader never sees half an image.
    let written = fs::create_dir_all(dir)
        .and_then(|()| fs::File::create(&partial))
        .and_then(|mut out| out.write_all(bytes))
        .and_then(|()| fs::rename(&partial, &target));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
}

/// Removes an entry that no longer decodes, so it is fetched again.
pub fn remove(dir: &Path, origin: &str, path: &str) {
    let _ = fs::remove_file(file(dir, origin, path));
}

/// Deletes the least recently written files once the cache exceeds its limit.
pub fn trim(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                (
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    metadata.len(),
                    entry.path(),
                )
            })
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    if total <= LIMIT_BYTES {
        return;
    }
    files.sort();
    for (_, size, path) in files {
        if total <= TRIM_TO_BYTES {
            break;
        }
        if fs::remove_file(path).is_ok() {
            total -= size;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_keyed_by_server_and_path() {
        let dir = std::env::temp_dir().join(format!("perfuse-cache-test-{}", std::process::id()));
        write(
            &dir,
            "http://a:32400/",
            "/library/metadata/1/thumb/9",
            b"poster",
        );
        assert_eq!(
            read(&dir, "http://a:32400/", "/library/metadata/1/thumb/9").as_deref(),
            Some(&b"poster"[..])
        );
        assert!(read(&dir, "http://b:32400/", "/library/metadata/1/thumb/9").is_none());
        assert!(read(&dir, "http://a:32400/", "/library/metadata/1/thumb/10").is_none());
        remove(&dir, "http://a:32400/", "/library/metadata/1/thumb/9");
        assert!(read(&dir, "http://a:32400/", "/library/metadata/1/thumb/9").is_none());
        trim(&dir);
        let _ = fs::remove_dir_all(&dir);
    }
}
