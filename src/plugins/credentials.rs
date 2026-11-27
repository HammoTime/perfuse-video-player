//! Sign-in credentials shared by every source. Plex, Jellyfin and legacy Audiobookshelf
//! logins hold a fixed token. Audiobookshelf's current logins hold a rotating pair.
use super::ServerKind;
use crate::media::{SourceError, SourceResult};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

// Deliberately no Debug: these credentials contain the access token.
#[derive(Clone, Deserialize, Serialize)]
pub struct Credentials {
    /// Credential blobs written before Jellyfin support belong to Plex.
    #[serde(default)]
    pub kind: ServerKind,
    pub server: String,
    pub token: SessionToken,
    pub client_id: String,
}

/// Locks shared token state, recovering it if a thread panicked while holding it.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// Existing Plex, Jellyfin and legacy Audiobookshelf tokens serialize as plain strings.
#[derive(Clone)]
pub struct SessionToken(Secret);

#[derive(Clone)]
enum Secret {
    Fixed(String),
    Rotating(Arc<Shared>),
}

// A rotating pair is shared by the UI, request workers and the Credential Manager writer.
// Readers hold `state` only to copy a value. `turn` serializes token exchanges and saves,
// which wait on the network or Credential Manager, without blocking those readers.
struct Shared {
    state: Mutex<Tokens>,
    turn: Mutex<()>,
}

