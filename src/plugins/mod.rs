//! Media source registration. Plugins are compiled into the application.

pub mod plex;

use crate::media::Source;
use serde::{Deserialize, Serialize};

// Deliberately no Debug: these credentials contain the access token.
#[derive(Clone, Deserialize, Serialize)]
pub struct Credentials {
    pub server: String,
    pub token: String,
    pub client_id: String,
}

pub struct Plugin {
    pub id: &'static str,
    pub name: &'static str,
    pub default_address: &'static str,
    /// Opens an authenticated connection. The second argument enables disk artwork caching.
    pub open: fn(&Credentials, bool) -> Result<Source, String>,
}

pub const BUILTIN: &[&Plugin] = &[&plex::PLUGIN];

pub fn by_id(id: &str) -> Option<&'static Plugin> {
    BUILTIN.iter().copied().find(|plugin| plugin.id == id)
}
