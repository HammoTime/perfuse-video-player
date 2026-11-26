//! Runs the real eframe update loop without creating an OS window.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use perfuse::media::{AudioTrack, Book, Chapter, PAGE_SIZE, Version};

fn login(size: [f32; 2]) -> Harness<'static, App> {
    app_harness(size, 0.25)
}

fn runtime_versions_fixture() -> Vec<runtime::LibraryInfo> {
    [
        ("libmpv", "mpv v0.42.0-38-gbcde6b93c"),
        ("FFmpeg", "N-128766-gcd4e96eb3"),
        ("libass", "0.17.5"),
        ("libplacebo", "v7.391.0 (v7.387.0-37-gc2cf6e7-dirty)"),
        ("subrandr", "1.4.0"),
        ("Direct3D shader compiler", "10.0.18362.1"),
    ]
    .into_iter()
    .map(|(name, version)| runtime::LibraryInfo {
        name: name.into(),
        version: version.into(),
    })
    .collect()
}

#[test]
fn playback_download_waits_for_permission() {
    for (download, confirm) in [
        (true, "Accept and download"),
        (false, "Accept and continue"),
    ] {
        let mut harness = login([860.0, 620.0]);
        harness.state_mut().runtime_prompt = Some(RuntimePrompt::FirstRun {
            download,
            error: None,
        });
        harness.run_steps(3);
        assert!(harness.state().runtime_rx.is_none());
        assert!(
            harness
                .query_by_label("Perfuse and playback licences")
                .is_some()
        );
        let screen = harness.ctx.content_rect();
        for label in runtime::LICENSE_LINKS
            .iter()
            .map(|(label, _)| *label)
            .chain(["Perfuse (AGPL-3.0-only)", confirm, "Decline and exit"])
        {
            assert!(screen.contains_rect(harness.get_by_label(label).rect()));
        }
        harness.get_by_label("Decline and exit").click();
        harness.step();
        let commands = &harness.output().viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(
            commands
                .iter()
                .any(|command| matches!(command, egui::ViewportCommand::Close))
        );
        assert!(harness.state().runtime_rx.is_none());
    }

    if std::env::var_os("PERFUSE_LIBMPV").is_none() {
        let mut harness = login([860.0, 620.0]);
        harness.state_mut().runtime_available = Some(runtime::test_available());
        harness.run_steps(3);
        harness.get_by_label("Update libraries").click();
        harness.run_steps(3);
        assert!(
            harness
                .query_by_label("Update playback libraries?")
                .is_some()
        );
        assert!(harness.state().runtime_rx.is_none());
        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
        assert!(harness.state().runtime_prompt.is_none());
        assert!(harness.state().runtime_rx.is_none());
    }
}

#[test]
fn playback_setup_reports_progress_failure_and_completion() {
    let mut harness = login([860.0, 620.0]);
    let (tx, rx) = mpsc::channel();
    harness.state_mut().runtime_rx = Some(rx);
    tx.send(runtime::Update::Status(
        "Downloading playback libraries (42%)".into(),
    ))
    .unwrap();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_label("Downloading playback libraries (42%)")
            .is_some()
    );
    tx.send(runtime::Update::Finished(
        Err("Download interrupted".into()),
    ))
    .unwrap();
    harness.run_steps(2);
    assert!(harness.query_by_label("Retry playback setup").is_some());
    assert!(harness.state().runtime_rx.is_none());
    assert!(harness.state().runtime_failed);
    let (tx, rx) = mpsc::channel();
    harness.state_mut().runtime_rx = Some(rx);
    tx.send(runtime::Update::Finished(Ok(()))).unwrap();
    harness.run_steps(2);
    assert!(harness.state().runtime_status.is_none());
    assert!(!harness.state().runtime_failed);
    assert!(harness.query_by_label("Retry playback setup").is_none());
}

#[test]
fn installed_libraries_and_update_availability() {
    let mut harness = library([860.0, 620.0], "home");
    let app = harness.state_mut();
    app.connection_open = true;
    app.runtime_installed.libraries = runtime_versions_fixture();
    harness.run_steps(3);
    harness.get_by_label("Updates & about").click();
    harness.run_steps(2);
    harness.get_by_label("Update").scroll_to_me();
    harness.run_steps(3);
    assert_eq!(
        harness.get_by_label("Check for Updates").rect().size(),
        harness.get_by_label("Update").rect().size()
    );
    assert!(harness.query_by_label("3rd party libraries").is_some());
    assert!(harness.query_by_label("FFmpeg").is_some());
    assert!(harness.query_by_label("N-128766-gcd4e96eb3").is_some());
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(860.0, 620.0));
    for label in ["Check for Updates", "Update"] {
        assert!(
            screen.contains_rect(harness.get_by_label(label).rect()),
            "{label} is clipped"
        );
    }
    assert!(
        harness
            .get_by_label("Update")
            .accesskit_node()
            .is_disabled()
    );
    let (tx, rx) = mpsc::channel();
    harness.state_mut().runtime_rx = Some(rx);
    tx.send(runtime::Update::Available(Some(runtime::test_available())))
        .unwrap();
    tx.send(runtime::Update::Finished(Ok(()))).unwrap();
    harness.run_steps(3);
    harness.get_by_label("Update").scroll_to_me();
    harness.run_steps(3);
    // Explicit development overrides intentionally disable managed updates.
    assert_eq!(
        harness
            .get_by_label("Update")
            .accesskit_node()
            .is_disabled(),
        !cfg!(windows) || std::env::var_os("PERFUSE_LIBMPV").is_some()
    );
    let (tx, rx) = mpsc::channel();
    harness.state_mut().runtime_rx = Some(rx);
    harness.run_steps(2);
    assert!(
        harness
            .get_by_label("Update")
            .accesskit_node()
            .is_disabled()
    );
    tx.send(runtime::Update::Available(None)).unwrap();
    tx.send(runtime::Update::Finished(Ok(()))).unwrap();
    harness.run_steps(3);
    let explanation = harness
        .get_by_label("Playback libraries update together as one runtime.")
        .rect();
    let result = harness.get_by_label("Libraries are up to date.").rect();
    assert!(
        result.left() > explanation.right(),
        "{explanation:?} {result:?}"
    );
    assert!((result.center().y - explanation.center().y).abs() < 1.0);
    assert!(
        harness
            .get_by_label("Update")
            .accesskit_node()
            .is_disabled()
    );
    let (tx, rx) = mpsc::channel();
    harness.state_mut().runtime_action = RuntimeAction::Check;
    harness.state_mut().runtime_rx = Some(rx);
    tx.send(runtime::Update::Finished(Err(
        "Library check failed.".into()
    )))
    .unwrap();
    harness.run_steps(3);
    assert!(
        matches!(&harness.state().runtime_result, Some(Err(error)) if error == "Library check failed.")
    );
    assert!(harness.state().runtime_status.is_none());
}

#[test]
fn sidebar_shows_server_groups_and_connection_count() {
    let mut harness = library([1440.0, 900.0], "home");
    harness.run_steps(3);
    harness.get_by_label("Collapse libraries").click();
    harness.run_steps(3);
    assert!(harness.state().servers[0].prefs.collapsed);
    harness.get_by_label("1 server connected");
}

#[test]
fn sidebar_actions_clear_the_scrollbar_when_libraries_overflow() {
    let mut harness = library([860.0, 620.0], "movies");
    let server = &mut harness.state_mut().servers[0];
    server.prefs.name = "A long server name that must truncate before the actions".into();
    server.sections.extend((3..30).map(|i| Library {
        id: i.to_string(),
        title: format!("Library {i}"),
        kind: Kind::Movie,
    }));
    harness.run_steps(4);
    let expand = harness.get_by_label("Collapse libraries").rect();
    let sort = harness.get_by_label("Sort libraries").rect();
    let right = harness.get_by_label("Servers & settings").rect().right();
    assert_eq!(expand.size(), Vec2::splat(24.0));
    assert_eq!(sort.size(), expand.size());
    assert!((sort.left() - expand.right() - 4.0).abs() < 1.0);
    // The group actions share the right edge of every sidebar row, and that edge
    // stays clear of the floating scroll bar at the panel's 16-point margin.
    assert!((right - sort.right()).abs() < 1.0);
    assert!(244.0 - 16.0 - sort.right() >= harness.ctx.style().spacing.scroll.bar_width + 4.0);
    harness.get_by_label("Sort libraries").click();
    harness.run_steps(3);
    harness.get_by_label("Server order");
}

#[test]
fn home_cards_reserve_the_library_row_and_overflow_opens_all_visible_libraries() {
    let mut harness = library([860.0, 780.0], "movies");
    let app = harness.state_mut();
    app.overview = true;
    app.servers[0].prefs.name = "Glass".into();
    app.servers[0].prefs.collapsed = true;
    app.servers[0].sections.clear();
    let other = session::Session {
        kind: ServerKind::Plex,
        server: "http://127.0.0.1:0".into(),
        token: "synthetic".into(),
        client_id: "ui-test".into(),
    };
    let mut second = Server::new(other.clone(), fixture_source(&other.server), vec![], false);
    second.prefs.name = "Archive".into();
    app.servers.push(second);
    harness.run_steps(4);
    let next_top = |h: &Harness<'_, App>| {
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Archive")
            .rect()
            .top()
    };
    let empty_top = next_top(&harness);
    let libraries = |names: &[&str]| {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| Library {
                id: i.to_string(),
                title: (*name).into(),
                kind: Kind::Movie,
            })
            .collect::<Vec<_>>()
    };
    harness.state_mut().servers[0].sections = libraries(&["A", "B", "C"]);
    harness.run_steps(3);
    assert_eq!(empty_top, next_top(&harness));
    assert!(
        harness
            .query_by_label("Show all libraries on Glass")
            .is_none()
    );
    harness.get_by_label("C");
    let names = [
        "Documentary films from around the world",
        "TV Shows",
        "Music concerts",
        "Kids",
        "Hidden",
    ];
    let server = &mut harness.state_mut().servers[0];
    server.sections = libraries(&names);
    server.prefs.hidden = vec!["4".into()];
    harness.run_steps(3);
    assert_eq!(empty_top, next_top(&harness));
    let more = harness.get_by_label("Show all libraries on Glass");
    let first = harness.get_by_label(names[0]).rect();
    let second = harness.get_by_label(names[1]).rect();
    assert!(first.right() < second.left() && second.right() < more.rect().left());
    assert_eq!(first.center().y, more.rect().center().y);
    assert!(harness.query_by_label("Music concerts").is_none());
    more.hover();
    harness.run_steps(5);
    harness.get_by_label("Libraries on Glass");
    assert_eq!(harness.get_all_by_label(names[0]).count(), 2);
    harness.get_by_label("Music concerts").hover();
    harness.run_steps(3);
    harness.get_by_label("Kids");
    assert!(harness.query_by_label("Hidden").is_none());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.query_by_label("Libraries on Glass").is_none());
    harness.get_by_label("Show all libraries on Glass").focus();
    harness.run_steps(3);
    harness.get_by_label("Libraries on Glass");
    harness.state_mut().servers[0].source = fixture_source("http://127.0.0.1:0");
    harness.get_by_label("Music concerts").click();
    harness.run_steps(3);
    assert_eq!(harness.state().selected_library.as_deref(), Some("2"));
}

#[test]
fn mixed_home_routes_audiobooks_to_continue_listening_and_respects_hidden_libraries() {
    let mut harness = library([1440.0, 900.0], "home");
    let app = harness.state_mut();
    app.overview = true;
    app.servers[0].items = app.on_deck.clone();
    let mut shelf = book_server();
    shelf.items = vec![book_item()];
    app.servers.push(shelf);
    harness.run_steps(4);
    harness.get_by_label("Continue Watching");
    harness.get_by_label("Continue Listening");
    let cover = harness.get_by_label("Open Listening to the Coast").rect();
    assert!((cover.width() - cover.height()).abs() <= 1.0);
    harness.state_mut().servers[1]
        .prefs
        .hidden
        .push("books".into());
    harness.run_steps(3);
    assert!(harness.query_by_label("Continue Listening").is_none());
    harness.get_by_label("Continue Watching");
}

