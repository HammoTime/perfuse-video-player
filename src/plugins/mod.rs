//! Media source registration. Plugins are compiled into the application.

pub mod audiobookshelf;
pub mod jellyfin;
pub mod plex;

pub use crate::http::base_url;
use crate::media::Source;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerKind {
    #[default]
    Plex,
    Jellyfin,
    Audiobookshelf,
}

impl ServerKind {
    pub fn plugin(self) -> &'static Plugin {
        match self {
            Self::Plex => &plex::PLUGIN,
            Self::Jellyfin => &jellyfin::PLUGIN,
            Self::Audiobookshelf => &audiobookshelf::PLUGIN,
        }
    }
}

// Deliberately no Debug: these credentials contain the access token.
#[derive(Clone, Deserialize, Serialize)]
pub struct Credentials {
    /// Credential blobs written before Jellyfin support belong to Plex.
    #[serde(default)]
    pub kind: ServerKind,
    pub server: String,
    pub token: audiobookshelf::auth::SessionToken,
    pub client_id: String,
}

pub struct Plugin {
    pub kind: ServerKind,
    pub id: &'static str,
    pub name: &'static str,
    pub default_address: &'static str,
    /// Opens an authenticated connection. The second argument enables disk artwork caching.
    pub open: fn(&Credentials, bool) -> Result<Source, String>,
}

pub const BUILTIN: &[&Plugin] = &[&plex::PLUGIN, &jellyfin::PLUGIN, &audiobookshelf::PLUGIN];

pub fn by_id(id: &str) -> Option<&'static Plugin> {
    BUILTIN.iter().copied().find(|plugin| plugin.id == id)
}
