//! Runs the real eframe update loop without creating an OS window.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use perfuse::media::{PAGE_SIZE, Version};

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
    harness.get_by_label("Update").scroll_to_me();
    harness.run_steps(3);
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
fn sidebar_library_heading_aligns_with_navigation_icons() {
    let mut harness = library([1440.0, 900.0], "home");
    harness.run_steps(3);
    let row_left = harness.get_by_label("Servers & settings").rect().left();
    let heading_left = harness.get_by_label("LIBRARIES").rect().left();
    assert!((heading_left - row_left - SIDEBAR_ICON_INSET).abs() < 1.0);
    let status_left = harness.get_by_label("Plex connected").rect().left();
    assert!((status_left - row_left - SIDEBAR_TEXT_INSET).abs() < 1.0);
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
        server: server.into(),
        token: "synthetic".into(),
        client_id: "ui-test".into(),
    })
    .unwrap()
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
                let art = harness.get_by_label("Open The Quiet Earth").rect();
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
    // Custom painted navigation still supports keyboard activation and exposes labels.
    harness.get_by_label("Servers & settings").focus();
    harness.run();
    harness.key_press(egui::Key::Space);
    harness.run();
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
        ("movies-desktop", "movies", [1440.0, 900.0], 1.0),
        ("movies-minimum", "movies", [860.0, 620.0], 1.0),
        ("movies-high-dpi", "movies", [860.0, 620.0], 1.5),
        ("shows-minimum", "shows", [860.0, 620.0], 1.0),
        ("episodes-minimum", "episodes", [860.0, 620.0], 1.0),
        ("details-desktop", "details", [1440.0, 900.0], 1.0),
        ("details-minimum", "details", [860.0, 620.0], 1.0),
        ("library-empty", "empty", [860.0, 620.0], 1.0),
        ("library-loading", "loading", [860.0, 620.0], 1.0),
        ("library-search", "search", [860.0, 620.0], 1.0),
        ("library-missing-art", "missing-art", [860.0, 620.0], 1.0),
    ] {
        let mut harness = library(size, screen);
        harness.set_pixels_per_point(scale);
        // A zoom change rescales the first frame's input rectangle. Restore
        // logical window bounds afterwards so 150% DPI exports 1290x930 pixels.
        if scale != 1.0 {
            harness.step();
            harness.set_size(size.into());
        }
        harness.remove_cursor();
        harness.run_steps(4);
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
    harness.get_by_label("Manual server connection").click();
    harness.run();
    harness.get_by_label("Plex server").focus();
    harness.run();
    assert!(harness.get_by_label("Plex server").is_focused());
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
        .get_by_label("Plex server")
        .type_text("http://localhost:32499");
    harness.run();
    assert_eq!(harness.state().server, "http://localhost:32499");
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert!(harness.get_by_label("Plex token").is_focused());
    harness
        .get_by_label("Plex token")
        .type_text("synthetic-ui-token");
    harness.run();
    assert_eq!(harness.state().token, "synthetic-ui-token");
    assert_eq!(
        harness.get_by_label("Plex token").accesskit_node().role(),
        egui::accesskit::Role::PasswordInput
    );
    harness.get_by_label("Connect to server").scroll_to_me();
    harness.run();
    let screen = harness.ctx.content_rect();
    assert!(screen.contains_rect(harness.get_by_label("Connect to server").rect()));
    // Intentionally don't click Connect: UI tests never access a real server.
}