#[test]
fn settings_tabs_show_the_hand_cursor() {
    let mut harness = library([860.0, 620.0], "movies");
    harness.state_mut().connection_open = true;
    harness.run_steps(4);

    for label in ["Servers", "Playback", "Updates & about"] {
        harness.get_by_label(label).hover();
        harness.run_steps(2);
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand,
            "{label}"
        );
    }
}

#[test]
fn settings_action_pairs_match_and_remove_requires_confirmation() {
    let mut harness = library([860.0, 620.0], "movies");
    harness.state_mut().connection_open = true;
    harness.state_mut().servers[0].prefs.name = "Glass".into();
    harness.state_mut().servers[0].prefs.collapsed = true;
    harness.run_steps(4);
    assert_eq!(
        harness.get_by_label("Movies").rect().size(),
        harness.get_by_label("TV Shows").rect().size()
    );
    let check = harness.get_by_label("Check connection").rect();
    let remove = harness.get_by_label("Remove server");
    assert_eq!(check.size(), remove.rect().size());
    assert!(check.right() < remove.rect().left());
    remove.scroll_to_me();
    harness.run_steps(4);
    for dismiss in ["Cancel", "Escape", "Remove"] {
        harness.get_by_label("Remove server").click();
        harness.run_steps(4);
        harness.get_by_label("Remove Glass?");
        let cancel = harness.get_by_label("Cancel");
        assert!(cancel.is_focused());
        let confirm = harness.get_by_label("Remove");
        assert_eq!(cancel.rect().size(), confirm.rect().size());
        assert!(cancel.rect().right() < confirm.rect().left());
        if dismiss == "Escape" {
            harness.key_press(egui::Key::Escape);
        } else {
            harness.get_by_label(dismiss).click();
        }
        harness.run_steps(4);
        assert!(harness.state().removing.is_none());
        if dismiss == "Remove" {
            harness.get_by_label("Make yourself at home.");
        } else {
            assert_eq!(harness.state().servers.len(), 1);
            assert!(harness.get_by_label("Remove server").is_focused());
        }
    }
}

#[test]
fn server_switch_discards_stale_items_and_cancel_keeps_both_servers() {
    let mut harness = library([1000.0, 720.0], "movies");
    let app = harness.state_mut();
    let second = "http://127.0.0.1:32500";
    let session = session::Session {
        kind: ServerKind::Jellyfin,
        server: second.into(),
        token: "synthetic".into(),
        client_id: "ui-test".into(),
    };
    let source = session::open(&session).unwrap();
    app.servers.push(Server::new(
        session,
        source.clone(),
        app.sections.clone(),
        false,
    ));
    let stale = app.generation.load(Ordering::Relaxed);
    app.activate(1);
    assert!(Arc::ptr_eq(app.source.as_ref().unwrap(), &source));
    assert!(app.page.is_none() && app.location.is_none());
    app.tx
        .send((stale, Reply::Details(Item::default())))
        .unwrap();
    app.connection_open = true;
    harness.run_steps(3);
    assert!(harness.state().detail.is_none());
    assert!(
        harness
            .ctx
            .content_rect()
            .contains_rect(harness.get_by_label("Add server").rect())
    );
    harness.get_by_label("Add server").click();
    harness.run_steps(3);
    assert!(harness.state().adding && harness.state().token.is_empty());
    assert!(harness.get_by_label("Plex").is_focused());
    assert!(harness.query_by_label("Server address").is_none());
    harness.get_by_label("Jellyfin").click();
    harness.run_steps(3);
    harness.get_by_label("Connect to Jellyfin");
    assert!(harness.get_by_label("Server address").is_focused());
    harness.get_by_label("Back to server types").click();
    harness.run_steps(3);
    assert!(harness.get_by_label("Jellyfin").is_focused());
    assert!(harness.query_by_label("Username").is_none());
    harness.get_by_label("Plex").click();
    harness.run_steps(3);
    harness.get_by_label("Connect to Plex");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(!harness.state().adding);
    assert!(harness.get_by_label("Add server").is_focused());
    assert_eq!(harness.state().servers.len(), 2);
    assert!(Arc::ptr_eq(
        harness.state().source.as_ref().unwrap(),
        &source
    ));
}

#[test]
fn each_server_home_opens_its_own_recently_added_view() {
    let mut harness = library([1440.0, 900.0], "movies");
    let app = harness.state_mut();
    app.servers[0].prefs.name = "Basement".into();
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        for path in [
            "/library/recentlyAdded",
            "/library/onDeck",
            "/library/sections/1/all",
            "/library/sections/2/all",
        ] {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&socket);
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            assert!(request.starts_with(&format!("GET {path}")));
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
            }
            let body = r#"{"MediaContainer":{"Metadata":[{"ratingKey":"1","type":"movie","title":"Living Room film"}]}}"#;
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    let session = session::Session {
        kind: ServerKind::Plex,
        server: address.clone(),
        token: "synthetic".into(),
        client_id: "ui-test".into(),
    };
    let mut second = Server::new(
        session,
        fixture_source(&address),
        app.sections.clone(),
        false,
    );
    second.prefs.name = "Living Room".into();
    app.servers.push(second);
    app.overview = true;
    harness.run_steps(3);
    harness.get_by_label("Basement Home");
    harness.get_by_label("Living Room Home").click();
    harness.run_steps(2);
    let deadline = Instant::now() + Duration::from_secs(5);
    while harness.state().busy && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
        harness.step();
    }
    assert_eq!(
        harness.state().page.as_ref().unwrap().items[0].title,
        "Living Room film"
    );
    worker.join().unwrap();
    let app = harness.state();
    assert_eq!(app.active, Some(1));
    assert!(!app.overview);
    let location = app.location.as_ref().unwrap();
    assert!(matches!(location.browse, Browse::RecentlyAdded));
    assert_eq!(location.title, "Living Room");
    assert!(Arc::ptr_eq(
        app.source.as_ref().unwrap(),
        &app.servers[1].source
    ));
}

fn app_harness(size: [f32; 2], frame_seconds: f32) -> Harness<'static, App> {
    Harness::builder()
        .with_size(size)
        .with_step_dt(frame_seconds)
        .with_os(egui::os::OperatingSystem::Windows)
        .build_eframe(|cc| {
            // Skip restore: tests must never access a user's saved credentials.
            let mut app = App::new(&cc.egui_ctx);
            app.server = "http://127.0.0.1:32499".into();
            app.token.clear();
            app.client_id = "perfuse-ui-test".into();
            app
        })
}

fn playback(size: [f32; 2]) -> Harness<'static, App> {
    let mut harness = app_harness(size, 1.0 / 60.0);
    let app = harness.state_mut();
    app.player = Some(player::Player::test_stub());
    app.playing_title = "A deliberately long movie title ".repeat(12);
    app.playing_duration = 3000.0;
    app.playing_seconds = 150.0;
    harness
}

fn library_art(index: usize, [width, height]: [usize; 2]) -> egui::ColorImage {
    let palette = [[45, 97, 119], [43, 70, 110], [133, 63, 45], [50, 109, 98]];
    let base = palette[index % palette.len()];
    let pixels = (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let x = x as f32;
                let y = y as f32;
                let height = height as f32;
                let horizon = height * (0.68 + (x * 0.04).sin() * 0.06);
                let light = (1.0 - y / (height * 1.5)) * if y > horizon { 0.35 } else { 1.0 };
                let sun = (x - width as f32 * 0.68).powi(2) + (y - height * 0.3).powi(2)
                    < (height * 0.1).powi(2);
                if sun {
                    Color32::from_rgb(225, 196, 139)
                } else {
                    Color32::from_rgb(
                        (base[0] as f32 * light) as u8,
                        (base[1] as f32 * light) as u8,
                        (base[2] as f32 * light) as u8,
                    )
                }
            })
        })
        .collect();
    egui::ColorImage::new([width, height], pixels)
}

fn fixture_source(server: &str) -> Source {
    session::open(&session::Session {
        kind: ServerKind::Plex,
        server: server.into(),
        token: "synthetic".into(),
        client_id: "ui-test".into(),
    })
    .unwrap()
}

fn attach_server(app: &mut App) {
    let source = app.source.clone().unwrap();
    app.servers.push(Server::new(
        session::Session {
            kind: ServerKind::Plex,
            server: app.server.clone(),
            token: "synthetic".into(),
            client_id: "ui-test".into(),
        },
        source,
        app.sections.clone(),
        false,
    ));
    app.active = Some(app.servers.len() - 1);
}

fn book_server() -> Server {
    let session = session::Session {
        kind: ServerKind::Audiobookshelf,
        server: "http://127.0.0.1:0".into(),
        token: "synthetic".into(),
        client_id: "ui-test".into(),
    };
    let source = session::open(&session).unwrap();
    let library = Library {
        id: "books".into(),
        title: "Audiobooks".into(),
        kind: Kind::Audiobook,
    };
    let mut server = Server::new(session, source, vec![library], false);
    server.prefs.name = "Reading Room".into();
    server
}

fn book_item() -> Item {
    Item {
        id: "coast".into(),
        library_id: Some("books".into()),
        title: "Listening to the Coast".into(),
        kind: Kind::Audiobook,
        year: Some(2023),
        summary: "A field guide to the birds and sounds of the coastal wetlands.".into(),
        duration_ms: Some(700_000),
        resume_ms: Some(250_000),
        thumb: Some("/fixture/shore".into()),
        versions: vec![Version {
            stream: Stream::Audiobook(vec![AudioTrack {
                file: "one".into(),
                start_ms: 0,
            }]),
            ..Default::default()
        }],
        book: Some(Box::new(Book {
            author: "Alex Morgan".into(),
            narrator: "Robin Gray".into(),
            series: String::new(),
            chapters: vec![
                Chapter {
                    title: "Tidal flats".into(),
                    start_ms: 0,
                    end_ms: 400_000,
                },
                Chapter {
                    title: "Salt marsh".into(),
                    start_ms: 400_000,
                    end_ms: 700_000,
                },
            ],
            bookmarks: vec![],
        })),
        ..Default::default()
    }
}

fn book_detail(size: [f32; 2]) -> Harness<'static, App> {
    let mut harness = app_harness(size, 0.25);
    let texture = harness
        .ctx
        .load_texture("shore", library_art(3, [256, 256]), Default::default());
    let app = harness.state_mut();
    let server = book_server();
    app.covers.insert("/fixture/shore".into(), texture);
    app.source = Some(server.source.clone());
    app.sections = server.sections.clone();
    app.servers.push(server);
    app.active = Some(0);
    app.selected_library = Some("books".into());
    app.detail = Some(book_item());
    harness
}

fn show_book_collection(app: &mut App, ctx: &egui::Context, home: bool) {
    let current = app.detail.take().unwrap();
    let mut items = vec![current.clone()];
    for (index, title) in [
        "Tide Tables",
        "Wayfinding",
        "The River Mouth",
        "Passage Birds",
        "The Winter Marsh",
    ]
    .into_iter()
    .enumerate()
    {
        let mut item = current.clone();
        let id = format!("fixture-book-{index}");
        app.covers.insert(
            id.clone(),
            ctx.load_texture(&id, library_art(index, [256, 256]), Default::default()),
        );
        item.thumb = Some(id.clone());
        item.id = id;
        item.title = title.into();
        item.resume_ms = None;
        items.push(item);
    }
    let (title, browse) = if home {
        app.selected_library = None;
        app.home_loaded = true;
        app.on_deck = items[..3].to_vec();
        (app.servers[0].name(), Browse::RecentlyAdded)
    } else {
        (
            "Audiobooks".into(),
            Browse::Library(app.sections[0].clone()),
        )
    };
    app.location = Some(Location {
        title,
        browse,
        search: String::new(),
        offset: 0,
        previous_pages: vec![],
        library: !home,
        parent: None,
    });
    app.page = Some(Page {
        total: Some(items.len() + usize::from(home)),
        items,
        offset: 0,
    });
}

fn book_library_search(size: [f32; 2]) -> Harness<'static, App> {
    let mut harness = book_detail(size);
    let ctx = harness.ctx.clone();
    let app = harness.state_mut();
    show_book_collection(app, &ctx, false);
    app.search = "Coast".into();
    app.location.as_mut().unwrap().search = app.search.clone();
    let page = app.page.as_mut().unwrap();
    page.items.truncate(1);
    page.total = Some(2);
    harness
}

