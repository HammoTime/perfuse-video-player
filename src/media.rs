use std::sync::Arc;

pub const PAGE_SIZE: usize = 36;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    Movie,
    Audiobook,
    Series,
    Season,
    Episode,
    #[default]
    Other,
}

#[derive(Clone, Default)]
pub struct Item {
    pub book: Option<Box<Book>>,
    pub library_id: Option<String>,
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

    pub fn subtitle(&self) -> String {
        if let Some(book) = &self.book
            && !book.author.is_empty()
        {
            return book.author.clone();
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

#[derive(Clone, Default)]
pub struct Book {
    pub author: String,
    pub narrator: String,
    pub series: String,
    pub chapters: Vec<Chapter>,
    pub bookmarks: Vec<Bookmark>,
}

#[derive(Clone, Default)]
pub struct Chapter {
    pub title: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Default)]
pub struct Bookmark {
    pub title: String,
    /// Server-side bookmark identity in seconds, including fractional positions.
    pub time: f64,
}

#[derive(Clone)]
pub struct AudioTrack {
    pub file: String,
    pub start_ms: u64,
}

#[derive(Clone, Default)]
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
}

#[derive(Clone, Default)]
pub enum Stream {
    /// File reference scoped to the plugin and the connected server.
    Direct(String),
    Audiobook(Vec<AudioTrack>),
    Multipart,
    #[default]
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

#[derive(Clone, Debug)]
pub struct Library {
    pub id: String,
    pub title: String,
    pub kind: Kind,
}

#[derive(Clone)]
pub enum Browse {
    RecentlyAdded,
    ContinueWatching,
    Library(Library),
    Children(String),
}

pub struct Page {
    pub items: Vec<Item>,
    pub offset: usize,
    pub total: Option<usize>,
}

impl Page {
    pub fn has_next(&self) -> bool {
        let end = self.offset.saturating_add(self.items.len());
        !self.items.is_empty()
            && self
                .total
                .map_or(self.items.len() >= PAGE_SIZE, |total| end < total)
    }
}

// Deliberately no Debug. The headers may contain an access token.
pub struct Playback {
    pub url: reqwest::Url,
    pub headers: Vec<String>,
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

pub type Source = Arc<dyn MediaSource>;

/// A single authenticated server. IDs and file references are local to that connection.
pub trait MediaSource: Send + Sync {
    fn address(&self) -> &str;
    fn libraries(&self) -> Result<Vec<Library>, String>;
    fn browse(&self, scope: &Browse, search: &str, offset: usize) -> Result<Page, String>;
    fn recently_released(&self, libraries: &[Library]) -> Result<Vec<Item>, String>;
    fn item(&self, id: &str) -> Result<Item, String>;
    fn bookmarks(&self, _id: &str) -> Result<Vec<Bookmark>, String> {
        Err("This source does not support bookmarks.".into())
    }
    fn add_bookmark(&self, _id: &str, _bookmark: &Bookmark) -> Result<Bookmark, String> {
        Err("This source does not support bookmarks.".into())
    }
    fn remove_bookmark(&self, _id: &str, _time: f64) -> Result<(), String> {
        Err("This source does not support bookmarks.".into())
    }
    fn cached_artwork(&self, path: &str) -> Option<image::RgbaImage>;
    fn artwork(&self, path: &str) -> Result<image::RgbaImage, String>;
    fn playback(&self, item: &Item, version: &Version) -> Result<Playback, String>;
    fn report(&self, item: &Item, version: &Version, progress: Progress) -> Result<(), String>;
}
