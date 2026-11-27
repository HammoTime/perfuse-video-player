//! Jellyfin movie and TV libraries. Only identifiers cross into playback and artwork URLs.
pub mod auth;
#[cfg(test)]
mod tests;

use super::{Credentials, OpenOptions, Plugin, ServerKind, SignInMethod, SignInRequest};
use crate::{cache::Artwork, http, media::*, plugins::credentials::lock};
use reqwest::{
    Url,
    blocking::{Client, RequestBuilder, Response},
    header,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const SERVER: &str = "Jellyfin";

pub static PLUGIN: Plugin = Plugin {
    kind: ServerKind::Jellyfin,
    id: "jellyfin",
    name: SERVER,
    default_address: "http://localhost:8096",
    address_example: "http://192.168.1.30:8096",
    description: "Movies and TV. Use your server address and Jellyfin account.",
    libraries: &[LibraryKind::Movies, LibraryKind::Shows],
    sign_in: &[
        SignInMethod::Password { required: false },
        SignInMethod::QuickConnect,
    ],
    authenticate,
    open: |session, options: OpenOptions| {
        Ok(Arc::new(Jellyfin {
            api: Api::new(
                &session.server,
                &session.client_id,
                Some(session.token.fixed()?),
            )?,
            artwork: Artwork::new(options.cache_artwork),
            views: Mutex::default(),
        }))
    },
};

fn authenticate(session: &mut Credentials, request: &SignInRequest<'_>) -> SourceResult<()> {
    let token = match request.method {
        SignInMethod::Password { .. } => auth::password(
            &session.server,
            &session.client_id,
            request.username,
            request.password,
        )?,
        SignInMethod::QuickConnect => auth::quick_connect(
            &session.server,
            &session.client_id,
            request.show_code,
            request.cancelled,
        )?,
        _ => {
            return Err(SourceError::Unsupported(
                "Jellyfin does not offer that sign-in.".into(),
            ));
        }
    };
    session.token = token.into();
    Ok(())
}

const TICKS_PER_MS: u64 = 10_000;
/// How long Home reuses the library list to tag items with their library.
const VIEWS_TTL: Duration = Duration::from_secs(15 * 60);

// No Debug: the MediaBrowser header authenticates both API requests and libmpv.
struct Api {
    base: Url,
    http: Client,
    authorization: String,
}

impl Api {
    fn new(server: &str, device: &str, token: Option<&str>) -> Result<Self, String> {
        let base = http::base_url(server)?;
        if device.len() > 200 || !header_parameter(device) {
            return Err("Invalid Jellyfin device identifier.".into());
        }
        let mut authorization = format!(
            r#"MediaBrowser Client="Perfuse", Device="Windows", DeviceId="{device}", Version="{}""#,
            env!("CARGO_PKG_VERSION")
        );
        if let Some(token) = token {
            if token.len() > 2048 || !header_parameter(token) {
                return Err("Jellyfin returned an invalid access token. Sign in again.".into());
            }
            authorization.push_str(&format!(r#", Token="{token}""#));
        }
        let mut value = header::HeaderValue::from_str(&authorization)
            .map_err(|_| "Invalid Jellyfin authorization header.".to_owned())?;
        value.set_sensitive(true);
        let mut headers = header::HeaderMap::new();
        headers.insert(header::AUTHORIZATION, value);
        Ok(Self {
            base,
            http: http::client(headers)?,
            authorization,
        })
    }

    fn url(&self, path: &str) -> Url {
        http::endpoint(&self.base, path)
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.http.get(self.url(path))
    }

    fn post(&self, path: &str) -> RequestBuilder {
        self.http.post(self.url(path))
    }
}

fn header_parameter(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b'"' | b'\\' | b','))
}

fn send(request: RequestBuilder) -> SourceResult<Response> {
    http::send(request, SERVER)
}

fn json<T: DeserializeOwned>(request: RequestBuilder) -> SourceResult<T> {
    http::json(send(request)?, SERVER)
}

fn identifier(id: &str) -> SourceResult<&str> {
    super::identifier(id, SERVER)
}

struct Jellyfin {
    api: Api,
    artwork: Artwork,
    // The last library list, which Home uses to tag Continue Watching and Recently Added.
    views: Mutex<Option<(Instant, Vec<Library>)>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Listing<T> {
    items: Vec<T>,
    start_index: Option<usize>,
    total_record_count: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct View {
    id: String,
    name: String,
    collection_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Entry {
    id: String,
    name: String,
    #[serde(rename = "Type")]
    kind: String,
    production_year: Option<u32>,
    premiere_date: Option<String>,
    index_number: Option<u32>,
    parent_index_number: Option<u32>,
    series_name: Option<String>,
    overview: Option<String>,
    run_time_ticks: Option<u64>,
    user_data: Option<UserData>,
    image_tags: Option<ImageTags>,
    backdrop_image_tags: Option<Vec<String>>,
    parent_backdrop_item_id: Option<String>,
    parent_backdrop_image_tags: Option<Vec<String>>,
    series_id: Option<String>,
    series_primary_image_tag: Option<String>,
    media_sources: Option<Vec<MediaFile>>,
    part_count: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct UserData {
    playback_position_ticks: Option<u64>,
    last_played_date: Option<String>,
    #[serde(default)]
    played: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ImageTags {
    primary: Option<String>,
}

fn image_ref(id: &str, kind: &str, tag: Option<&String>) -> Option<String> {
    Some(format!(
        "{}/{kind}/{}",
        identifier(id).ok()?,
        identifier(tag?).ok()?
    ))
}

impl Entry {
    fn into_item(self, library_id: Option<&str>) -> SourceResult<Item> {
        identifier(&self.id)?;
        let thumb = image_ref(
            &self.id,
            "Primary",
            self.image_tags.as_ref().and_then(|t| t.primary.as_ref()),
        )
        .or_else(|| {
            image_ref(
                self.series_id.as_deref()?,
                "Primary",
                self.series_primary_image_tag.as_ref(),
            )
        });
        let art = image_ref(
            &self.id,
            "Backdrop",
            self.backdrop_image_tags
                .as_ref()
                .and_then(|tags| tags.first()),
        )
        .or_else(|| {
            image_ref(
                self.parent_backdrop_item_id.as_deref()?,
                "Backdrop",
                self.parent_backdrop_image_tags.as_ref()?.first(),
            )
        });
        let user = self.user_data.unwrap_or_default();
        Ok(Item {
            book: Book::default(),
            id: self.id,
            title: self.name,
            kind: match self.kind.as_str() {
                "Movie" => Kind::Movie,
                "Series" => Kind::Series,
                "Season" => Kind::Season,
                "Episode" => Kind::Episode,
                _ => Kind::Other,
            },
            library_id: library_id.map(str::to_owned),
            last_viewed_at: user
                .last_played_date
                .as_deref()
                .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
                .and_then(|date| u64::try_from(date.timestamp()).ok()),
            year: self.production_year,
            release_date: self
                .premiere_date
                .and_then(|date| date.get(..10).map(str::to_owned)),
            index: self.index_number,
            season_number: self.parent_index_number,
            series_title: self.series_name,
            summary: self.overview.unwrap_or_default(),
            thumb,
            art,
            duration_ms: self.run_time_ticks.map(|t| t / TICKS_PER_MS),
            resume_ms: user.playback_position_ticks.map(|t| t / TICKS_PER_MS),
            watched: user.played,
            versions: self
                .media_sources
                .unwrap_or_default()
                .into_iter()
                .map(|file| file.version(self.part_count.unwrap_or(1) > 1))
                .collect(),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MediaFile {
    id: Option<String>,
    container: Option<String>,
    media_streams: Option<Vec<Track>>,
    supports_direct_play: Option<bool>,
    video_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Track {
    #[serde(rename = "Type")]
    kind: String,
    codec: Option<String>,
    height: Option<u32>,
}

impl MediaFile {
    fn version(self, multipart: bool) -> Version {
        let tracks = self.media_streams.unwrap_or_default();
        let video = tracks.iter().find(|s| s.kind == "Video");
        let audio = tracks.iter().find(|s| s.kind == "Audio");
        let stream = if multipart {
            Stream::Multipart
        } else if self.supports_direct_play == Some(false)
            || self.video_type.as_deref().is_some_and(|t| t != "VideoFile")
        {
            Stream::Unavailable
        } else {
            self.id
                .filter(|id| identifier(id).is_ok())
                .map_or(Stream::Unavailable, Stream::Direct)
        };
        Version {
            stream,
            container: self.container,
            video_codec: video.and_then(|s| s.codec.clone()),
            audio_codec: audio.and_then(|s| s.codec.clone()),
            video_resolution: video.and_then(|s| s.height).map(|h| format!("{h}p")),
        }
    }
}

impl Jellyfin {
    fn page(
        &self,
        request: RequestBuilder,
        search: &str,
        offset: usize,
        library: Option<&str>,
    ) -> SourceResult<Page> {
        let mut request = request
            .query(&[("startIndex", offset), ("limit", PAGE_SIZE)])
            .query(&[("fields", "Overview"), ("enableUserData", "true")]);
        if !search.trim().is_empty() {
            request = request.query(&[("searchTerm", search.trim())]);
        }
        let result: Listing<Entry> = json(request)?;
        if result.start_index.unwrap_or(0) != offset || result.items.len() > PAGE_SIZE {
            return Err("Jellyfin did not return the requested page. Refresh the library.".into());
        }
        Ok(Page::new(
            result
                .items
                .into_iter()
                .map(|item| item.into_item(library))
                .collect::<Result<_, _>>()?,
            offset,
            result.total_record_count,
        ))
    }

    fn library_items(
        &self,
        library: &Library,
        types: &str,
        sort: &str,
        order: &str,
    ) -> SourceResult<RequestBuilder> {
        Ok(self.api.get("/Items").query(&[
            ("parentId", identifier(&library.id)?),
            ("recursive", "true"),
            ("includeItemTypes", types),
            ("sortBy", sort),
            ("sortOrder", order),
            ("collapseBoxSetItems", "false"),
        ]))
    }

    /// The library list, reused for a while so Home does not fetch it for every row.
    fn views(&self) -> SourceResult<Vec<Library>> {
        if let Some((fetched, libraries)) = &*lock(&self.views)
            && fetched.elapsed() < VIEWS_TTL
        {
            return Ok(libraries.clone());
        }
        self.libraries()
    }

    // Jellyfin Home results do not carry a library ID. Resolve membership in batches
    // so hidden libraries also disappear from cross-server Continue Watching.
    fn tag_home_libraries(&self, items: &mut [Item]) -> SourceResult<()> {
        if items.is_empty() {
            return Ok(());
        }
        #[derive(Deserialize)]
        struct Membership {
            #[serde(rename = "Id")]
            id: String,
        }
        for library in self.views()? {
            let kind = if library.kind == LibraryKind::Shows {
                Kind::Episode
            } else {
                Kind::Movie
            };
            let ids: Vec<_> = items
                .iter()
                .filter(|i| i.kind == kind && i.library_id.is_none())
                .map(|i| i.id.as_str())
                .collect();
            if ids.is_empty() {
                continue;
            }
            let members: Listing<Membership> = json(self.api.get("/Items").query(&[
                ("parentId", library.id.as_str()),
                ("recursive", "true"),
                ("ids", &ids.join(",")),
                ("enableImages", "false"),
                ("enableUserData", "false"),
            ]))?;
            for member in members.items {
                if let Some(item) = items.iter_mut().find(|i| i.id == member.id) {
                    item.library_id = Some(library.id.clone());
                }
            }
        }
        Ok(())
    }
}

impl MediaSource for Jellyfin {
    fn address(&self) -> &str {
        self.api.base.as_str()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            recently_released: true,
            children: true,
            sign_out: true,
            ..Capabilities::default()
        }
    }

    fn libraries(&self) -> SourceResult<Vec<Library>> {
        let result: Listing<View> = json(self.api.get("/UserViews"))?;
        let mut libraries = Vec::new();
        for view in result.items {
            let kind = match view.collection_type.as_deref() {
                Some("movies") => LibraryKind::Movies,
                Some("tvshows") => LibraryKind::Shows,
                _ => continue,
            };
            identifier(&view.id)?;
            libraries.push(Library {
                id: view.id,
                title: view.name,
                kind,
            });
        }
        *lock(&self.views) = Some((Instant::now(), libraries.clone()));
        Ok(libraries)
    }

    fn browse(&self, scope: &Browse, search: &str, offset: usize) -> SourceResult<Page> {
        let request = match scope {
            Browse::Library(library) => {
                let types = if library.kind == LibraryKind::Shows {
                    "Series"
                } else {
                    "Movie"
                };
                let request = self.library_items(library, types, "SortName", "Ascending")?;
                return self.page(request, search, offset, Some(&library.id));
            }
            Browse::Children(id) => {
                let request = self.api.get("/Items").query(&[
                    ("parentId", identifier(id)?),
                    ("recursive", "false"),
                    ("includeItemTypes", "Season,Episode"),
                    ("sortBy", "IndexNumber,SortName"),
                ]);
                return self.page(request, search, offset, None);
            }
            Browse::RecentlyAdded => self.api.get("/Items").query(&[
                ("recursive", "true"),
                ("sortBy", "DateCreated,SortName"),
                ("sortOrder", "Descending,Ascending"),
                ("collapseBoxSetItems", "false"),
            ]),
            Browse::ContinueWatching => self.api.get("/UserItems/Resume"),
        };
        let mut page = self.page(
            request.query(&[("includeItemTypes", "Movie,Episode")]),
            search,
            offset,
            None,
        )?;
        // Best effort: untagged items stay visible rather than failing the whole Home row.
        let _ = self.tag_home_libraries(&mut page.items);
        Ok(page)
    }

    fn recently_released(&self, libraries: &[Library]) -> SourceResult<Vec<Item>> {
        let mut items = Vec::new();
        for library in libraries {
            let types = if library.kind == LibraryKind::Shows {
                "Episode"
            } else {
                "Movie"
            };
            let request = self.library_items(library, types, "PremiereDate", "Descending")?;
            items.extend(self.page(request, "", 0, Some(&library.id))?.items);
        }
        Ok(newest_released(items))
    }

    fn item(&self, id: &str) -> SourceResult<Item> {
        let entry: Entry = json(self.api.get(&format!("/Items/{}", identifier(id)?)))?;
        if entry.id != id {
            return Err("Jellyfin returned a different item than requested.".into());
        }
        entry.into_item(None)
    }

    fn cached_artwork(&self, path: &str) -> Option<image::RgbaImage> {
        self.artwork.cached(self.address(), path)
    }

    fn artwork(&self, path: &str) -> SourceResult<image::RgbaImage> {
        let mut parts = path.split('/');
        let (Some(id), Some(kind @ ("Primary" | "Backdrop")), Some(tag), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err("Invalid Jellyfin image reference.".into());
        };
        let request = self
            .api
            .get(&format!("/Items/{}/Images/{kind}", identifier(id)?))
            .query(&[
                ("tag", identifier(tag)?),
                ("maxWidth", if kind == "Backdrop" { "1280" } else { "360" }),
                ("maxHeight", if kind == "Backdrop" { "720" } else { "540" }),
                ("format", "jpg"),
            ]);
        self.artwork.fetch(self.address(), path, request)
    }

    fn playback(&self, item: &Item, version: &Version, track: usize) -> SourceResult<Playback> {
        if track != 0 {
            return Err("Jellyfin versions have a single playable file.".into());
        }
        let mut url = self
            .api
            .url(&format!("/Videos/{}/stream", identifier(&item.id)?));
        url.query_pairs_mut()
            .append_pair("static", "true")
            .append_pair("mediaSourceId", identifier(version.stream.direct_file()?)?);
        Ok(Playback {
            url,
            headers: vec![Playback::header("authorization", &self.api.authorization)?],
        })
    }

    fn report(&self, item: &Item, version: &Version, progress: Progress) -> SourceResult<()> {
        let path = match progress.state {
            PlaybackState::Started => "/Sessions/Playing",
            PlaybackState::Playing | PlaybackState::Paused => "/Sessions/Playing/Progress",
            PlaybackState::Stopped => "/Sessions/Playing/Stopped",
        };
        let request = self.api.post(path).timeout(Duration::from_secs(4)).json(&serde_json::json!({
            "ItemId": identifier(&item.id)?,
            "MediaSourceId": identifier(version.stream.direct_file()?)?,
            "PositionTicks": progress.position_ms.saturating_mul(TICKS_PER_MS).min(i64::MAX as u64),
            "IsPaused": matches!(progress.state, PlaybackState::Paused),
            "PlayMethod": "DirectPlay",
            "CanSeek": true,
        }));
        http::check_status(send(request)?.status(), SERVER)
    }

    /// Ends this device's session, which revokes its access token on the server.
    fn sign_out(&self) -> SourceResult<()> {
        let request = self
            .api
            .post("/Sessions/Logout")
            .timeout(Duration::from_secs(4));
        http::check_status(send(request)?.status(), SERVER)
    }
}