fn start_book_fixture(app: &mut App) {
    let item = app.detail.clone().unwrap();
    let source = app.source.clone().unwrap();
    app.player = Some(player::Player::test_stub());
    app.playing_seconds = item.resume_ms.unwrap() as f64 / 1000.0;
    app.playing_duration = item.duration_ms.unwrap() as f64 / 1000.0;
    app.book_started(&item, &source);
}

#[test]
fn audiobook_detail_and_docked_player_open_and_collapse_without_a_video_surface() {
    let mut harness = book_detail([1480.0, 1080.0]);
    harness.run_steps(4);
    harness.get_by_label("AUDIOBOOK");
    harness.get_by_label("Resume");
    harness.get_by_label("Play from beginning");
    harness.get_by_label("Chapters");
    start_book_fixture(harness.state_mut());
    harness.run_steps(4);
    harness
        .get_all_by_label("Open player")
        .max_by(|a, b| a.rect().bottom().total_cmp(&b.rect().bottom()))
        .unwrap()
        .click();
    harness.run_steps(4);
    harness.get_by_label("Add bookmark");
    harness.get_by_label("Servers & settings");
    harness.get_by_label("Back 30 seconds");
    harness.get_by_label("Forward 30 seconds");
    let chapters = harness.get_by_label("Chapters (2)").rect();
    let speed = harness.get_by_label("Speed 1×").rect();
    harness.get_by_label("Speed 1×").click();
    harness.run_steps(3);
    let option = harness.get_by_label("1× (normal)");
    assert!(option.is_focused());
    assert!(option.rect().top() > speed.bottom());
    assert!(option.rect().right() < chapters.left());
    assert_eq!(harness.get_by_label("Chapters (2)").rect(), chapters);
    harness.key_press(egui::Key::ArrowDown);
    harness.run_steps(3);
    assert!(harness.get_by_label("1.25×").is_focused());
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert!(harness.get_by_label("Speed 1.25×").is_focused());
    harness.get_by_label("Sleep").click();
    harness.run_steps(3);
    harness.get_by_label("End of chapter").click();
    harness.run_steps(3);
    harness.get_by_label("Sleep · End of chapter");
    harness.get_by_label("Sleep · End of chapter").click();
    harness.run_steps(3);
    assert!(harness.get_by_label("End of chapter").is_focused());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.query_by_label("Off").is_none());
    assert!(harness.get_by_label("Sleep · End of chapter").is_focused());
    harness.get_by_label("Bookmarks (0)").click();
    harness.run_steps(3);
    harness.get_by_label("Speed 1.25×").click();
    harness.run_steps(3);
    harness.get_by_label("Chapters (2)").click();
    harness.run_steps(3);
    assert!(harness.query_by_label("1× (normal)").is_none());
    harness.get_by_label("Salt marsh");
    harness.get_by_label("Collapse player").click();
    harness.run_steps(4);
    harness.get_by_label("AUDIOBOOK");
    assert!(!harness.state().book.full);
    assert!(harness.state().surface.is_none());
    // The same book ID on another source is a separate playback target.
    harness.state_mut().source = Some(book_server().source);
    harness.run_steps(3);
    harness.get_by_label("Resume");
    harness
        .get_by_label("Open player: Listening to the Coast")
        .click();
    harness.run_steps(3);
    assert!(harness.state().book.full);
    harness.get_by_label("Servers & settings").click();
    harness.run_steps(3);
    assert!(harness.state().connection_open);
    assert!(!harness.state().book.full);
    harness.get_by_label("Open player: Listening to the Coast");
}

#[test]
fn audiobook_dock_respects_the_sidebar_and_viewport_breakpoints() {
    for width in [860.0, 1000.0, 1180.0, 2560.0] {
        let mut harness = book_detail([width, 900.0]);
        start_book_fixture(harness.state_mut());
        harness.run_steps(4);
        let now = harness
            .get_by_label("Open player: Listening to the Coast")
            .rect();
        let pause = harness
            .get_all_by_label("Pause")
            .map(|node| node.rect())
            .max_by(|a, b| a.bottom().total_cmp(&b.bottom()))
            .unwrap();
        assert!((now.left() - 268.0).abs() <= 1.0, "{width}: {now:?}");
        assert!((pause.width() - 40.0).abs() <= 1.0);
        assert!(now.bottom() < 900.0 && now.top() > 812.0);
        let center = if width <= 1000.0 {
            width - 24.0 - 120.0
        } else {
            (244.0 + width) * 0.5
        };
        assert!(
            (pause.center().x - center).abs() <= 1.0,
            "{width}: {pause:?}"
        );
        let volume = harness.get_by_label("Volume").rect();
        assert!(harness.ctx.content_rect().contains_rect(volume));
        assert!(!volume.intersects(pause));
        if width <= 1000.0 {
            assert!(harness.query_by_label("Playback speed 1×").is_none());
        } else {
            let stop = harness.get_by_label("Stop").rect();
            assert!((width - stop.right() - 24.0).abs() <= 1.0);
            let speed = harness.get_by_label("Playback speed 1×");
            let trigger = speed.rect();
            speed.click();
            harness.run_steps(3);
            let last_option = harness.get_by_label("3×").rect();
            assert!(last_option.bottom() < trigger.top());
            harness.key_press(egui::Key::Escape);
            harness.run_steps(3);
            assert!(harness.get_by_label("Playback speed 1×").is_focused());
        }
    }
}

#[test]
fn expanded_audiobook_controls_fill_and_center_in_the_wide_player_column() {
    let mut harness = book_detail([2560.0, 1440.0]);
    start_book_fixture(harness.state_mut());
    harness.state_mut().book.full = true;
    harness.run_steps(4);

    let chapter_tab = harness.get_by_label("Chapters (2)").rect();
    let chapter_seek = harness.get_by_label("Playback position").rect();
    let pause = harness.get_by_label("Pause").rect();
    let speed = harness.get_by_label("Speed 1×").rect();
    let volume = harness.get_by_label("Volume").rect();
    let left = 244.0 + 48.0; // Sidebar and page padding.
    let right = 2560.0 - 48.0 - 360.0 - 40.0; // Padding, chapter panel and column gap.
    let center = (left + right) / 2.0;
    assert!(chapter_tab.left() > right + 40.0);
    assert!((chapter_seek.left() - left).abs() <= 1.0);
    assert!((chapter_seek.right() - right).abs() <= 1.0);
    assert!((pause.center().x - center).abs() <= 1.0);
    assert!((speed.union(volume).center().x - center).abs() <= 1.0);
    assert!(
        harness.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::LineSegment { points: [a, b], .. }
                if a.y == b.y && a.y > chapter_seek.bottom() && a.y < pause.top()
                    && (a.x - left).abs() <= 1.0 && (b.x - right).abs() <= 1.0)
        }),
        "The whole-book timeline should span the same column as the chapter scrubber"
    );
}

#[test]
fn audiobook_volume_is_adjustable_from_the_dock_and_full_player() {
    let mut harness = book_detail([860.0, 620.0]);
    start_book_fixture(harness.state_mut());
    for full in [false, true] {
        harness.state_mut().book.full = full;
        harness.run_steps(4);
        let volume = harness.get_by_label("Volume");
        assert!(harness.ctx.content_rect().contains_rect(volume.rect()));
        volume.click();
        harness.run_steps(3);
        let slider = harness.get_by_label("Audiobook volume");
        assert!(harness.ctx.content_rect().contains_rect(slider.rect()));
        let previous = harness.state().volume.gain();
        slider.focus();
        harness.run_steps(2);
        harness.key_press(egui::Key::ArrowLeft);
        harness.run_steps(3);
        let adjusted = harness.state().volume.gain();
        assert!(adjusted > 0.0 && adjusted < previous);
        harness.get_by_label("Mute").click();
        harness.run_steps(3);
        assert_eq!(harness.state().volume.gain(), 0.0);
        harness.get_by_label("Unmute").click();
        harness.run_steps(3);
        assert_eq!(harness.state().volume.gain(), adjusted);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(3);
        assert!(harness.query_by_label("Audiobook volume").is_none());
    }
}

// Review the real library without network requests, credentials or desktop windows.
fn library(size: [f32; 2], screen: &str) -> Harness<'static, App> {
    let mut harness = app_harness(size, 0.25);
    let ctx = harness.ctx.clone();
    let app = harness.state_mut();
    app.source = Some(fixture_source("http://127.0.0.1:32499"));
    app.sections = vec![
        Library {
            id: "1".into(),
            title: "Movies".into(),
            kind: Kind::Movie,
        },
        Library {
            id: "2".into(),
            title: "TV Shows".into(),
            kind: Kind::Series,
        },
    ];
    attach_server(app);
    let titles = [
        "The Quiet Earth",
        "Beyond the Blue",
        "After Hours",
        "Northern Lights",
        "A Place Between the Stars and the Sea",
        "The Last Train",
        "Wild Coast",
        "Solstice",
        "Small Hours",
        "Open Water",
        "The Long Way Home",
        "Still Life",
    ];
    let items: Vec<_> = titles.iter().enumerate().map(|(index, title)| {
        let path = format!("/fixture/{index}");
        let art = format!("{path}/art");
        // Deterministic abstract artwork; never fetched from a live service.
        app.covers.insert(path.clone(), ctx.load_texture(&path, library_art(index, [160, 240]), Default::default()));
        app.covers.insert(art.clone(), ctx.load_texture(&art, library_art(index, [320, 180]), Default::default()));
        Item {
            id: (index + 1).to_string(), title: (*title).into(), kind: Kind::Movie,
            year: Some(2020 + index as u32 % 6), thumb: Some(path), art: Some(art),
            summary: "A journey along a forgotten coastline brings two strangers together, and changes the way they see the world.".into(),
            duration_ms: Some(7_200_000), resume_ms: (index < 3).then_some(1_200_000 + index as u64 * 900_000),
            watched: index == 5,
            versions: vec![Version {
                video_resolution: Some("4k".into()),
                video_codec: Some("hevc".into()),
                stream: Stream::Multipart,
                ..Default::default()
            }],
            ..Default::default()
        }
    }).collect();
    let home = screen == "home";
    app.selected_library = (!home).then(|| "1".into());
    app.location = Some(Location {
        title: if home { "Home" } else { "Movies" }.into(),
        browse: if home {
            Browse::RecentlyAdded
        } else {
            Browse::Library(app.sections[0].clone())
        },
        search: String::new(),
        offset: 0,
        previous_pages: vec![],
        library: !home,
        parent: None,
    });
    app.on_deck = items[..3].to_vec();
    app.page = Some(Page {
        items,
        offset: 0,
        total: Some(12),
    });
    match screen {
        "missing-art" => app.covers.clear(),
        "search" => {
            app.search = "Quiet".into();
            app.location.as_mut().unwrap().search = app.search.clone();
            app.page.as_mut().unwrap().items.truncate(1);
            app.page.as_mut().unwrap().total = Some(1);
        }
        "shows" => {
            app.selected_library = Some("2".into());
            app.location.as_mut().unwrap().title = "TV Shows".into();
            for item in &mut app.page.as_mut().unwrap().items {
                item.kind = Kind::Series;
                item.versions.clear();
            }
        }
        "empty" => app.page.as_mut().unwrap().items.clear(),
        "loading" => {
            app.page = None;
            app.busy = true;
        }
        "details" => app.detail = Some(app.page.as_ref().unwrap().items[0].clone()),
        "episodes" => {
            app.selected_library = Some("2".into());
            let parent = app.page.as_ref().unwrap().items[0].clone();
            let location = app.location.as_mut().unwrap();
            location.title = "Season 1".into();
            location.library = false;
            location.parent = Some(parent);
            for (index, item) in app.page.as_mut().unwrap().items.iter_mut().enumerate() {
                item.kind = Kind::Episode;
                item.series_title = Some("The Quiet Earth".into());
                item.season_number = Some(1);
                item.index = Some(index as u32 + 1);
            }
        }
        _ => {}
    }
    harness
}

