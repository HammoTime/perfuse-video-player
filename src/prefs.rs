//! Non-sensitive UI preferences (such as sidebar library ordering) and the random
//! per-install client identifier. Never stores tokens, credentials, or raw server URLs.

use perfuse::{
    media::{Library, LibraryKind},
    plugins::{Credentials, ServerKind},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    NameAsc,
    NameDesc,
    TypeThenName,
    Custom,
    /// Also used for sorts added by a newer Perfuse, so its preferences still load.
    /// Serde requires this fallback to be the last variant.
    #[default]
    #[serde(other)]
    Server,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryOrder {
    #[serde(default)]
    pub sort: LibrarySort,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub hidden: Vec<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub position: usize,
    #[serde(default)]
    pub custom_keys: Vec<String>,
}

// Unknown fields are ignored when loading, so a newer build's file still loads here.
#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    libraries: HashMap<String, LibraryOrder>,
    #[serde(default)]
    volume: Volume,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device_id: Option<String>,
    /// Top-level sections from a newer Perfuse, kept when this build saves.
    #[serde(flatten)]
    newer: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Volume {
    level: f64,
    muted: bool,
    #[serde(skip)]
    changed: bool,
}

impl Default for Volume {
    fn default() -> Self {
        Self {
            level: 100.0,
            muted: false,
            changed: false,
        }
    }
}

impl Volume {
    pub fn load() -> Self {
        file()
            .map(|path| read_store(&path).volume)
            .unwrap_or_default()
    }

    pub fn level(self) -> f64 {
        self.level.clamp(0.0, 100.0)
    }

    pub fn gain(self) -> f64 {
        if self.muted { 0.0 } else { self.level() }
    }

    pub fn set(&mut self, value: f64) -> bool {
        let value = value.clamp(0.0, 100.0);
        if value == self.gain() {
            return false;
        }
        self.muted = value == 0.0;
        if !self.muted {
            self.level = value;
        }
        self.changed = true;
        true
    }

    pub fn save(&mut self) {
        if self.changed {
            if let Some(path) = file() {
                self.save_to(&path);
            }
            self.changed = false;
        }
    }

    fn save_to(self, path: &Path) {
        update_store(path, |store| store.volume = self);
    }
}

/// `%LOCALAPPDATA%\Perfuse\preferences.json` on Windows, XDG config elsewhere.
/// Tests never read or write the user's preferences file.
fn file() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Perfuse"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|path| path.join("perfuse"))
    };
    base.map(|path| path.join("preferences.json"))
}

