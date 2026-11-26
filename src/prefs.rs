//! Non-sensitive UI preferences (such as sidebar library ordering).
//! Never stores tokens, credentials, or raw server URLs.

use perfuse::{
    media::{Kind, Library},
    plugins::{Credentials, ServerKind},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LibrarySort {
    #[default]
    Server,
    NameAsc,
    NameDesc,
    TypeThenName,
    Custom,
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

#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    libraries: HashMap<String, LibraryOrder>,
    #[serde(default)]
    volume: Volume,
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

fn read_store(path: &Path) -> Store {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
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
    let mut store = read_store(path);
    update(&mut store);
    let Ok(bytes) = serde_json::to_vec_pretty(&store) else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };
    let partial = path.with_extension(format!("{}.part", std::process::id()));
    let written = fs::create_dir_all(dir)
        .and_then(|()| fs::File::create(&partial))
        .and_then(|mut out| out.write_all(&bytes))
        .and_then(|()| fs::rename(&partial, path));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
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
                let rank = |kind: Kind| match kind {
                    Kind::Movie => 0,
                    Kind::Series => 1,
                    Kind::Audiobook => 2,
                    _ => 3,
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
                kind: Kind::Series,
            },
            Library {
                id: "1".into(),
                title: "Movies".into(),
                kind: Kind::Movie,
            },
            Library {
                id: "2".into(),
                title: "anime".into(),
                kind: Kind::Series,
            },
            Library {
                id: "4".into(),
                title: "Documentaries".into(),
                kind: Kind::Movie,
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
        let _ = fs::remove_dir_all(&dir);
    }
}