#[test]
fn library_layout_and_scrolling() {
    for (size, scale) in [
        ([860.0, 620.0], 1.0),
        ([1440.0, 900.0], 1.0),
        ([2560.0, 1440.0], 1.0),
        ([860.0, 620.0], 1.5),
    ] {
        for view in [
            "home",
            "movies",
            "shows",
            "episodes",
            "details",
            "search",
            "empty",
            "loading",
            "missing-art",
        ] {
            let mut harness = library(size, view);
            harness.set_pixels_per_point(scale);
            if scale != 1.0 {
                harness.step();
                harness.set_size(size.into());
            }
            harness.run_steps(4);
            // AccessKit reports physical pixels; egui's layout uses logical points.
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::from(size) * scale);
            assert!(screen.contains_rect(harness.get_by_label("Servers & settings").rect()));
            // Vertical scrolling is intentional. Cards must never overflow horizontally.
            for node in harness.query_all_by_label_contains("Open ") {
                let rect = node.rect();
                if view == "home"
                    && rect.top() < harness.get_by_label("Recently Added").rect().top()
                {
                    // Continue Watching intentionally scrolls horizontally.
                    assert_eq!(rect.size(), Vec2::new(232.0, 130.0) * scale);
                    continue;
                }
                assert!(
                    rect.left() >= 208.0 * scale && rect.right() <= size[0] * scale,
                    "{view}: artwork overflows: {rect:?}"
                );
            }
            if view == "home" {
                let continue_heading = harness.get_by_label("Continue Watching").rect();
                let recent_heading = harness.get_by_label("Recently Added").rect();
                assert!(
                    screen.contains_rect(continue_heading) && screen.contains_rect(recent_heading)
                );
                assert!(recent_heading.top() > continue_heading.bottom() + 150.0 * scale);
            }
            if view == "movies" {
                harness.get_by_label("Still Life").scroll_to_me();
                harness.run_steps(4);
                assert!(screen.contains_rect(harness.get_by_label("Still Life").rect()));
            }
            if view == "details" {
                let art = harness.get_by_label("Cover of The Quiet Earth").rect();
                let resume = harness.get_by_label("Resume").rect();
                assert!(screen.contains_rect(resume));
                assert!(resume.top() > art.bottom(), "Resume overlaps the hero");
            }
        }
    }
}

#[test]
fn library_search_entry_and_keyboard_navigation() {
    let mut harness = library([860.0, 620.0], "movies");
    harness.run_steps(4);
    harness.get_by_label("Search library").focus();
    harness.run();
    harness
        .get_by_label("Search library")
        .type_text("Northern Lights");
    harness.run();
    assert_eq!(harness.state().search, "Northern Lights");
    let screen = harness.ctx.content_rect();
    for label in ["Search library", "Search", "Clear"] {
        assert!(
            screen.contains_rect(harness.get_by_label(label).rect()),
            "{label} is clipped"
        );
    }
    harness.key_press(egui::Key::Enter);
    harness.run_steps(4);
    assert_eq!(
        harness.state().location.as_ref().unwrap().search,
        "Northern Lights"
    );
    // Custom painted navigation still supports keyboard activation and exposes labels.
    harness.get_by_label("Servers & settings").focus();
    harness.run_steps(4);
    harness.key_press(egui::Key::Space);
    harness.run_steps(4);
    assert!(harness.state().connection_open);
}

#[test]
fn parent_libraries_share_search_and_pagination_at_minimum_size() {
    let size = [860.0, 620.0];
    let mut movies = library(size, "search");
    movies.state_mut().page.as_mut().unwrap().total = Some(2);
    let mut books = book_library_search(size);
    movies.run_steps(4);
    books.run_steps(4);

    for label in [
        "Refresh",
        "Search library",
        "Search",
        "Clear",
        "Search Results",
        "Previous page",
        "Next page",
    ] {
        let rect = movies.get_by_label(label).rect();
        assert_eq!(books.get_by_label(label).rect(), rect, "{label}");
        assert!(books.ctx.content_rect().contains_rect(rect), "{label}");
    }
    let controls = ["Search library", "Search", "Clear", "Refresh"]
        .map(|label| books.get_by_label(label).rect());
    for pair in controls.windows(2) {
        assert!(pair[0].right() < pair[1].left());
        assert!((pair[0].center().y - pair[1].center().y).abs() <= 1.0);
    }
    let mut wide = book_library_search([1440.0, 900.0]);
    wide.run_steps(4);
    let wide_search = wide.get_by_label("Search library").rect();
    assert_eq!(wide_search.width(), 360.0);
    assert!(controls[0].width() < wide_search.width());

    let poster = movies.get_by_label("Open The Quiet Earth").rect();
    let cover = books.get_by_label("Open Listening to the Coast").rect();
    assert_eq!(poster.min, cover.min);
    assert_eq!(poster.width(), cover.width());
    assert_eq!(cover.width(), cover.height());

    for (harness, query) in [(&mut movies, "Quiet"), (&mut books, "Coast")] {
        harness.get_by_label("Next page").click();
        harness.run_steps(4);
        let location = harness.state().location.as_ref().unwrap();
        assert_eq!(location.offset, 1);
        assert_eq!(location.previous_pages, [0]);
        assert_eq!(location.search, query);
    }
}

#[test]
fn library_hover_cursors_match_the_control() {
    let size = [860.0, 620.0];
    let movies = library(size, "movies");
    let mut books = book_detail(size);
    let ctx = books.ctx.clone();
    show_book_collection(books.state_mut(), &ctx, false);

    for (mut harness, title) in [
        (movies, "The Quiet Earth"),
        (books, "Listening to the Coast"),
    ] {
        harness.run_steps(4);
        for (label, cursor) in [
            ("Servers & settings", egui::CursorIcon::PointingHand),
            ("Search library", egui::CursorIcon::Text),
            ("Search", egui::CursorIcon::PointingHand),
            (&format!("Open {title}"), egui::CursorIcon::PointingHand),
            (title, egui::CursorIcon::PointingHand),
        ] {
            harness.get_by_label(label).scroll_to_me();
            harness.run_steps(4);
            harness.get_by_label(label).hover();
            harness.run_steps(2);
            assert_eq!(
                harness.output().platform_output.cursor_icon,
                cursor,
                "{label}"
            );
        }
    }
}

#[test]
fn server_status_popover_hover_pin_and_keyboard() {
    let mut harness = library([860.0, 620.0], "home");
    let trigger = "1 server connected";
    harness.run_steps(3);
    harness.get_by_label(trigger).hover();
    harness.run_steps(5);
    let manage = harness.get_by_label("Manage servers").rect();
    assert!(manage.bottom() < harness.get_by_label(trigger).rect().top());
    harness.hover_at(manage.center());
    harness.run_steps(3);
    assert!(harness.query_by_label("Everything is reachable.").is_some());
    harness.remove_cursor();
    harness.run_steps(3);
    assert!(harness.query_by_label("Manage servers").is_none());

    harness.get_by_label(trigger).click();
    harness.run_steps(3);
    harness.remove_cursor();
    harness.run_steps(3);
    assert!(harness.query_by_label("Manage servers").is_some());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(harness.query_by_label("Manage servers").is_none());

    harness.state_mut().servers[0].online = false;
    harness.get_by_label("Servers & settings").focus();
    harness.run_steps(3);
    harness.get_by_label("Cannot connect to servers").focus();
    harness.run_steps(3);
    assert!(harness.query_by_label("No servers reachable").is_some());
    assert!(harness.query_by_label("Retry").is_some());
    harness.get_by_label("Manage servers").click();
    harness.run_steps(3);
    assert!(harness.state().connection_open);
}

