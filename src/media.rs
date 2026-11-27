use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::{fmt, sync::Arc};

pub const PAGE_SIZE: usize = 36;
/// The most items a Recently Released row shows across a server's libraries.
pub const RELEASED_LIMIT: usize = 24;

/// Why a request to a media source failed. The message is written for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// The server no longer accepts this sign-in. Signing in again can fix it.
    Unauthorized(String),
    /// The server could not be reached or did not answer in time.
    Unreachable(String),
    /// This source does not offer the requested feature.
    Unsupported(String),
    /// Any other refusal or unexpected response.
    Failed(String),
}

impl SourceError {
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Unauthorized(_))
    }

    pub fn is_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable(_))
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Unauthorized(message)
            | Self::Unreachable(message)
            | Self::Unsupported(message)
            | Self::Failed(message) => message,
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for SourceError {}

impl From<String> for SourceError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<&str> for SourceError {
    fn from(message: &str) -> Self {
        Self::Failed(message.to_owned())
    }
}

impl From<SourceError> for String {
    fn from(error: SourceError) -> Self {
        match error {
            SourceError::Unauthorized(message)
            | SourceError::Unreachable(message)
            | SourceError::Unsupported(message)
            | SourceError::Failed(message) => message,
        }
    }
}

pub type SourceResult<T> = Result<T, SourceError>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum Kind {
    Movie,
    Audiobook,
    Series,
    Season,
    Episode,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Item {
    /// Book metadata. Empty unless `kind` is `Kind::Audiobook`.
    pub book: Book,
    pub library_id: Option<String>,
    /// When the signed-in user last played this item, in Unix seconds.
    pub last_viewed_at: Option<u64>,
    pub id: String,
    pub title: String,
    pub kind: Kind,
    pub year: Option<u32>,
    /// Release or air date as YYYY-MM-DD.
    pub release_date: Option<String>,
    pub index: Option<u32>,
    pub season_number: Option<u32>,
    pub series_title: Option<String>,
    pub summary: String,
    pub thumb: Option<String>,
    pub art: Option<String>,
    pub duration_ms: Option<u64>,
    pub resume_ms: Option<u64>,
    pub watched: bool,
    pub versions: Vec<Version>,
}

impl Item {
    pub fn playable(&self) -> bool {
        matches!(self.kind, Kind::Movie | Kind::Episode | Kind::Audiobook)
    }

    pub fn is_book(&self) -> bool {
        self.kind == Kind::Audiobook
    }

    pub fn subtitle(&self) -> String {
        if self.is_book() && !self.book.author.is_empty() {
            return self.book.author.clone();
        }
        if self.kind == Kind::Episode {
            return format!(
                "{} · S{:02} E{:02}",
                self.series_title.as_deref().unwrap_or("Episode"),
                self.season_number.unwrap_or(0),
                self.index.unwrap_or(0)
            );
        }
        self.year.map_or_else(
            || {
                match self.kind {
                    Kind::Movie => "movie",
                    Kind::Audiobook => "audiobook",
                    Kind::Series => "show",
                    Kind::Season => "season",
                    Kind::Episode => "episode",
                    Kind::Other => "item",
                }
                .into()
            },
            |year| year.to_string(),
        )
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Book {
    pub author: String,
    pub narrator: String,
    pub series: String,
    pub chapters: Vec<Chapter>,
    pub bookmarks: Vec<Bookmark>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Chapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Bookmark {
    /// The server's identity for this bookmark. Pass it back to remove the bookmark.
    pub id: String,
    pub title: String,
    pub position_ms: u64,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct AudioTrack {
    pub file: String,
    pub start_ms: u64,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Version {
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub video_resolution: Option<String>,
    pub container: Option<String>,
    pub stream: Stream,
}

impl Version {
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

    /// How many separately opened files make up this version.
    pub fn tracks(&self) -> usize {
        match &self.stream {
            Stream::Audiobook(tracks) => tracks.len(),
            _ => 1,
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub enum Stream {
    /// File reference scoped to the plugin and the connected server.
    Direct(String),
    /// Book tracks in playback order on one continuous timeline.
    Audiobook(Vec<AudioTrack>),
    Multipart,
    #[default]
    #[serde(other)]
    Unavailable,
}

impl Stream {
    pub fn direct_file(&self) -> Result<&str, &'static str> {
        match self {
            Self::Direct(file) => Ok(file),
            Self::Audiobook(_) => Err("Select an audiobook track to play."),
            Self::Multipart => {
                Err("Multipart titles are not supported yet. Choose a single-file version.")
            }
            Self::Unavailable => Err("No original file is available for direct playback."),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub enum LibraryKind {
    Movies,
    Shows,
    Audiobooks,
}

impl LibraryKind {
    /// Whether this library is listened to rather than watched.
    pub fn is_audio(self) -> bool {
        self == Self::Audiobooks
    }

    /// Lower-case name for prose, such as "No movie libraries available".
    pub fn noun(self) -> &'static str {
        match self {
            Self::Movies => "movie",
            Self::Shows => "TV",
            Self::Audiobooks => "audiobook",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Library {
    pub id: String,
    pub title: String,
    pub kind: LibraryKind,
}

#[derive(Clone)]
pub enum Browse {
    RecentlyAdded,
    ContinueWatching,
    Library(Library),
    Children(String),
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Page {
    pub items: Vec<Item>,
    /// The server offset of the first item.
    pub offset: usize,
    pub total: Option<usize>,
    /// The server offset that continues this listing, if there is more.
    pub next: Option<usize>,
}

impl Page {
    /// A page whose continuation follows directly from its items and the server total.
    pub fn new(items: Vec<Item>, offset: usize, total: Option<usize>) -> Self {
        let end = offset.saturating_add(items.len());
        let more = !items.is_empty() && total.map_or(items.len() >= PAGE_SIZE, |total| end < total);
        Self {
            items,
            offset,
            total,
            next: more.then_some(end),
        }
    }

    pub fn has_next(&self) -> bool {
        self.next.is_some()
    }
}

/// Browses `scope` and keeps only items `visible` accepts, reading further server pages
/// until a full page is visible or the listing ends. The total is dropped whenever items
/// were skipped, because it would count hidden items.
pub fn browse_visible(
    source: &dyn MediaSource,
    scope: &Browse,
    search: &str,
    offset: usize,
    visible: &dyn Fn(&Item) -> bool,
) -> SourceResult<Page> {
    let mut page = source.browse(scope, search, offset)?;
    if page.items.iter().all(visible) {
        return Ok(page);
    }
    let mut items = Vec::with_capacity(PAGE_SIZE);
    // Bound the extra requests when most of a listing is hidden.
    for _ in 0..8 {
        let start = page.offset;
        for (index, item) in page.items.into_iter().enumerate() {
            if !visible(&item) {
                continue;
            }
            if items.len() == PAGE_SIZE {
                return Ok(Page {
                    items,
                    offset,
                    total: None,
                    next: Some(start + index),
                });
            }
            items.push(item);
        }
        match page.next {
            Some(next) if items.len() < PAGE_SIZE => page = source.browse(scope, search, next)?,
            next => {
                return Ok(Page {
                    items,
                    offset,
                    total: None,
                    next,
                });
            }
        }
    }
    Ok(Page {
        items,
        offset,
        total: None,
        next: page.next,
    })
}

/// Keeps the newest dated, playable items for a Recently Released row.
pub(crate) fn newest_released(mut items: Vec<Item>) -> Vec<Item> {
    items.retain(|item| item.playable() && item.release_date.is_some());
    // ISO dates sort chronologically as strings.
    items.sort_by(|a, b| b.release_date.cmp(&a.release_date));
    items.truncate(RELEASED_LIMIT);
    items
}

// Deliberately no Debug. The headers may contain an access token.
pub struct Playback {
    pub url: reqwest::Url,
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

impl Playback {
    /// A request header marked sensitive, so it is never printed.
    pub fn header(name: &'static str, value: &str) -> SourceResult<(HeaderName, HeaderValue)> {
        let mut value = HeaderValue::from_str(value)
            .map_err(|_| "The server returned a credential that cannot be sent.")?;
        value.set_sensitive(true);
        Ok((HeaderName::from_static(name), value))
    }

    /// `Name: value` entries for libmpv's header list, added one at a time.
    pub fn header_lines(&self) -> Vec<String> {
        self.headers
            .iter()
            .filter_map(|(name, value)| Some(format!("{name}: {}", value.to_str().ok()?)))
            .collect()
    }
}

#[derive(Clone, Copy)]
pub enum PlaybackState {
    Started,
    Playing,
    Paused,
    Stopped,
}

pub struct Progress {
    pub state: PlaybackState,
    pub position_ms: u64,
    pub duration_ms: u64,
}

/// Optional features a connected source offers. The UI checks these instead of the
/// server type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Items can carry server-side bookmarks.
    pub bookmarks: bool,
    /// Home can show a Recently Released row.
    pub recently_released: bool,
    /// Series and seasons can be opened with `Browse::Children`.
    pub children: bool,
    /// `sign_out` revokes the sign-in on the server.
    pub sign_out: bool,
}

pub type Source = Arc<dyn MediaSource>;

/// A single authenticated server. IDs and file references are local to that connection.
pub trait MediaSource: Send + Sync {
    fn address(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn libraries(&self) -> SourceResult<Vec<Library>>;
    fn browse(&self, scope: &Browse, search: &str, offset: usize) -> SourceResult<Page>;
    fn recently_released(&self, _libraries: &[Library]) -> SourceResult<Vec<Item>> {
        Ok(Vec::new())
    }
    fn item(&self, id: &str) -> SourceResult<Item>;
    fn bookmarks(&self, _id: &str) -> SourceResult<Vec<Bookmark>> {
        Err(unsupported_bookmarks())
    }
    fn add_bookmark(&self, _id: &str, _title: &str, _position_ms: u64) -> SourceResult<Bookmark> {
        Err(unsupported_bookmarks())
    }
    fn remove_bookmark(&self, _id: &str, _bookmark: &Bookmark) -> SourceResult<()> {
        Err(unsupported_bookmarks())
    }
    fn cached_artwork(&self, path: &str) -> Option<image::RgbaImage>;
    fn artwork(&self, path: &str) -> SourceResult<image::RgbaImage>;
    /// Opens track `track` of `version`. Single-file versions have only track 0.
    fn playback(&self, item: &Item, version: &Version, track: usize) -> SourceResult<Playback>;
    fn report(&self, item: &Item, version: &Version, progress: Progress) -> SourceResult<()>;
    /// Revokes this sign-in on the server where `capabilities().sign_out` says it can.
    fn sign_out(&self) -> SourceResult<()> {
        Ok(())
    }
}

fn unsupported_bookmarks() -> SourceError {
    SourceError::Unsupported("This server does not support bookmarks.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A Home listing of `total` items. Every third item belongs to the hidden library.
    struct Listing {
        total: usize,
        requests: Mutex<Vec<usize>>,
    }

    impl MediaSource for Listing {
        fn address(&self) -> &str {
            "http://fixture.test/"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        fn libraries(&self) -> SourceResult<Vec<Library>> {
            Ok(Vec::new())
        }
        fn browse(&self, _: &Browse, _: &str, offset: usize) -> SourceResult<Page> {
            self.requests.lock().unwrap().push(offset);
            let items = (offset..self.total.min(offset + PAGE_SIZE))
                .map(|n| Item {
                    id: n.to_string(),
                    library_id: Some(if n % 3 == 0 { "hidden" } else { "shown" }.into()),
                    ..Item::default()
                })
                .collect();
            Ok(Page::new(items, offset, Some(self.total)))
        }
        fn item(&self, _: &str) -> SourceResult<Item> {
            Err(SourceError::Unsupported("unused".into()))
        }
        fn cached_artwork(&self, _: &str) -> Option<image::RgbaImage> {
            None
        }
        fn artwork(&self, _: &str) -> SourceResult<image::RgbaImage> {
            Err(SourceError::Unsupported("unused".into()))
        }
        fn playback(&self, _: &Item, _: &Version, _: usize) -> SourceResult<Playback> {
            Err(SourceError::Unsupported("unused".into()))
        }
        fn report(&self, _: &Item, _: &Version, _: Progress) -> SourceResult<()> {
            Ok(())
        }
    }

    #[test]
    fn hidden_items_never_shorten_a_page_or_skew_its_total() {
        let source = Listing {
            total: 100,
            requests: Mutex::default(),
        };
        let visible = |item: &Item| item.library_id.as_deref() != Some("hidden");
        let first = browse_visible(&source, &Browse::RecentlyAdded, "", 0, &visible).unwrap();
        assert_eq!(first.items.len(), PAGE_SIZE);
        assert!(first.items.iter().all(visible));
        // Totals count hidden items, so a filtered page reports none.
        assert_eq!(first.total, None);
        // 36 visible items end at item 53, so the next page starts with item 55.
        assert_eq!(first.items.last().unwrap().id, "53");
        assert_eq!(first.next, Some(55));
        let rest = browse_visible(&source, &Browse::RecentlyAdded, "", 55, &visible).unwrap();
        assert_eq!(rest.items.first().unwrap().id, "55");
        assert_eq!(rest.next, None);
        assert_eq!(first.items.len() + rest.items.len(), 66);
        assert_eq!(*source.requests.lock().unwrap(), [0, 36, 55, 91]);

        // Without hidden items the server's page is returned untouched.
        let all = browse_visible(&source, &Browse::RecentlyAdded, "", 0, &|_| true).unwrap();
        assert_eq!(
            (all.items.len(), all.total, all.next),
            (PAGE_SIZE, Some(100), Some(36))
        );
    }

    #[test]
    fn source_errors_keep_their_kind_and_message() {
        let error = SourceError::Unauthorized("Sign in again.".into());
        assert!(error.is_unauthorized() && !error.is_unreachable());
        assert_eq!(String::from(error), "Sign in again.");
        let failed: SourceError = "Unexpected reply.".into();
        assert_eq!(failed.to_string(), "Unexpected reply.");
    }
}