#[test]
fn cancelled_and_failed_login_can_be_retried() {
    let mut harness = login([860.0, 620.0]);
    let (pending, events) = auth::Login::test_channel();
    harness.state_mut().login = Some(pending);
    events
        .send(auth::Event::Browser(
            "https://app.source.tv/auth#?code=synthetic".into(),
        ))
        .unwrap();
    harness.run_steps(2);
    assert!(harness.get_by_label("Cancel sign-in").rect().is_positive());
    // OpenUrl remains egui output in this harness; no OS browser is launched.
    harness.get_by_label("Cancel sign-in").click();
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
            access_token: Some("synthetic-resource-token".into()),
            provides: "server".into(),
            connections: vec![auth::Connection {
                uri: "https://plex.example.test:32400".into(),
                local: false,
            }],
        },
        auth::Server {
            name: "Family Library".into(),
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
    harness.state_mut().available_servers = discovered_servers();
    harness.run();
    for server in discovered_servers() {
        let label = format!("Connect to {}", server.name);
        let rect = harness.get_by_label(&label).rect();
        assert!(
            harness.ctx.content_rect().contains_rect(rect),
            "Server choice is clipped: {rect:?}"
        );
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
        ("login-manual", [860.0, 620.0], 1.0),
        ("login-waiting", [860.0, 620.0], 1.0),
        ("login-expired", [860.0, 620.0], 1.0),
        ("login-servers", [860.0, 620.0], 1.0),
        ("login-download-permission", [860.0, 620.0], 1.0),
        ("login-settings", [860.0, 620.0], 1.0),
        ("login-libraries", [1100.0, 900.0], 1.0),
    ] {
        let mut harness = login(size);
        harness.set_pixels_per_point(scale);
        if scale != 1.0 {
            harness.step();
            harness.set_size(size.into());
        }
        match name {
            "login-manual" => {
                harness.get_by_label("Manual server connection").click();
            }
            "login-waiting" => {
                let (pending, _events) = auth::Login::test_channel();
                harness.state_mut().login = Some(pending);
                harness.state_mut().login_url =
                    Some("https://app.source.tv/auth#?code=synthetic".into());
            }
            "login-expired" => {
                harness.state_mut().error =
                    Some("Plex sign-in expired. Sign in again to continue.".into());
            }
            "login-servers" => {
                harness.state_mut().available_servers = discovered_servers();
            }
            "login-download-permission" => {
                harness.state_mut().runtime_prompt = Some(RuntimePrompt::FirstRun {
                    download: true,
                    error: None,
                });
            }
            "login-settings" | "login-libraries" => {
                let app = harness.state_mut();
                app.source = Some(fixture_source("http://127.0.0.1:32499"));
                app.sections.push(Library {
                    id: "1".into(),
                    title: "Movies".into(),
                    kind: Kind::Movie,
                });
                app.connection_open = true;
                app.runtime_installed.libraries = runtime_versions_fixture();
            }
            _ => {}
        }
        harness.remove_cursor();
        harness.run_steps(4);
        if name == "login-manual" {
            harness.get_by_label("Connect to server").scroll_to_me();
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
    harness.state_mut().server_sections = sections.clone();
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
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run_steps(2);
    assert_eq!(order(&harness), ["Anime", "TV Shows", "Movies"]);
    assert_eq!(
        harness.state().library_order.sort,
        prefs::LibrarySort::Custom
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
    assert!(harness.state().carousel_offsets["continue"].0 > 100.0);
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
    assert!(harness.state().carousel_offsets["continue"].0 < 1.0);

    harness
        .get_by_label("Scroll Recently Released forward")
        .click();
    harness.run_steps(20);
    assert!(harness.state().carousel_offsets["released"].0 > 100.0);

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
    assert!((screen.right() - close_btn.right() - 20.0).abs() < 1.5);

    let home_nav = harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Home")
        .rect();
    let settings_nav = harness.get_by_label("Servers & settings").rect();
    assert!((home_nav.left() - 28.0).abs() < 1.5);
    assert!((home_nav.top() - 72.0 - 28.0).abs() < 2.0);
    assert!((screen.bottom() - settings_nav.bottom() - 28.0).abs() < 2.0);

    let prev_page_btn = harness.get_by_label("Previous page").rect();
    let sidebar_right = home_nav.right() + 28.0;
    let prev_left_pad = prev_page_btn.left() - sidebar_right;
    let prev_bottom_pad = screen.bottom() - prev_page_btn.bottom();
    assert!(
        (prev_left_pad - 28.0).abs() < 2.5 && (prev_bottom_pad - 28.0).abs() < 2.5,
        "pagination margins should be equal (28pt): left={prev_left_pad} bottom={prev_bottom_pad}"
    );

    harness.get_by_label("Next page").click();
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
        !prev_disabled && prev_rect.bottom() <= prev_page_btn.top() - 28.0,
        "Previous Recently Added page not clickable: disabled={prev_disabled} rect={prev_rect:?} pagination_top={}",
        prev_page_btn.top() - 28.0
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