#[test]
#[ignore = "offscreen library design review; run explicitly to export PNGs"]
fn library_snapshots() {
    std::fs::create_dir_all(".artifacts/ui").unwrap();
    let mut failures = Vec::new();
    for (name, screen, size, scale) in [
        ("home-desktop", "home", [1440.0, 900.0], 1.0),
        ("home-minimum", "home", [860.0, 620.0], 1.0),
        ("server-home-minimum", "server-home", [860.0, 620.0], 1.0),
        ("server-status", "home", [860.0, 620.0], 1.0),
        ("home-library-menu", "home", [1280.0, 800.0], 1.0),
        ("movies-desktop", "movies", [1440.0, 900.0], 1.0),
        ("movies-minimum", "movies", [860.0, 620.0], 1.0),
        ("movies-high-dpi", "movies", [860.0, 620.0], 1.5),
        ("shows-minimum", "shows", [860.0, 620.0], 1.0),
        ("episodes-minimum", "episodes", [860.0, 620.0], 1.0),
        ("details-desktop", "details", [1440.0, 900.0], 1.0),
        ("details-minimum", "details", [860.0, 620.0], 1.0),
        ("audiobook-home", "book-home", [1440.0, 960.0], 1.0),
        ("audiobook-home-minimum", "book-home", [860.0, 620.0], 1.0),
        ("audiobook-details", "book-details", [1280.0, 900.0], 1.0),
        ("audiobook-library", "book-dock", [1440.0, 900.0], 1.0),
        (
            "audiobook-library-minimum",
            "book-search",
            [860.0, 620.0],
            1.0,
        ),
        ("audiobook-dock-volume", "book-dock", [1440.0, 900.0], 1.0),
        ("audiobook-dock-wide", "book-dock", [2560.0, 900.0], 1.0),
        (
            "audiobook-player-wide",
            "book-player",
            [2560.0, 1440.0],
            1.0,
        ),
        (
            "audiobook-player-menu",
            "book-player-menu",
            [1480.0, 1080.0],
            1.0,
        ),
        (
            "audiobook-player-minimum",
            "book-player",
            [860.0, 620.0],
            1.0,
        ),
        ("library-empty", "empty", [860.0, 620.0], 1.0),
        ("library-loading", "loading", [860.0, 620.0], 1.0),
        ("library-search", "search", [860.0, 620.0], 1.0),
        ("library-missing-art", "missing-art", [860.0, 620.0], 1.0),
    ] {
        let mut harness = if screen == "book-search" {
            book_library_search(size)
        } else if screen.starts_with("book-") {
            book_detail(size)
        } else if screen == "server-home" {
            library(size, "home")
        } else {
            library(size, screen)
        };
        if screen == "book-home" {
            let ctx = harness.ctx.clone();
            show_book_collection(harness.state_mut(), &ctx, true);
        }
        if screen == "server-home" {
            let app = harness.state_mut();
            app.servers[0].prefs.name = "Basement".into();
            app.location.as_mut().unwrap().title = app.servers[0].name();
            let page = app.page.as_mut().unwrap();
            page.total = Some(page.items.len() + 1);
        }
        if matches!(screen, "book-dock" | "book-player" | "book-player-menu") {
            let ctx = harness.ctx.clone();
            let app = harness.state_mut();
            if screen == "book-player-menu" {
                // Keep the chapter list longer than the viewport so this comparison
                // catches a menu pushing, clipping or displacing the scrolling pane.
                let item = app.detail.as_mut().unwrap();
                let chapters = &mut item.book.as_mut().unwrap().chapters;
                for index in 3..=20 {
                    let start = chapters.last().unwrap().end_ms;
                    chapters.push(Chapter {
                        title: format!("Coastal walk {index}"),
                        start_ms: start,
                        end_ms: start + 300_000,
                    });
                }
                item.duration_ms = Some(chapters.last().unwrap().end_ms);
            }
            start_book_fixture(app);
            app.book.full = screen != "book-dock";
            if screen == "book-dock" {
                show_book_collection(app, &ctx, false);
            }
        }
        if screen == "home" {
            let app = harness.state_mut();
            app.servers[0].prefs.name = "Glass".into();
            app.servers[0].sections.push(Library {
                id: "3".into(),
                title: "Lectures".into(),
                kind: Kind::Series,
            });
            app.servers[0].items = app.page.as_ref().unwrap().items.clone();
            app.servers[0].artwork = app.covers.clone();
            let session = session::Session {
                kind: ServerKind::Plex,
                server: "http://second.example.test".into(),
                token: "synthetic".into(),
                client_id: "ui-test".into(),
            };
            let mut second = Server::new(
                session.clone(),
                fixture_source(&session.server),
                app.sections.clone(),
                false,
            );
            second.prefs.name = "Living Room".into();
            app.servers.push(second);
            app.overview = true;
        }
        if name == "home-library-menu" {
            let server = &mut harness.state_mut().servers[0];
            server.sections.push(Library {
                id: "4".into(),
                title: "Concert films".into(),
                kind: Kind::Movie,
            });
        }
        harness.set_pixels_per_point(scale);
        // A zoom change rescales the first frame's input rectangle. Restore
        // logical window bounds afterwards so 150% DPI exports 1290x930 pixels.
        if scale != 1.0 {
            harness.step();
            harness.set_size(size.into());
        }
        harness.remove_cursor();
        harness.run_steps(4);
        if name == "audiobook-dock-volume" {
            harness.get_by_label("Volume").click();
            harness.run_steps(3);
            harness.get_by_label("Audiobook volume");
        }
        if screen == "book-dock" {
            let first = harness.get_by_label("Open Listening to the Coast").rect();
            let second = harness.get_by_label("Open Tide Tables").rect();
            assert!((first.width() - first.height()).abs() <= 1.0);
            assert_eq!(first.top(), second.top());
            assert!((second.left() - first.right() - 24.0).abs() <= 1.0);
            harness.get_by_label("Search library");
        }
        if screen == "book-player-menu" {
            harness.get_by_label("Speed 1×").click();
            harness.run_steps(3);
            let row_y = harness.get_by_label("Pause").rect().center().y;
            for label in [
                "Previous chapter",
                "Back 30 seconds",
                "Forward 30 seconds",
                "Next chapter",
            ] {
                let rect = harness.get_by_label(label).rect();
                assert!(harness.ctx.content_rect().contains_rect(rect));
                assert!(
                    (rect.center().y - row_y).abs() <= 1.0,
                    "{label} is off-center"
                );
            }
        }
        if name == "server-status" {
            harness.get_by_label("2 servers connected").hover();
            harness.run_steps(5);
            assert!(harness.query_by_label("All servers connected").is_some());
        }
        if screen == "loading" {
            // The initial chooser can take extra frames to size and load its icons.
            // Capture the library spinner at a fixed time regardless of that work.
            harness.input_mut().time = Some(8.0);
            harness.step();
        }
        if name == "home-library-menu" {
            harness.get_by_label("Show all libraries on Glass").hover();
            harness.run_steps(5);
            harness.get_by_label("Libraries on Glass");
        } else if screen == "home" {
            let card_header = |name| {
                harness
                    .get_by_role_and_label(egui::accesskit::Role::Button, name)
                    .rect()
            };
            let glass = card_header("Glass");
            let other = card_header("Living Room");
            assert!((glass.width() - 280.0).abs() < 1.0);
            assert!((glass.width() - other.width()).abs() < 1.0);
            assert!(other.right() <= harness.ctx.content_rect().right());
            let chip = |name| {
                harness
                    .get_all_by_label(name)
                    .map(|node| node.rect())
                    .find(|r| glass.x_range().contains(r.center().x) && r.top() >= glass.bottom())
                    .unwrap()
            };
            let movies = chip("Movies");
            for name in ["TV Shows", "Lectures"] {
                assert!((chip(name).top() - movies.top()).abs() < 1.0);
            }
        }
        let image = harness
            .render()
            .expect("offscreen rendering must be available");
        assert_eq!(
            image.dimensions(),
            ((size[0] * scale) as u32, (size[1] * scale) as u32)
        );
        image.save(format!(".artifacts/ui/{name}.png")).unwrap();
        if let Err(error) = egui_kittest::try_image_snapshot(&image, name) {
            failures.push(error.to_string());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn playback_controls_fit_the_window() {
    for size in [[860.0, 620.0], [1440.0, 900.0], [2560.0, 1440.0]] {
        let mut harness = playback(size);
        harness.run_steps(3);
        let screen = harness.ctx.content_rect();
        let mut previous = None;
        for label in ["Back 10s", "Pause", "Forward 10s"] {
            let rect = harness.get_by_label(label).rect();
            assert!(screen.contains_rect(rect), "{label} is clipped: {rect:?}");
            assert!(rect.width() < 200.0, "{label} is stretched: {rect:?}");
            if let Some(right) = previous {
                assert!(rect.left() >= right, "Playback buttons overlap");
            }
            previous = Some(rect.right());
        }
        for label in [
            "Fullscreen",
            "Audio track",
            "Subtitles",
            "Previous chapter",
            "Next chapter",
        ] {
            let rect = harness.get_by_label(label).rect();
            assert!(screen.contains_rect(rect), "{label} is clipped: {rect:?}");
        }
        for node in harness.get_all_by_label("Volume") {
            assert!(
                screen.contains_rect(node.rect()),
                "Volume control is clipped"
            );
        }
    }
}

#[test]
fn playback_fullscreen_button_and_shortcuts() {
    fn expect_fullscreen(harness: &mut Harness<'_, App>, expected: bool) {
        harness.run_steps(3);
        assert_eq!(harness.state().fullscreen, expected);
    }

    let mut harness = playback([860.0, 620.0]);
    harness.run_steps(3);
    harness.get_by_label("Fullscreen").click();
    expect_fullscreen(&mut harness, true);
    assert!(
        harness
            .ctx
            .content_rect()
            .contains_rect(harness.get_by_label("Exit fullscreen").rect())
    );
    harness.get_by_label("Exit fullscreen").click();
    expect_fullscreen(&mut harness, false);
    harness.key_press(egui::Key::F11);
    expect_fullscreen(&mut harness, true);
    harness.key_press(egui::Key::Escape);
    expect_fullscreen(&mut harness, false);
}

#[test]
fn fullscreen_controls_hide_and_reveal_the_video() {
    fn advance(harness: &mut Harness<'_, App>, seconds: f64) {
        harness.input_mut().time = Some(harness.ctx.input(|i| i.time) + seconds);
        harness.step();
        harness.input_mut().time = None;
        harness.run_steps(30); // Let opacity transitions finish without wall-clock sleeps.
    }

    let mut harness = playback([1440.0, 900.0]);
    harness.run_steps(3);
    advance(&mut harness, 5.0);
    harness.get_by_label("Pause"); // Windowed controls stay visible.
    harness.key_press(egui::Key::F11);
    harness.run_steps(3);
    harness.get_by_label("Exit fullscreen");
    let last_active = harness.state().last_playback_interaction;
    harness.input_mut().time = Some(last_active + 1.9);
    harness.step();
    harness.get_by_label("Pause");
    harness.input_mut().time = Some(last_active + 2.1);
    harness.step();
    harness.input_mut().time = None;
    harness.run_steps(3);
    assert!(harness.state().controls_opacity > 0.0 && harness.state().controls_opacity < 1.0);
    assert_eq!(harness.ctx.available_rect(), harness.ctx.content_rect());
    harness.run_steps(30); // The 375ms fade-out.
    assert_eq!(harness.state().controls_opacity, 0.0);
    assert!(harness.query_by_label("Pause").is_none());
    assert!(harness.query_by_label("Exit fullscreen").is_none());
    assert_eq!(harness.ctx.available_rect(), harness.ctx.content_rect());

    // An isolated twitch, or separate short movements, must not reveal the UI.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(700.0, 400.0)));
    harness.run_steps(2);
    assert!(harness.query_by_label("Pause").is_none());
    advance(&mut harness, 1.1);
    assert!(harness.query_by_label("Pause").is_none());
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(710.0, 400.0)));
    harness.run_steps(2);
    assert!(harness.query_by_label("Pause").is_none());
    advance(&mut harness, 0.5);

    // Continuous movement for one second reveals both panels.
    let start = harness.ctx.input(|i| i.time) + 0.1;
    let mut saw_fade_in = false;
    for sample in 0..=6 {
        harness.input_mut().time = Some(start + f64::from(sample) * 0.2);
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(egui::pos2(
                720.0 + sample as f32 * 10.0,
                400.0,
            )));
        harness.run_steps(2);
        if sample < 5 {
            assert!(harness.query_by_label("Pause").is_none());
        }
        if !saw_fade_in && harness.state().controls_opacity > 0.0 {
            assert!(harness.state().controls_opacity < 1.0);
            saw_fade_in = true;
        }
    }
    assert!(saw_fade_in);
    harness.input_mut().time = None;
    harness.run_steps(24); // The 300ms fade-in at the harness frame rate.
    assert_eq!(harness.state().controls_opacity, 1.0);
    harness.get_by_label("Pause");
    harness.get_by_label("Exit fullscreen");
    // The controls never reserve any video space, even at full opacity.
    assert_eq!(harness.ctx.available_rect(), harness.ctx.content_rect());
    assert!(
        !harness.output().shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Rect(rect)
            if rect.fill == PANEL && rect.rect.width() > 1000.0)
        }),
        "Playback controls have an opaque panel background"
    );
    advance(&mut harness, 2.1);
    assert!(harness.query_by_label("Pause").is_none());

    harness.key_press(egui::Key::Space);
    harness.run_steps(2);
    harness.get_by_label("Pause");
    advance(&mut harness, 2.1);
    assert!(harness.query_by_label("Pause").is_none());
    harness.key_press(egui::Key::Escape);
    harness.run_steps(3);
    assert!(!harness.state().fullscreen);
    harness.get_by_label("Fullscreen");
    harness.get_by_label("Pause");
}

#[test]
fn login_controls_fit_the_window() {
    for size in [[860.0, 620.0], [1440.0, 900.0]] {
        let mut harness = login(size);
        harness.run();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size.into());
        assert!(harness.query_by_label("Sign in with Plex").is_none());
        assert!(harness.query_by_label("Server address").is_none());
        let plex = harness.get_by_label("Plex");
        let jellyfin = harness.get_by_label("Jellyfin").rect();
        let audiobookshelf = harness.get_by_label("Audiobookshelf").rect();
        assert!(screen.contains_rect(plex.rect()) && screen.contains_rect(jellyfin));
        if size[0] > 1000.0 {
            assert_eq!(plex.rect().top(), jellyfin.top());
            assert_eq!(jellyfin.top(), audiobookshelf.top());
            assert!(
                audiobookshelf.left() > jellyfin.right() && screen.contains_rect(audiobookshelf)
            );
        } else {
            assert!(jellyfin.top() > plex.rect().bottom());
            assert!(audiobookshelf.top() > jellyfin.bottom());
        }
        plex.click();
        harness.run();
        let sign_in = harness.get_by_label("Sign in with Plex").rect();
        let remember = harness.get_by_label("Remember me on this computer").rect();
        assert!(
            screen.contains_rect(sign_in),
            "Sign-in button is clipped: {sign_in:?}"
        );
        assert!(
            screen.contains_rect(remember),
            "Remember me is clipped: {remember:?}"
        );
        assert!(remember.top() >= sign_in.bottom(), "Login controls overlap");
    }
}

#[test]
fn manual_login_accepts_keyboard_input_and_scrolls_into_view() {
    let mut harness = login([860.0, 620.0]);
    harness.run();
    harness.get_by_label("Plex").click();
    harness.run();
    harness
        .get_by_label("Connect manually with an address and token")
        .click();
    harness.run();
    harness.get_by_label("Server address").focus();
    harness.run();
    assert!(harness.get_by_label("Server address").is_focused());
    // Replace the default address using Windows' Ctrl+A shortcut.
    harness.key_press_modifiers(
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
        egui::Key::A,
    );
    harness
        .get_by_label("Server address")
        .type_text("http://localhost:32499");
    harness.run();
    assert_eq!(harness.state().server, "http://localhost:32499");
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(harness.get_by_label("Access token").is_focused());
    harness
        .get_by_label("Access token")
        .type_text("synthetic-ui-token");
    harness.run();
    assert_eq!(harness.state().token, "synthetic-ui-token");
    assert_eq!(
        harness.get_by_label("Access token").accesskit_node().role(),
        egui::accesskit::Role::PasswordInput
    );
    harness.get_by_label("Connect").scroll_to_me();
    harness.run();
    let screen = harness.ctx.content_rect();
    assert!(screen.contains_rect(harness.get_by_label("Connect").rect()));
    // Intentionally don't click Connect: UI tests never access a real server.
}