struct Tokens {
    pair: TokenPair,
    rotate_saved: Option<Arc<RotateSaved>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TokenPair {
    // A stable local identity distinguishes a restored login from a replacement account.
    pub login: u64,
    pub revision: u64,
    pub(crate) access: String,
    pub(crate) refresh: String,
}

pub type Refresh<'a> = dyn FnMut(&mut TokenPair) -> SourceResult<()> + 'a;
type RotateSaved = dyn Fn(&mut TokenPair, &mut Refresh<'_>) -> SourceResult<()> + Send + Sync;

#[derive(Serialize)]
#[serde(untagged)]
enum StoredRef<'a> {
    Fixed(&'a str),
    Rotating(&'a TokenPair),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Fixed(String),
    Rotating(TokenPair),
}

impl Serialize for SessionToken {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.0 {
            Secret::Fixed(value) => StoredRef::Fixed(value).serialize(serializer),
            Secret::Rotating(shared) => {
                let pair = lock(&shared.state).pair.clone();
                StoredRef::Rotating(&pair).serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for SessionToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Stored::deserialize(deserializer)? {
            Stored::Fixed(value) => value.into(),
            Stored::Rotating(pair) => pair.into(),
        })
    }
}

impl From<TokenPair> for SessionToken {
    fn from(pair: TokenPair) -> Self {
        Self(Secret::Rotating(Arc::new(Shared {
            state: Mutex::new(Tokens {
                pair,
                rotate_saved: None,
            }),
            turn: Mutex::new(()),
        })))
    }
}

impl From<String> for SessionToken {
    fn from(value: String) -> Self {
        Self(Secret::Fixed(value))
    }
}

impl From<&str> for SessionToken {
    fn from(value: &str) -> Self {
        Self::from(value.to_owned())
    }
}

impl SessionToken {
    pub fn fixed(&self) -> SourceResult<&str> {
        match &self.0 {
            Secret::Fixed(value) => Ok(value),
            Secret::Rotating(_) => Err("This sign-in is for a different media source.".into()),
        }
    }

    /// A new login with a fresh local identity.
    pub(crate) fn rotating(access: String, refresh: String) -> Self {
        use std::hash::{BuildHasher, RandomState};
        let login = RandomState::new().hash_one(&refresh);
        TokenPair {
            login,
            revision: 0,
            access,
            refresh,
        }
        .into()
    }

    pub(crate) fn is_rotating(&self) -> bool {
        matches!(self.0, Secret::Rotating(_))
    }

    pub(crate) fn access(&self) -> String {
        match &self.0 {
            Secret::Fixed(value) => value.clone(),
            Secret::Rotating(shared) => lock(&shared.state).pair.access.clone(),
        }
    }

    pub(crate) fn refresh_token(&self) -> Option<String> {
        match &self.0 {
            Secret::Rotating(shared) => Some(lock(&shared.state).pair.refresh.clone()),
            Secret::Fixed(_) => None,
        }
    }

    pub fn snapshot(&self) -> Option<TokenPair> {
        match &self.0 {
            Secret::Rotating(shared) => Some(lock(&shared.state).pair.clone()),
            Secret::Fixed(_) => None,
        }
    }

    /// Save a stable snapshot and subscribe to subsequent rotations, or subscribe on restore.
    pub fn remember(
        &self,
        save: impl FnOnce(&SessionToken) -> SourceResult<()>,
        rotate: impl Fn(&mut TokenPair, &mut Refresh<'_>) -> SourceResult<()> + Send + Sync + 'static,
    ) -> SourceResult<()> {
        let Secret::Rotating(shared) = &self.0 else {
            return save(self);
        };
        // No exchange can rotate the pair between this snapshot and its save.
        let _turn = lock(&shared.turn);
        let pair = lock(&shared.state).pair.clone();
        save(&pair.into())?;
        lock(&shared.state).rotate_saved = Some(Arc::new(rotate));
        Ok(())
    }

    pub fn forget(&self, remove: impl FnOnce(Option<u64>) -> SourceResult<()>) -> SourceResult<()> {
        let Secret::Rotating(shared) = &self.0 else {
            return remove(None);
        };
        let _turn = lock(&shared.turn);
        let login = lock(&shared.state).pair.login;
        remove(Some(login))?;
        lock(&shared.state).rotate_saved = None;
        Ok(())
    }

    /// Exchanges the refresh token after the server rejected `stale`, and returns the
    /// current access token. Workers that saw the same rejection share one exchange.
    pub(crate) fn rotate(&self, stale: &str, exchange: &mut Refresh<'_>) -> SourceResult<String> {
        let Secret::Rotating(shared) = &self.0 else {
            return Err(SourceError::Unauthorized(
                "This sign-in cannot be renewed. Sign in again.".into(),
            ));
        };
        let _turn = lock(&shared.turn);
        let (mut pair, saved) = {
            let tokens = lock(&shared.state);
            if tokens.pair.access != stale {
                return Ok(tokens.pair.access.clone());
            }
            (tokens.pair.clone(), tokens.rotate_saved.clone())
        };
        match saved {
            Some(rotate) => rotate(&mut pair, exchange)?,
            None => exchange(&mut pair)?,
        }
        let access = pair.access.clone();
        lock(&shared.state).pair = pair;
        Ok(access)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread, time::Duration};

    #[test]
    fn readers_are_not_blocked_by_a_slow_exchange() {
        let token = SessionToken::rotating("old".into(), "refresh".into());
        let (started, exchanging) = mpsc::channel();
        let (finish, finished) = mpsc::channel::<()>();
        let worker = {
            let token = token.clone();
            thread::spawn(move || {
                token.rotate("old", &mut |pair: &mut TokenPair| {
                    started.send(()).unwrap();
                    finished.recv().unwrap();
                    pair.access = "new".into();
                    pair.revision += 1;
                    Ok(())
                })
            })
        };
        exchanging.recv_timeout(Duration::from_secs(5)).unwrap();
        // The exchange is still waiting, yet the current token and a snapshot are readable.
        assert_eq!(token.access(), "old");
        assert_eq!(token.snapshot().unwrap().revision, 0);
        finish.send(()).unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), "new");
        // A second worker that saw the old token reuses the completed exchange.
        let reused = token.rotate("old", &mut |_: &mut TokenPair| panic!("rotated twice"));
        assert_eq!(reused.unwrap(), "new");
    }

    #[test]
    fn stored_tokens_round_trip_in_both_formats() {
        let fixed: SessionToken = serde_json::from_str(r#""plain""#).unwrap();
        assert_eq!(fixed.fixed().unwrap(), "plain");
        let pair = SessionToken::rotating("a".into(), "r".into());
        let json = serde_json::to_value(&pair).unwrap();
        assert_eq!(json["access"], "a");
        let restored: SessionToken = serde_json::from_value(json).unwrap();
        assert_eq!(restored.snapshot().unwrap().refresh, "r");
    }
}
