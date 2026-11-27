//! Audiobookshelf's native-client login and rotating bearer credentials.
use crate::{
    http,
    media::{SourceError, SourceResult},
    plugins::{Credentials, SessionToken, SignInMethod, SignInRequest, TokenPair},
};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, RequestBuilder, Response},
    header::{AUTHORIZATION, HeaderValue},
};
use serde::{Deserialize, de::DeserializeOwned};
use std::time::Duration;

pub(super) const SERVER: &str = "Audiobookshelf";
const INVALID: &str = "Invalid Audiobookshelf sign-in. Sign in again.";

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
    fn pair(self) -> SourceResult<(String, String)> {
        match (self.access_token, self.refresh_token) {
            (Some(access), Some(refresh)) if valid(&access) && valid(&refresh) => {
                Ok((access, refresh))
            }
            _ => Err(SourceError::Unauthorized(INVALID.into())),
        }
    }
}

fn send(request: RequestBuilder) -> SourceResult<Response> {
    http::send(request, SERVER)
}

fn sensitive(value: &str) -> SourceResult<HeaderValue> {
    let mut header = HeaderValue::from_str(value).map_err(|_| INVALID.to_owned())?;
    header.set_sensitive(true);
    Ok(header)
}

pub(super) fn authenticate(
    session: &mut Credentials,
    request: &SignInRequest<'_>,
) -> SourceResult<()> {
    match request.method {
        SignInMethod::Password { .. } => password(session, request.username, request.password),
        _ => Err(SourceError::Unsupported(
            "Audiobookshelf does not offer that sign-in.".into(),
        )),
    }
}

pub fn password(session: &mut Credentials, username: &str, password: &str) -> SourceResult<()> {
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
        SessionToken::rotating(access, refresh)
    };
    Ok(())
}

pub(super) struct Api {
    pub(super) base: Url,
    client: Client,
    pub(super) token: SessionToken,
}

impl Api {
    pub(super) fn new(session: &Credentials) -> SourceResult<Self> {
        let base = http::base_url(&session.server)?;
        if !valid(&session.token.access())
            || session
                .token
                .refresh_token()
                .is_some_and(|refresh| !valid(&refresh))
        {
            return Err(SourceError::Unauthorized(INVALID.into()));
        }
        Ok(Self {
            base,
            client: http::client(Default::default())?,
            token: session.token.clone(),
        })
    }

    pub(super) fn url(&self, path: &str) -> Url {
        http::endpoint(&self.base, path)
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

    pub(super) fn playback_header(
        &self,
        path: &str,
    ) -> SourceResult<(reqwest::header::HeaderName, HeaderValue)> {
        // A book can be opened after a long pause. Renew an expired bearer before
        // handing the media URL to libmpv, which cannot perform a token exchange.
        if self.token.is_rotating() {
            let request = self
                .client
                .head(self.url(path))
                .timeout(Duration::from_secs(8));
            http::check_status(self.send(request)?.status(), SERVER)?;
        }
        crate::media::Playback::header("authorization", &format!("Bearer {}", self.token.access()))
    }

    pub(super) fn send(&self, request: RequestBuilder) -> SourceResult<Response> {
        let access = self.token.access();
        let with_token = |req: RequestBuilder, token: &str| {
            send(req.header(AUTHORIZATION, sensitive(&format!("Bearer {token}"))?))
        };
        let retry = request.try_clone();
        let response = with_token(request, &access)?;
        if response.status() != StatusCode::UNAUTHORIZED || !self.token.is_rotating() {
            return Ok(response);
        }
        let Some(retry) = retry else {
            return Ok(response);
        };
        drop(response);
        // Multiple workers may observe the same 401. The token rotates exactly once.
        let access = self.token.rotate(&access, &mut |pair: &mut TokenPair| {
            let response = send(
                self.post("/auth/refresh")
                    .header("x-refresh-token", sensitive(&pair.refresh)?),
            )?;
            (pair.access, pair.refresh) = http::json::<Login>(response, SERVER)?.user.pair()?;
            pair.revision = pair.revision.saturating_add(1);
            Ok(())
        })?;
        with_token(retry, &access)
    }

    pub(super) fn json<T: DeserializeOwned>(&self, request: RequestBuilder) -> SourceResult<T> {
        http::json(self.send(request)?, SERVER)
    }

    /// Ends the session behind the refresh token, so neither token can be renewed.
    pub(super) fn sign_out(&self) -> SourceResult<()> {
        let Some(refresh) = self.token.refresh_token() else {
            return Ok(());
        };
        let response = send(
            self.post("/logout")
                .timeout(Duration::from_secs(4))
                .header("x-refresh-token", sensitive(&refresh)?),
        )?;
        http::check_status(response.status(), SERVER)
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
    use std::sync::{Arc, Mutex};

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
        let (name, value) = api.playback_header("/api/items/book/file/1").unwrap();
        assert_eq!(name, "authorization");
        assert_eq!(value.to_str().unwrap(), "Bearer new-access");
        assert!(value.is_sensitive());
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
        session.token = SessionToken::rotating("old".into(), "original".into());
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
    fn a_rejected_refresh_reports_an_expired_sign_in() {
        let (server, worker) = fixture(vec![
            ("GET /api/libraries", 401, vec![]),
            ("POST /auth/refresh", 401, vec![]),
        ]);
        let mut session = credentials(server);
        session.token = SessionToken::rotating("old".into(), "revoked".into());
        let api = Api::new(&session).unwrap();
        let error = api.json::<Value>(api.get("/api/libraries")).unwrap_err();
        assert!(error.is_unauthorized());
        worker.join().unwrap();
    }

    #[test]
    fn sign_out_ends_the_refresh_session() {
        let (server, worker) = fixture(vec![("POST /logout", 200, vec![])]);
        let mut session = credentials(server);
        session.token = SessionToken::rotating("access".into(), "refresh".into());
        Api::new(&session).unwrap().sign_out().unwrap();
        let requests = worker.join().unwrap();
        assert!(requests[0].headers.contains("x-refresh-token: refresh"));
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