#[test]
fn add_server_dialog_grows_to_fit_each_form_and_keeps_headings_left_aligned() {
    let mut harness = library([1280.0, 800.0], "movies");
    harness.state_mut().connection_open = true;
    harness.run_steps(4);
    harness.get_by_label("Add server").click();
    harness.run_steps(4);
    let bounds = |h: &Harness<'_, App>| h.ctx.memory(|m| m.area_rect("add_server").unwrap());
    let chooser = bounds(&harness);
    assert!((chooser.width() - 640.0).abs() <= 1.0);
    assert!(
        (harness.get_by_label("Add a server").rect().left() - chooser.left() - 32.0).abs() <= 1.0
    );
    for (kind, title, last) in [
        ("Jellyfin", "Connect to Jellyfin", "Use Quick Connect"),
        ("Audiobookshelf", "Connect to Audiobookshelf", "Connect"),
        ("Plex", "Connect to Plex", "Connect"),
    ] {
        harness.get_by_label(kind).click();
        harness.run_steps(4);
        if kind == "Plex" {
            harness
                .get_by_label("Connect manually with an address and token")
                .click();
            harness.run_steps(4);
        }
        let form = bounds(&harness);
        assert!(
            (form.width() - 640.0).abs() <= 1.0,
            "{kind} width: {form:?}"
        );
        assert!(form.height() > chooser.height());
        assert!(
            form.shrink(30.0)
                .contains_rect(harness.get_by_label(last).rect())
        );
        assert!((harness.get_by_label(title).rect().left() - form.left() - 128.0).abs() <= 1.0);
        harness.get_by_label("Back to server types").click();
        harness.run_steps(4);
    }
    harness.get_by_label("Close").click();
    harness.run_steps(4);
    assert!(harness.get_by_label("Add server").is_focused());
}

#[test]
fn jellyfin_sign_in_isolated_from_plex_and_quick_connect_can_be_cancelled() {
    let mut harness = login([860.0, 620.0]);
    harness.state_mut().token = "synthetic-plex-token".into();
    harness.run();
    harness.get_by_label("Jellyfin").click();
    harness.run();
    assert!(harness.state().server.is_empty() && harness.state().token.is_empty());
    assert!(harness.get_by_label("Server address").is_focused());
    let username = harness.get_by_label("Username").rect();
    let password = harness.get_by_label("Password");
    assert!(password.rect().left() > username.right());
    assert_eq!(
        password.accesskit_node().role(),
        egui::accesskit::Role::PasswordInput
    );
    harness.get_by_label("Connect").scroll_to_me();
    harness.run();
    harness.get_by_label("Connect").click();
    harness.run();
    harness.get_by_label("Check the highlighted fields");
    harness.get_by_label("Enter your server address.");
    harness.get_by_label("Enter your Jellyfin username.");
    assert!(!harness.state().busy);
    harness.state_mut().server = "http://127.0.0.1:8096".into();
    harness.get_by_label("Use Quick Connect").scroll_to_me();
    harness.run();
    assert!(
        harness
            .ctx
            .content_rect()
            .contains_rect(harness.get_by_label("Use Quick Connect").rect())
    );
    let app = harness.state_mut();
    let generation = app.begin();
    app.quick_code = Some("012345".into());
    harness.run_steps(2);
    harness.get_by_label("012 345");
    harness.get_by_label("Cancel").click();
    harness.run();
    harness
        .state()
        .tx
        .send((generation, Reply::QuickConnectCode("123456".into())))
        .unwrap();
    harness.run_steps(2);
    assert!(!harness.state().busy && harness.state().quick_code.is_none());
}

#[test]
fn audiobookshelf_requires_password_and_clears_it_on_back() {
    let password = egui::accesskit::Role::PasswordInput;
    let mut harness = login([1200.0, 900.0]);
    harness.get_by_label("Audiobookshelf").click();
    harness.run();
    harness.get_by_label("Connect to Audiobookshelf");
    assert!(harness.get_by_label("Server address").is_focused());
    assert!(harness.query_by_label("Use Quick Connect").is_none());
    harness.get_by_role_and_label(password, "Password");
    harness.get_by_label("Connect").click();
    harness.run();
    for error in [
        "Enter your server address.",
        "Enter your username.",
        "Enter your password.",
    ] {
        harness.get_by_label(error);
    }
    assert!(!harness.state().busy);
    harness.get_by_role_and_label(password, "Password").focus();
    harness.run();
    harness
        .get_by_role_and_label(password, "Password")
        .type_text("synthetic");
    harness.run();
    assert_eq!(harness.state().password, "synthetic");
    harness.get_by_label("Back to server types").click();
    harness.run();
    assert!(harness.state().password.is_empty());
    assert!(harness.get_by_label("Audiobookshelf").is_focused());
}

#[test]
fn cancelled_and_failed_login_can_be_retried() {
    let mut harness = login([860.0, 620.0]);
    harness.get_by_label("Plex").click();
    harness.run();
    let (pending, events) = auth::Login::test_channel();
    harness.state_mut().login = Some(pending);
    events
        .send(auth::Event::Browser(
            "https://app.source.tv/auth#?code=synthetic".into(),
        ))
        .unwrap();
    harness.run_steps(2);
    assert!(harness.get_by_label("Cancel").rect().is_positive());
    // OpenUrl remains egui output in this harness; no OS browser is launched.
    harness.get_by_label("Cancel").click();
    harness.run();
    assert!(harness.state().login.is_none());
    assert!(harness.state().login_url.is_none());
    assert!(
        !harness
            .get_by_label("Sign in with Plex")
            .accesskit_node()
            .is_disabled()
    );

    let (pending, events) = auth::Login::test_channel();
    harness.state_mut().login = Some(pending);
    events
        .send(auth::Event::Failed(
            "Plex sign-in expired. Please try again.".into(),
        ))
        .unwrap();
    harness.run();
    assert!(harness.state().login.is_none());
    harness.get_by_label("Plex sign-in expired. Please try again.");
    harness.get_by_label("Dismiss").click();
    harness.run();
    assert!(harness.state().error.is_none());
    harness.get_by_label("Sign in with Plex");
}

fn discovered_servers() -> Vec<auth::Server> {
    vec![
        auth::Server {
            name: "Living Room · A server with a deliberately long name".into(),
            owned: Some(true),
            access_token: Some("synthetic-resource-token".into()),
            provides: "server".into(),
            connections: vec![auth::Connection {
                uri: "https://plex.example.test:32400".into(),
                local: false,
            }],
        },
        auth::Server {
            name: "Family Library".into(),
            owned: Some(false),
            access_token: Some("synthetic-other-token".into()),
            provides: "server".into(),
            connections: vec![auth::Connection {
                uri: "http://127.0.0.1:32499".into(),
                local: true,
            }],
        },
    ]
}

#[test]
fn discovered_server_controls_fit_the_window() {
    let mut harness = login([860.0, 620.0]);
    harness.state_mut().choose_source(Some(ServerKind::Plex));
    harness.state_mut().offer_plex_servers(discovered_servers());
    harness.run();
    for server in discovered_servers() {
        let label = server.name;
        let rect = harness.get_by_label(&label).rect();
        assert!(
            harness.ctx.content_rect().contains_rect(rect),
            "Server choice is clipped: {rect:?}"
        );
    }
    harness.get_by_label("Add 1 server");
    harness.get_by_label("Family Library").click();
    harness.run();
    harness.get_by_label("Add 2 servers");
}

#[test]
fn plex_selection_keeps_connected_servers_when_a_later_server_is_offline() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let worker = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(&socket);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(line.starts_with("GET /library/sections "));
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                break;
            }
        }
        let body =
            r#"{"MediaContainer":{"Directory":[{"key":"1","type":"movie","title":"Movies"}]}}"#;
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
    });
    let mut harness = login([1280.0, 800.0]);
    let mut choices = discovered_servers();
    choices[0].connections[0].uri = address;
    choices[1].connections[0].uri = "http://127.0.0.1:0".into();
    let app = harness.state_mut();
    app.choose_source(Some(ServerKind::Plex));
    app.remember = false;
    let (login, events) = auth::Login::test_channel();
    app.login = Some(login);
    events.send(auth::Event::SignedIn(choices)).unwrap();
    harness.run_steps(3);
    harness.get_by_label("Family Library").click();
    harness.run_steps(2);
    harness.get_by_label("Add 2 servers").click();
    harness.run_steps(2);
    let deadline = Instant::now() + Duration::from_secs(8);
    while harness.state().busy && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
        harness.run_steps(1);
    }
    assert!(harness.state().error.is_some());
    assert_eq!(harness.state().servers.len(), 1);
    harness.get_by_label("Already added");
    harness
        .get_by_label("Finish with connected servers")
        .click();
    harness.run_steps(3);
    harness.get_by_label("Found 1 library: Movies");
    worker.join().unwrap();
}

#[test]
fn connected_server_can_be_named_and_finished_from_either_entry_point() {
    for settings in [false, true] {
        let mut harness = if settings {
            library([1280.0, 800.0], "movies")
        } else {
            login([1280.0, 800.0])
        };
        if settings {
            harness.state_mut().connection_open = true;
            harness.run_steps(3);
            harness.get_by_label("Add server").click();
            harness.run_steps(3);
        }
        harness.get_by_label("Jellyfin").click();
        harness.run_steps(3);
        let app = harness.state_mut();
        app.remember = false;
        let session = session::Session {
            kind: ServerKind::Jellyfin,
            server: "http://127.0.0.1:0".into(),
            token: "synthetic".into(),
            client_id: "ui-test".into(),
        };
        let source = session::open(&session).unwrap();
        let generation = app.begin();
        app.tx
            .send((
                generation,
                Reply::Connected(
                    source,
                    vec![Library {
                        id: "a".into(),
                        title: "Movies".into(),
                        kind: Kind::Movie,
                    }],
                    session,
                ),
            ))
            .unwrap();
        harness.run_steps(3);
        harness.get_by_label("Found 1 library: Movies");
        let field = harness.get_by_label("Name in Perfuse");
        assert!(field.is_focused());
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_label("Name in Perfuse").type_text("Media");
        harness.run_steps(2);
        if settings {
            harness.get_by_label("Done").click();
            harness.run_steps(3);
            assert!(harness.get_by_label("Add server").is_focused());
            assert_eq!(harness.state().active, Some(0));
        } else {
            harness.get_by_label("Add another server").click();
            harness.run_steps(3);
            harness.get_by_label("Connected so far");
            harness.get_by_label("Continue to Perfuse").click();
            harness.run_steps(3);
            assert!(harness.state().overview && harness.state().source.is_some());
        }
        assert_eq!(harness.state().servers.last().unwrap().name(), "Media");
    }
}

