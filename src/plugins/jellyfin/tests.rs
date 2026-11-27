use super::*;
use crate::http_fixture::{fixture, ok};
use crate::plugins::{Credentials, OpenOptions, ServerKind, by_id};
use serde_json::{Value, json};
use std::io::Cursor;

fn source(server: &str, token: &str) -> Source {
    let plugin = by_id("jellyfin").unwrap();
    (plugin.open)(
        &Credentials {
            kind: ServerKind::Jellyfin,
            server: server.into(),
            token: token.into(),
            client_id: "test-device".into(),
        },
        OpenOptions::default(),
    )
    .unwrap()
}

fn views() -> Value {
    json!({"Items":[
        {"Id":"tv","Name":"Television","CollectionType":"tvshows"},
        {"Id":"music","Name":"Music","CollectionType":"music"},
        {"Id":"films","Name":"Films","CollectionType":"movies"}
    ]})
}

fn episode() -> Value {
    json!({"Id":"episode", "Name":"Pilot", "Type":"Episode", "SeriesName":"Orbit",
        "ParentIndexNumber":1, "IndexNumber":2, "RunTimeTicks":3_600_000_000_u64,
        "PremiereDate":"2026-01-16T00:00:00Z", "Overview":"A mission begins.",
        "UserData":{"Played":false,"PlaybackPositionTicks":12_340_000,"LastPlayedDate":"1970-01-02T00:00:00.1234567Z"},
        "ImageTags":{"Primary":"frame"}, "ParentBackdropItemId":"show","ParentBackdropImageTags":["background"],
        "MediaSources":[{"Id":"sd","Container":"mp4"},{"Id":"hd","Container":"mkv",
            "SupportsDirectPlay":true,"VideoType":"VideoFile","Path":"https://elsewhere.invalid/file",
            "MediaStreams":[{"Type":"Video","Codec":"hevc","Height":1080},{"Type":"Audio","Codec":"aac"}]}]})
}

#[test]
fn sign_in_browse_choose_version_and_report_progress() {
    let mut png = Cursor::new(Vec::new());
    image::RgbaImage::new(1, 1)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let (server, worker) = fixture(vec![
        ok(
            "POST /Users/AuthenticateByName",
            json!({"AccessToken":"test-token"}),
        ),
        ok("GET /UserViews", views()),
        ok(
            "GET /Items",
            json!({"StartIndex":36, "TotalRecordCount":37, "Items":[{"Id":"show","Name":"Orbit","Type":"Series"}]}),
        ),
        ok(
            "GET /Items",
            json!({"Items":[{"Id":"season","Name":"Season 1","Type":"Season"}]}),
        ),
        ok("GET /Items", json!({"Items":[episode()]})),
        ok("GET /Items/episode", episode()),
        ("GET /Items/show/Images/Backdrop", 200, png.into_inner()),
        ("POST /Sessions/Playing", 204, vec![]),
        ("POST /Sessions/Playing/Progress", 204, vec![]),
        ("POST /Sessions/Playing/Stopped", 204, vec![]),
    ]);
    let token = auth::password(&server, "test-device", "viewer", "test-password").unwrap();
    let source = source(&server, &token);
    let libraries = source.libraries().unwrap();
    assert_eq!(libraries.len(), 2);
    let page = source
        .browse(&Browse::Library(libraries[0].clone()), "A & B", 36)
        .unwrap();
    assert_eq!(page.offset, 36);
    assert!(!page.has_next());
    assert_eq!(page.items[0].kind, Kind::Series);
    let seasons = source
        .browse(&Browse::Children(page.items[0].id.clone()), "", 0)
        .unwrap();
    assert_eq!(seasons.items[0].kind, Kind::Season);
    let episodes = source
        .browse(&Browse::Children(seasons.items[0].id.clone()), "", 0)
        .unwrap();
    let item = source.item(&episodes.items[0].id).unwrap();
    assert_eq!(item.subtitle(), "Orbit · S01 E02");
    assert_eq!(item.duration_ms, Some(360_000));
    assert_eq!(item.resume_ms, Some(1234));
    assert_eq!(item.last_viewed_at, Some(86_400));
    assert_eq!(item.release_date.as_deref(), Some("2026-01-16"));
    assert_eq!(
        source
            .artwork(item.art.as_deref().unwrap())
            .unwrap()
            .dimensions(),
        (1, 1)
    );
    let version = &item.versions[1];
    assert_eq!(version.label(), "1080P · HEVC · AAC · MKV");
    let playback = source.playback(&item, version, 0).unwrap();
    let authorization = playback.header_lines().remove(0);
    assert_eq!(
        playback.url.as_str(),
        format!("{server}/Videos/episode/stream?static=true&mediaSourceId=hd")
    );
    assert!(authorization.starts_with("authorization: MediaBrowser "));
    for state in [
        PlaybackState::Started,
        PlaybackState::Paused,
        PlaybackState::Stopped,
    ] {
        source
            .report(
                &item,
                version,
                Progress {
                    state,
                    position_ms: 1234,
                    duration_ms: 360_000,
                },
            )
            .unwrap();
    }
    let requests = worker.join().unwrap();
    assert_eq!(
        requests[0].body,
        json!({"Username":"viewer","Pw":"test-password"})
    );
    assert!(
        requests[0].headers.contains("DeviceId=\"test-device\"")
            && !requests[0].headers.contains("Token=")
    );
    for request in &requests[1..] {
        assert!(
            request
                .headers
                .to_ascii_lowercase()
                .contains(&authorization.to_ascii_lowercase())
        );
    }
    assert_eq!(requests[2].query("searchTerm").as_deref(), Some("A & B"));
    assert_eq!(requests[2].query("startIndex").as_deref(), Some("36"));
    assert_eq!(
        requests[2].query("includeItemTypes").as_deref(),
        Some("Series")
    );
    for (index, parent) in [(2, "tv"), (3, "show"), (4, "season")] {
        assert_eq!(requests[index].query("parentId").as_deref(), Some(parent));
    }
    assert_eq!(requests[6].query("tag").as_deref(), Some("background"));
    for req in &requests[7..] {
        assert_eq!(req.body["ItemId"], "episode");
        assert_eq!(req.body["MediaSourceId"], "hd");
        assert_eq!(req.body["PositionTicks"], 12_340_000);
    }
    assert_eq!(requests[8].body["IsPaused"], true);
}

