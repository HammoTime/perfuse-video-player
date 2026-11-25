use reqwest::{Url, blocking::Client, header};
use serde::Deserialize;
use std::{io::Read, path::PathBuf, time::Duration};

pub const PAGE_SIZE: usize = 36;
const RELEASED_LIMIT: usize = 24;
const JSON_LIMIT: u64 = 16 * 1024 * 1024;

// Intentionally no Debug: the client owns an authentication token.
#[derive(Clone)]
pub struct Plex {
    base: Url,
    token: String,
    http: Client,
    cache: Option<PathBuf>,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    #[serde(default)]
    pub rating_key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    pub year: Option<u32>,
    /// Release or air date as `YYYY-MM-DD`.
    pub originally_available_at: Option<String>,
    pub index: Option<u32>,
    pub parent_index: Option<u32>,
    pub grandparent_title: Option<String>,
    #[serde(default)]
    pub summary: String,
    pub thumb: Option<String>,
    pub art: Option<String>,
    pub duration: Option<u64>,
    pub view_offset: Option<u64>,
    #[serde(default)]
    pub view_count: u32,
    #[serde(default, rename = "Media")]
    pub media: Vec<Media>,
}

impl Item {
    pub fn playable(&self) -> bool {
        matches!(self.kind.as_str(), "movie" | "episode")
    }

    pub fn subtitle(&self) -> String {
        if self.kind == "episode" {
            format!(
                "{} · S{:02} E{:02}",
                self.grandparent_title.as_deref().unwrap_or("Episode"),
                self.parent_index.unwrap_or(0),
                self.index.unwrap_or(0)
            )
        } else {
            self.year
                .map(|y| y.to_string())
                .unwrap_or_else(|| self.kind.clone())
        }
    }
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub video_resolution: Option<String>,
    pub container: Option<String>,
    #[serde(default, rename = "Part")]
    pub parts: Vec<Part>,
}

