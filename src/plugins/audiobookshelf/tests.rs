use super::*;
use crate::{
    http_fixture::{fixture, ok},
    plugins::{Credentials, OpenOptions},
};
use serde_json::{Value, json};
use std::io::Cursor;

pub(super) fn credentials(server: &str) -> Credentials {
    Credentials {
        kind: ServerKind::Audiobookshelf,
        server: server.into(),
        token: "test-token".into(),
        client_id: "test-device".into(),
    }
}

fn source(server: &str) -> Source {
    (PLUGIN.open)(&credentials(server), OpenOptions::default()).unwrap()
}

fn entry(id: &str, tracks: usize) -> Value {
    json!({
        "id": id, "libraryId": "books", "mediaType": "book",
        "addedAt": 1709000000000_u64, "updatedAt": 1710000000000_u64,
        "media": {
            "metadata": {
                "title": "The Crossing", "authorName": "Ada Birch",
                "narratorName": "Jo West", "seriesName": "Rivers",
                "publishedYear": "2020", "description": "A river journey."
            },
            "numTracks": tracks, "duration": 200.5,
            "coverPath": "https://other.invalid/cover.png"
        }
    })
}

#[test]
fn paginates_audio_books_across_mixed_library_pages() {
    let catalog: Vec<_> = (0..72)
        .map(|n| entry(&format!("book-{n}"), usize::from(n < 35)))
        .collect();
    let tail =
        json!({"results":[entry("penultimate", 1), entry("last", 1)], "offset":72,"total":74});
    let (server, worker) = fixture(vec![
        ok(
            "GET /api/libraries",
            json!({"libraries":[
                {"id":"podcasts","name":"Podcasts","mediaType":"podcast"},
                {"id":"books","name":"Library","mediaType":"book"}
            ]}),
        ),
        ok(
            "GET /api/libraries/books/items",
            json!({"results":catalog, "offset":0,"total":74}),
        ),
        ok("GET /api/libraries/books/items", tail.clone()),
        ok(
            "GET /api/me",
            json!({"mediaProgress":[
                {"libraryItemId":"penultimate","currentTime":40.5,"isFinished":false,"lastUpdate":10000}
            ]}),
        ),
        ok("GET /api/libraries/books/items", tail),
        ok("GET /api/me", json!({"mediaProgress":[]})),
    ]);
    let source = source(&server);
    let libraries = source.libraries().unwrap();
    assert_eq!(libraries.len(), 1);
    assert_eq!(libraries[0].kind, LibraryKind::Audiobooks);
    let page = source
        .browse(&Browse::Library(libraries[0].clone()), "", 0)
        .unwrap();
    assert_eq!(page.items.len(), PAGE_SIZE);
    assert!(page.has_next());
    assert_eq!(page.items[35].id, "penultimate");
    assert_eq!(page.items[35].resume_ms, Some(40_500));
    let next = source
        .browse(&Browse::Library(libraries[0].clone()), "", PAGE_SIZE)
        .unwrap();
    assert!(!next.has_next());
    assert_eq!(next.items.len(), 1);
    assert_eq!(next.items[0].id, "last");
    let requests = worker.join().unwrap();
    assert_eq!(requests[1].query("page").as_deref(), Some("0"));
    assert_eq!(requests[2].query("page").as_deref(), Some("1"));
    assert_eq!(
        requests[2].query("sort").as_deref(),
        Some("media.metadata.title")
    );
}

#[test]
fn recent_pages_keep_server_order_for_books_added_together() {
    let mut newer = entry("latest", 1);
    newer["libraryId"] = json!("second");
    newer["addedAt"] = json!(1_800_000_000_000_u64);
    let tied: Vec<_> = (0..=PAGE_SIZE)
        .rev()
        .map(|i| entry(&format!("book-{i:02}"), 1))
        .collect();
    let books = json!({"offset":0,"total":tied.len(),"results":tied});
    let (server, worker) = fixture(vec![
        ok(
            "GET /api/libraries",
            json!({"libraries":[
                {"id":"books","name":"Books","mediaType":"book"},
                {"id":"second","name":"More","mediaType":"book"}
            ]}),
        ),
        ok("GET /api/libraries/books/items", books.clone()),
        ok(
            "GET /api/libraries/second/items",
            json!({"offset":0,"total":1,"results":[newer]}),
        ),
        ok("GET /api/me", json!({"mediaProgress":[]})),
        // The next page continues each library where the first page stopped.
        ok("GET /api/libraries/books/items", books),
        ok("GET /api/me", json!({"mediaProgress":[]})),
    ]);
    let source = source(&server);
    let first = source.browse(&Browse::RecentlyAdded, "", 0).unwrap();
    assert!(first.has_next());
    assert_eq!(first.items[0].id, "latest");
    assert_eq!(first.items[1].id, "book-36");
    let next = source
        .browse(&Browse::RecentlyAdded, "", PAGE_SIZE)
        .unwrap();
    assert!(!next.has_next());
    assert_eq!(
        next.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
        ["book-01", "book-00"]
    );
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 6);
    assert_eq!(requests[1].query("sort").as_deref(), Some("addedAt"));
    assert_eq!(requests[1].query("desc").as_deref(), Some("1"));
}

