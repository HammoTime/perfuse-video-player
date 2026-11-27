//! Audiobookshelf book libraries. Cover and track URLs are built on the connected origin.
pub mod auth;
mod bookmarks;
mod playback;
#[cfg(test)]
mod tests;

use super::{OpenOptions, Plugin, ServerKind, SignInMethod};
use crate::{cache::Artwork, media::*, plugins::credentials::lock};
use auth::SERVER;
use serde::Deserialize;
use std::{
    cmp::Reverse,
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub static PLUGIN: Plugin = Plugin {
    kind: ServerKind::Audiobookshelf,
    id: "audiobookshelf",
    name: SERVER,
    default_address: "http://localhost:13378",
    address_example: "https://abs.example.com",
    description: "Audiobooks with chapters and bookmarks.",
    libraries: &[LibraryKind::Audiobooks],
    sign_in: &[SignInMethod::Password { required: true }],
    authenticate: auth::authenticate,
    open: |session, options: OpenOptions| {
        Ok(Arc::new(Audiobookshelf {
            api: auth::Api::new(session)?,
            artwork: Artwork::new(options.cache_artwork),
            cursors: Mutex::default(),
        }))
    },
};

// Remembered cursors grow with the pages a user visits. Start over past this many.
const CURSOR_LIMIT: usize = 1024;

/// The server counts ebooks and podcasts in its page offsets, so a visible book offset
/// does not map to a server offset. Remember where each visited page continues.
#[derive(Default)]
struct Cursors {
    /// (library, visible offset) to the raw server offset of that page's first book.
    library: HashMap<(String, usize), usize>,
    /// Recently Added visible offset to each library's raw offset, or None once exhausted.
    recent: HashMap<usize, Vec<(String, Option<usize>)>>,
}

impl Cursors {
    fn trim(&mut self) {
        if self.library.len() + self.recent.len() > CURSOR_LIMIT {
            *self = Self::default();
        }
    }
}

struct Audiobookshelf {
    api: auth::Api,
    artwork: Artwork,
    cursors: Mutex<Cursors>,
}

fn identifier(id: &str) -> SourceResult<&str> {
    super::identifier(id, SERVER)
}

fn millis(seconds: f64) -> Option<u64> {
    let ms = (seconds * 1000.0).round();
    (seconds >= 0.0 && ms < u64::MAX as f64).then_some(ms as u64)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: String,
    library_id: String,
    media_type: String,
    #[serde(default)]
    added_at: u64,
    #[serde(default)]
    updated_at: u64,
    media: BookMedia,
    user_media_progress: Option<UserProgress>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookMedia {
    metadata: Metadata,
    duration: Option<f64>,
    cover_path: Option<String>,
    num_tracks: Option<usize>,
    #[serde(default)]
    tracks: Vec<playback::Track>,
    #[serde(default)]
    chapters: Vec<ServerChapter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    title: String,
    author_name: Option<String>,
    narrator_name: Option<String>,
    series_name: Option<String>,
    description_plain: Option<String>,
    description: Option<String>,
    published_year: Option<String>,
    published_date: Option<String>,
}

#[derive(Deserialize)]
struct ServerChapter {
    start: f64,
    end: f64,
    title: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserProgress {
    library_item_id: Option<String>,
    episode_id: Option<String>,
    current_time: Option<f64>,
    last_update: Option<u64>,
    #[serde(default)]
    is_finished: bool,
    #[serde(default)]
    hide_from_continue_listening: bool,
}

impl Entry {
    fn has_audio(&self) -> bool {
        self.media_type == "book" && self.media.num_tracks.unwrap_or(self.media.tracks.len()) > 0
    }

    fn in_library(&self, id: &str) -> SourceResult<()> {
        if self.library_id != id {
            return Err("Audiobookshelf returned a book from a different library.".into());
        }
        Ok(())
    }

    fn into_item(self, detail: bool) -> SourceResult<Item> {
        identifier(&self.id)?;
        identifier(&self.library_id)?;
        let (versions, duration_ms) = if detail {
            if self
                .media
                .num_tracks
                .is_some_and(|count| count != self.media.tracks.len())
            {
                return Err("Audiobookshelf returned an incomplete track list.".into());
            }
            let (version, length) = playback::version(self.media.tracks)?;
            (vec![version], Some(length))
        } else {
            (Vec::new(), self.media.duration.and_then(millis))
        };
        let mut chapters = Vec::<Chapter>::new();
        for entry in self.media.chapters {
            let (Some(start_ms), Some(end_ms)) = (millis(entry.start), millis(entry.end)) else {
                continue;
            };
            let end_ms = end_ms.min(duration_ms.unwrap_or(u64::MAX));
            if start_ms < end_ms && chapters.last().is_none_or(|last| last.end_ms <= start_ms) {
                chapters.push(Chapter {
                    title: entry.title,
                    start_ms,
                    end_ms,
                });
            }
        }
        let meta = self.media.metadata;
        let progress = self
            .user_media_progress
            .filter(|p| {
                p.episode_id.is_none() && p.library_item_id.as_ref().is_none_or(|id| id == &self.id)
            })
            .unwrap_or_default();
        let thumb = self
            .media
            .cover_path
            .filter(|s| !s.is_empty())
            .map(|_| format!("cover/{}/{}", self.id, self.updated_at));
        Ok(Item {
            kind: Kind::Audiobook,
            id: self.id,
            library_id: Some(self.library_id),
            title: meta.title,
            duration_ms,
            thumb,
            versions,
            book: Book {
                author: meta.author_name.unwrap_or_default(),
                narrator: meta.narrator_name.unwrap_or_default(),
                series: meta.series_name.unwrap_or_default(),
                chapters,
                bookmarks: Vec::new(),
            },
            year: meta.published_year.and_then(|s| s.parse().ok()),
            release_date: meta.published_date,
            summary: meta
                .description_plain
                .or(meta.description)
                .unwrap_or_default(),
            last_viewed_at: progress.last_update.map(|ms| ms / 1000),
            resume_ms: progress
                .current_time
                .and_then(millis)
                .filter(|_| !progress.is_finished)
                .map(|ms| ms.min(duration_ms.unwrap_or(u64::MAX))),
            watched: progress.is_finished,
            ..Item::default()
        })
    }
}

#[derive(Deserialize)]
struct Listing {
    offset: usize,
    total: usize,
    results: Vec<Entry>,
}

/// Books with their raw server positions, and the raw position of the first unread book.
type Catalog = (Vec<(usize, Entry)>, Option<usize>);

impl Audiobookshelf {
    fn progress(&self) -> SourceResult<HashMap<String, UserProgress>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Me {
            media_progress: Vec<UserProgress>,
        }
        let user: Me = self.api.json(self.api.get("/api/me"))?;
        Ok(user
            .media_progress
            .into_iter()
            .filter(|p| p.episode_id.is_none())
            .filter_map(|p| Some((p.library_item_id.clone()?, p)))
            .collect())
    }

    // Return up to 'take' audiobooks and the raw position of the first unread one.
    fn catalog(
        &self,
        library: &str,
        sort: &str,
        mut raw: usize,
        mut skip: usize,
        take: usize,
    ) -> SourceResult<Catalog> {
        const LIMIT: usize = PAGE_SIZE * 2;
        let path = format!("/api/libraries/{}/items", identifier(library)?);
        let mut books = Vec::new();
        loop {
            let page = raw / LIMIT;
            let listing: Listing = self.api.json(
                self.api
                    .get(&path)
                    .query(&[("limit", LIMIT), ("page", page)])
                    .query(&[
                        ("sort", sort),
                        ("desc", if sort == "addedAt" { "1" } else { "0" }),
                        ("minified", "1"),
                    ]),
            )?;
            let count = listing.results.len();
            if listing.offset != page * LIMIT
                || count > LIMIT
                || (count < LIMIT && listing.offset.saturating_add(count) < listing.total)
            {
                return Err("Audiobookshelf returned an incomplete or unexpected page.".into());
            }
            for (index, entry) in listing.results.into_iter().enumerate().skip(raw % LIMIT) {
                entry.in_library(library)?;
                if !entry.has_audio() {
                    continue;
                }
                if skip > 0 {
                    skip -= 1;
                } else if books.len() == take {
                    return Ok((books, Some(listing.offset + index)));
                } else {
                    books.push((listing.offset + index, entry));
                }
            }
            raw = listing.offset.saturating_add(count);
            if raw >= listing.total || count == 0 {
                return Ok((books, None));
            }
        }
    }

    fn library_page(&self, id: &str, offset: usize) -> SourceResult<(Vec<Entry>, Option<usize>)> {
        let cursor = if offset == 0 {
            Some(0)
        } else {
            lock(&self.cursors)
                .library
                .get(&(id.to_owned(), offset))
                .copied()
        };
        let (entries, next) = self.catalog(
            id,
            "media.metadata.title",
            cursor.unwrap_or(0),
            if cursor.is_some() { 0 } else { offset },
            PAGE_SIZE,
        )?;
        let end = offset + entries.len();
        if let Some(raw) = next {
            let mut cursors = lock(&self.cursors);
            cursors.trim();
            cursors.library.insert((id.to_owned(), end), raw);
        }
        let total = next.is_none().then_some(end);
        Ok((entries.into_iter().map(|(_, entry)| entry).collect(), total))
    }

    /// Merges each library's newest books. Visited pages continue from remembered
    /// per-library positions, so later pages cost one request per library.
    fn recently_added(&self, offset: usize) -> SourceResult<(Vec<Entry>, Option<usize>)> {
        let start = lock(&self.cursors).recent.get(&offset).cloned();
        let start = match start {
            Some(start) => start,
            None if offset == 0 => self
                .libraries()?
                .into_iter()
                .map(|library| (library.id, Some(0)))
                .collect(),
            None => return self.recently_added_prefix(offset),
        };
        let mut candidates = Vec::new();
        let mut fetched_to = Vec::with_capacity(start.len());
        for (index, (library, raw)) in start.iter().enumerate() {
            let Some(raw) = *raw else {
                fetched_to.push(None);
                continue;
            };
            let (books, next) = self.catalog(library, "addedAt", raw, 0, PAGE_SIZE)?;
            candidates.extend(
                books
                    .into_iter()
                    .map(|(position, entry)| (index, position, entry)),
            );
            fetched_to.push(next);
        }
        // Newest first. Ties keep library order, then each library's server order.
        candidates
            .sort_by_key(|(index, position, entry)| (Reverse(entry.added_at), *index, *position));
        let unused = candidates.split_off(PAGE_SIZE.min(candidates.len()));
        // Each library continues at its first book left off this page, or after its fetch.
        let next: Vec<_> = start
            .into_iter()
            .zip(fetched_to)
            .enumerate()
            .map(|(index, ((library, _), fetched))| {
                let first_unused = unused
                    .iter()
                    .filter(|(from, ..)| *from == index)
                    .map(|(_, position, _)| *position)
                    .min();
                (library, first_unused.or(fetched))
            })
            .collect();
        let end = offset + candidates.len();
        let more = next.iter().any(|(_, raw)| raw.is_some());
        if more {
            let mut cursors = lock(&self.cursors);
            cursors.trim();
            cursors.recent.insert(end, next);
        }
        let total = (!more).then_some(end);
        Ok((
            candidates.into_iter().map(|(_, _, entry)| entry).collect(),
            total,
        ))
    }

    /// Recently Added from an offset no earlier page reached, by merging each library's prefix.
    fn recently_added_prefix(&self, offset: usize) -> SourceResult<(Vec<Entry>, Option<usize>)> {
        let end = offset.saturating_add(PAGE_SIZE);
        let mut books = Vec::new();
        let mut more = false;
        for library in self.libraries()? {
            let (items, next) = self.catalog(&library.id, "addedAt", 0, 0, end)?;
            books.extend(items.into_iter().map(|(_, entry)| entry));
            // Preserve each library's server order for ties, even as the next page widens its prefix.
            books.sort_by_key(|e| Reverse(e.added_at));
            more |= next.is_some() || books.len() > end;
            books.truncate(end);
        }
        let total = (!more).then_some(books.len());
        Ok((books.into_iter().skip(offset).collect(), total))
    }

    // Search and items-in-progress support a limit but no offset. Grow the requested
    // prefix when ebooks, podcasts or hidden entries would otherwise hide later books.
    fn prefix_page(
        &self,
        offset: usize,
        mut load: impl FnMut(usize) -> SourceResult<Vec<Entry>>,
        include: impl Fn(&Entry) -> bool,
    ) -> SourceResult<(Vec<Entry>, Option<usize>)> {
        let end = offset.saturating_add(PAGE_SIZE);
        let mut limit = end.saturating_add(1);
        loop {
            let mut entries = load(limit)?;
            let exhausted = entries.len() < limit;
            if entries.len() > limit {
                return Err("Audiobookshelf exceeded the requested limit.".into());
            }
            entries.retain(|e| e.has_audio() && include(e));
            if exhausted || entries.len() > end {
                let total = exhausted.then_some(entries.len());
                return Ok((
                    entries.into_iter().skip(offset).take(PAGE_SIZE).collect(),
                    total,
                ));
            }
            limit = limit
                .checked_mul(2)
                .ok_or("Audiobookshelf returned too many results.")?;
        }
    }

    fn search(
        &self,
        library: &str,
        search: &str,
        offset: usize,
    ) -> SourceResult<(Vec<Entry>, Option<usize>)> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Hit {
            library_item: Entry,
        }
        #[derive(Deserialize)]
        struct Search {
            book: Vec<Hit>,
        }
        let path = format!("/api/libraries/{}/search", identifier(library)?);
        self.prefix_page(
            offset,
            |limit| {
                let result: Search = self.api.json(
                    self.api
                        .get(&path)
                        .query(&[("q", search)])
                        .query(&[("limit", limit)]),
                )?;
                result
                    .book
                    .into_iter()
                    .map(|hit| {
                        hit.library_item.in_library(library)?;
                        Ok(hit.library_item)
                    })
                    .collect()
            },
            |_| true,
        )
    }

    fn cover_id(path: &str) -> SourceResult<&str> {
        let mut parts = path.split('/');
        let (Some("cover"), Some(id), Some(revision), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err("Invalid audiobook cover reference.".into());
        };
        revision
            .parse::<u64>()
            .map_err(|_| "Invalid audiobook cover reference.")?;
        identifier(id)
    }
}

impl MediaSource for Audiobookshelf {
    fn address(&self) -> &str {
        self.api.base.as_str()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            bookmarks: true,
            // Legacy API tokens cannot be revoked by signing out.
            sign_out: self.api.token.is_rotating(),
            ..Capabilities::default()
        }
    }

    fn libraries(&self) -> SourceResult<Vec<Library>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct View {
            id: String,
            name: String,
            media_type: String,
        }
        #[derive(Deserialize)]
        struct Libraries {
            libraries: Vec<View>,
        }
        let views: Libraries = self.api.json(self.api.get("/api/libraries"))?;
        views
            .libraries
            .into_iter()
            .filter(|lib| lib.media_type == "book")
            .map(|lib| {
                identifier(&lib.id)?;
                Ok(Library {
                    id: lib.id,
                    title: lib.name,
                    kind: LibraryKind::Audiobooks,
                })
            })
            .collect()
    }

    fn browse(&self, scope: &Browse, search: &str, offset: usize) -> SourceResult<Page> {
        let mut progress = HashMap::new();
        let (mut entries, total) = match scope {
            Browse::Library(lib) if !search.trim().is_empty() => {
                self.search(&lib.id, search.trim(), offset)?
            }
            Browse::Library(lib) => self.library_page(&lib.id, offset)?,
            Browse::RecentlyAdded => self.recently_added(offset)?,
            Browse::Children(_) => {
                return Err(SourceError::Unsupported(
                    "Audiobooks do not have child items.".into(),
                ));
            }
            Browse::ContinueWatching => {
                #[derive(Deserialize)]
                #[serde(rename_all = "camelCase")]
                struct Listening {
                    library_items: Vec<Entry>,
                }
                progress = self.progress()?;
                if progress.is_empty() {
                    return Ok(Page::new(vec![], offset, Some(0)));
                }
                self.prefix_page(
                    offset,
                    |limit| {
                        let result: Listening = self.api.json(
                            self.api
                                .get("/api/me/items-in-progress")
                                .query(&[("limit", limit)]),
                        )?;
                        Ok(result.library_items)
                    },
                    |book| {
                        progress.get(&book.id).is_some_and(|p| {
                            !p.is_finished
                                && !p.hide_from_continue_listening
                                && p.current_time.unwrap_or_default() > 0.0
                        })
                    },
                )?
            }
        };
        if !entries.is_empty() && !matches!(scope, Browse::ContinueWatching) {
            progress = self.progress()?;
        }
        for entry in &mut entries {
            entry.user_media_progress = progress.remove(&entry.id);
        }
        if matches!(scope, Browse::ContinueWatching) {
            entries.sort_by_key(|e| {
                Reverse(e.user_media_progress.as_ref().and_then(|p| p.last_update))
            });
        }
        Ok(Page::new(
            entries
                .into_iter()
                .map(|e| e.into_item(false))
                .collect::<Result<_, _>>()?,
            offset,
            total,
        ))
    }

    fn item(&self, id: &str) -> SourceResult<Item> {
        let entry: Entry = self.api.json(
            self.api
                .get(&format!("/api/items/{}", identifier(id)?))
                .query(&[("expanded", "1"), ("include", "progress")]),
        )?;
        if entry.id != id {
            return Err("Audiobookshelf returned a different item than requested.".into());
        }
        if !entry.has_audio() {
            return Err("This item has no audiobook tracks.".into());
        }
        let mut item = entry.into_item(true)?;
        item.book.bookmarks = self.bookmarks(id)?;
        Ok(item)
    }

    fn cached_artwork(&self, path: &str) -> Option<image::RgbaImage> {
        Self::cover_id(path).ok()?;
        self.artwork.cached(self.address(), path)
    }

    fn artwork(&self, path: &str) -> SourceResult<image::RgbaImage> {
        let id = Self::cover_id(path)?;
        let request = self
            .api
            .get(&format!("/api/items/{id}/cover"))
            .query(&[("width", "512"), ("format", "jpeg")]);
        let response = self.api.send(request)?;
        self.artwork.store_response(self.address(), path, response)
    }

    fn bookmarks(&self, id: &str) -> SourceResult<Vec<Bookmark>> {
        bookmarks::list(&self.api, id)
    }
    fn add_bookmark(&self, id: &str, title: &str, position_ms: u64) -> SourceResult<Bookmark> {
        bookmarks::add(&self.api, id, title, position_ms)
    }
    fn remove_bookmark(&self, id: &str, bookmark: &Bookmark) -> SourceResult<()> {
        bookmarks::remove(&self.api, id, bookmark)
    }
    fn playback(&self, item: &Item, version: &Version, track: usize) -> SourceResult<Playback> {
        self.book_playback(item, version, track)
    }
    fn report(&self, item: &Item, version: &Version, progress: Progress) -> SourceResult<()> {
        self.book_report(item, version, progress)
    }
    fn sign_out(&self) -> SourceResult<()> {
        self.api.sign_out()
    }
}
