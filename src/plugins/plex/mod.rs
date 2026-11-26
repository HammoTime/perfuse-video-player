pub mod auth;

use super::{Plugin, ServerKind};
use crate::{
    cache::Artwork,
    http,
    media::{
        Browse, Item, Kind, Library, MediaSource, PAGE_SIZE, Page, Playback, PlaybackState,
        Progress, Stream, Version,
    },
};
use reqwest::{Url, blocking::Client, header};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};

const RELEASED_LIMIT: usize = 24;

pub static PLUGIN: Plugin = Plugin {
    kind: ServerKind::Plex,
    id: "plex",
    name: "Plex",
    default_address: "http://localhost:32400",
    open: |credentials, cache_artwork| {
        Ok(Arc::new(Plex::new(
            &credentials.server,
            &credentials.token,
            &credentials.client_id,
            cache_artwork,
        )?))
    },
};

// Intentionally no Debug: the client owns an authentication token.
struct Plex {
    base: Url,
    token: String,
    http: Client,
    artwork: Artwork,
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PlexItem {
    #[serde(default)]
    rating_key: String,
    #[serde(rename = "librarySectionID")]
    library_section_id: Option<serde_json::Value>,
    last_viewed_at: Option<u64>,
    #[serde(default)]
    title: String,
    #[serde(default, rename = "type")]
    kind: String,
    year: Option<u32>,
    /// Release or air date as `YYYY-MM-DD`.
    originally_available_at: Option<String>,
    index: Option<u32>,
    parent_index: Option<u32>,
    grandparent_title: Option<String>,
    #[serde(default)]
    summary: String,
    thumb: Option<String>,
    art: Option<String>,
    duration: Option<u64>,
    view_offset: Option<u64>,
    #[serde(default)]
    view_count: u32,
    #[serde(default, rename = "Media")]
    media: Vec<Media>,
}

fn kind(value: &str) -> Kind {
    match value {
        "movie" => Kind::Movie,
        "show" => Kind::Series,
        "season" => Kind::Season,
        "episode" => Kind::Episode,
        _ => Kind::Other,
    }
}

impl From<PlexItem> for Item {
    fn from(item: PlexItem) -> Self {
        Self {
            id: item.rating_key,
            library_id: item.library_section_id.and_then(|v| match v {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            }),
            last_viewed_at: item.last_viewed_at,
            title: item.title,
            kind: kind(&item.kind),
            year: item.year,
            release_date: item.originally_available_at,
            index: item.index,
            season_number: item.parent_index,
            series_title: item.grandparent_title,
            summary: item.summary,
            thumb: item.thumb,
            art: item.art,
            duration_ms: item.duration,
            resume_ms: item.view_offset,
            watched: item.view_count > 0,
            versions: item.media.into_iter().map(Version::from).collect(),
        }
    }
}

#[derive(Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Media {
    video_codec: Option<String>,
    audio_codec: Option<String>,
    video_resolution: Option<String>,
    container: Option<String>,
    #[serde(default, rename = "Part")]
    parts: Vec<Part>,
}

impl From<Media> for Version {
    fn from(media: Media) -> Self {
        let stream = match media.parts.len() {
            0 => Stream::Unavailable,
            1 => media
                .parts
                .into_iter()
                .next()
                .and_then(|part| part.key)
                .map_or(Stream::Unavailable, Stream::Direct),
            _ => Stream::Multipart,
        };
        Self {
            video_codec: media.video_codec,
            audio_codec: media.audio_codec,
            video_resolution: media.video_resolution,
            container: media.container,
            stream,
        }
    }
}