#[test]
fn searches_book_matches_with_encoded_input_and_a_user_page_offset() {
    let count = PAGE_SIZE + 1;
    let matches: Vec<_> = (0..count)
        .map(|n| json!({"libraryItem": entry(&format!("book-{n}"), 1)}))
        .collect();
    let (server, worker) = fixture(vec![
        ok("GET /api/libraries/books/search", json!({"book":matches})),
        ok("GET /api/me", json!({"mediaProgress":[]})),
    ]);
    let source = source(&server);
    let library = Library {
        id: "books".into(),
        title: "Library".into(),
        kind: LibraryKind::Audiobooks,
    };
    let page = source
        .browse(&Browse::Library(library), "River & Bay", PAGE_SIZE)
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, "book-36");
    assert!(!page.has_next());
    let requests = worker.join().unwrap();
    assert_eq!(requests[0].query("q").as_deref(), Some("River & Bay"));
}

#[test]
fn continue_listening_skips_podcasts_ebooks_and_hidden_progress() {
    let mut podcast = entry("podcast", 1);
    podcast["mediaType"] = json!("podcast");
    let (server, worker) = fixture(vec![
        ok(
            "GET /api/me",
            json!({"mediaProgress":[
                {"libraryItemId":"novel", "currentTime":24.25, "lastUpdate":18000, "isFinished":false},
                {"libraryItemId":"hidden", "currentTime":40, "lastUpdate":20000, "isFinished":false, "hideFromContinueListening":true},
                {"libraryItemId":"podcast", "episodeId":"intro", "currentTime":12, "isFinished":false},
                {"libraryItemId":"ebook", "ebookProgress":0.1, "currentTime":0, "isFinished":false}
            ]}),
        ),
        ok(
            "GET /api/me/items-in-progress",
            json!({"libraryItems":[
                entry("hidden", 1), entry("novel", 2), podcast, entry("ebook", 0)
            ]}),
        ),
    ]);
    let page = source(&server)
        .browse(&Browse::ContinueWatching, "", 0)
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, "novel");
    assert_eq!(page.items[0].library_id.as_deref(), Some("books"));
    assert_eq!(page.items[0].resume_ms, Some(24_250));
    assert_eq!(page.items[0].last_viewed_at, Some(18));
    worker.join().unwrap();
}

#[test]
fn book_detail_maps_chapters_tracks_progress_and_cover_on_its_own_origin() {
    let mut book = entry("novel", 2);
    book["media"]["duration"] = Value::Null;
    book["media"]["chapters"] = json!([
        {"id":0, "title":"Departure", "start":0.0, "end":100.25},
        {"id":1, "title":"Return", "start":100.25, "end":200.5}
    ]);
    book["media"]["tracks"] = json!([
        {"index":1, "ino":"711", "startOffset":0.0, "duration":100.25,
            "contentUrl":"https://other.invalid/audio/1", "mimeType":"audio/mpeg"},
        {"index":2, "ino":"722", "startOffset":100.25, "duration":100.25,
            "contentUrl":"https://other.invalid/audio/2", "mimeType":"audio/mpeg"}
    ]);
    book["userMediaProgress"] = json!({
        "libraryItemId":"novel", "currentTime":112.5, "duration":200.5,
        "isFinished":false, "lastUpdate":1710000000000_u64
    });
    let mut png = Cursor::new(Vec::new());
    image::RgbaImage::new(2, 1)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let (server, worker) = fixture(vec![
        ok("GET /api/items/novel", book),
        ok("GET /api/me/bookmarks/novel", json!({"bookmarks":[]})),
        ("GET /api/items/novel/cover", 200, png.into_inner()),
    ]);
    let source = source(&server);
    let item = source.item("novel").unwrap();
    assert_eq!(item.kind, Kind::Audiobook);
    assert!(item.playable());
    assert_eq!(item.title, "The Crossing");
    assert_eq!(item.year, Some(2020));
    assert_eq!(item.library_id.as_deref(), Some("books"));
    assert_eq!(item.resume_ms, Some(112_500));
    assert_eq!(item.duration_ms, Some(200_500));
    assert_eq!(item.last_viewed_at, Some(1_710_000_000));
    let book = item.book;
    assert_eq!(book.author, "Ada Birch");
    assert_eq!(book.narrator, "Jo West");
    assert_eq!(book.series, "Rivers");
    assert_eq!(book.chapters[1].title, "Return");
    assert_eq!(book.chapters[1].start_ms, 100_250);
    assert_eq!(book.chapters[1].end_ms, 200_500);
    let Stream::Audiobook(tracks) = &item.versions[0].stream else {
        panic!("expected a multi-track audiobook");
    };
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[1].start_ms, 100_250);
    let cover = item.thumb.unwrap();
    assert!(!cover.contains("other.invalid"));
    assert_eq!(source.artwork(&cover).unwrap().dimensions(), (2, 1));
    let requests = worker.join().unwrap();
    assert_eq!(requests[0].query("expanded").as_deref(), Some("1"));
    assert_eq!(requests[0].query("include").as_deref(), Some("progress"));
    assert!(requests.iter().all(|r| {
        r.headers
            .to_lowercase()
            .contains("authorization: bearer test-token")
    }));
}

#[test]
fn rejects_mismatched_details_and_invalid_identifiers() {
    let (server, worker) = fixture(vec![ok("GET /api/items/wanted", entry("different", 1))]);
    let source = source(&server);
    assert!(source.item("../bad").is_err());
    assert!(source.item("wanted").is_err());
    assert!(source.artwork("https://other.invalid/cover").is_err());
    assert_eq!(worker.join().unwrap().len(), 1);
}
