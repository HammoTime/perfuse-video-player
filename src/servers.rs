use super::*;

#[derive(Default, PartialEq)]
pub(super) enum StatusPopover {
    #[default]
    Closed,
    Waiting(f64),
    Open,
    Pinned,
    Dismissed,
}

impl StatusPopover {
    fn update(&mut self, trigger: &egui::Response, popup: &egui::Popup<'_>) -> bool {
        let ctx = &trigger.ctx;
        let now = ctx.input(|i| i.time);
        if trigger.clicked() {
            *self = if *self == Self::Pinned {
                Self::Dismissed
            } else {
                Self::Pinned
            };
        } else if *self != Self::Pinned && trigger.gained_focus() {
            *self = Self::Open;
        } else {
            match *self {
                Self::Closed if trigger.hovered() => {
                    *self = Self::Waiting(now + 0.25);
                    ctx.request_repaint_after(Duration::from_millis(250));
                }
                Self::Waiting(_) | Self::Dismissed if !trigger.hovered() => *self = Self::Closed,
                Self::Waiting(until) if now >= until => *self = Self::Open,
                Self::Waiting(until) => {
                    ctx.request_repaint_after(Duration::from_secs_f64(until - now))
                }
                Self::Open if !trigger.hovered() && !trigger.has_focus() => {
                    // Include the gap so the pointer can travel from the button into the panel.
                    let over_panel = popup
                        .get_popup_rect()
                        .zip(ctx.pointer_hover_pos())
                        .is_some_and(|(panel, p)| {
                            panel.contains(p)
                                || (trigger.rect.x_range().contains(p.x)
                                    && panel.union(trigger.rect).contains(p))
                        });
                    let focus = ctx
                        .memory(|m| m.focused())
                        .and_then(|id| ctx.read_response(id));
                    if !over_panel && !focus.is_some_and(|r| r.layer_id.id == popup.get_id()) {
                        *self = Self::Closed;
                    }
                }
                _ => {}
            }
        }
        matches!(self, Self::Open | Self::Pinned)
    }
}

enum Check {
    Libraries(Vec<Library>, Duration),
    Continue(Vec<Item>, Artwork),
    Offline,
}

pub(super) struct Server {
    pub session: session::Session,
    pub source: Source,
    pub sections: Vec<Library>,
    pub prefs: prefs::LibraryOrder,
    pub online: bool,
    pub dragging_section: Option<usize>,
    remembered: bool,
    latency: Option<Duration>,
    pending: Option<Receiver<Check>>,
    checking: bool,
    pub(super) items: Vec<Item>,
    pub(super) artwork: Artwork,
    expanded: Option<bool>,
    rename: Option<String>,
}

impl Server {
    pub fn new(
        session: session::Session,
        source: Source,
        sections: Vec<Library>,
        remembered: bool,
    ) -> Self {
        Self {
            prefs: prefs::load_library_order(&session.server),
            session,
            source,
            sections,
            online: true,
            dragging_section: None,
            remembered,
            latency: None,
            pending: None,
            checking: false,
            items: vec![],
            artwork: Artwork::default(),
            expanded: None,
            rename: None,
        }
    }