#[derive(Clone, Deserialize, Default)]
struct Part {
    key: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct Section {
    key: String,
    title: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Container {
    #[serde(default, rename = "Directory")]
    directories: Vec<Section>,
    #[serde(default, rename = "Metadata")]
    items: Vec<PlexItem>,
    total_size: Option<usize>,
    offset: Option<usize>,
}

#[derive(Deserialize)]
struct Response {
    #[serde(rename = "MediaContainer")]
    container: Container,
}

impl Plex {
    fn new(
        server: &str,
        token: &str,
        client_id: &str,
        cache_artwork: bool,
    ) -> Result<Self, String> {
        let base = http::base_url(server)?;
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
        Ok(Self {
            base,
            token: token.to_owned(),
            http: http::client(headers)?,
            artwork: Artwork::new(cache_artwork),
        })
    }

    fn url(&self, path: &str) -> Result<Url, String> {
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
        http::json::<Response>(response, "Plex").map(|r| r.container)
    }
}

impl MediaSource for Plex {
    fn address(&self) -> &str {
        self.base.as_str()
    }

    fn libraries(&self) -> Result<Vec<Library>, String> {
        Ok(self
            .request("/library/sections", &[], None)?
            .directories
            .into_iter()
            .filter(|s| matches!(s.kind.as_str(), "movie" | "show"))
            .map(|s| Library {
                id: s.key,
                title: s.title,
                kind: kind(&s.kind),
            })
            .collect())
    }

    fn browse(&self, scope: &Browse, search: &str, offset: usize) -> Result<Page, String> {
        let path = match scope {
            Browse::RecentlyAdded => "/library/recentlyAdded".to_owned(),
            Browse::ContinueWatching => "/library/onDeck".to_owned(),
            Browse::Library(library) => section_path(&library.id)?,
            Browse::Children(id) => format!("{}/children", metadata_path(id)?),
        };
        let query = if search.trim().is_empty() {
            vec![]
        } else {
            vec![("title", search.trim().to_owned())]
        };
        let result = self.request(&path, &query, Some(offset))?;
        if offset > 0 && result.offset.unwrap_or(0) != offset {
            return Err(
                "Plex did not honor the requested page. Refresh the library to start again.".into(),
            );
        }
        Ok(Page {
            offset: result.offset.unwrap_or(0),
            total: result.total_size,
            items: result.items.into_iter().map(Item::from).collect(),
        })
    }

    /// Newest movies and episodes by release date across the given libraries.
    fn recently_released(&self, sections: &[Library]) -> Result<Vec<Item>, String> {
        let mut items = vec![];
        for section in sections {
            // Episodes rather than shows, so new episodes of older series appear.
            let kind = if section.kind == Kind::Series {
                "4"
            } else {
                "1"
            };
            let query = [
                ("type", kind.to_owned()),
                ("sort", "originallyAvailableAt:desc".to_owned()),
            ];
            let result = self.request(&section_path(&section.id)?, &query, Some(0))?;
            items.extend(
                result
                    .items
                    .into_iter()
                    .map(Item::from)
                    .filter(|item| item.playable() && item.release_date.is_some()),
            );
        }
        // ISO dates sort chronologically as strings.
        items.sort_by(|a, b| b.release_date.cmp(&a.release_date));
        items.truncate(RELEASED_LIMIT);
        Ok(items)
    }

    fn item(&self, id: &str) -> Result<Item, String> {
        self.request(&metadata_path(id)?, &[], None)?
            .items
            .into_iter()
            .next()
            .map(Item::from)
            .ok_or_else(|| "This item is no longer available.".to_owned())
    }

    /// Artwork from the local disk cache only, without a network request.
    fn cached_artwork(&self, path: &str) -> Option<image::RgbaImage> {
        self.artwork.cached(self.address(), path)
    }

    /// Fetches artwork from the server and stores it in the disk cache.
    fn artwork(&self, path: &str) -> Result<image::RgbaImage, String> {
        let source = self.url(path)?;
        let backdrop = source.path().split('/').any(|part| part == "art");
        let request = self.http.get(self.url("/photo/:/transcode")?).query(&[
            ("url", source.path()),
            ("width", if backdrop { "1280" } else { "360" }),
            ("height", if backdrop { "720" } else { "540" }),
            ("minSize", "1"),
        ]);
        self.artwork.fetch(self.address(), path, request)
    }

    fn playback(&self, _: &Item, version: &Version) -> Result<Playback, String> {
        let url = self.url(version.stream.direct_file()?)?;
        if !url.path().starts_with("/library/parts/")
            || url
                .query_pairs()
                .any(|(key, _)| key.eq_ignore_ascii_case("X-Plex-Token"))
        {
            return Err("Plex did not return a library media file.".into());
        }
        Ok(Playback {
            url,
            headers: vec![format!("X-Plex-Token: {}", self.token)],
        })
    }

    fn report(&self, item: &Item, _: &Version, progress: Progress) -> Result<(), String> {
        let state = match progress.state {
            PlaybackState::Started => return Ok(()),
            PlaybackState::Playing => "playing",
            PlaybackState::Paused => "paused",
            PlaybackState::Stopped => "stopped",
        };
        let response = self
            .http
            .get(self.url("/:/timeline")?)
            .timeout(Duration::from_secs(4))
            .query(&[
                ("ratingKey", item.id.clone()),
                ("key", metadata_path(&item.id)?),
                ("state", state.to_owned()),
                ("time", progress.position_ms.to_string()),
                ("duration", progress.duration_ms.to_string()),
            ])
            .send()
            .map_err(|_| "Playback progress could not reach Plex.".to_owned())?;
        http::check_status(response.status(), "Plex")
    }
}

fn metadata_path(id: &str) -> Result<String, String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Plex returned an invalid item identifier.".into());
    }
    Ok(format!("/library/metadata/{id}"))
}

