//! Audiobookshelf's native-client login and rotating bearer credentials.
use crate::{http, plugins::Credentials};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, RequestBuilder, Response},
    header::{AUTHORIZATION, HeaderValue},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::sync::{Arc, Mutex};

const SERVER: &str = "Audiobookshelf";
const INVALID: &str = "Invalid Audiobookshelf sign-in. Sign in again.";

// Existing Plex, Jellyfin and legacy Audiobookshelf tokens serialize as plain strings.
// A rotating pair is shared by the UI, request workers and Credential Manager writer.
#[derive(Clone, Deserialize, Serialize)]
#[serde(transparent)]
pub struct SessionToken(Secret);

#[derive(Clone, Deserialize, Serialize)]
#[serde(untagged)]
enum Secret {
    Fixed(String),
    Rotating(Arc<Mutex<Tokens>>),
}

#[derive(Deserialize, Serialize)]
struct Tokens {
    #[serde(flatten)]
    pair: TokenPair,
    #[serde(skip)]
    rotate_saved: Option<Box<RotateSaved>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TokenPair {
    // A stable local identity distinguishes a restored login from a replacement account.
    pub login: u64,
    pub revision: u64,
    access: String,
    refresh: String,
}

pub type Refresh<'a> = dyn FnMut(&mut TokenPair) -> Result<(), String> + 'a;
type RotateSaved = dyn Fn(&mut TokenPair, &mut Refresh<'_>) -> Result<(), String> + Send + Sync;

impl From<TokenPair> for SessionToken {
    fn from(pair: TokenPair) -> Self {
        Self(Secret::Rotating(Arc::new(Mutex::new(Tokens {
            pair,
            rotate_saved: None,
        }))))
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
    pub fn fixed(&self) -> Result<&str, String> {
        match &self.0 {
            Secret::Fixed(value) => Ok(value),
            Secret::Rotating(_) => Err("This sign-in is for a different media source.".into()),
        }
    }

    fn pair(access: String, refresh: String) -> Self {
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

    fn access(&self) -> String {
        match &self.0 {
            Secret::Fixed(value) => value.clone(),
            Secret::Rotating(tokens) => tokens.lock().unwrap().pair.access.clone(),
        }
    }

    pub fn snapshot(&self) -> Option<TokenPair> {
        match &self.0 {
            Secret::Rotating(tokens) => Some(tokens.lock().unwrap().pair.clone()),
            Secret::Fixed(_) => None,
        }
    }

    /// Save a stable snapshot and subscribe to subsequent rotations, or subscribe on restore.
    pub fn remember(
        &self,
        save: impl FnOnce(&SessionToken) -> Result<(), String>,
        rotate: impl Fn(&mut TokenPair, &mut Refresh<'_>) -> Result<(), String> + Send + Sync + 'static,
    ) -> Result<(), String> {
        if let Secret::Rotating(shared) = &self.0 {
            let mut tokens = shared.lock().unwrap();
            save(&tokens.pair.clone().into())?;
            tokens.rotate_saved = Some(Box::new(rotate));
        } else {
            save(self)?;
        }
        Ok(())
    }

    pub fn forget(
        &self,
        remove: impl FnOnce(Option<u64>) -> Result<(), String>,
    ) -> Result<(), String> {
        if let Secret::Rotating(shared) = &self.0 {
            let mut tokens = shared.lock().unwrap();
            remove(Some(tokens.pair.login))?;
            tokens.rotate_saved = None;
            Ok(())
        } else {
            remove(None)
        }
    }
}

#[derive(Deserialize)]
struct Login {
    user: UserTokens,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserTokens {
    token: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
}

fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
        })
}

impl UserTokens {
    fn pair(self) -> Result<(String, String), String> {
        match (self.access_token, self.refresh_token) {
            (Some(access), Some(refresh)) if valid(&access) && valid(&refresh) => {
                Ok((access, refresh))
            }
            _ => Err(INVALID.into()),
        }
    }
}

fn send(request: RequestBuilder) -> Result<Response, String> {
    request.send().map_err(|_| {
        "Cannot reach Audiobookshelf. Check the address, network, and TLS certificate.".into()
    })
}

fn sensitive(value: &str) -> Result<HeaderValue, String> {
    let mut header = HeaderValue::from_str(value).map_err(|_| INVALID.to_owned())?;
    header.set_sensitive(true);
    Ok(header)
}

pub fn password(session: &mut Credentials, username: &str, password: &str) -> Result<(), String> {
    if username.trim().is_empty() || password.is_empty() {
        return Err("Enter your Audiobookshelf username and password.".into());
    }
    let mut url = http::base_url(&session.server)?;
    url.set_path("/login");
    let response = send(
        http::client(Default::default())?
            .post(url)
            .header("x-return-tokens", "true")
            .json(&serde_json::json!({ "username": username.trim(), "password": password })),
    )?;
    if response.status() == StatusCode::UNAUTHORIZED {
        return Err("Audiobookshelf rejected the username or password.".into());
    }
    let user = http::json::<Login>(response, SERVER)?.user;
    session.token = if user.access_token.is_none() && user.refresh_token.is_none() {
        user.token.filter(|s| valid(s)).ok_or(INVALID)?.into()
    } else {
        let (access, refresh) = user.pair()?;
        SessionToken::pair(access, refresh)
    };
    Ok(())
}

pub(super) struct Api {
    pub(super) base: Url,
    client: Client,
    token: SessionToken,
}

impl Api {
    pub(super) fn new(session: &Credentials) -> Result<Self, String> {
        let base = http::base_url(&session.server)?;
        if !valid(&session.token.access())
            || matches!(&session.token.0, Secret::Rotating(tokens) if !valid(&tokens.lock().unwrap().pair.refresh))
        {
            return Err(INVALID.into());
        }
        Ok(Self {
            base,
            client: http::client(Default::default())?,
            token: session.token.clone(),
        })
    }

    pub(super) fn url(&self, path: &str) -> Url {
        let mut url = self.base.clone();
        url.set_path(path);
        url
    }

    pub(super) fn get(&self, path: &str) -> RequestBuilder {
        self.client.get(self.url(path))
    }
    pub(super) fn post(&self, path: &str) -> RequestBuilder {
        self.client.post(self.url(path))
    }
    pub(super) fn patch(&self, path: &str) -> RequestBuilder {
        self.client.patch(self.url(path))
    }
    pub(super) fn delete(&self, path: &str) -> RequestBuilder {
        self.client.delete(self.url(path))
    }

    pub(super) fn playback_header(&self, path: &str) -> Result<String, String> {
        // A book can be opened after a long pause. Renew an expired bearer before
        // handing the media URL to libmpv, which cannot perform a token exchange.
        if matches!(self.token.0, Secret::Rotating(_)) {
            let response = self.send(self.client.head(self.url(path)))?;
            http::check_status(response.status(), SERVER)?;
        }
        Ok(format!("Authorization: Bearer {}", self.token.access()))
    }

    pub(super) fn send(&self, request: RequestBuilder) -> Result<Response, String> {
        let access = self.token.access();
        let with_token = |req: RequestBuilder, token: &str| {
            send(req.header(AUTHORIZATION, sensitive(&format!("Bearer {token}"))?))
        };
        let retry = request.try_clone();
        let response = with_token(request, &access)?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        let (Secret::Rotating(shared), Some(retry)) = (&self.token.0, retry) else {
            return Ok(response);
        };
        drop(response);
        let mut tokens = shared.lock().unwrap();
        // Multiple workers may observe the same 401. Rotate exactly once.
        if tokens.pair.access == access {
            let Tokens { pair, rotate_saved } = &mut *tokens;
            let mut refresh = |pair: &mut TokenPair| {
                let response = send(
                    self.post("/auth/refresh")
                        .header("x-refresh-token", sensitive(&pair.refresh)?),
                )?;
                (pair.access, pair.refresh) = http::json::<Login>(response, SERVER)?.user.pair()?;
                pair.revision = pair.revision.saturating_add(1);
                Ok(())
            };
            if let Some(rotate) = rotate_saved {
                rotate(pair, &mut refresh)?;
            } else {
                refresh(pair)?;
            }
        }
        let access = tokens.pair.access.clone();
        drop(tokens);
        with_token(retry, &access)
    }

    pub(super) fn json<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, String> {
        http::json(self.send(request)?, SERVER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        http_fixture::{fixture, ok},
        plugins::ServerKind,
    };
    use serde_json::{Value, json};

    fn credentials(server: String) -> Credentials {
        Credentials {
            kind: ServerKind::Audiobookshelf,
            server,
            token: "".into(),
            client_id: "test-device".into(),
        }
    }

    #[test]
    fn password_refreshes_and_retries_with_rotated_credentials() {
        let (server, worker) = fixture(vec![
            ok(
                "POST /login",
                json!({"user":{"token":"legacy","accessToken":"old-access","refreshToken":"old-refresh"}}),
            ),
            ("HEAD /api/items/book/file/1", 401, vec![]),
            ok(
                "POST /auth/refresh",
                json!({"user":{"accessToken":"new-access","refreshToken":"new-refresh"}}),
            ),
            ("HEAD /api/items/book/file/1", 200, vec![]),
        ]);
        let mut session = credentials(server);
        password(&mut session, " reader ", "test-password").unwrap();
        let api = Api::new(&session).unwrap();
        let updates = Arc::new(Mutex::new(vec![]));
        let saved = updates.clone();
        session
            .token
            .remember(
                |_| Ok(()),
                move |pair, refresh| {
                    assert_eq!(pair.refresh, "old-refresh");
                    refresh(pair)?;
                    saved
                        .lock()
                        .unwrap()
                        .push(serde_json::to_value(pair).unwrap());
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(
            api.playback_header("/api/items/book/file/1").unwrap(),
            "Authorization: Bearer new-access"
        );
        let requests = worker.join().unwrap();
        assert!(requests[0].headers.contains("x-return-tokens: true"));
        assert_eq!(
            requests[0].body,
            json!({"username":"reader","password":"test-password"})
        );
        assert!(
            requests[1]
                .headers
                .contains("authorization: Bearer old-access")
        );
        assert!(requests[2].headers.contains("x-refresh-token: old-refresh"));
        assert!(!requests[2].headers.contains("authorization:"));
        assert!(
            requests[3]
                .headers
                .contains("authorization: Bearer new-access")
        );
        assert_eq!(updates.lock().unwrap().len(), 1);
        let blob = serde_json::to_value(&session).unwrap();
        assert_eq!(blob["token"], updates.lock().unwrap()[0]);
        let restored: Credentials = serde_json::from_value(blob).unwrap();
        let pair = restored.token.snapshot().unwrap();
        assert_eq!(pair.access, "new-access");
        assert_eq!(pair.refresh, "new-refresh");
        assert_eq!(pair.revision, 1);
    }

    #[test]
    fn forgotten_sessions_still_refresh_without_writing_credentials() {
        let (server, worker) = fixture(vec![
            ("GET /api/libraries", 401, vec![]),
            ok(
                "POST /auth/refresh",
                json!({"user":{"accessToken":"new","refreshToken":"rotated"}}),
            ),
            ok("GET /api/libraries", Value::Null),
        ]);
        let mut session = credentials(server);
        session.token = SessionToken::pair("old".into(), "original".into());
        let api = Api::new(&session).unwrap();
        session
            .token
            .remember(
                |_| Ok(()),
                |_, _| panic!("forgotten sessions must not be saved"),
            )
            .unwrap();
        let login = session.token.snapshot().unwrap().login;
        session
            .token
            .forget(|old| {
                assert_eq!(old, Some(login));
                Ok(())
            })
            .unwrap();
        let _: Value = api.json(api.get("/api/libraries")).unwrap();
        assert_eq!(session.token.snapshot().unwrap().refresh, "rotated");
        worker.join().unwrap();
    }

    #[test]
    fn accepts_legacy_sign_in_and_rejects_invalid_tokens_and_redirects() {
        let (server, worker) = fixture(vec![
            ok("POST /login", json!({"user":{"token":"legacy-token"}})),
            ok(
                "POST /login",
                json!({"user":{"accessToken":"new","token":"stale"}}),
            ),
            ok("POST /login", json!({"user":{"token":"bad\r\nheader"}})),
            ("POST /login", 302, vec![]),
        ]);
        let mut session = credentials(server);
        password(&mut session, "reader", "test-password").unwrap();
        assert_eq!(session.token.fixed().unwrap(), "legacy-token");
        for _ in 0..3 {
            assert!(password(&mut session, "reader", "test-password").is_err());
        }
        worker.join().unwrap();
    }
}
