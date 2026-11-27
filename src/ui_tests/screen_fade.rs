use super::*;

// The old image has a unique texture ID. Inspecting the real paint output also
// catches an opaque placeholder drawn instead of the previous screen.
fn poster_alpha(ui: &Harness<'_, App>, texture: egui::TextureId) -> Option<u8> {
    ui.output().shapes.iter().find_map(|entry| {
        if entry.shape.texture_id() != texture {
            return None;
        }
        match &entry.shape {
            egui::Shape::Rect(rect) => Some(rect.fill.a()),
            egui::Shape::Mesh(mesh) => mesh.vertices.iter().map(|vertex| vertex.color.a()).max(),
            _ => None,
        }
    })
}

fn caption_shapes(ui: &Harness<'_, App>) -> Vec<egui::epaint::ClippedShape> {
    let pin = ui.get_by_label("Always on top").rect();
    let caption = egui::Rect::from_min_max(
        pin.min,
        egui::pos2(ui.ctx.content_rect().right(), pin.bottom()),
    );
    ui.output()
        .shapes
        .iter()
        .filter(|entry| {
            let bounds = entry.shape.visual_bounding_rect();
            bounds.is_positive() && caption.contains_rect(bounds)
        })
        .cloned()
        .collect()
}

#[test]
fn opening_a_home_card_holds_its_paint_then_reveals_the_detail() {
    // Keep the real request outstanding. Inject its generation-tagged reply at
    // a chosen frame so this test does not depend on worker scheduling.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut ui = app_harness([860.0, 620.0], 1.0 / 60.0);
    let texture = ui.ctx.load_texture(
        "screen fade fixture",
        egui::ColorImage::filled([8, 8], Color32::WHITE),
        Default::default(),
    );
    let app = ui.state_mut();
    app.source = Some(fixture_source(&format!(
        "http://{}",
        listener.local_addr().unwrap()
    )));
    attach_server(app);
    let movie = Item {
        id: "42".into(),
        title: "Shelf title".into(),
        kind: Kind::Movie,
        thumb: Some("/poster".into()),
        ..Default::default()
    };
    app.servers[0].items = vec![movie.clone()];
    app.servers[0]
        .artwork
        .insert("/poster".into(), texture.clone());
    app.overview = true;
    ui.run_steps(12);
    assert_eq!(poster_alpha(&ui, texture.id()), Some(255));
    let chrome = caption_shapes(&ui);
    assert!(!chrome.is_empty());

    ui.get_by_label("Open Shelf title").click();
    ui.run_steps(2);
    assert!(ui.state().busy && ui.state().detail.is_none());
    assert!(ui.query_by_label("Open Shelf title").is_none());
    assert_eq!(poster_alpha(&ui, texture.id()), Some(255));
    assert_eq!(caption_shapes(&ui), chrome);

    let detail = Item {
        title: "Detail title".into(),
        thumb: None,
        ..movie
    };
    let app = ui.state();
    let generation = app.generation.load(Ordering::Relaxed);
    app.tx.send((generation, Reply::Details(detail))).unwrap();
    ui.run_steps(3);
    ui.get_by_label("Detail title");
    assert_eq!(caption_shapes(&ui), chrome);
    if prefs::client_animations_enabled() {
        assert!(poster_alpha(&ui, texture.id()).is_some_and(|alpha| alpha > 0 && alpha < 255));
    } else {
        assert_eq!(poster_alpha(&ui, texture.id()), None);
    }
    ui.run_steps(10);
    assert_eq!(poster_alpha(&ui, texture.id()), None);
    ui.state_mut().detail.as_mut().unwrap().title = "Refreshed title".into();
    ui.step();
    ui.get_by_label("Refreshed title");
    assert_eq!(poster_alpha(&ui, texture.id()), None);
    assert_eq!(caption_shapes(&ui), chrome);
}