#[test]
#[cfg(windows)]
#[ignore = "offscreen snapshot comparison; CI runs this explicitly on Windows"]
fn login_snapshots() {
    std::fs::create_dir_all(".artifacts/ui").unwrap();
    let mut failures = Vec::new();
    for (name, size, scale) in [
        ("login-minimum", [860.0, 620.0], 1.0),
        ("login-desktop", [1440.0, 900.0], 1.0),
        ("login-high-dpi", [860.0, 620.0], 1.5),
        ("login-plex", [1200.0, 900.0], 1.0),
        ("login-manual", [860.0, 620.0], 1.0),
        ("login-jellyfin", [1200.0, 900.0], 1.0),
        ("login-audiobookshelf", [1200.0, 900.0], 1.0),
        ("add-server-types", [1280.0, 800.0], 1.0),
        ("remove-server", [1100.0, 800.0], 1.0),
        ("add-server-plex-manual", [1280.0, 800.0], 1.0),
        ("add-server-jellyfin", [1280.0, 800.0], 1.0),
        ("add-server-audiobookshelf", [1280.0, 800.0], 1.0),
        ("login-waiting", [860.0, 620.0], 1.0),
        ("login-expired", [860.0, 620.0], 1.0),
        ("login-servers", [860.0, 620.0], 1.0),
        ("login-download-permission", [860.0, 620.0], 1.0),
        ("login-settings", [860.0, 620.0], 1.0),
        ("settings-desktop", [1280.0, 800.0], 1.0),
        ("login-libraries", [1100.0, 900.0], 1.0),
    ] {
        let mut harness = login(size);
        harness.set_pixels_per_point(scale);
        if scale != 1.0 {
            harness.step();
            harness.set_size(size.into());
        }
        match name {
            "login-plex" => {
                harness.get_by_label("Plex").click();
            }
            "login-jellyfin" => {
                harness.get_by_label("Jellyfin").click();
            }
            "login-audiobookshelf" => {
                harness.get_by_label("Audiobookshelf").click();
            }
            "login-manual" => {
                harness.get_by_label("Plex").click();
                harness.run();
                harness
                    .get_by_label("Connect manually with an address and token")
                    .click();
            }
            "login-waiting" => {
                harness.state_mut().choose_source(Some(ServerKind::Plex));
                let (pending, _events) = auth::Login::test_channel();
                harness.state_mut().login = Some(pending);
                harness.state_mut().login_url =
                    Some("https://app.source.tv/auth#?code=synthetic".into());
            }
            "login-expired" => {
                harness.state_mut().choose_source(Some(ServerKind::Plex));
                harness.state_mut().error =
                    Some("Plex sign-in expired. Sign in again to continue.".into());
            }
            "login-servers" => {
                harness.state_mut().choose_source(Some(ServerKind::Plex));
                harness.state_mut().offer_plex_servers(discovered_servers());
            }
            "login-download-permission" => {
                harness.state_mut().runtime_prompt = Some(RuntimePrompt::FirstRun {
                    download: true,
                    error: None,
                });
            }
            "login-settings"
            | "settings-desktop"
            | "login-libraries"
            | "remove-server"
            | "add-server-types"
            | "add-server-plex-manual"
            | "add-server-audiobookshelf"
            | "add-server-jellyfin" => {
                let app = harness.state_mut();
                app.source = Some(fixture_source("http://127.0.0.1:32499"));
                app.sections.push(Library {
                    id: "1".into(),
                    title: "Movies".into(),
                    kind: Kind::Movie,
                });
                attach_server(app);
                app.servers[0].prefs.name = "Living Room".into();
                app.servers[0].sections.push(Library {
                    id: "2".into(),
                    title: "TV Shows".into(),
                    kind: Kind::Series,
                });
                if name == "settings-desktop" {
                    let address = "http://archive.example.test:32400";
                    let session = session::Session {
                        kind: ServerKind::Plex,
                        server: address.into(),
                        token: "synthetic".into(),
                        client_id: "ui-test".into(),
                    };
                    let mut archive = Server::new(
                        session,
                        fixture_source(address),
                        app.sections.clone(),
                        false,
                    );
                    archive.prefs.name = "Archive".into();
                    archive.online = false;
                    app.servers.push(archive);
                }
                app.settings_tab = if name == "login-libraries" {
                    SettingsTab::Updates
                } else {
                    SettingsTab::Servers
                };
                app.connection_open = true;
                app.runtime_installed.libraries = runtime_versions_fixture();
            }
            _ => {}
        }
        if name.starts_with("add-server-") {
            harness.run_steps(4);
            harness.get_by_label("Add server").click();
            harness.run_steps(4);
            if name == "add-server-jellyfin" {
                harness.get_by_label("Jellyfin").click();
            } else if name == "add-server-audiobookshelf" {
                harness.get_by_label("Audiobookshelf").click();
            } else if name == "add-server-plex-manual" {
                harness.get_by_label("Plex").click();
                harness.run_steps(4);
                harness
                    .get_by_label("Connect manually with an address and token")
                    .click();
            }
        }
        if name == "remove-server" {
            harness.run_steps(4);
            harness.get_by_label("Remove server").click();
        }
        harness.remove_cursor();
        harness.run_steps(4);
        if name == "login-manual" {
            harness.get_by_label("Connect").scroll_to_me();
            harness.run_steps(4);
        }
        if name == "login-libraries" {
            harness.get_by_label("Update").scroll_to_me();
            harness.run_steps(4);
        }
        let image = harness
            .render()
            .expect("offscreen rendering must be available");
        image.save(format!(".artifacts/ui/{name}.png")).unwrap();
        // DirectX adapters can rasterize one row at the scrolled clip edge differently.
        let clip_edge_pixels = match name {
            "login-manual" => 80,
            "login-libraries" => 32,
            _ => 0,
        };
        let options =
            egui_kittest::SnapshotOptions::default().failed_pixel_count_threshold(clip_edge_pixels);
        if let Err(error) = egui_kittest::try_image_snapshot_options(&image, name, &options) {
            failures.push(error.to_string());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn window_edges_and_corners_resize_unless_maximized_or_fullscreen() {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 700.0));
    assert_eq!(
        resize_direction(rect, egui::pos2(2.0, 2.0), true),
        Some(egui::ResizeDirection::NorthWest)
    );
    assert_eq!(
        resize_direction(rect, egui::pos2(998.0, 2.0), true),
        Some(egui::ResizeDirection::NorthEast)
    );
    assert_eq!(
        resize_direction(rect, egui::pos2(2.0, 698.0), true),
        Some(egui::ResizeDirection::SouthWest)
    );
    assert_eq!(
        resize_direction(rect, egui::pos2(998.0, 698.0), true),
        Some(egui::ResizeDirection::SouthEast)
    );
    assert_eq!(
        resize_direction(rect, egui::pos2(500.0, 2.0), true),
        Some(egui::ResizeDirection::North)
    );
    assert_eq!(resize_direction(rect, egui::pos2(500.0, 2.0), false), None);
    assert_eq!(
        resize_direction(rect, egui::pos2(2.0, 2.0), false),
        Some(egui::ResizeDirection::West)
    );
    assert_eq!(
        resize_direction(rect, egui::pos2(500.0, 698.0), true),
        Some(egui::ResizeDirection::South)
    );
    assert_eq!(
        resize_direction(rect, egui::pos2(2.0, 350.0), true),
        Some(egui::ResizeDirection::West)
    );
    assert_eq!(
        resize_direction(rect, egui::pos2(998.0, 350.0), true),
        Some(egui::ResizeDirection::East)
    );
    assert_eq!(resize_direction(rect, egui::pos2(500.0, 350.0), true), None);

    let mut harness = login([1000.0, 700.0]);
    harness.run_steps(2);
    for (pos, expected_dir, expected_cursor) in [
        (
            egui::pos2(2.0, 350.0),
            egui::ResizeDirection::West,
            egui::CursorIcon::ResizeHorizontal,
        ),
        (
            egui::pos2(998.0, 698.0),
            egui::ResizeDirection::SouthEast,
            egui::CursorIcon::ResizeNwSe,
        ),
        (
            egui::pos2(500.0, 2.0),
            egui::ResizeDirection::North,
            egui::CursorIcon::ResizeVertical,
        ),
    ] {
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(pos));
        harness.step();
        assert_eq!(
            harness.output().platform_output.cursor_icon,
            expected_cursor
        );
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        let commands = &harness.output().viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(
            commands.iter().any(
                |c| matches!(c, egui::ViewportCommand::BeginResize(dir) if *dir == expected_dir)
            ),
            "expected BeginResize({expected_dir:?}) at {pos:?}, got {commands:?}"
        );
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }

    // Fullscreen windows must not emit resize commands on edge presses.
    harness.state_mut().fullscreen = true;
    let pos = egui::pos2(2.0, 350.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(pos));
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let commands = &harness.output().viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(
        !commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::BeginResize(_)))
    );
}

#[test]
fn always_on_top_toggle_and_mini_player_shows_only_play_button() {
    let mut harness = playback([860.0, 620.0]);
    harness.run_steps(3);
    assert!(!harness.state().always_on_top);
    harness.get_by_label("Always on top").click();
    harness.step();
    assert!(harness.state().always_on_top);
    let commands = &harness.output().viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(commands.iter().any(|c| matches!(
        c,
        egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop)
    )));
    harness.run_steps(2);
    harness.get_by_label("Disable always on top").click();
    harness.step();
    assert!(!harness.state().always_on_top);
    let commands = &harness.output().viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(commands.iter().any(|c| matches!(
        c,
        egui::ViewportCommand::WindowLevel(egui::WindowLevel::Normal)
    )));

    // Compact picture-in-picture size shows only the Play/Pause button.
    for size in [[320.0, 180.0], [240.0, 135.0]] {
        let mut harness = playback(size);
        harness.run_steps(3);
        let screen = harness.ctx.content_rect();
        let pause = harness.get_by_label("Pause").rect();
        assert!(screen.contains_rect(pause), "Pause is clipped at {size:?}");
        for hidden in [
            "Back 10s",
            "Forward 10s",
            "Fullscreen",
            "Audio track",
            "Subtitles",
            "Previous chapter",
            "Next chapter",
            "Volume",
            "Always on top",
            "Close window",
            "Minimize",
            "Maximize",
        ] {
            assert!(
                harness.query_by_label(hidden).is_none(),
                "{hidden} should be hidden at {size:?}"
            );
        }
        harness.state_mut().paused = true;
        harness.run_steps(2);
        let play = harness.get_by_label("Play").rect();
        assert!(screen.contains_rect(play), "Play is clipped at {size:?}");
        assert!(harness.query_by_label("Pause").is_none());

        // Growing back restores the titlebar and full playback controls.
        harness.set_size(Vec2::new(860.0, 620.0));
        harness.run_steps(3);
        assert!(harness.query_by_label("Back 10s").is_some());
        assert!(harness.query_by_label("Fullscreen").is_some());
        assert!(harness.query_by_label("Always on top").is_some());
    }
}

#[test]
fn sidebar_libraries_can_be_sorted_and_reordered() {
    let mut harness = library([1000.0, 700.0], "home");
    let sections = vec![
        Library {
            id: "3".into(),
            title: "TV Shows".into(),
            kind: Kind::Series,
        },
        Library {
            id: "1".into(),
            title: "Movies".into(),
            kind: Kind::Movie,
        },
        Library {
            id: "2".into(),
            title: "Anime".into(),
            kind: Kind::Series,
        },
    ];
    harness.state_mut().servers[0].sections = sections.clone();
    harness.state_mut().sections = sections;
    harness.run_steps(3);

    let order = |harness: &Harness<'_, App>| -> Vec<String> {
        harness
            .state()
            .sections
            .iter()
            .map(|s| s.title.clone())
            .collect()
    };

    harness.get_by_label("Sort libraries").click();
    harness.run_steps(2);
    harness.get_by_label("Name (A–Z)").hover();
    harness.run_steps(2);
    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );
    harness.get_by_label("Name (A–Z)").click();
    harness.run_steps(2);
    assert_eq!(order(&harness), ["Anime", "Movies", "TV Shows"]);
    let anime_y = harness.get_by_label("Anime").rect().center().y;
    let movies_y = harness.get_by_label("Movies").rect().center().y;
    let tv_y = harness.get_by_label("TV Shows").rect().center().y;
    assert!(anime_y < movies_y && movies_y < tv_y);

    harness.get_by_label("Sort libraries").click();
    harness.run_steps(2);
    harness.get_by_label("Name (Z–A)").click();
    harness.run_steps(2);
    assert_eq!(order(&harness), ["TV Shows", "Movies", "Anime"]);

    harness.get_by_label("Sort libraries").click();
    harness.run_steps(2);
    harness.get_by_label("Movies, then TV").click();
    harness.run_steps(2);
    assert_eq!(order(&harness), ["Movies", "Anime", "TV Shows"]);

    harness.get_by_label("Sort libraries").click();
    harness.run_steps(2);
    harness.get_by_label("Server order").click();
    harness.run_steps(2);
    assert_eq!(order(&harness), ["TV Shows", "Movies", "Anime"]);

    // Drag the last library ("Anime") to the top of the list.
    let from = harness.get_by_label("Anime").rect().center();
    let to = egui::pos2(from.x, harness.get_by_label("TV Shows").rect().top() + 2.0);
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(from));
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(to));
    harness.run_steps(2);
    assert_eq!(
        harness.output().platform_output.cursor_icon,
        egui::CursorIcon::Grabbing
    );
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(2);
    assert_eq!(order(&harness), ["Anime", "TV Shows", "Movies"]);
    assert_eq!(
        harness.state().servers[0].prefs.sort,
        prefs::LibrarySort::Custom
    );
}

