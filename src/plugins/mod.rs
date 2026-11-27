//! Media source registration. Plugins are compiled into the application.

pub mod audiobookshelf;
mod credentials;
pub mod jellyfin;
pub mod plex;

pub use crate::http::base_url;
use crate::media::{LibraryKind, Source, SourceError, SourceResult};
pub use credentials::{Credentials, Refresh, SessionToken, TokenPair};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Deserialize, Serialize)]
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

/// A way to sign in that the add-server form offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInMethod {
    /// The provider's browser sign-in. It discovers the account's servers.
    Account,
    /// A server address and an access token.
    Token,
    /// A server address, username and password.
    Password { required: bool },
    /// A code approved from a device that is already signed in.
    QuickConnect,
}

/// The form values and callbacks one sign-in attempt needs.
pub struct SignInRequest<'a> {
    pub method: SignInMethod,
    pub username: &'a str,
    pub password: &'a str,
    /// Shows a Quick Connect code to the user.
    pub show_code: &'a dyn Fn(String),
    /// Whether the user abandoned this attempt.
    pub cancelled: &'a dyn Fn() -> bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OpenOptions {
    /// Keep fetched artwork in the disk cache.
    pub cache_artwork: bool,
}

pub struct Plugin {
    pub kind: ServerKind,
    pub id: &'static str,
    pub name: &'static str,
    pub default_address: &'static str,
    /// An address shown as "For example …" under the address field.
    pub address_example: &'static str,
    /// What this kind of server offers, on the server type chooser.
    pub description: &'static str,
    /// Library types this source can list.
    pub libraries: &'static [LibraryKind],
    /// Ways to sign in, in the order the form offers them.
    pub sign_in: &'static [SignInMethod],
    /// Exchanges the form's values for a token in `credentials.token`.
    pub authenticate: fn(&mut Credentials, &SignInRequest<'_>) -> SourceResult<()>,
    /// Opens an authenticated connection.
    pub open: fn(&Credentials, OpenOptions) -> SourceResult<Source>,
}

impl Plugin {
    pub fn offers(&self, method: SignInMethod) -> bool {
        self.sign_in.contains(&method)
    }

    pub fn password_sign_in(&self) -> Option<bool> {
        self.sign_in.iter().find_map(|method| match method {
            SignInMethod::Password { required } => Some(*required),
            _ => None,
        })
    }

    /// Whether every library this source lists is listened to rather than watched.
    pub fn audio_only(&self) -> bool {
        self.libraries.iter().all(|kind| kind.is_audio())
    }

    /// Library types for prose, such as "movie or TV".
    pub fn library_nouns(&self) -> String {
        self.libraries
            .iter()
            .map(|kind| kind.noun())
            .collect::<Vec<_>>()
            .join(" or ")
    }
}

pub const BUILTIN: &[&Plugin] = &[&plex::PLUGIN, &jellyfin::PLUGIN, &audiobookshelf::PLUGIN];

pub fn by_id(id: &str) -> Option<&'static Plugin> {
    BUILTIN.iter().copied().find(|plugin| plugin.id == id)
}

/// Accepts only server identifiers that are safe in a URL path and query.
pub(crate) fn identifier<'a>(id: &'a str, server: &str) -> SourceResult<&'a str> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(SourceError::Failed(format!(
            "{server} returned an invalid identifier."
        )));
    }
    Ok(id)
}

/// Sign-in methods that need no exchange, because the form supplies the token.
fn token_only(_: &mut Credentials, request: &SignInRequest<'_>) -> SourceResult<()> {
    match request.method {
        SignInMethod::Token => Ok(()),
        _ => Err(SourceError::Unsupported(
            "This server does not offer that sign-in method.".into(),
        )),
    }
}
