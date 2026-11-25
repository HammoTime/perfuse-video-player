//! Runs the real eframe update loop without creating an OS window.
use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};

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

// Review the real library without network requests, credentials or desktop windows.
fn library(size: [f32; 2], screen: &str) -> Harness<'static, App> {
    let mut harness = app_harness(size, 0.25);
    let ctx = harness.ctx.clone();
    let app = harness.state_mut();
    app.plex = Some(Plex::new("http://127.0.0.1:32499", "synthetic", "ui-test").unwrap());
    app.sections = vec![
        Section {
            key: "1".into(),
            title: "Movies".into(),
            kind: "movie".into(),
        },
        Section {
            key: "2".into(),
            title: "TV Shows".into(),
            kind: "show".into(),
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
            rating_key: (index + 1).to_string(), title: (*title).into(), kind: "movie".into(),
            year: Some(2020 + index as u32 % 6), thumb: Some(path), art: Some(art),
            summary: "A journey along a forgotten coastline brings two strangers together, and changes the way they see the world.".into(),
            duration: Some(7_200_000), view_offset: (index < 3).then_some(1_200_000 + index as u64 * 900_000),
            view_count: u32::from(index == 5),
            media: vec![plex::Media { video_resolution: Some("4k".into()), video_codec: Some("hevc".into()), ..Default::default() }],
            ..Default::default()
        }
    }).collect();
    let home = screen == "home";
    app.selected_library = (!home).then(|| "1".into());
    app.location = Some(Location {
        title: if home { "Home" } else { "Movies" }.into(),
        path: if home {
            "/library/recentlyAdded"
        } else {
            "/library/sections/1/all"
        }
        .into(),
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
                item.kind = "show".into();
                item.media.clear();
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
                item.kind = "episode".into();
                item.grandparent_title = Some("The Quiet Earth".into());
                item.parent_index = Some(1);
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
            "https://app.plex.tv/auth#?code=synthetic".into(),
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
                    Some("https://app.plex.tv/auth#?code=synthetic".into());
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
                app.plex =
                    Some(Plex::new("http://127.0.0.1:32499", "synthetic", "ui-test").unwrap());
                app.sections.push(Section {
                    key: "1".into(),
                    title: "Movies".into(),
                    kind: "movie".into(),
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
