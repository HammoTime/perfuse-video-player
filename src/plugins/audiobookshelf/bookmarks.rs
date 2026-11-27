use super::{SERVER, auth::Api, identifier, millis};
use crate::{
    http,
    media::{Bookmark, SourceResult},
};
use reqwest::StatusCode;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SavedBookmark {
    library_item_id: String,
    time: f64,
    title: String,
}

#[derive(Deserialize)]
struct BookmarkList {
    bookmarks: Vec<SavedBookmark>,
}

impl SavedBookmark {
    // Audiobookshelf identifies a bookmark by its exact time in seconds. Rust prints the
    // shortest text that parses back to the same number, which the delete path expects.
    fn into_bookmark(self) -> SourceResult<Bookmark> {
        let position_ms = millis(self.time).ok_or("The bookmark position is invalid.")?;
        Ok(Bookmark {
            id: self.time.to_string(),
            title: self.title,
            position_ms,
        })
    }
}

/// The server time behind a bookmark ID, rejecting anything that is not a plain number.
fn server_time(id: &str) -> SourceResult<f64> {
    id.parse::<f64>()
        .ok()
        .filter(|time| {
            time.is_finite() && *time >= 0.0 && id.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        })
        .ok_or_else(|| "The bookmark position is invalid.".into())
}

pub(super) fn list(api: &Api, id: &str) -> SourceResult<Vec<Bookmark>> {
    let id = identifier(id)?;
    let response = api.send(api.get(&format!("/api/me/bookmarks/{id}")))?;
    // Before v2.36 bookmarks were available only in the current user's profile.
    let list: BookmarkList = if response.status() == StatusCode::NOT_FOUND {
        api.json(api.get("/api/me"))?
    } else {
        http::json(response, SERVER)?
    };
    let mut bookmarks = list
        .bookmarks
        .into_iter()
        .filter(|bookmark| bookmark.library_item_id == id)
        .map(SavedBookmark::into_bookmark)
        .collect::<Result<Vec<_>, _>>()?;
    bookmarks.sort_by_key(|bookmark| bookmark.position_ms);
    Ok(bookmarks)
}

pub(super) fn add(api: &Api, id: &str, title: &str, position_ms: u64) -> SourceResult<Bookmark> {
    let id = identifier(id)?;
    let title = title.trim();
    if title.is_empty() {
        return Err("Enter a bookmark title.".into());
    }
    let time = position_ms as f64 / 1000.0;
    let saved: SavedBookmark = api.json(
        api.post(&format!("/api/me/item/{id}/bookmark"))
            .json(&json!({ "time": time, "title": title })),
    )?;
    if saved.library_item_id != id || saved.time != time {
        return Err("Audiobookshelf returned a different bookmark.".into());
    }
    saved.into_bookmark()
}

pub(super) fn remove(api: &Api, id: &str, bookmark: &Bookmark) -> SourceResult<()> {
    let id = identifier(id)?;
    let time = server_time(&bookmark.id)?;
    let response = api.send(api.delete(&format!("/api/me/item/{id}/bookmark/{time}")))?;
    http::check_status(response.status(), SERVER)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_fixture::{fixture, ok};
    use crate::plugins::audiobookshelf::tests::credentials;

    #[test]
    fn lists_creates_and_deletes_bookmarks_at_exact_positions() {
        let entry = json!({"libraryItemId":"book","title":"A note","time":7.125});
        let (origin, requests) = fixture(vec![
            ok("GET /api/me/bookmarks/book", json!({"bookmarks":[&entry]})),
            ok("POST /api/me/item/book/bookmark", entry),
            (
                "DELETE /api/me/item/book/bookmark/7.125",
                200,
                b"OK".to_vec(),
            ),
        ]);
        let api = Api::new(&credentials(&origin)).unwrap();
        assert!(list(&api, "../other").is_err());
        let unsafe_id = Bookmark {
            id: "1/../../users".into(),
            ..Bookmark::default()
        };
        assert!(remove(&api, "book", &unsafe_id).is_err());
        let bookmarks = list(&api, "book").unwrap();
        assert_eq!(bookmarks[0].title, "A note");
        assert_eq!(bookmarks[0].position_ms, 7125);
        let saved = add(&api, "book", &bookmarks[0].title, bookmarks[0].position_ms).unwrap();
        assert_eq!(saved, bookmarks[0]);
        remove(&api, "book", &saved).unwrap();
        let requests = requests.join().unwrap();
        assert_eq!(requests[1].body, json!({"time":7.125,"title":"A note"}));
    }

    #[test]
    fn legacy_profile_bookmarks_are_scoped_to_the_selected_book() {
        let (origin, requests) = fixture(vec![
            ("GET /api/me/bookmarks/book", 404, vec![]),
            ok(
                "GET /api/me",
                json!({"bookmarks":[
                    {"libraryItemId":"book", "time":9, "title":"Later"},
                    {"libraryItemId":"other", "time":3, "title":"Another book"},
                    {"libraryItemId":"book", "time":0, "title":"Opening"}
                ]}),
            ),
        ]);
        let api = Api::new(&credentials(&origin)).unwrap();
        let bookmarks = list(&api, "book").unwrap();
        assert_eq!(
            bookmarks
                .iter()
                .map(|b| (b.title.as_str(), b.id.as_str()))
                .collect::<Vec<_>>(),
            [("Opening", "0"), ("Later", "9")]
        );
        requests.join().unwrap();
    }
}