fn section_path(id: &str) -> Result<String, String> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Plex returned an invalid library identifier.".into());
    }
    Ok(format!("/library/sections/{id}/all"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn validates_trust_boundaries_and_decodes_plex() {
        let plex = Plex::new("http://localhost:32400", "secret", "test-client", false).unwrap();
        assert!(plex.url("/library/parts/1/file.mkv").is_ok());
        for path in [
            "https://evil.test/file",
            "//evil.test/file",
            "http://user@localhost:32400/file",
        ] {
            assert!(plex.url(path).is_err());
        }
        assert!(Plex::new("file:///tmp/test", "secret", "test", false).is_err());
        assert!(
            Plex::new(
                "http://localhost:32400/?token=secret",
                "secret",
                "test",
                false
            )
            .is_err()
        );
        assert!(
            Plex::new(
                "http://localhost:32400",
                "bad\r\nHeader: value",
                "test",
                false
            )
            .is_err()
        );
        assert!(metadata_path("../1").is_err());
        assert!(section_path("1?token=x").is_err());
        let response: Response = serde_json::from_str(r#"{"MediaContainer":{"offset":36,"totalSize":80,"Metadata":[{"ratingKey":"42","title":"Test","type":"movie","Media":[{"videoCodec":"hevc","Part":[{"key":"/library/parts/4/file.mkv"}]}]}]}}"#).unwrap();
        let page = Page {
            items: response
                .container
                .items
                .into_iter()
                .map(Item::from)
                .collect(),
            offset: 36,
            total: Some(80),
        };
        assert!(page.items[0].playable());
        assert_eq!(
            page.items[0].versions[0].stream.direct_file(),
            Ok("/library/parts/4/file.mkv")
        );
        assert!(page.has_next());
        assert!(http::read_limited(&b"1234"[..], 3).is_err());
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
        let plex = Plex::new(
            &format!("http://{address}"),
            "test-secret",
            "integration",
            false,
        )
        .unwrap();
        let library = Library {
            id: "1".into(),
            title: "Movies".into(),
            kind: Kind::Movie,
        };
        let page = plex
            .browse(&Browse::Library(library), "quiet orbit", 36)
            .unwrap();
        assert_eq!(page.offset, 36);
        assert_eq!(page.items[0].title, "Quiet Orbit");
        assert!(!page.has_next());
        assert!(plex.libraries().unwrap_err().contains("redirected"));
        assert!(plex.libraries().unwrap_err().contains("denied access"));
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
        let plex = Plex::new(
            &format!("http://{address}"),
            "test-secret",
            "integration",
            false,
        )
        .unwrap();
        let sections = [("1", Kind::Movie), ("2", Kind::Series)].map(|(id, kind)| Library {
            id: id.into(),
            title: String::new(),
            kind,
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
