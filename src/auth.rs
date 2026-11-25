//! Plex's browser PIN flow. Passwords are entered only on Plex's own website.
use reqwest::{blocking::Client, header};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

pub enum Event {
    Browser(String),
    SignedIn(Vec<Server>),
    Failed(String),
}

// No Debug implementations: resources contain server access tokens.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub name: String,
    pub access_token: Option<String>,
    #[serde(default)]
    pub provides: String,
    #[serde(default)]
    pub connections: Vec<Connection>,
}

#[derive(Clone, Deserialize)]
pub struct Connection {
    pub uri: String,
    #[serde(default)]
    pub local: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pin {
    id: u64,
    code: String,
    #[serde(default)]
    auth_token: Option<String>,
    expires_in: Option<u64>,
}

pub struct Login {
    pub events: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
}

impl Login {
    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, mpsc::Sender<Event>) {
        let (tx, events) = mpsc::channel();
        (
            Self {
                events,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
            tx,
        )
    }

    pub fn start(client_id: String) -> Self {
        let (tx, events) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = cancelled.clone();
        thread::spawn(move || {
            let result = (|| {
                let http = client(&client_id, None)?;
                let pin: Pin = decode(
                    http.post("https://plex.tv/api/v2/pins")
                        .query(&[("strong", "true")])
                        .send(),
                )?;
                if pin.id == 0 || pin.code.is_empty() || pin.code.len() > 256 {
                    return Err(
                        "Plex returned an invalid sign-in code. Try signing in again.".into(),
                    );
                }
                if cancellation.load(Ordering::Relaxed) {
                    return Ok(());
                }
                let _ = tx.send(Event::Browser(browser_url(&client_id, &pin.code)));
                let deadline = Instant::now()
                    + Duration::from_secs(pin.expires_in.unwrap_or(900).clamp(1, 1800));
                while Instant::now() < deadline {
                    // Cancellation is prompt, including when the browser is left open.
                    for _ in 0..10 {
                        if cancellation.load(Ordering::Relaxed) {
                            return Ok(());
                        }
                        thread::sleep(Duration::from_millis(200));
                    }
                    let checked: Pin = decode(
                        http.get(format!("https://plex.tv/api/v2/pins/{}", pin.id))
                            .send(),
                    )?;
                    if let Some(token) = checked.auth_token.filter(|t| !t.is_empty()) {
                        let servers = resources(&client_id, &token)?;
                        if !cancellation.load(Ordering::Relaxed) {
                            let _ = tx.send(Event::SignedIn(servers));
                        }
                        return Ok(());
                    }
                }
                Err("Plex sign-in expired. Click Sign in with Plex to try again.".into())
            })();
            if let Err(error) = result {
                let _ = tx.send(Event::Failed(error));
            }
        });
        Self { events, cancelled }
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

fn browser_url(client_id: &str, code: &str) -> String {
    let query = reqwest::Url::parse_with_params(
        "https://app.plex.tv/auth",
        [
            ("clientID", client_id),
            ("code", code),
            ("context[device][product]", "Perfuse"),
        ],
    )
    .expect("constant Plex URL is valid");
    format!(
        "https://app.plex.tv/auth#?{}",
        query.query().unwrap_or_default()
    )
}

fn client(client_id: &str, token: Option<&str>) -> Result<Client, String> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        "X-Plex-Product",
        header::HeaderValue::from_static("Perfuse"),
    );
    headers.insert(
        "X-Plex-Version",
        header::HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
    );
    headers.insert(
        "X-Plex-Platform",
        header::HeaderValue::from_static("Windows"),
    );
    headers.insert(
        "X-Plex-Device-Name",
        header::HeaderValue::from_static("Perfuse for Windows"),
    );
    headers.insert(
        "X-Plex-Client-Identifier",
        header::HeaderValue::from_str(client_id)
            .map_err(|_| "Invalid device identifier.".to_owned())?,
    );
    if let Some(token) = token {
        let mut value = header::HeaderValue::from_str(token)
            .map_err(|_| "Plex returned an invalid token.".to_owned())?;
        value.set_sensitive(true);
        headers.insert("X-Plex-Token", value);
    }
    Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| "Could not initialize Plex sign-in networking.".into())
}

fn decode<T: DeserializeOwned>(
    response: Result<reqwest::blocking::Response, reqwest::Error>,
) -> Result<T, String> {
    let response = response.map_err(|_| {
        "Cannot reach Plex sign-in. Check your connection and try again.".to_owned()
    })?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            401 | 403 => "Plex sign-in was denied. Sign in again.".into(),
            404 | 410 => "Plex sign-in expired. Sign in again.".into(),
            429 => "Plex is rate limiting sign-in. Wait a moment and try again.".into(),
            _ => format!(
                "Plex sign-in returned HTTP {}. Try again.",
                response.status().as_u16()
            ),
        });
    }
    let bytes = crate::plex::read_limited(response, 2 * 1024 * 1024)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| "Plex returned an unexpected sign-in response.".into())
}

fn resources(client_id: &str, token: &str) -> Result<Vec<Server>, String> {
    let servers: Vec<Server> = decode(
        client(client_id, Some(token))?
            .get("https://clients.plex.tv/api/v2/resources")
            .query(&[("includeHttps", "1"), ("includeRelay", "0")])
            .send(),
    )?;
    Ok(usable_servers(servers, client_id))
}

fn usable_servers(servers: Vec<Server>, client_id: &str) -> Vec<Server> {
    servers
        .into_iter()
        .filter_map(|mut server| {
            if !server.provides.split(',').any(|p| p.trim() == "server") {
                return None;
            }
            let token = server.access_token.as_deref()?;
            server.connections.retain(|connection| {
                crate::plex::Plex::new(&connection.uri, token, client_id).is_ok()
            });
            // Prefer secure local connections, then secure remote connections. No endpoint probing.
            server
                .connections
                .sort_by_key(|c| (!c.uri.starts_with("https://"), !c.local));
            server.connections.dedup_by(|a, b| a.uri == b.uri);
            (!server.connections.is_empty()).then_some(server)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_url_and_resource_boundaries() {
        let url = reqwest::Url::parse(&browser_url("device & id", "code+&?=")).unwrap();
        assert_eq!(url.origin().ascii_serialization(), "https://app.plex.tv");
        assert!(url.query().is_none());
        let fragment = reqwest::Url::parse(&format!(
            "https://test/?{}",
            url.fragment().unwrap().trim_start_matches('?')
        ))
        .unwrap();
        let query: std::collections::HashMap<_, _> = fragment.query_pairs().collect();
        assert_eq!(query["clientID"], "device & id");
        assert_eq!(query["code"], "code+&?=");
        let servers = serde_json::from_str(r#"[
            {"name":"Local", "provides":"server", "accessToken":"synthetic", "connections":[
                {"uri":"file:///etc/passwd"}, {"uri":"https://example.test/?token=bad"},
                {"uri":"https://user:pass@example.test"}, {"uri":"http://127.0.0.1:32400", "local":true},
                {"uri":"https://secure.example.test", "local":true}]},
            {"name":"Client", "provides":"player", "accessToken":"synthetic", "connections":[]}
        ]"#).unwrap();
        let servers = usable_servers(servers, "test");
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].connections.len(), 2);
        assert_eq!(servers[0].connections[0].uri, "https://secure.example.test");
        let pin: Pin = serde_json::from_str(
            r#"{"id":42,"code":"synthetic","authToken":null,"expiresIn":300}"#,
        )
        .unwrap();
        assert_eq!(pin.id, 42);
        assert!(pin.auth_token.is_none());
    }
}