    pub fn name(&self) -> String {
        if self.prefs.name.is_empty() {
            reqwest::Url::parse(&self.session.server)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| "Plex server".into())
        } else {
            self.prefs.name.clone()
        }
    }

    pub fn persist(&self) {
        prefs::save_library_order(&self.session.server, &self.prefs);
    }
    pub fn libraries(&self) -> Vec<Library> {
        prefs::apply_library_order(&self.sections, &self.prefs)
            .into_iter()
            .filter(|lib| !self.prefs.hidden.contains(&lib.id))
            .collect()
    }

    pub fn check(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        self.checking = true;
        let (source, ctx) = (self.source.clone(), ctx.clone());
        let mut previous_art = self.artwork.clone();
        thread::spawn(move || {
            let start = Instant::now();
            if let Ok(libraries) = source.libraries() {
                if tx
                    .send(Check::Libraries(libraries, start.elapsed()))
                    .is_err()
                {
                    return;
                }
                ctx.request_repaint();
                if let Ok(page) = source.browse(&Browse::ContinueWatching, "", 0) {
                    let mut artwork = Artwork::default();
                    for item in &page.items {
                        let Some(path) = item
                            .art
                            .as_ref()
                            .filter(|_| item.kind == Kind::Movie)
                            .or(item.thumb.as_ref())
                        else {
                            continue;
                        };
                        if artwork.contains(path) {
                            continue;
                        }
                        if let Some(texture) = previous_art.textures.remove(path) {
                            artwork.textures.insert(path.clone(), texture);
                            continue;
                        }
                        if let Some(img) = source
                            .cached_artwork(path)
                            .or_else(|| source.artwork(path).ok())
                        {
                            let image = egui::ColorImage::from_rgba_unmultiplied(
                                [img.width() as usize, img.height() as usize],
                                img.as_raw(),
                            );
                            artwork.insert(
                                path.clone(),
                                ctx.load_texture(path, image, egui::TextureOptions::LINEAR),
                            );
                        }
                    }
                    let _ = tx.send(Check::Continue(page.items, artwork));
                }
            } else {
                let _ = tx.send(Check::Offline);
            }
            drop(tx);
            ctx.request_repaint();
        });
    }

    fn status_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add(egui::Separator::default().spacing(1.0));
        control_row(ui, 58.0, |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (tile, _) = ui.allocate_exact_size(Vec2::splat(32.0), egui::Sense::hover());
            ui.painter().rect_filled(tile, 8, CONTROL);
            Icon::ServerPlex.paint(ui, tile.center(), 18.0, TEXT);
            let (icon, color, status) = self.status();
            let offline = !self.online && !self.checking;
            let width = ui.available_width() - if offline { 68.0 } else { 28.0 };
            ui.allocate_ui_with_layout(
                Vec2::new(width, 36.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.add(
                        egui::Label::new(
                            RichText::new(self.name()).size(13.0).strong().color(TEXT),
                        )
                        .truncate(),
                    );
                    ui.label(RichText::new(status).size(12.0).color(if offline {
                        ERROR_TEXT
                    } else {
                        MUTED
                    }));
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if offline {
                    if ui
                        .add(egui::Button::new("Retry").corner_radius(14))
                        .clicked()
                    {
                        self.check(ctx);
                    }
                } else {
                    ui.add(icon.image(16.0, color));
                }
            });
        });
    }

    fn status(&self) -> (Icon, Color32, String) {
        if self.checking {
            (Icon::Refresh, MUTED, "Checking…".into())
        } else if !self.online {
            (
                Icon::StatusOffline,
                Color32::from_rgb(255, 138, 128),
                "Offline".into(),
            )
        } else {
            (
                Icon::StatusConnected,
                GREEN,
                self.latency.map_or_else(
                    || "Connected".into(),
                    |t| format!("Connected · {} ms", t.as_millis()),
                ),
            )
        }
    }
}

impl App {
    pub(super) fn activate(&mut self, i: usize) {
        let server = &self.servers[i];
        self.active = Some(i);
        self.source = Some(server.source.clone());
        self.sections = server.libraries();
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.busy = false;
        self.history.clear();
        self.detail = None;
        self.location = None;
        self.selected_library = None;
        self.search.clear();
        self.page = None;
        self.covers.clear();
        self.home_loaded = false;
        self.on_deck.clear();
        self.released.clear();
        self.carousel_offsets.clear();
        self.error = None;
        self.notice = None;
    }