impl Media {
    pub fn label(&self) -> String {
        [
            self.video_resolution.as_deref(),
            self.video_codec.as_deref(),
            self.audio_codec.as_deref(),
            self.container.as_deref(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
        .to_uppercase()
    }
}

#[derive(Clone, Deserialize, Default)]
pub struct Part {
    pub key: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Section {
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Container {
    #[serde(default, rename = "Directory")]
    directories: Vec<Section>,
    #[serde(default, rename = "Metadata")]
    items: Vec<Item>,
    total_size: Option<usize>,
    offset: Option<usize>,
}

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "MediaContainer")]
    container: Container,
}

pub struct Page {
    pub items: Vec<Item>,
    pub offset: usize,
    pub total: Option<usize>,
}

impl Page {
    pub fn has_next(&self) -> bool {
        let end = self.offset + self.items.len();
        !self.items.is_empty()
            && self
                .total
                .map_or(self.items.len() >= PAGE_SIZE, |total| end < total)
    }
}

impl Plex {
    pub fn new(server: &str, token: &str, client_id: &str) -> Result<Self, String> {
        let base = Url::parse(server.trim()).map_err(|_| "Enter a valid server URL.".to_owned())?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err("Use a server origin, e.g. http://192.168.1.10:32400, without a path or credentials.".into());
        }
        let token = token.trim();
        if token.is_empty() {
            return Err("Enter your Plex token.".into());
        }
        let mut headers = header::HeaderMap::new();
        let mut secret = header::HeaderValue::from_str(token)
            .map_err(|_| "The token contains invalid header characters.".to_owned())?;
        secret.set_sensitive(true);
        headers.insert("X-Plex-Token", secret);
        headers.insert(
            "X-Plex-Product",
            header::HeaderValue::from_static("Perfuse"),
        );
        headers.insert(
            "X-Plex-Version",
            header::HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
        );
        headers.insert(
            "X-Plex-Client-Identifier",
            header::HeaderValue::from_str(client_id)
                .map_err(|_| "Invalid client identifier.".to_owned())?,
        );
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );
        let http = Client::builder()
            .default_headers(headers)
            // Never forward the Plex token to a redirect target.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| "Could not initialize networking.".to_owned())?;
        Ok(Self {
            base,
            token: token.to_owned(),
            http,
            cache: crate::cache::dir(),
        })
    }

    pub fn url(&self, path: &str) -> Result<Url, String> {
        let url = self
            .base
            .join(path)
            .map_err(|_| "Plex returned an invalid media path.".to_owned())?;
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err("Plex returned a path outside the connected server.".into());
        }
        Ok(url)
    }

    pub fn player_header(&self) -> String {
        format!("X-Plex-Token: {}", self.token)
    }

    pub fn timeline(&self, id: &str, state: &str, time: u64, duration: u64) -> Result<(), String> {
        let response = self
            .http
            .get(self.url("/:/timeline")?)
            .timeout(Duration::from_secs(4))
            .query(&[
                ("ratingKey", id.to_owned()),
                ("key", metadata_path(id)?),
                ("state", state.to_owned()),
                ("time", time.to_string()),
                ("duration", duration.to_string()),
            ])
            .send()
            .map_err(|_| "Playback progress could not reach Plex.".to_owned())?;
        check_status(response.status())
    }

    fn request(
        &self,
        path: &str,
        query: &[(&str, String)],
        offset: Option<usize>,
    ) -> Result<Container, String> {
        let mut request = self.http.get(self.url(path)?).query(query);
        if let Some(offset) = offset {
            request = request
                .header("X-Plex-Container-Start", offset)
                .header("X-Plex-Container-Size", PAGE_SIZE);
        }
        let response = request.send().map_err(|_| {
            "Cannot reach Plex. Check the address, network, and TLS certificate.".to_owned()
        })?;
        check_status(response.status())?;
        let bytes = read_limited(response, JSON_LIMIT)?;
        serde_json::from_slice::<Response>(&bytes)
            .map(|r| r.container)
            .map_err(|_| "Plex returned an unexpected response (expected JSON).".to_owned())
    }

    pub fn sections(&self) -> Result<Vec<Section>, String> {
        Ok(self
            .request("/library/sections", &[], None)?
            .directories
            .into_iter()
            .filter(|s| matches!(s.kind.as_str(), "movie" | "show"))
            .collect())
    }

    pub fn browse(&self, path: &str, search: &str, offset: usize) -> Result<Page, String> {
        let query = if search.trim().is_empty() {
            vec![]
        } else {
            vec![("title", search.trim().to_owned())]
        };
        let result = self.request(path, &query, Some(offset))?;
        if offset > 0 && result.offset.unwrap_or(0) != offset {
            return Err(
                "Plex did not honor the requested page. Refresh the library to start again.".into(),
            );
        }
        Ok(Page {
            offset: result.offset.unwrap_or(0),
            total: result.total_size,
            items: result.items,
        })
    }

    /// Newest movies and episodes by release date across the given libraries.
    pub fn recently_released(&self, sections: &[Section]) -> Result<Vec<Item>, String> {
        let mut items = vec![];
        for section in sections {
            // Episodes rather than shows, so new episodes of older series appear.
            let kind = if section.kind == "show" { "4" } else { "1" };
            let query = [
                ("type", kind.to_owned()),
                ("sort", "originallyAvailableAt:desc".to_owned()),
            ];
            let result = self.request(&section_path(&section.key)?, &query, Some(0))?;
            items.extend(
                result
                    .items
                    .into_iter()
                    .filter(|item| item.playable() && item.originally_available_at.is_some()),
            );
        }
        // ISO dates sort chronologically as strings.
        items.sort_by(|a, b| b.originally_available_at.cmp(&a.originally_available_at));
        items.truncate(RELEASED_LIMIT);
        Ok(items)
    }

    pub fn metadata(&self, id: &str) -> Result<Item, String> {
        self.request(&metadata_path(id)?, &[], None)?
            .items
            .into_iter()
            .next()
            .ok_or_else(|| "This item is no longer available.".to_owned())
    }

    /// Artwork from the local disk cache only, without a network request.
    pub fn cached_artwork(&self, path: &str) -> Option<image::RgbaImage> {
        let dir = self.cache.as_deref()?;
        let bytes = crate::cache::read(dir, self.base.as_str(), path)?;
        let image = decode_artwork(&bytes);
        if image.is_err() {
            crate::cache::remove(dir, self.base.as_str(), path);
        }
        image.ok()
    }

    /// Fetches artwork from the server and stores it in the disk cache.
    pub fn artwork(&self, path: &str) -> Result<image::RgbaImage, String> {
        let source = self.url(path)?;
        let backdrop = source.path().split('/').any(|part| part == "art");
        let response = self
            .http
            .get(self.url("/photo/:/transcode")?)
            .query(&[
                ("url", source.path()),
                ("width", if backdrop { "1280" } else { "360" }),
                ("height", if backdrop { "720" } else { "540" }),
                ("minSize", "1"),
            ])
            .send()
            .map_err(|_| "Artwork unavailable.".to_owned())?;
        check_status(response.status())?;
        let bytes = read_limited(response, 8 * 1024 * 1024)?;
        let image = decode_artwork(&bytes)?;
        if let Some(dir) = &self.cache {
            crate::cache::write(dir, self.base.as_str(), path, &bytes);
        }
        Ok(image)
    }
}

fn decode_artwork(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "Unknown artwork format.".to_owned())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    reader
        .decode()
        .map(|image| image.into_rgba8())
        .map_err(|_| "Cannot decode artwork.".to_owned())
}

pub fn metadata_path(id: &str) -> Result<String, String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Plex returned an invalid item identifier.".into());
    }
    Ok(format!("/library/metadata/{id}"))
}

pub fn section_path(id: &str) -> Result<String, String> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Plex returned an invalid library identifier.".into());
    }
    Ok(format!("/library/sections/{id}/all"))
}

