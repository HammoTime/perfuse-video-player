use super::{auth::Api, identifier};
use crate::{http, media::Bookmark};
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

fn valid_time(time: f64) -> Result<(), String> {
    if time.is_finite() && time >= 0.0 {
        Ok(())
    } else {
        Err("The bookmark position is invalid.".into())
    }
}

impl SavedBookmark {
    fn into_bookmark(self) -> Result<Bookmark, String> {
        valid_time(self.time)?;
        Ok(Bookmark {
            time: self.time,
            title: self.title,
        })
    }
}

pub(super) fn list(api: &Api, id: &str) -> Result<Vec<Bookmark>, String> {
    let id = identifier(id)?;
    let response = api.send(api.get(&format!("/api/me/bookmarks/{id}")))?;
    // Before v2.36 bookmarks were available only in the current user's profile.
    let list: BookmarkList = if response.status() == StatusCode::NOT_FOUND {
        api.json(api.get("/api/me"))?
    } else {
        http::json(response, "Audiobookshelf")?
    };
    let mut bookmarks = list
        .bookmarks
        .into_iter()
        .filter(|bookmark| bookmark.library_item_id == id)
        .map(SavedBookmark::into_bookmark)
        .collect::<Result<Vec<_>, _>>()?;
    bookmarks.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(bookmarks)
}

pub(super) fn add(api: &Api, id: &str, bookmark: &Bookmark) -> Result<Bookmark, String> {
    let id = identifier(id)?;
    valid_time(bookmark.time)?;
    let title = bookmark.title.trim();
    if title.is_empty() {
        return Err("Enter a bookmark title.".into());
    }
    let saved: SavedBookmark = api.json(
        api.post(&format!("/api/me/item/{id}/bookmark"))
            .json(&json!({ "time": bookmark.time, "title": title })),
    )?;
    if saved.library_item_id != id || saved.time != bookmark.time {
        return Err("Audiobookshelf returned a different bookmark.".into());
    }
    saved.into_bookmark()
}

pub(super) fn remove(api: &Api, id: &str, time: f64) -> Result<(), String> {
    let id = identifier(id)?;
    valid_time(time)?;
    let response = api.send(api.delete(&format!("/api/me/item/{id}/bookmark/{time}")))?;
    http::check_status(response.status(), "Audiobookshelf")
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
        assert!(remove(&api, "book", f64::NAN).is_err());
        let bookmarks = list(&api, "book").unwrap();
        assert_eq!(bookmarks[0].title, "A note");
        let saved = add(&api, "book", &bookmarks[0]).unwrap();
        remove(&api, "book", saved.time).unwrap();
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
                .map(|b| b.title.as_str())
                .collect::<Vec<_>>(),
            ["Opening", "Later"]
        );
        requests.join().unwrap();
    }
}