    pub(super) fn poll_servers(&mut self, ctx: &egui::Context) {
        let due = Instant::now() >= self.next_health_check;
        if due {
            self.next_health_check = Instant::now() + Duration::from_secs(60);
        }
        if !self.servers.is_empty() {
            ctx.request_repaint_after(
                self.next_health_check
                    .saturating_duration_since(Instant::now()),
            );
        }
        for (i, s) in self.servers.iter_mut().enumerate() {
            if due {
                s.check(ctx);
            }
            while let Some(rx) = &s.pending {
                match rx.try_recv() {
                    Ok(Check::Libraries(libs, latency)) => {
                        s.online = true;
                        s.checking = false;
                        s.latency = Some(latency);
                        s.sections = libs;
                        if self.active == Some(i) {
                            self.sections = s.libraries();
                        }
                    }
                    Ok(Check::Continue(items, art)) => {
                        s.items = items;
                        s.artwork = art;
                    }
                    Ok(Check::Offline) => {
                        s.checking = false;
                        s.online = false;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        s.pending = None;
                        s.checking = false;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
    }

    pub(super) fn open_server(&mut self, i: usize, ctx: &egui::Context) {
        self.activate(i);
        self.home(ctx);
    }

    pub(super) fn group_heading(&mut self, ui: &mut egui::Ui, i: usize, ctx: &egui::Context) {
        let s = &mut self.servers[i];
        let name = s.name();
        let collapsed = s.prefs.collapsed;
        ui.spacing_mut().item_spacing.x = 4.0;
        let label = if collapsed {
            "Expand libraries"
        } else {
            "Collapse libraries"
        };
        ui.add(Icon::ServerPlex.image(16.0, MUTED))
            .on_hover_text("Plex");
        let open = ui
            .allocate_ui_with_layout(
                Vec2::new(92.0, 28.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(92.0);
                    ui.add(
                        egui::Button::new(
                            RichText::new(name.to_uppercase())
                                .size(12.0)
                                .strong()
                                .color(MUTED),
                        )
                        .frame(false)
                        .truncate(),
                    )
                },
            )
            .inner
            .on_hover_text(format!("Plex – {name}"))
            .clicked();
        let icon = if collapsed {
            Icon::PageNext
        } else {
            Icon::ChevronDown
        };
        if ui.add(IconButton::new(icon, label).size(24.0)).clicked() {
            s.prefs.collapsed = !collapsed;
            s.persist();
        }
        if !s.online {
            ui.add(Icon::StatusOffline.image(14.0, ERROR_TEXT));
        }
        if open {
            self.open_server(i, ctx);
        }
    }

    pub(super) fn status_button(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let count = self.servers.len();
        let online = self.servers.iter().filter(|s| s.online).count();
        let (icon, color, label, title, summary) = if self.servers.iter().any(|s| s.checking) {
            (
                Icon::Refresh,
                MUTED,
                "Checking servers…".into(),
                "Checking servers".into(),
                "Checking server connections.",
            )
        } else if online == 0 {
            (
                Icon::StatusOffline,
                ERROR_TEXT,
                "Cannot connect to servers".into(),
                "No servers reachable".into(),
                "Check your network connection and that your servers are running.",
            )
        } else if online < count {
            (
                Icon::StatusPartial,
                Color32::from_rgb(242, 201, 76),
                format!("{online} (of {count}) servers connected"),
                format!(
                    "{} server{} unreachable",
                    count - online,
                    if count - online == 1 { "" } else { "s" }
                ),
                "Content from unreachable servers is hidden until they reconnect.",
            )
        } else {
            (
                Icon::StatusConnected,
                GREEN,
                format!(
                    "{count} server{} connected",
                    if count == 1 { "" } else { "s" }
                ),
                "All servers connected".into(),
                "Everything is reachable.",
            )
        };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), egui::Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        let divider = Color32::from_rgb(42, 42, 46);
        let frame = egui::Frame::popup(ui.style())
            .fill(PANEL)
            .stroke(egui::Stroke::new(1.0_f32, divider))
            .corner_radius(14)
            .inner_margin(egui::Margin {
                top: 22,
                bottom: 5,
                ..egui::Margin::same(16)
            })
            .shadow(egui::epaint::Shadow {
                offset: [0, 16],
                blur: 40,
                spread: 0,
                color: Color32::from_black_alpha(140),
            });
        let popup = egui::Popup::from_response(&response)
            .align(egui::RectAlign::TOP_START)
            .gap(8.0)
            .width(340.0)
            .frame(frame);
        let open = self.status_popover.update(&response, &popup);
        if open || response.has_focus() || response.hovered() {
            ui.painter().rect_filled(rect, 8, PANEL);
        }
        if open || response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                8,
                egui::Stroke::new(1.0_f32, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        icon.paint(
            ui,
            rect.left_center() + Vec2::new(SIDEBAR_ICON_INSET + 10.0, 0.0),
            14.0,
            color,
        );
        ui.painter().text(
            rect.left_center() + Vec2::new(SIDEBAR_TEXT_INSET, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(12.0),
            TEXT,
        );
        if !open {
            return;
        }
        let panel = popup
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
            .show(|ui| {
                ui.set_width(340.0 - frame.total_margin().sum().x);
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(title).size(15.0).strong().color(TEXT));
                ui.add_space(4.0);
                ui.label(RichText::new(summary).size(12.0).color(MUTED));
                ui.add_space(12.0);
                ui.visuals_mut().widgets.noninteractive.bg_stroke =
                    egui::Stroke::new(1.0_f32, divider);
                let max_height = (rect.top() - ctx.content_rect().top() - 160.0).max(56.0);
                egui::ScrollArea::vertical()
                    .max_height(max_height)
                    .show(ui, |ui| {
                        for (i, server) in self.servers.iter_mut().enumerate() {
                            ui.push_id(i, |ui| server.status_row(ui, ctx));
                        }
                    });
                ui.add(egui::Separator::default().spacing(1.0));
                ui.add_space(5.0);
                control_row(ui, 32.0, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(10.0, 4.0);
                    let check = egui::Button::image_and_text(
                        Icon::Refresh.image(14.0, ACCENT),
                        RichText::new("Check again").size(13.0).color(ACCENT),
                    )
                    .frame_when_inactive(false);
                    if ui.add(check).clicked() {
                        for server in self
                            .servers
                            .iter_mut()
                            .filter(|s| online == count || !s.online)
                        {
                            server.check(ctx);
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Manage servers").size(13.0).color(ACCENT),
                                )
                                .frame_when_inactive(false),
                            )
                            .clicked()
                        {
                            self.settings_tab = SettingsTab::Servers;
                            self.connection_open = true;
                            ui.close();
                        }
                    });
                });
            });
        if panel.is_some_and(|p| {
            p.response.should_close() || (p.response.clicked_elsewhere() && !response.clicked())
        }) {
            self.status_popover = StatusPopover::Dismissed;
            ctx.request_repaint();
        }
    }

    pub(super) fn servers_home(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut open = None;
        let mut library = None;
        let mut item = None;
        egui::ScrollArea::vertical()
            .id_salt("all_servers")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Home").size(30.0).strong());
                    if ui
                        .add(IconButton::new(Icon::Refresh, "Refresh servers"))
                        .clicked()
                    {
                        for s in &mut self.servers {
                            s.check(ctx);
                        }
                    }
                });
                ui.colored_label(
                    MUTED,
                    "Pick up where you left off, across all your servers.",
                );
                ui.add_space(20.0);
                ui.heading("Servers");
                let gap = 16.0;
                let card_width = 280.0;
                let cols = ((ui.available_width() + gap) / (card_width + gap))
                    .floor()
                    .max(1.0) as usize;
                let card_frame = egui::Frame::new()
                    .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(42, 42, 46)))
                    .inner_margin(16)
                    .corner_radius(14);
                for (row_index, row) in self.servers.chunks(cols).enumerate() {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        for (column, s) in row.iter().enumerate() {
                            let i = row_index * cols + column;
                            let card = card_frame
                                .fill(if s.online {
                                    PANEL
                                } else {
                                    Color32::from_rgb(26, 23, 24)
                                })
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.set_width(
                                            card_width - card_frame.total_margin().sum().x,
                                        );
                                        control_row(ui, 48.0, |ui| {
                                            let (rect, _) = ui.allocate_exact_size(
                                                Vec2::splat(48.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(rect, 12, CONTROL);
                                            Icon::ServerPlex.paint(ui, rect.center(), 26.0, TEXT);
                                            ui.vertical(|ui| {
                                                ui.spacing_mut().item_spacing.y = 2.0;
                                                let text_height = ui.fonts_mut(|fonts| {
                                                    fonts.row_height(&egui::FontId::proportional(
                                                        18.0,
                                                    )) + fonts.row_height(
                                                        &egui::FontId::proportional(12.0),
                                                    )
                                                }) + 2.0;
                                                ui.add_space(((48.0 - text_height) / 2.0).max(0.0));
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(s.name()).size(18.0).strong(),
                                                    )
                                                    .truncate(),
                                                );
                                                ui.label(
                                                    RichText::new("Plex").color(MUTED).size(12.0),
                                                );
                                            });
                                        });
                                        let (icon, color, status) = s.status();
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            ui.add(icon.image(14.0, color));
                                            ui.label(RichText::new(status).size(13.0));
                                        });
                                        ui.horizontal_wrapped(|ui| {
                                            ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                                            ui.spacing_mut().button_padding = Vec2::new(8.0, 3.0);
                                            for lib in s.libraries() {
                                                let glyph = if lib.kind == Kind::Series {
                                                    Icon::TvShows
                                                } else {
                                                    Icon::Movies
                                                };
                                                let chip = egui::Button::image_and_text(
                                                    glyph.image(12.0, MUTED),
                                                    RichText::new(&lib.title).size(12.0),
                                                )
                                                .min_size(Vec2::new(0.0, 26.0))
                                                .corner_radius(13);
                                                if ui.add(chip).clicked() {
                                                    library = Some((i, lib));
                                                }
                                            }
                                        });
                                    });
                                });
                            // Register behind the chips, which have their own library destinations.
                            let header = egui::Rect::from_min_size(
                                card.response.rect.min,
                                Vec2::new(card.response.rect.width(), 76.0),
                            );
                            let button = ui.interact(
                                header,
                                ui.id().with(("server_card", i)),
                                egui::Sense::click(),
                            );
                            button.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s.name())
                            });
                            if button.has_focus() {
                                ui.painter().rect_stroke(
                                    header,
                                    12,
                                    egui::Stroke::new(1.0_f32, ACCENT),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if button.on_hover_text(&s.session.server).clicked() {
                                open = Some(i);
                            }
                        }
                    });
                }
                let mut watching: Vec<_> = self
                    .servers
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.online)
                    .flat_map(|(i, s)| {
                        s.items
                            .iter()
                            .filter(|item| {
                                !item
                                    .library_id
                                    .as_ref()
                                    .is_some_and(|id| s.prefs.hidden.contains(id))
                            })
                            .map(move |entry| (i, s, entry))
                    })
                    .collect();
                watching.sort_by_key(|(_, _, entry)| std::cmp::Reverse(entry.last_viewed_at));
                if !watching.is_empty() {
                    ui.add_space(20.0);
                    ui.heading("Continue Watching");
                    egui::ScrollArea::horizontal().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (i, s, entry) in watching {
                                ui.push_id((i, &entry.id), |ui| {
                                    ui.vertical(|ui| {
                                        if continue_card(ui, entry, &s.artwork) {
                                            item = Some((i, entry.clone()));
                                        }
                                        ui.horizontal(|ui| {
                                            ui.add(Icon::ServerPlex.image(13.0, MUTED));
                                            ui.small(s.name());
                                        });
                                    });
                                });
                            }
                        });
                    });
                }
                for s in self
                    .servers
                    .iter()
                    .filter(|s| !s.online && !s.items.is_empty())
                {
                    ui.colored_label(
                        MUTED,
                        format!(
                            "{} items from {} hidden while offline",
                            s.items.len(),
                            s.name()
                        ),
                    );
                }
                if self.servers.iter().all(|s| !s.online) {
                    ui.heading("Can't reach any of your servers");
                    if ui.button("Try again").clicked() {
                        for s in &mut self.servers {
                            s.check(ctx);
                        }
                    }
                }
            });
        if let Some(i) = open {
            self.open_server(i, ctx);
        }
        if let Some((i, lib)) = library {
            self.activate(i);
            self.overview = false;
            self.connection_open = false;
            self.library(&lib, ctx);
        }
        if let Some((i, item)) = item {
            self.activate(i);
            self.overview = false;
            self.open_item(&item, ctx);
        }
    }

    pub(super) fn offline_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(i) = self.active {
            ui.heading(format!("Can't reach {}", self.servers[i].name()));
            if ui.button("Try again").clicked() {
                self.servers[i].check(ctx);
            }
            if ui.button("Server settings").clicked() {
                self.settings_tab = SettingsTab::Servers;
                self.connection_open = true;
            }
        }
    }

    pub(super) fn server_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let count = self.servers.len();
        let online = self
            .servers
            .iter()
            .filter(|s| s.online && !s.checking)
            .count();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new("Media servers").size(23.0).strong());
                ui.label(
                    RichText::new(format!("{online} of {count} connected"))
                        .size(14.0)
                        .color(MUTED),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let add = egui::Button::image_and_text(
                    Icon::Add.image(16.0, Color32::BLACK),
                    RichText::new("Add server")
                        .size(14.0)
                        .strong()
                        .color(Color32::BLACK),
                )
                .fill(ACCENT)
                .corner_radius(18)
                .min_size(Vec2::new(108.0, 36.0));
                if ui.add_enabled(!self.busy, add).clicked() {
                    self.adding = true;
                    self.server.clear();
                    self.token.clear();
                    self.remember = cfg!(windows);
                    self.available_servers.clear();
                    self.error = None;
                }
            });
        });
        ui.add_space(2.0);
        let mut reorder = None;
        for (i, s) in self.servers.iter_mut().enumerate() {
            let row = egui::Frame::new()
                .fill(PANEL)
                .corner_radius(14)
                .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(45, 46, 52)))
                .inner_margin(16)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.push_id(("settings_server", &s.session.server.clone()), |ui| {
                        let mut expanded = s.expanded.unwrap_or(i == 0);
                        control_row(ui, 44.0, |ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            if count > 1 {
                                let handle = ui.add(
                                    egui::Button::image(Icon::DragHandle.image(18.0, MUTED))
                                        .sense(egui::Sense::click_and_drag())
                                        .frame(false),
                                );
                                handle.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        true,
                                        "Reorder server",
                                    )
                                });
                                handle.dnd_set_drag_payload(i);
                                if handle.has_focus() {
                                    if i > 0 && ui.input(|k| k.key_pressed(egui::Key::ArrowUp)) {
                                        reorder = Some((i, i - 1));
                                    }
                                    if i + 1 < count
                                        && ui.input(|k| k.key_pressed(egui::Key::ArrowDown))
                                    {
                                        reorder = Some((i, i + 1));
                                    }
                                }
                                handle.on_hover_text(
                                    "Drag to reorder, or use the up and down arrows",
                                );
                            }
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(44.0), egui::Sense::hover());
                            ui.painter()
                                .rect_filled(rect, 12, Color32::from_rgb(56, 46, 32));
                            Icon::ServerPlex.paint(
                                ui,
                                rect.center(),
                                26.0,
                                Color32::from_rgb(244, 189, 95),
                            );
                            let name_width = (ui.available_width() - 190.0).max(65.0);
                            ui.allocate_ui_with_layout(
                                Vec2::new(name_width, 44.0),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(name_width);
                                    ui.spacing_mut().item_spacing.y = 3.0;
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(s.name()).size(18.0).strong(),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(s.name());
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "Plex · {} {}",
                                                s.sections.len(),
                                                if s.sections.len() == 1 {
                                                    "library"
                                                } else {
                                                    "libraries"
                                                }
                                            ))
                                            .size(13.0)
                                            .color(MUTED),
                                        )
                                        .truncate(),
                                    );
                                },
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let expand_label = if expanded {
                                        "Hide server details"
                                    } else {
                                        "Show server details"
                                    };
                                    if ui
                                        .add(
                                            IconButton::new(
                                                if expanded {
                                                    Icon::ChevronDown
                                                } else {
                                                    Icon::PageNext
                                                },
                                                expand_label,
                                            )
                                            .size(28.0),
                                        )
                                        .clicked()
                                    {
                                        expanded = !expanded;
                                        s.expanded = Some(expanded);
                                    }
                                    if ui
                                        .add(
                                            IconButton::new(Icon::Rename, "Rename server")
                                                .size(28.0),
                                        )
                                        .clicked()
                                    {
                                        s.rename = Some(s.name());
                                        expanded = true;
                                        s.expanded = Some(true);
                                    }
                                    let (icon, color, status) = s.status();
                                    badge(
                                        ui,
                                        icon,
                                        status.split(" · ").next().unwrap_or(&status),
                                        color,
                                    );
                                },
                            );
                        });
                        if expanded {
                            ui.add_space(2.0);
                            ui.separator();
                            if let Some(name) = &mut s.rename {
                                let save = ui
                                    .horizontal(|ui| {
                                        let label = ui.label("Display name");
                                        let edit = ui
                                            .add(
                                                egui::TextEdit::singleline(name)
                                                    .desired_width(170.0),
                                            )
                                            .labelled_by(label.id);
                                        ui.add(IconButton::new(Icon::Check, "Save name")).clicked()
                                            || (edit.lost_focus()
                                                && ui.input(|k| k.key_pressed(egui::Key::Enter)))
                                    })
                                    .inner;
                                if save {
                                    let name = s.rename.take().unwrap_or_default();
                                    if !name.trim().is_empty() {
                                        s.prefs.name = name.trim().into();
                                        s.persist();
                                    }
                                } else if ui.input(|k| k.key_pressed(egui::Key::Escape)) {
                                    s.rename = None;
                                }
                            }
                            setting_line(ui, "Address", |ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(&s.session.server).size(14.0))
                                        .truncate(),
                                )
                                .on_hover_text(&s.session.server);
                            });
                            setting_line(ui, "Visible libraries", |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing = Vec2::splat(6.0);
                                    ui.spacing_mut().button_padding = Vec2::new(10.0, 5.0);
                                    for lib in &s.sections {
                                        let shown = !s.prefs.hidden.contains(&lib.id);
                                        let glyph = if lib.kind == Kind::Series {
                                            Icon::TvShows
                                        } else {
                                            Icon::Movies
                                        };
                                        let btn = egui::Button::image_and_text(
                                            glyph.image(15.0, if shown { ACCENT } else { MUTED }),
                                            RichText::new(&lib.title).size(14.0),
                                        )
                                        .selected(shown)
                                        .corner_radius(10);
                                        if ui
                                        .add(btn)
                                        .on_hover_text(
                                            "Show or hide this library in the sidebar and on Home",
                                        )
                                        .clicked()
                                    {
                                        if shown {
                                            s.prefs.hidden.push(lib.id.clone());
                                        } else {
                                            s.prefs.hidden.retain(|id| *id != lib.id);
                                        }
                                        s.persist();
                                        if self.active == Some(i) {
                                            self.sections = s.libraries();
                                            self.overview = true;
                                        }
                                    }
                                    }
                                });
                            });
                            setting_line(ui, "Sign-in", |ui| {
                                let mut remembered = s.remembered;
                                if ui
                                    .add_enabled(
                                        cfg!(windows),
                                        egui::Checkbox::new(
                                            &mut remembered,
                                            "Remember this server",
                                        ),
                                    )
                                    .changed()
                                {
                                    let result = if remembered {
                                        session::save(&s.session)
                                    } else {
                                        session::clear(&s.session.server)
                                    };
                                    match result {
                                        Ok(()) => s.remembered = remembered,
                                        Err(error) => self.error = Some(error),
                                    }
                                }
                            });
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.spacing_mut().button_padding = Vec2::new(9.0, 5.0);
                                let check = egui::Button::image_and_text(
                                    Icon::Refresh.image(15.0, MUTED),
                                    "Check connection",
                                )
                                .frame(false);
                                if ui.add_enabled(!s.checking, check).clicked() {
                                    s.check(ctx);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let remove = egui::Button::image_and_text(
                                            Icon::SignOut.image(15.0, ERROR_TEXT),
                                            RichText::new("Remove server")
                                                .size(14.0)
                                                .color(ERROR_TEXT),
                                        )
                                        .frame(false);
                                        if ui.add(remove).clicked() {
                                            self.removing = Some(i);
                                        }
                                    },
                                );
                            });
                        }
                    });
                });
            if let Some(from) = row.response.dnd_release_payload::<usize>()
                && *from < count
                && *from != i
            {
                reorder = Some((*from, i));
            }
        }
        if let Some((from, to)) = reorder {
            self.servers[from.min(to)..=from.max(to)].rotate_left(if from < to {
                1
            } else {
                from - to
            });
            if self.active == Some(from) {
                self.active = Some(to);
            } else if let Some(active) = self.active
                && (from.min(to)..=from.max(to)).contains(&active)
            {
                self.active = Some(if from < to { active - 1 } else { active + 1 });
            }
            for (i, s) in self.servers.iter_mut().enumerate() {
                s.prefs.position = i;
                s.persist();
            }
        }
    }

    pub(super) fn server_dialogs(&mut self, ctx: &egui::Context) {
        if self.adding {
            let modal = egui::Modal::new("add_server".into()).show(ctx, |ui| {
                ui.set_width(440.0);
                ui.heading("Add a Plex server");
                egui::ScrollArea::vertical()
                    .max_height(ctx.content_rect().height() * 0.68)
                    .show(ui, |ui| self.connection_fields(ui, ctx));
                if let Some(error) = &self.error {
                    ui.colored_label(ERROR_TEXT, error);
                }
                ui.add(Pill::secondary(Icon::Close, "Close")).clicked()
            });
            if modal.inner || modal.should_close() {
                self.generation.fetch_add(1, Ordering::Relaxed);
                self.busy = false;
                self.login = None;
                self.login_url = None;
                self.adding = false;
                self.token.clear();
                self.available_servers.clear();
                self.error = None;
            }
        }
        if let Some(i) = self.removing {
            let modal = egui::Modal::new("remove_server".into()).show(ctx, |ui| {
                ui.heading(format!("Remove {}?", self.servers[i].name()));
                ui.label(
                    "Perfuse will forget this server's sign-in. Your progress stays on the server.",
                );
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        self.removing = None;
                    }
                    ui.button("Remove").clicked()
                })
                .inner
            });
            if modal.inner {
                if self.servers[i].remembered
                    && let Err(error) = session::clear(&self.servers[i].session.server)
                {
                    self.error = Some(error);
                } else {
                    self.servers.remove(i);
                    for (n, s) in self.servers.iter_mut().enumerate() {
                        if s.prefs.position != n {
                            s.prefs.position = n;
                            s.persist();
                        }
                    }
                    if self.servers.is_empty() {
                        self.active = None;
                        self.source = None;
                        self.server.clear();
                        self.token.clear();
                        self.generation.fetch_add(1, Ordering::Relaxed);
                        self.busy = false;
                        self.connection_open = false;
                        self.covers.clear();
                        self.detail = None;
                        self.notice = None;
                        self.error = None;
                    } else {
                        self.activate(0);
                        self.overview = true;
                    }
                }
                self.removing = None;
            } else if modal.should_close() {
                self.removing = None;
            }
        }
    }
}

fn setting_line(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            Vec2::new(110.0, 22.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(110.0);
                ui.label(RichText::new(label).size(14.0).color(MUTED));
            },
        );
        ui.vertical(content);
    });
}