#[test]
fn audiobook_home_matches_video_layout_and_pages_recently_added() {
    let size = [860.0, 620.0];
    let mut video = library(size, "home");
    video.run_steps(4);
    let mut harness = book_detail(size);
    let ctx = harness.ctx.clone();
    show_book_collection(harness.state_mut(), &ctx, true);
    harness.run_steps(4);

    let screen = harness.ctx.content_rect();
    let refresh = harness.get_by_label("Refresh").rect();
    assert_eq!(refresh, video.get_by_label("Refresh").rect());
    assert!(screen.contains_rect(refresh));
    let heading = harness.get_by_label("Continue Listening").rect();
    assert_eq!(
        heading.min,
        video.get_by_label("Continue Watching").rect().min
    );
    let forward = harness.get_by_label("Scroll Continue Listening forward");
    assert_eq!(
        forward.rect().size(),
        video
            .get_by_label("Scroll Continue Watching forward")
            .rect()
            .size()
    );
    assert!(screen.contains_rect(forward.rect()));
    assert!((heading.center().y - forward.rect().center().y).abs() < 4.0);
    forward.click();
    harness.run_steps(20);
    let listening_offset = harness.state().carousel_offsets["continue"];
    assert!(listening_offset > 1.0);
    assert_eq!(harness.state().page.as_ref().unwrap().offset, 0);

    let grid_cover = harness.get_by_label("Open The River Mouth").rect();
    let last_cover = harness.get_by_label("Open The Winter Marsh").rect();
    assert_eq!(grid_cover.size(), Vec2::splat(grid_cover.width()));
    assert_eq!(grid_cover.left(), last_cover.left());
    assert!(last_cover.top() > grid_cover.bottom());
    assert!(grid_cover.right() <= screen.right());
    assert!(
        harness
            .query_by_label("Scroll Recently Added forward")
            .is_none()
    );

    let listening_ids: Vec<_> = harness
        .state()
        .on_deck
        .iter()
        .map(|item| item.id.clone())
        .collect();
    harness
        .get_by_label("Next Recently Added page")
        .scroll_to_me();
    harness.run_steps(4);
    let next = harness.get_by_label("Next Recently Added page");
    assert!(screen.contains_rect(next.rect()));
    next.click();
    harness.run_steps(3);
    assert_eq!(harness.state().location.as_ref().unwrap().offset, 6);
    assert_eq!(
        harness.state().carousel_offsets["continue"],
        listening_offset
    );
    assert!(
        harness
            .state()
            .on_deck
            .iter()
            .map(|item| &item.id)
            .eq(&listening_ids)
    );

    // Render the last page using the same fixture, with a single remaining title.
    let app = harness.state_mut();
    app.begin();
    app.busy = false;
    app.page = Some(Page {
        items: vec![book_item()],
        offset: 6,
        total: Some(7),
    });
    harness.run_steps(4);
    assert!(
        harness
            .get_by_label("Next Recently Added page")
            .accesskit_node()
            .is_disabled()
    );
    harness
        .get_by_label("Previous Recently Added page")
        .scroll_to_me();
    harness.run_steps(4);
    harness.get_by_label("Previous Recently Added page").click();
    harness.run_steps(2);
    assert_eq!(harness.state().location.as_ref().unwrap().offset, 0);
    assert!(
        harness
            .state()
            .on_deck
            .iter()
            .map(|item| &item.id)
            .eq(&listening_ids)
    );
}

#[test]
fn home_pagination_only_changes_recently_added_and_carousels_scroll_with_arrows() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    // Horizontal arrows scroll Continue Watching and Recently Released.
    let mut harness = library([860.0, 620.0], "home");
    {
        let items = harness.state().page.as_ref().unwrap().items.clone();
        let app = harness.state_mut();
        app.home_loaded = true;
        app.on_deck = items[..5].to_vec();
        app.released = items[5..10].to_vec();
    }
    harness.run_steps(4);
    // Scroll arrows sit directly next to each carousel heading in left-to-right order.
    for heading in ["Continue Watching", "Recently Released"] {
        let title = harness.get_by_label(heading).rect();
        let back = harness
            .get_by_label(&format!("Scroll {heading} back"))
            .rect();
        let forward = harness
            .get_by_label(&format!("Scroll {heading} forward"))
            .rect();
        assert!(
            back.left() > title.right() && back.left() - title.right() <= 30.0,
            "{heading} back button should sit next to heading: title={title:?} back={back:?}"
        );
        assert!(
            forward.left() > back.right() && forward.left() - back.right() <= 30.0,
            "{heading} forward button should follow back: back={back:?} forward={forward:?}"
        );
        assert!((back.center().y - title.center().y).abs() < 4.0);
        assert!((forward.center().y - title.center().y).abs() < 4.0);
    }
    assert!(
        harness
            .get_by_label("Scroll Continue Watching back")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        !harness
            .get_by_label("Scroll Continue Watching forward")
            .accesskit_node()
            .is_disabled()
    );
    harness
        .get_by_label("Scroll Continue Watching forward")
        .click();
    harness.run_steps(20);
    assert!(harness.state().carousel_offsets["continue"] > 100.0);
    assert!(
        !harness
            .get_by_label("Scroll Continue Watching back")
            .accesskit_node()
            .is_disabled()
    );
    harness
        .get_by_label("Scroll Continue Watching back")
        .click();
    harness.run_steps(20);
    assert!(harness.state().carousel_offsets["continue"] < 1.0);

    harness
        .get_by_label("Scroll Recently Released forward")
        .click();
    harness.run_steps(20);
    assert!(harness.state().carousel_offsets["released"] > 100.0);

    let mut overview = library([860.0, 900.0], "home");
    let app = overview.state_mut();
    app.overview = true;
    app.servers[0].items = app.page.as_ref().unwrap().items.clone();
    overview.run_steps(4);
    let first = overview.get_by_label("Open The Quiet Earth").rect();
    let mut previous = first;
    for label in [
        "Open After Hours",
        "Open A Place Between the Stars and the Sea",
        "Open Wild Coast",
    ] {
        let row = overview.get_by_label(label).rect();
        assert!(row.top() > previous.bottom() && (row.left() - first.left()).abs() < 1.0);
        previous = row;
    }
    let next_page = overview.get_by_label("Open Small Hours").rect();
    assert_eq!(next_page.top(), first.top());
    assert!(next_page.left() > first.right());
    overview
        .get_by_label("Scroll Continue Watching forward")
        .click();
    overview.run_steps(20);
    assert!(overview.state().carousel_offsets["overview_continue"] > 100.0);

    // Next page on Home only requests and updates Recently Added.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(AtomicUsize::new(0));
    let seen = requests.clone();
    let server = thread::spawn(move || {
        for expected_start in ["36", "0"] {
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
            let text = String::from_utf8(request).unwrap().to_lowercase();
            assert!(
                text.starts_with("get /library/recentlyadded "),
                "unexpected request during Home pagination: {text}"
            );
            assert!(
                text.contains(&format!("x-plex-container-start: {expected_start}\r\n")),
                "expected offset {expected_start} in {text}"
            );
            seen.fetch_add(1, Ordering::Relaxed);
            let body = if expected_start == "36" {
                r#"{"MediaContainer":{"offset":36,"totalSize":38,"Metadata":[{"ratingKey":"37","title":"Page Two Film","type":"movie","year":2026},{"ratingKey":"38","title":"Second Page Encore","type":"movie","year":2026}]}}"#
            } else {
                r#"{"MediaContainer":{"offset":0,"totalSize":38,"Metadata":[{"ratingKey":"1","title":"Page One Film","type":"movie","year":2025}]}}"#
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });

    let mut harness = library([1440.0, 900.0], "home");
    {
        let items = harness.state().page.as_ref().unwrap().items.clone();
        let app = harness.state_mut();
        app.source = Some(fixture_source(&format!("http://{address}")));
        app.home_loaded = true;
        app.on_deck = items[..2].to_vec();
        app.released = items[2..4].to_vec();
        let mut page_one = Vec::new();
        for index in 0..PAGE_SIZE {
            page_one.push(Item {
                id: (index + 1).to_string(),
                title: format!("Page One Item {}", index + 1),
                kind: Kind::Movie,
                ..Default::default()
            });
        }
        app.page = Some(Page {
            items: page_one,
            offset: 0,
            total: Some(38),
        });
    }
    let deck_art = harness.state().on_deck[0].thumb.clone().unwrap();
    let released_art = harness.state().released[0].thumb.clone().unwrap();
    harness.run_steps(3);
    // Recently Added page arrows sit beside the heading/range, and button panel margins are equal.
    let screen = harness.ctx.content_rect();
    let recent_title = harness.get_by_label("Recently Added").rect();
    let recent_prev = harness.get_by_label("Previous Recently Added page").rect();
    let recent_next = harness.get_by_label("Next Recently Added page").rect();
    assert!(
        recent_prev.left() > recent_title.right()
            && recent_prev.left() - recent_title.right() < 160.0,
        "Recently Added arrows should sit next to heading: title={recent_title:?} prev={recent_prev:?}"
    );
    assert!(recent_next.left() > recent_prev.right());
    assert!((recent_prev.center().y - recent_title.center().y).abs() < 4.0);

    let back_btn = harness.get_by_label("Back").rect();
    let close_btn = harness.get_by_label("Close window").rect();
    assert!((back_btn.left() - 20.0).abs() < 1.5);
    assert!((back_btn.top() - 20.0).abs() < 1.5);
    // Windows caption buttons sit flush with the window's top-right corner.
    assert!((screen.right() - close_btn.right()).abs() < 1.5);
    assert!((close_btn.top() - screen.top()).abs() < 1.5);
    assert_eq!(close_btn.size(), Vec2::new(46.0, 32.0));

    let home_nav = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Home")
        .rect();
    let settings_nav = harness.get_by_label("Servers & settings").rect();
    // Top-level rows share one icon and text column from the sidebar edge.
    assert_eq!(home_nav.left(), settings_nav.left());
    assert_eq!(home_nav.right(), settings_nav.right());
    assert!((home_nav.top() - 72.0 - 28.0).abs() < 2.0);
    assert!((screen.bottom() - settings_nav.bottom() - 28.0).abs() < 2.0);

    assert!(harness.query_by_label("Previous page").is_none());
    assert!(harness.query_by_label("Next page").is_none());
    harness
        .get_by_label("Next Recently Added page")
        .scroll_to_me();
    harness.run_steps(4);
    harness.get_by_label("Next Recently Added page").click();
    for _ in 0..100 {
        harness.step();
        if !harness.state().busy {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!harness.state().busy);
    assert_eq!(harness.state().page.as_ref().unwrap().offset, 36);
    assert_eq!(harness.state().on_deck.len(), 2);
    assert_eq!(harness.state().released.len(), 2);
    assert!(harness.state().covers.contains(&deck_art));
    assert!(harness.state().covers.contains(&released_art));
    assert!(harness.query_by_label("Continue Watching").is_some());
    assert!(harness.query_by_label("Recently Released").is_some());
    assert!(harness.query_by_label("Page Two Film").is_some());

    harness
        .get_by_label("Previous Recently Added page")
        .scroll_to_me();
    harness.run_steps(4);
    let prev_rect = harness.get_by_label("Previous Recently Added page").rect();
    let prev_disabled = harness
        .get_by_label("Previous Recently Added page")
        .accesskit_node()
        .is_disabled();
    assert!(
        !prev_disabled && screen.contains_rect(prev_rect),
        "Previous Recently Added page not clickable: disabled={prev_disabled} rect={prev_rect:?}"
    );
    harness.get_by_label("Previous Recently Added page").click();
    for _ in 0..100 {
        harness.step();
        if !harness.state().busy {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!harness.state().busy);
    assert_eq!(harness.state().page.as_ref().unwrap().offset, 0);
    assert!(harness.query_by_label("Continue Watching").is_some());
    assert!(harness.query_by_label("Page One Film").is_some());
    assert_eq!(requests.load(Ordering::Relaxed), 2);
    server.join().unwrap();
}