fn check_status(status: reqwest::StatusCode) -> Result<(), String> {
    if status.is_success() {
        Ok(())
    } else if matches!(status.as_u16(), 401 | 403) {
        Err("Plex denied access. Check your token and library permissions.".into())
    } else if status.is_redirection() {
        Err("Plex redirected the request. Connect to the final server address directly.".into())
    } else {
        Err(format!("Plex returned HTTP {}.", status.as_u16()))
    }
}

pub(crate) fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The Plex response was interrupted.".to_owned())?;
    if bytes.len() as u64 > limit {
        return Err("The Plex response exceeded the size limit.".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_trust_boundaries_and_decodes_plex() {
        let plex = Plex::new("http://localhost:32400", "secret", "test-client").unwrap();
        assert!(plex.url("/library/parts/1/file.mkv").is_ok());
        for path in [
            "https://evil.test/file",
            "//evil.test/file",
            "http://user@localhost:32400/file",
        ] {
            assert!(plex.url(path).is_err());
        }
        assert!(Plex::new("file:///tmp/test", "secret", "test").is_err());
        assert!(Plex::new("http://localhost:32400/?token=secret", "secret", "test").is_err());
        assert!(Plex::new("http://localhost:32400", "bad\r\nHeader: value", "test").is_err());
        assert!(metadata_path("../1").is_err());
        assert!(section_path("1?token=x").is_err());
        let response: Response = serde_json::from_str(r#"{"MediaContainer":{"offset":36,"totalSize":80,"Metadata":[{"ratingKey":"42","title":"Test","type":"movie","Media":[{"videoCodec":"hevc","Part":[{"key":"/library/parts/4/file.mkv"}]}]}]}}"#).unwrap();
        assert!(response.container.items[0].playable());
        assert_eq!(
            response.container.items[0].media[0].parts[0].key.as_deref(),
            Some("/library/parts/4/file.mkv")
        );
        let page = Page {
            items: response.container.items,
            offset: 36,
            total: Some(80),
        };
        assert!(page.has_next());
        assert!(read_limited(&b"1234"[..], 3).is_err());
    }

    #[test]
    fn http_auth_pagination_and_redirects() {
        use std::{io::Write, net::TcpListener, thread};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for index in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    assert_eq!(stream.read(&mut byte).unwrap(), 1);
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(request.contains("x-plex-token: test-secret\r\n"));
                assert!(request.contains("x-plex-client-identifier: integration\r\n"));
                assert!(!request.lines().next().unwrap().contains("test-secret"));
                if index == 0 {
                    assert!(request.starts_with("get /library/sections/1/all?title=quiet+orbit "));
                    assert!(request.contains("x-plex-container-start: 36\r\n"));
                    assert!(request.contains("x-plex-container-size: 36\r\n"));
                    let body = r#"{"MediaContainer":{"offset":36,"totalSize":37,"Metadata":[{"ratingKey":"37","title":"Quiet Orbit","type":"movie"}]}}"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                } else if index == 1 {
                    write!(stream, "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/leak\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                } else {
                    write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                }
            }
        });
        let plex = Plex::new(&format!("http://{address}"), "test-secret", "integration").unwrap();
        let page = plex
            .browse("/library/sections/1/all", "quiet orbit", 36)
            .unwrap();
        assert_eq!(page.offset, 36);
        assert_eq!(page.items[0].title, "Quiet Orbit");
        assert!(!page.has_next());
        assert!(plex.sections().unwrap_err().contains("redirected"));
        assert!(plex.sections().unwrap_err().contains("denied access"));
        server.join().unwrap();
    }

    #[test]
    fn recently_released_merges_libraries_by_date() {
        use std::{io::Write, net::TcpListener, thread};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for expected in [
                "get /library/sections/1/all?type=1&sort=originallyavailableat%3adesc ",
                "get /library/sections/2/all?type=4&sort=originallyavailableat%3adesc ",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    assert_eq!(stream.read(&mut byte).unwrap(), 1);
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(request.starts_with(expected), "{request}");
                let body = if expected.contains("/1/") {
                    r#"{"MediaContainer":{"Metadata":[{"ratingKey":"1","title":"New Film","type":"movie","originallyAvailableAt":"2026-11-01"},{"ratingKey":"2","title":"Undated","type":"movie"}]}}"#
                } else {
                    r#"{"MediaContainer":{"Metadata":[{"ratingKey":"3","title":"Newer Episode","type":"episode","originallyAvailableAt":"2026-11-20"}]}}"#
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        let plex = Plex::new(&format!("http://{address}"), "test-secret", "integration").unwrap();
        let sections = [("1", "movie"), ("2", "show")].map(|(key, kind)| Section {
            key: key.into(),
            title: String::new(),
            kind: kind.into(),
        });
        let titles: Vec<_> = plex
            .recently_released(&sections)
            .unwrap()
            .into_iter()
            .map(|item| item.title)
            .collect();
        assert_eq!(titles, ["Newer Episode", "New Film"]);
        server.join().unwrap();
    }
}