#[test]
fn home_keeps_library_membership_and_release_order() {
    let movie =
        json!({"Id":"movie","Type":"Movie","Name":"Film","PremiereDate":"2026-10-12T00:00:00Z"});
    let episodes = json!({"Items":[episode()]});
    let (server, worker) = fixture(vec![
        ok("GET /UserViews", views()),
        // Home reuses the library list instead of fetching it for every row.
        ok("GET /UserItems/Resume", episodes.clone()),
        ok("GET /Items", json!({"Items":[{"Id":"episode"}]})),
        ok("GET /Items", json!({"Items":[movie]})),
        ok("GET /Items", json!({"Items":[{"Id":"movie"}]})),
        ok("GET /Items", episodes),
        ok("GET /Items", json!({"Items":[movie]})),
    ]);
    let source = source(&server, "test-token");
    let libraries = source.libraries().unwrap();
    let resume = source.browse(&Browse::ContinueWatching, "", 0).unwrap();
    assert_eq!(resume.items[0].library_id.as_deref(), Some("tv"));
    let added = source.browse(&Browse::RecentlyAdded, "", 0).unwrap();
    assert_eq!(added.items[0].library_id.as_deref(), Some("films"));
    let released = source.recently_released(&libraries).unwrap();
    assert_eq!(released.len(), 2);
    assert_eq!(released[0].id, "movie");
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 7);
    assert_eq!(requests[2].query("ids").as_deref(), Some("episode"));
    assert_eq!(
        requests[3].query("sortBy").as_deref(),
        Some("DateCreated,SortName")
    );
    assert_eq!(requests[5].query("sortBy").as_deref(), Some("PremiereDate"));
}

#[test]
fn quick_connect_exchanges_only_after_approval() {
    let (server, worker) = fixture(vec![
        ok(
            "POST /QuickConnect/Initiate",
            json!({"Code":"072359","Secret":"test-challenge"}),
        ),
        ok("GET /QuickConnect/Connect", json!({"Authenticated":true})),
        ok(
            "POST /Users/AuthenticateWithQuickConnect",
            json!({"AccessToken":"test-token"}),
        ),
    ]);
    let token = auth::quick_connect(
        &server,
        "test-device",
        |code| assert_eq!(code, "072359"),
        || false,
    )
    .unwrap();
    assert_eq!(token, "test-token");
    let requests = worker.join().unwrap();
    assert_eq!(
        requests[1].query("secret").as_deref(),
        Some("test-challenge")
    );
    assert_eq!(requests[2].body, json!({"Secret":"test-challenge"}));
}

#[test]
fn rejects_redirects_invalid_credentials_and_unsafe_references() {
    let (server, worker) = fixture(vec![
        ("POST /Users/AuthenticateByName", 302, vec![]),
        ("GET /UserViews", 401, b"private body".to_vec()),
    ]);
    assert!(
        auth::password(&server, "device", "user", "")
            .unwrap_err()
            .message()
            .contains("redirected")
    );
    let source = source(&server, "test-token");
    assert!(source.libraries().unwrap_err().is_unauthorized());
    worker.join().unwrap();
    assert!(Api::new("https://user:pw@example.test", "device", None).is_err());
    assert!(Api::new(&server, "device", Some("a\", Device=\"bad")).is_err());
    assert!(Api::new(&server, "device\r\n", Some("token")).is_err());
    assert!(source.item("../Users").is_err());
    assert!(source.artwork("//example.test/Primary/tag").is_err());
    let mut entry = episode();
    entry["PartCount"] = json!(2);
    let item = serde_json::from_value::<Entry>(entry)
        .unwrap()
        .into_item(None)
        .unwrap();
    assert!(matches!(item.versions[0].stream, Stream::Multipart));
    assert!(source.playback(&item, &item.versions[0], 0).is_err());
    let mut version = item.versions[0].clone();
    version.stream = Stream::Direct("//other.test/file?token=bad".into());
    assert!(source.playback(&item, &version, 0).is_err());
}