fn server_key(server: &str) -> String {
    // FNV-1a hash keeps keys stable across Rust versions without writing server URLs.
    let normalized = server.trim().trim_end_matches('/').to_ascii_lowercase();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in normalized.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Reads the store for display. A damaged file reads as defaults and stays in place until
/// the next locked update sets it aside.
fn read_store(path: &Path) -> Store {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Reads the store under the update lock. A damaged file is moved to a timestamped `.bad`
/// sibling rather than overwritten. `None` means the file could not be read or set aside,
/// so the update must leave it alone.
fn read_for_update(path: &Path) -> Option<Store> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Some(Store::default()),
        Err(_) => return None,
    };
    if let Ok(store) = serde_json::from_slice(&bytes) {
        return Some(store);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let name = path.file_name()?.to_string_lossy();
    fs::rename(path, path.with_file_name(format!("{name}.{stamp}.bad")))
        .ok()
        .map(|()| Store::default())
}

// An exclusive Windows handle serializes read-modify-write across app instances. The lock
// file is empty and Windows releases the handle on exit.
#[cfg(windows)]
fn lock_store(path: &Path) -> Option<fs::File> {
    use std::{
        os::windows::fs::OpenOptionsExt,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;
    let started = Instant::now();
    loop {
        match fs::File::options()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path)
        {
            Ok(file) => return Some(file),
            Err(error)
                if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32)
                    && started.elapsed() < Duration::from_secs(3) =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(not(windows))]
fn lock_store(_path: &Path) -> Option<()> {
    Some(())
}

fn load_from(path: &Path, server: &str) -> LibraryOrder {
    read_store(path)
        .libraries
        .get(&server_key(server))
        .cloned()
        .unwrap_or_default()
}

fn save_to(path: &Path, server: &str, order: &LibraryOrder) {
    update_store(path, |store| {
        store.libraries.insert(server_key(server), order.clone());
    });
}

fn update_store(path: &Path, update: impl FnOnce(&mut Store)) {
    let Some(dir) = path.parent() else {
        return;
    };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let Some(_lock) = lock_store(&path.with_extension("lock")) else {
        return;
    };
    let Some(mut store) = read_for_update(path) else {
        return;
    };
    update(&mut store);
    let Ok(bytes) = serde_json::to_vec_pretty(&store) else {
        return;
    };
    let partial = path.with_extension(format!("{}.part", std::process::id()));
    // Flush the new contents to disk before the rename replaces the previous file.
    let written = fs::File::create(&partial)
        .and_then(|mut out| out.write_all(&bytes).and_then(|()| out.sync_all()))
        .and_then(|()| fs::rename(&partial, path));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
}

/// A persistent random client identifier for this installation: `perfuse-` and 32 hex
/// digits, created once and kept in the preferences file.
pub fn device_id() -> String {
    match file() {
        Some(path) => device_id_in(&path),
        None => new_device_id().unwrap_or_else(fallback_device_id),
    }
}

fn device_id_in(path: &Path) -> String {
    let saved = |store: &Store| store.device_id.clone().filter(|id| valid_device_id(id));
    if let Some(id) = saved(&read_store(path)) {
        return id;
    }
    let Some(mut id) = new_device_id() else {
        return fallback_device_id();
    };
    update_store(path, |store| match saved(store) {
        // Another instance created the identifier first.
        Some(existing) => id = existing,
        None => store.device_id = Some(id.clone()),
    });
    id
}

fn valid_device_id(id: &str) -> bool {
    // Sent in request headers, so only short visible ASCII tokens are accepted.
    (1..=200).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn new_device_id() -> Option<String> {
    let mut bytes = [0u8; 16];
    #[cfg(windows)]
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes).ok()?;
    #[cfg(not(windows))]
    std::io::Read::read_exact(&mut fs::File::open("/dev/urandom").ok()?, &mut bytes).ok()?;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!("perfuse-{hex}"))
}

/// Used only if the system random generator fails; it is not saved.
fn fallback_device_id() -> String {
    format!(
        "perfuse-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}

fn preference_scope(session: &Credentials) -> String {
    match session.kind {
        // Keep the existing hash for Plex libraries, names and ordering.
        ServerKind::Plex => session.server.clone(),
        kind => format!("{}/{}", kind.plugin().id, session.server.trim()),
    }
}

pub fn load_library_order(session: &Credentials) -> LibraryOrder {
    file()
        .map(|path| load_from(&path, &preference_scope(session)))
        .unwrap_or_default()
}

pub fn save_library_order(session: &Credentials, order: &LibraryOrder) {
    if let Some(path) = file() {
        save_to(&path, &preference_scope(session), order);
    }
}

/// Removes a server's saved library order, sidebar name and hidden libraries.
pub fn forget_server(session: &Credentials) {
    if let Some(path) = file() {
        forget_in(&path, &preference_scope(session));
    }
}

fn forget_in(path: &Path, server: &str) {
    let key = server_key(server);
    if read_store(path).libraries.contains_key(&key) {
        update_store(path, |store| {
            store.libraries.remove(&key);
        });
    }
}

/// Applies the chosen library sort or custom drag order to the server's section list.
pub fn apply_library_order(server_sections: &[Library], order: &LibraryOrder) -> Vec<Library> {
    let mut result = server_sections.to_vec();
    match order.sort {
        LibrarySort::Server => result,
        LibrarySort::NameAsc => {
            result.sort_by(|a, b| {
                a.title
                    .to_lowercase()
                    .cmp(&b.title.to_lowercase())
                    .then_with(|| a.title.cmp(&b.title))
                    .then_with(|| a.id.cmp(&b.id))
            });
            result
        }
        LibrarySort::NameDesc => {
            result.sort_by(|a, b| {
                b.title
                    .to_lowercase()
                    .cmp(&a.title.to_lowercase())
                    .then_with(|| b.title.cmp(&a.title))
                    .then_with(|| a.id.cmp(&b.id))
            });
            result
        }
        LibrarySort::TypeThenName => {
            result.sort_by(|a, b| {
                let rank = |kind: LibraryKind| match kind {
                    LibraryKind::Movies => 0,
                    LibraryKind::Shows => 1,
                    LibraryKind::Audiobooks => 2,
                };
                rank(a.kind)
                    .cmp(&rank(b.kind))
                    .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
                    .then_with(|| a.title.cmp(&b.title))
                    .then_with(|| a.id.cmp(&b.id))
            });
            result
        }
        LibrarySort::Custom => {
            let mut ordered = Vec::with_capacity(server_sections.len());
            let mut used = vec![false; server_sections.len()];
            for key in &order.custom_keys {
                if let Some(index) = server_sections
                    .iter()
                    .enumerate()
                    .position(|(i, s)| !used[i] && s.id == *key)
                {
                    used[index] = true;
                    ordered.push(server_sections[index].clone());
                }
            }
            for (index, section) in server_sections.iter().enumerate() {
                if !used[index] {
                    ordered.push(section.clone());
                }
            }
            ordered
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_sections() -> Vec<Library> {
        vec![
            Library {
                id: "3".into(),
                title: "TV Shows".into(),
                kind: LibraryKind::Shows,
            },
            Library {
                id: "1".into(),
                title: "Movies".into(),
                kind: LibraryKind::Movies,
            },
            Library {
                id: "2".into(),
                title: "anime".into(),
                kind: LibraryKind::Shows,
            },
            Library {
                id: "4".into(),
                title: "Documentaries".into(),
                kind: LibraryKind::Movies,
            },
        ]
    }

    #[test]
    fn sorts_libraries_by_each_mode_and_preserves_new_sections_in_custom_order() {
        let sections = sample_sections();
        let titles = |list: Vec<Library>| list.into_iter().map(|s| s.title).collect::<Vec<_>>();

        assert_eq!(
            titles(apply_library_order(&sections, &LibraryOrder::default())),
            ["TV Shows", "Movies", "anime", "Documentaries"]
        );
        assert_eq!(
            titles(apply_library_order(
                &sections,
                &LibraryOrder {
                    sort: LibrarySort::NameAsc,
                    custom_keys: vec![],
                    ..Default::default()
                }
            )),
            ["anime", "Documentaries", "Movies", "TV Shows"]
        );
        assert_eq!(
            titles(apply_library_order(
                &sections,
                &LibraryOrder {
                    sort: LibrarySort::NameDesc,
                    custom_keys: vec![],
                    ..Default::default()
                }
            )),
            ["TV Shows", "Movies", "Documentaries", "anime"]
        );
        assert_eq!(
            titles(apply_library_order(
                &sections,
                &LibraryOrder {
                    sort: LibrarySort::TypeThenName,
                    custom_keys: vec![],
                    ..Default::default()
                }
            )),
            ["Documentaries", "Movies", "anime", "TV Shows"]
        );
        assert_eq!(
            titles(apply_library_order(
                &sections,
                &LibraryOrder {
                    sort: LibrarySort::Custom,
                    // Includes a stale removed key ("99") and omits key "1" (newly added).
                    custom_keys: vec!["4".into(), "99".into(), "2".into()],
                    ..Default::default()
                }
            )),
            ["Documentaries", "anime", "TV Shows", "Movies"]
        );
    }

    #[test]
    fn library_preferences_are_scoped_to_the_source_at_each_origin() {
        let mut session = Credentials {
            kind: ServerKind::Plex,
            server: "http://library.local".into(),
            token: "test".into(),
            client_id: "test".into(),
        };
        let key = |session: &Credentials| server_key(&preference_scope(session));
        assert_eq!(key(&session), server_key("http://library.local"));
        session.kind = ServerKind::Jellyfin;
        assert_eq!(key(&session), server_key("jellyfin/http://library.local"));
        session.kind = ServerKind::Audiobookshelf;
        assert_eq!(
            key(&session),
            server_key("audiobookshelf/http://library.local")
        );
    }

    #[test]
    fn volume_and_mute_survive_relaunch_and_library_preference_updates() {
        let dir = std::env::temp_dir().join(format!("perfuse-volume-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        fs::write(&path, r#"{"libraries":{}}"#).unwrap();
        let mut volume = read_store(&path).volume;
        assert_eq!(volume.gain(), 100.0);
        assert!(volume.set(37.0));
        volume.save_to(&path);
        let mut restored = read_store(&path).volume;
        assert_eq!(restored.gain(), 37.0);
        assert!(restored.set(0.0));
        restored.save_to(&path);
        let order = LibraryOrder {
            collapsed: true,
            ..Default::default()
        };
        save_to(&path, "test", &order);
        let mut restored = read_store(&path).volume;
        assert_eq!(restored.gain(), 0.0);
        assert!(!restored.set(0.0));
        assert!(restored.set(restored.level()));
        assert_eq!(restored.gain(), 37.0);
        restored.save_to(&path);
        assert_eq!(load_from(&path, "test"), order);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn saves_and_restores_library_order_per_server_without_raw_urls() {
        let dir = std::env::temp_dir().join(format!("perfuse-prefs-test-{}", std::process::id()));
        let path = dir.join("preferences.json");
        let order = LibraryOrder {
            sort: LibrarySort::Custom,
            custom_keys: vec!["2".into(), "1".into()],
            ..Default::default()
        };
        save_to(&path, "http://server-a:32400/", &order);
        assert_eq!(load_from(&path, "http://SERVER-A:32400"), order);
        assert_eq!(
            load_from(&path, "http://server-b:32400"),
            LibraryOrder::default()
        );
        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("server-a"));
        forget_in(&path, "http://Server-A:32400");
        assert_eq!(
            load_from(&path, "http://server-a:32400"),
            LibraryOrder::default()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn backups(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "bad"))
            .collect()
    }

    #[test]
    fn damaged_preferences_are_set_aside_once_instead_of_overwritten() {
        let dir = std::env::temp_dir().join(format!("perfuse-prefs-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        let damaged = br#"{"libraries":{"#;
        fs::write(&path, damaged).unwrap();
        assert_eq!(read_store(&path).volume.gain(), 100.0);
        assert_eq!(fs::read(&path).unwrap(), damaged);
        let order = LibraryOrder {
            collapsed: true,
            ..Default::default()
        };
        save_to(&path, "test", &order);
        let saved = backups(&dir);
        assert_eq!(saved.len(), 1);
        assert_eq!(fs::read(&saved[0]).unwrap(), damaged);
        assert_eq!(load_from(&path, "test"), order);
        save_to(&path, "other", &order);
        assert_eq!(backups(&dir), saved);
        assert_eq!(load_from(&path, "test"), order);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn preferences_from_a_newer_build_load_and_keep_unknown_sections() {
        let dir = std::env::temp_dir().join(format!("perfuse-prefs-newer-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        let key = server_key("test");
        fs::write(
            &path,
            format!(
                r#"{{"libraries":{{"{key}":{{"sort":"recently_added","collapsed":true,"pinned":["1"]}}}},
                "volume":{{"level":40.0,"boost":2}},"subtitles":{{"size":3}}}}"#
            ),
        )
        .unwrap();
        let order = load_from(&path, "test");
        assert_eq!(order.sort, LibrarySort::Server);
        assert!(order.collapsed);
        assert_eq!(read_store(&path).volume.gain(), 40.0);
        save_to(&path, "other", &LibraryOrder::default());
        assert!(backups(&dir).is_empty());
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["subtitles"]["size"], 3);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn device_id_is_random_header_safe_and_stable() {
        let dir = std::env::temp_dir().join(format!("perfuse-device-id-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preferences.json");
        fs::write(&path, r#"{"volume":{"level":40.0}}"#).unwrap();
        let id = device_id_in(&path);
        let hex = id.strip_prefix("perfuse-").unwrap();
        assert!(hex.len() == 32 && hex.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(valid_device_id(&id));
        assert_eq!(device_id_in(&path), id);
        assert_eq!(read_store(&path).volume.gain(), 40.0);
        let mut volume = read_store(&path).volume;
        assert!(volume.set(20.0));
        volume.save_to(&path);
        assert_eq!(device_id_in(&path), id);
        assert_ne!(device_id_in(&dir.join("other.json")), id);
        let _ = fs::remove_dir_all(dir);
    }
}
