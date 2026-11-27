//! Last-known server metadata, so Perfuse can show libraries and Home rows before the
//! first refresh. Files hold no credentials and are named by a hash of the source and
//! server origin. The app writes them only for servers whose sign-in is remembered.

use crate::media::{Item, Library, Page};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Bump when a stored shape changes, so older files are ignored rather than misread.
const VERSION: u32 = 1;
const LIMIT_BYTES: u64 = 16 * 1024 * 1024;

/// What the server list and sidebar need: the libraries and Continue rows.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Overview {
    pub libraries: Vec<Library>,
    pub continue_items: Vec<Item>,
}

/// A server's Home: the first Recently Added page and the rows above it.
#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Home {
    pub page: Page,
    pub on_deck: Vec<Item>,
    pub released: Vec<Item>,
}

#[derive(Clone, Copy)]
enum Part {
    Overview,
    Home,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Home => "home",
        }
    }
}

#[derive(Deserialize, Serialize)]
struct Envelope<T> {
    version: u32,
    saved_at: u64,
    value: T,
}

/// A stored value and when it was saved, in Unix seconds.
pub struct Saved<T> {
    pub value: T,
    pub saved_at: u64,
}

/// One server's stored metadata, keyed by source and origin.
#[derive(Clone)]
pub struct Store {
    dir: Option<PathBuf>,
    key: String,
}

impl Store {
    pub fn new(source_id: &str, server: &str) -> Self {
        Self::in_dir(dir(), source_id, server)
    }

    fn in_dir(dir: Option<PathBuf>, source_id: &str, server: &str) -> Self {
        let origin = reqwest::Url::parse(server.trim())
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_else(|_| server.trim().to_owned());
        // FNV-1a keeps names stable across Rust releases, unlike std's hasher.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in source_id.bytes().chain([0]).chain(origin.bytes()) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
        Self {
            dir,
            key: format!("{hash:016x}"),
        }
    }

    pub fn overview(&self) -> Option<Saved<Overview>> {
        self.load(Part::Overview)
    }

    pub fn save_overview(&self, overview: &Overview) {
        self.save(Part::Overview, overview);
    }

    pub fn home(&self) -> Option<Saved<Home>> {
        self.load(Part::Home)
    }

    pub fn save_home(&self, home: &Home) {
        self.save(Part::Home, home);
    }

    /// Removes everything stored for this server, such as when it is removed.
    pub fn forget(&self) {
        for part in [Part::Overview, Part::Home] {
            if let Some(path) = self.path(part) {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn path(&self, part: Part) -> Option<PathBuf> {
        let dir = self.dir.as_deref()?;
        Some(dir.join(format!("{}-{}.json", self.key, part.name())))
    }

    fn load<T: DeserializeOwned>(&self, part: Part) -> Option<Saved<T>> {
        let path = self.path(part)?;
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .ok()?
            .take(LIMIT_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        let envelope = (bytes.len() as u64 <= LIMIT_BYTES)
            .then(|| serde_json::from_slice::<Envelope<T>>(&bytes).ok())
            .flatten()
            .filter(|envelope| envelope.version == VERSION);
        let Some(envelope) = envelope else {
            // Unreadable, oversized or from another format version. Fetch it again.
            let _ = fs::remove_file(&path);
            return None;
        };
        Some(Saved {
            value: envelope.value,
            saved_at: envelope.saved_at,
        })
    }

    fn save<T: Serialize>(&self, part: Part, value: &T) {
        let Some(path) = self.path(part) else {
            return;
        };
        let saved_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |time| time.as_secs());
        let Ok(bytes) = serde_json::to_vec(&Envelope {
            version: VERSION,
            saved_at,
            value,
        }) else {
            return;
        };
        if bytes.len() as u64 <= LIMIT_BYTES {
            write(&path, &bytes);
        }
    }
}

/// `%LOCALAPPDATA%\Perfuse\Metadata` on Windows, the XDG cache elsewhere.
/// Tests never touch the user's cache.
fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::cache::dir()
        .and_then(|artwork| artwork.parent().map(Path::to_path_buf))
        .map(|base| base.join("Metadata"))
}

fn write(target: &Path, bytes: &[u8]) {
    let Some(dir) = target.parent() else {
        return;
    };
    // A unique partial name keeps concurrent writers from interleaving into one file.
    let partial = target.with_extension(format!(
        "{}-{:?}.part",
        std::process::id(),
        std::thread::current().id()
    ));
    let written = fs::create_dir_all(dir)
        .and_then(|()| fs::File::create(&partial))
        .and_then(|mut out| out.write_all(bytes).and_then(|()| out.sync_all()))
        .and_then(|()| fs::rename(&partial, target));
    if written.is_err() {
        let _ = fs::remove_file(&partial);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{Kind, LibraryKind};

    #[test]
    fn snapshots_round_trip_per_server_and_reject_other_versions() {
        let dir = std::env::temp_dir().join(format!("perfuse-metadata-{}", std::process::id()));
        let store = Store::in_dir(Some(dir.clone()), "plex", "http://a:32400");
        let other = Store::in_dir(Some(dir.clone()), "jellyfin", "http://a:32400");
        let same_origin = Store::in_dir(Some(dir.clone()), "plex", "HTTP://A:32400/");
        assert_eq!(store.key, same_origin.key);
        store.save_overview(&Overview {
            libraries: vec![Library {
                id: "1".into(),
                title: "Films".into(),
                kind: LibraryKind::Movies,
            }],
            continue_items: vec![Item {
                id: "9".into(),
                kind: Kind::Movie,
                ..Item::default()
            }],
        });
        let loaded = store.overview().unwrap().value;
        assert_eq!(loaded.libraries[0].title, "Films");
        assert_eq!(loaded.continue_items[0].kind, Kind::Movie);
        assert!(other.overview().is_none());

        let path = store.path(Part::Overview).unwrap();
        fs::write(&path, br#"{"version":0,"saved_at":0,"value":{}}"#).unwrap();
        assert!(store.overview().is_none());
        assert!(!path.exists(), "a stale file is removed");
        store.save_home(&Home::default());
        store.forget();
        assert!(store.home().is_none());
        let _ = fs::remove_dir_all(dir);
    }
}
