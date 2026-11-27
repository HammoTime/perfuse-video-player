use super::*;

#[derive(Default, PartialEq)]
pub(super) enum HoverPopover {
    #[default]
    Closed,
    Waiting(f64),
    Open,
    Pinned,
    Dismissed,
}

impl HoverPopover {
    fn dismiss_if_closed(
        &mut self,
        panel: Option<egui::InnerResponse<()>>,
        trigger: &egui::Response,
    ) {
        if panel.is_some_and(|p| {
            p.response.should_close() || (p.response.clicked_elsewhere() && !trigger.clicked())
        }) {
            *self = Self::Dismissed;
            trigger.ctx.request_repaint();
        }
    }
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

/// How often servers refresh their libraries and Continue rows in the background.
/// Refresh in a server's Home forces one sooner.
pub(super) const REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);

enum Check {
    Libraries(Vec<Library>, Duration),
    Continue(Vec<Item>, Artwork),
    Offline(String),
    Expired,
}

pub(super) struct Server {
    pub session: session::Session,
    pub source: Source,
    pub sections: Vec<Library>,
    pub prefs: prefs::LibraryOrder,
    pub online: bool,
    /// The server rejected this sign-in. Retrying cannot help; signing in again can.
    pub expired: bool,
    pub dragging_section: Option<usize>,
    pub(super) remembered: bool,
    latency: Option<Duration>,
    pending: Option<Receiver<Check>>,
    checking: bool,
    offline_reason: Option<String>,
    pub(super) items: Vec<Item>,
    pub(super) artwork: Artwork,
    /// The last Home seen for this server, shown while a fresh one loads.
    pub(super) home: Option<metadata::Home>,
    pub(super) store: metadata::Store,
    expanded: Option<bool>,
    rename: Option<String>,
    library_popover: HoverPopover,
    focus_remove: bool,
}

impl Server {
    pub fn new(
        session: session::Session,
        source: Source,
        sections: Vec<Library>,
        remembered: bool,
    ) -> Self {
        Self {
            prefs: prefs::load_library_order(&session),
            store: metadata::Store::new(session.kind.plugin().id, &session.server),
            session,
            source,
            sections,
            online: true,
            expired: false,
            dragging_section: None,
            remembered,
            latency: None,
            pending: None,
            checking: false,
            offline_reason: None,
            items: vec![],
            artwork: Artwork::default(),
            home: None,
            expanded: None,
            rename: None,
            library_popover: HoverPopover::default(),
            focus_remove: false,
        }
    }

    /// A remembered server shown from its last snapshot until its first refresh.
    pub fn restored(restored: Restored, ctx: &egui::Context) -> Self {
        let mut server = Self::new(restored.session, restored.source, vec![], true);
        if let Some(overview) = restored.overview {
            server.sections = overview.libraries;
            server.items = overview.continue_items;
        }
        for (path, image) in restored.art {
            let texture = ctx.load_texture(&path, image, egui::TextureOptions::LINEAR);
            server.artwork.insert(path, texture);
        }
        server.home = restored.home;
        server
    }

    /// Stops keeping snapshots for a server whose sign-in is no longer remembered.
    pub fn forget_snapshots(&mut self) {
        self.home = None;
        let store = self.store.clone();
        tasks::spawn(move || store.forget());
    }

    pub fn name(&self) -> String {
        if self.prefs.name.is_empty() {
            reqwest::Url::parse(&self.session.server)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{} server", self.session.kind.plugin().name))
        } else {
            self.prefs.name.clone()
        }
    }

    pub fn persist(&self) {
        prefs::save_library_order(&self.session, &self.prefs);
    }
    pub fn icon(&self) -> Icon {
        connection::source_icon(self.session.kind)
    }
    pub fn libraries(&self) -> Vec<Library> {
        prefs::apply_library_order(&self.sections, &self.prefs)
            .into_iter()
            .filter(|lib| !self.prefs.hidden.contains(&lib.id))
            .collect()
    }

    pub(super) fn item_visible(&self, item: &Item) -> bool {
        !item
            .library_id
            .as_ref()
            .is_some_and(|id| self.prefs.hidden.contains(id))
    }

    fn card_libraries(&mut self, ui: &mut egui::Ui) -> Option<Library> {
        const HEIGHT: f32 = 28.0;
        const GAP: f32 = 6.0;
        let libraries = self.libraries();
        let available = ui.available_width();
        let mut selected = None;
        control_row(ui, HEIGHT, |ui| {
            ui.set_min_height(HEIGHT);
            ui.spacing_mut().item_spacing.x = GAP;
            ui.spacing_mut().button_padding = Vec2::new(8.0, 3.0);
            let widths: Vec<_> = libraries
                .iter()
                .map(|lib| chip_width(ui, &lib.title, 12.0, 12.0))
                .collect();
            let fits = widths.iter().sum::<f32>() + GAP * widths.len().saturating_sub(1) as f32
                <= available;
            let overflow = !fits && libraries.len() > 2;
            let count = if overflow { 2 } else { libraries.len() };
            let mut room = available - GAP * count.saturating_sub(1) as f32;
            if overflow {
                room -= HEIGHT + GAP;
            }
            for (index, lib) in libraries.iter().take(count).enumerate() {
                // Give a short second chip its natural width when the first title is long.
                let limit = if index == 0 && count == 2 {
                    room - widths[1].min(room / 2.0)
                } else {
                    room
                };
                let width = widths[index].min(limit);
                let chip = egui::Button::image_and_text(
                    library_icon(lib).image(12.0, MUTED),
                    RichText::new(lib.title.replace(['\r', '\n'], " ")).size(12.0),
                )
                .corner_radius(EDGE)
                .truncate();
                if ui
                    .add_sized([width, HEIGHT], chip)
                    .on_hover_text(&lib.title)
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .clicked()
                {
                    selected = Some(lib.clone());
                }
                room -= width;
            }
            if overflow {
                let label = format!("Show all libraries on {}", self.name());
                let more = ui.add(IconButton::new(Icon::ChevronDown, &label).size(HEIGHT));
                let frame = popover_frame();
                let popup = egui::Popup::from_response(&more)
                    .align(egui::RectAlign::BOTTOM_END)
                    .gap(8.0)
                    .width(280.0)
                    .frame(frame);
                let open = self.library_popover.update(&more, &popup);
                ui.ctx()
                    .accesskit_node_builder(more.id, |node| node.set_expanded(open));
                if open {
                    let panel = popup
                        .close_behavior(egui::PopupCloseBehavior::IgnoreClicks)
                        .show(|ui| {
                            ui.set_width(280.0 - frame.total_margin().sum().x);
                            ui.spacing_mut().item_spacing.y = 6.0;
                            ui.label(
                                RichText::new(format!("Libraries on {}", self.name()))
                                    .size(15.0)
                                    .strong(),
                            );
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .max_height((ui.ctx().content_rect().height() * 0.45).min(320.0))
                                .show(ui, |ui| {
                                    for lib in &libraries {
                                        let button = egui::Button::image_and_text(
                                            library_icon(lib).image(16.0, MUTED),
                                            &lib.title,
                                        )
                                        .frame_when_inactive(false)
                                        .wrap();
                                        if ui
                                            .add(button)
                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .clicked()
                                        {
                                            selected = Some(lib.clone());
                                            ui.close();
                                        }
                                    }
                                });
                        });
                    self.library_popover.dismiss_if_closed(panel, &more);
                }
            } else {
                self.library_popover = HoverPopover::Closed;
            }
        });
        selected
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
        let store = self.remembered.then(|| self.store.clone());
        tasks::spawn(move || {
            let start = Instant::now();
            let libraries = match source.libraries() {
                Ok(libraries) => libraries,
                Err(error) => {
                    let _ = tx.send(if error.is_unauthorized() {
                        Check::Expired
                    } else {
                        Check::Offline(error.into())
                    });
                    ctx.request_repaint();
                    return;
                }
            };
            if tx
                .send(Check::Libraries(libraries.clone(), start.elapsed()))
                .is_err()
            {
                return;
            }
            ctx.request_repaint();
            if let Ok(page) = source.browse(&Browse::ContinueWatching, "", 0) {
                if let Some(store) = &store {
                    store.save_overview(&metadata::Overview {
                        libraries,
                        continue_items: page.items.clone(),
                    });
                }
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
            drop(tx);
            ctx.request_repaint();
        });
    }

    /// Shows the server's status. Returns true when the user asks to sign in again.
    fn status_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        let mut sign_in = false;
        ui.add(egui::Separator::default().spacing(1.0));
        control_row(ui, 58.0, |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (tile, _) = ui.allocate_exact_size(Vec2::splat(32.0), egui::Sense::hover());
            ui.painter().rect_filled(tile, 0, CONTROL);
            self.icon().paint(ui, tile.center(), 18.0, TEXT);
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
                if offline && self.expired {
                    sign_in = ui
                        .add(egui::Button::new("Sign in").corner_radius(EDGE))
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked();
                } else if offline {
                    let retry = ui
                        .add(egui::Button::new("Retry").corner_radius(EDGE))
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    let retry = match &self.offline_reason {
                        Some(reason) => retry.on_hover_text(reason),
                        None => retry,
                    };
                    if retry.clicked() {
                        self.check(ctx);
                    }
                } else {
                    ui.add(icon.image(16.0, color));
                }
            });
        });
        sign_in
    }

    fn status(&self) -> (Icon, Color32, String) {
        if self.checking {
            (Icon::Refresh, MUTED, "Checking…".into())
        } else if self.expired {
            (
                Icon::StatusOffline,
                Color32::from_rgb(255, 138, 128),
                "Sign-in expired".into(),
            )
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
        self.book.full = false;
        let server = &self.servers[i];
        self.active = Some(i);
        self.source = Some(server.source.clone());
        self.sections = server.libraries();
        self.generation.fetch_add(1, Ordering::Relaxed);
        self.busy = false;
        self.refreshing = false;
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
        let due = Instant::now() >= self.next_refresh;
        if due {
            self.next_refresh = Instant::now() + REFRESH_INTERVAL;
        }
        if !self.servers.is_empty() {
            ctx.request_repaint_after(self.next_refresh.saturating_duration_since(Instant::now()));
        }
        for (i, s) in self.servers.iter_mut().enumerate() {
            if due {
                s.check(ctx);
            }
            while let Some(rx) = &s.pending {
                match rx.try_recv() {
                    Ok(Check::Libraries(libs, latency)) => {
                        s.online = true;
                        s.expired = false;
                        s.offline_reason = None;
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
                    Ok(Check::Offline(reason)) => {
                        s.checking = false;
                        s.online = false;
                        s.offline_reason = Some(reason);
                    }
                    Ok(Check::Expired) => {
                        s.checking = false;
                        s.online = false;
                        s.expired = true;
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

    pub(super) fn group_heading(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        ctx: &egui::Context,
        show_sort: bool,
    ) {
        let s = &mut self.servers[i];
        let name = s.name();
        let collapsed = s.prefs.collapsed;
        ui.spacing_mut().item_spacing.x = 4.0;
        let label = if collapsed {
            "Expand libraries"
        } else {
            "Collapse libraries"
        };
        let kind = s.session.kind.plugin().name;
        // Put the server mark on the sidebar icon column and its name on the text column.
        let (mark, _) = ui.allocate_exact_size(
            Vec2::new(SIDEBAR_TEXT_INSET - ui.spacing().item_spacing.x, 28.0),
            egui::Sense::hover(),
        );
        s.icon().paint(
            ui,
            mark.left_center() + Vec2::new(SIDEBAR_ICON_INSET + 10.0, 0.0),
            16.0,
            MUTED,
        );
        ui.interact(mark, ui.id().with("server_mark"), egui::Sense::hover())
            .on_hover_text(kind);
        // Keep both controls together at the shared right edge of the sidebar rows.
        let trailing = 24.0
            + if show_sort {
                28.0
            } else if !s.online {
                18.0
            } else {
                0.0
            };
        let name_width = (ui.available_width() - trailing - 4.0).max(0.0);
        let open = ui
            .allocate_ui_with_layout(
                Vec2::new(name_width, 28.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_width(name_width);
                    ui.spacing_mut().button_padding = Vec2::ZERO;
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
            .on_hover_text(format!("{kind} – {name}"))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
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
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        let divider = Color32::from_rgb(42, 42, 46);
        let frame = popover_frame().inner_margin(egui::Margin {
            top: 22,
            bottom: 5,
            ..egui::Margin::same(16)
        });
        let popup = egui::Popup::from_response(&response)
            .align(egui::RectAlign::TOP_START)
            .gap(8.0)
            .width(340.0)
            .frame(frame);
        let open = self.status_popover.update(&response, &popup);
        if open || response.has_focus() || response.hovered() {
            ui.painter().rect_filled(rect, EDGE, PANEL);
        }
        if open || response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                EDGE,
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
        let mut sign_in = None;
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
                            if ui.push_id(i, |ui| server.status_row(ui, ctx)).inner {
                                sign_in = Some(i);
                            }
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
                    if ui
                        .add(check)
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
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
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .clicked()
                        {
                            self.settings_tab = SettingsTab::Servers;
                            self.book.full = false;
                            self.connection_open = true;
                            ui.close();
                        }
                    });
                });
            });
        self.status_popover.dismiss_if_closed(panel, &response);
        if let Some(i) = sign_in {
            self.status_popover = HoverPopover::Closed;
            self.sign_in_again(i);
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
                    .corner_radius(0);
                for (row_index, row) in self.servers.chunks_mut(cols).enumerate() {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::splat(gap);
                        for (column, s) in row.iter_mut().enumerate() {
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
                                            ui.painter().rect_filled(rect, 0, CONTROL);
                                            s.icon().paint(ui, rect.center(), 26.0, TEXT);
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
                                                    RichText::new(s.session.kind.plugin().name)
                                                        .color(MUTED)
                                                        .size(12.0),
                                                );
                                            });
                                        });
                                        let (icon, color, status) = s.status();
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                            ui.add(icon.image(14.0, color));
                                            ui.label(RichText::new(status).size(13.0));
                                        });
                                        if let Some(lib) = s.card_libraries(ui) {
                                            library = Some((i, lib));
                                        }
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
                                    0,
                                    egui::Stroke::new(1.0_f32, ACCENT),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if button
                                .on_hover_text(&s.session.server)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                open = Some(i);
                            }
                        }
                    });
                }
                let (watching, listening): (Vec<_>, Vec<_>) = self
                    .servers
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.online)
                    .flat_map(|(i, s)| {
                        s.items
                            .iter()
                            .filter(|item| s.item_visible(item))
                            .map(move |entry| (i, s, entry))
                    })
                    .partition(|(_, _, entry)| entry.kind != Kind::Audiobook);
                for (id, heading, width, mut entries) in [
                    (
                        "overview_continue",
                        "Continue Watching",
                        HOME_CARD_WIDTH,
                        watching,
                    ),
                    (
                        "overview_listening",
                        "Continue Listening",
                        BOOK_CARD_WIDTH,
                        listening,
                    ),
                ] {
                    if entries.is_empty() {
                        continue;
                    }
                    entries.sort_by_key(|(_, _, entry)| std::cmp::Reverse(entry.last_viewed_at));
                    ui.add_space(20.0);
                    let shelf = if entries[0].2.kind == Kind::Audiobook {
                        Shelf::Books {
                            count: Some(entries.len()),
                        }
                    } else {
                        Shelf::Video(4)
                    };
                    home_carousel(
                        ui,
                        id,
                        heading,
                        self.carousel_offsets.entry(id).or_default(),
                        &entries,
                        shelf,
                        |ui, &(i, server, entry)| {
                            ui.push_id((i, &entry.id), |ui| {
                                ui.vertical(|ui| {
                                    ui.set_width(width);
                                    let clicked = if entry.kind == Kind::Audiobook {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        let playing = self
                                            .book
                                            .is_current(&server.source, &entry.id)
                                            .then_some(BookCardPlayback {
                                                paused: self.playback.paused,
                                                elapsed_ms: (self.playback.playing_seconds * 1000.0)
                                                    as u64,
                                                finished: self.book.completed,
                                            });
                                        book_card(ui, entry, &server.artwork, width, playing)
                                    } else {
                                        continue_card(ui, entry, &server.artwork)
                                    };
                                    if clicked {
                                        item = Some((i, entry.clone()));
                                    }
                                    ui.horizontal(|ui| {
                                        if entry.kind == Kind::Audiobook {
                                            ui.spacing_mut().item_spacing.x = 6.0;
                                        }
                                        ui.add(server.icon().image(13.0, MUTED));
                                        ui.add(
                                            egui::Label::new(if entry.kind == Kind::Audiobook {
                                                RichText::new(server.name()).size(12.0).color(MUTED)
                                            } else {
                                                RichText::new(server.name()).small()
                                            })
                                            .truncate(),
                                        );
                                    });
                                });
                            });
                        },
                    );
                }
                for s in self.servers.iter().filter(|s| !s.online) {
                    let count = s.items.iter().filter(|item| s.item_visible(item)).count();
                    if count > 0 {
                        ui.colored_label(
                            MUTED,
                            format!("{count} items from {} hidden while offline", s.name()),
                        );
                    }
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
            if self.servers[i].expired {
                ui.heading(format!("Sign in to {} again", self.servers[i].name()));
                ui.label(
                    RichText::new(format!(
                        "{} no longer accepts the saved sign-in. It may have expired or been revoked.",
                        self.servers[i].name()
                    ))
                    .color(MUTED),
                );
                if ui
                    .add(Pill::primary(Icon::SignInBrowser, "Sign in again"))
                    .clicked()
                {
                    self.sign_in_again(i);
                }
                return;
            }
            ui.heading(format!("Can't reach {}", self.servers[i].name()));
            if let Some(reason) = &self.servers[i].offline_reason {
                ui.label(RichText::new(reason).color(MUTED));
            }
            if ui
                .button("Try again")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.servers[i].check(ctx);
            }
            if ui
                .button("Server settings")
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                self.settings_tab = SettingsTab::Servers;
                self.connection_open = true;
            }
        }
    }

    pub(super) fn server_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let background = self.background_tx.clone();
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
                .corner_radius(0)
                .min_size(Vec2::new(108.0, 36.0));
                let button = ui.add_enabled(!self.busy, add);
                self.sign_in.form.focus("Add server", &button);
                if button.clicked() {
                    self.adding = true;
                    self.choose_source(None);
                }
            });
        });
        ui.add_space(2.0);
        let mut reorder = None;
        for (i, s) in self.servers.iter_mut().enumerate() {
            let row = egui::Frame::new()
                .fill(PANEL)
                .corner_radius(0)
                .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(45, 46, 52)))
                .inner_margin(16)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let row_id = (
                        "settings_server",
                        s.session.kind.plugin().id,
                        s.session.server.clone(),
                    );
                    ui.push_id(row_id, |ui| {
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
                            let (fill, ink) = match s.session.kind {
                                ServerKind::Plex => (
                                    Color32::from_rgb(56, 46, 32),
                                    Color32::from_rgb(244, 189, 95),
                                ),
                                ServerKind::Jellyfin => (
                                    Color32::from_rgb(38, 40, 64),
                                    Color32::from_rgb(161, 170, 255),
                                ),
                                ServerKind::Audiobookshelf => (CONTROL, TEXT),
                            };
                            ui.painter().rect_filled(rect, 0, fill);
                            s.icon().paint(ui, rect.center(), 26.0, ink);
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
                                                "{} · {} {}",
                                                s.session.kind.plugin().name,
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
                                    let width = s
                                        .sections
                                        .iter()
                                        .map(|lib| chip_width(ui, &lib.title, 14.0, 15.0))
                                        .fold(0.0_f32, f32::max)
                                        .min(ui.available_width());
                                    for lib in &s.sections {
                                        let shown = !s.prefs.hidden.contains(&lib.id);
                                        let glyph = library_icon(lib);
                                        let btn = egui::Button::image_and_text(
                                            glyph.image(15.0, if shown { ACCENT } else { MUTED }),
                                            RichText::new(&lib.title).size(14.0),
                                        )
                                        .selected(shown)
                                        .truncate()
                                        .corner_radius(0);
                                        if ui
                                        .add_sized([width, 28.0], btn)
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
                                    // Credential Manager can wait on another instance's lock.
                                    s.remembered = remembered;
                                    if !remembered {
                                        s.forget_snapshots();
                                    }
                                    let (session, tx) = (s.session.clone(), background.clone());
                                    tasks::spawn(move || {
                                        let result = if remembered {
                                            session::save(&session)
                                        } else {
                                            session::clear(&session)
                                        };
                                        let _ = tx.send(Background::Remembered {
                                            kind: session.kind,
                                            origin: session::origin(&session.server),
                                            wanted: remembered,
                                            result,
                                        });
                                    });
                                }
                            });
                            ui.separator();
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                let (check, remove) = Pill::pair(
                                    ui,
                                    Pill::secondary(Icon::Refresh, "Check connection"),
                                    Pill::danger(Icon::SignOut, "Remove server"),
                                );
                                if ui.add_enabled(!s.checking, check).clicked() {
                                    s.check(ctx);
                                }
                                let remove = ui.add(remove);
                                if std::mem::take(&mut s.focus_remove) {
                                    remove.request_focus();
                                }
                                if remove.clicked() {
                                    self.removing = Some((i, remove.id));
                                }
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
            self.add_server_dialog(ctx);
        }
        if let Some((i, trigger)) = self.removing {
            let frame = popover_frame();
            let modal = egui::Modal::new("remove_server".into())
                .frame(frame)
                .backdrop_color(Color32::from_black_alpha(153))
                .show(ctx, |ui| {
                    let width = 460.0_f32.min(ctx.content_rect().width() - 48.0);
                    ui.set_width(width - frame.total_margin().sum().x);
                    ui.spacing_mut().item_spacing.y = 12.0;
                    let server = &self.servers[i];
                    ui.label(RichText::new(format!("Remove {}?", server.name())).heading().size(22.0));
                    let kind = server.session.kind.plugin().name;
                    let message = if server.source.capabilities().sign_out {
                        format!(
                            "Perfuse will sign out of this {kind} server and forget its saved sign-in. Your progress stays on the server."
                        )
                    } else {
                        format!(
                            "Perfuse will forget its saved sign-in for this {kind} server. The sign-in stays valid until you revoke it in your {kind} account. Your progress stays on the server."
                        )
                    };
                    ui.label(RichText::new(message).color(MUTED));
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (cancel, remove) = Pill::pair(
                            ui,
                            Pill::secondary(Icon::Cancel, "Cancel").min_width(116.0),
                            Pill::danger(Icon::SignOut, "Remove"),
                        );
                        ui.add_space((ui.available_width() - cancel.width(ui) - remove.width(ui) - 8.0).max(0.0));
                        let cancel = ui.add(cancel);
                        if ui.memory(|m| m.focused().is_none() || m.has_focus(trigger)) {
                            cancel.request_focus();
                        }
                        if cancel.clicked() {
                            ui.close();
                        }
                        ui.add(remove).clicked()
                    }).inner
                });
            if modal.inner {
                let removed = self.servers.remove(i);
                // A book from this server saves its final position before the sign-out.
                let book = if self.book.uses_source(&removed.source) {
                    self.book_finished();
                    self.playback.player.take().map(player::Player::retire)
                } else {
                    None
                };
                self.forget_removed(removed, book);
                for (n, s) in self.servers.iter_mut().enumerate() {
                    if s.prefs.position != n {
                        s.prefs.position = n;
                        s.persist();
                    }
                }
                if self.servers.is_empty() {
                    self.active = None;
                    self.source = None;
                    self.sign_in.server.clear();
                    self.sign_in.token.clear();
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
                self.removing = None;
            } else if modal.should_close() {
                self.removing = None;
                self.servers[i].focus_remove = true;
            }
        }
    }
}

/// A remembered server, opened with its last snapshot on a worker, so startup never waits
/// on Credential Manager, networking setup or image decoding.
pub(super) struct Restored {
    session: session::Session,
    source: Source,
    overview: Option<metadata::Overview>,
    home: Option<metadata::Home>,
    art: Vec<(String, egui::ColorImage)>,
}

pub(super) struct Startup {
    servers: Vec<Restored>,
    unreadable: Vec<session::Unreadable>,
    error: Option<String>,
}

/// Results from work that outlives the view that started it, such as saving or
/// removing a sign-in. Unlike navigation replies these are never discarded.
pub(super) enum Background {
    Notice(String),
    /// The outcome of saving (`wanted`) or forgetting a server's sign-in.
    Remembered {
        kind: ServerKind,
        origin: Option<String>,
        wanted: bool,
        result: Result<(), String>,
    },
}

/// The artwork a server's Continue row shows for `item`.
fn continue_art(item: &Item) -> Option<&String> {
    item.art
        .as_ref()
        .filter(|_| item.kind == Kind::Movie)
        .or(item.thumb.as_ref())
}

fn restore_saved(plex_server: String) -> Startup {
    let saved = session::load();
    let single_plex = saved
        .sessions
        .iter()
        .filter(|s| s.kind == ServerKind::Plex)
        .count()
        == 1;
    let mut servers = Vec::new();
    let mut unopened = Vec::new();
    for mut saved in saved.sessions {
        if saved.kind == ServerKind::Plex {
            let matches = session::origin(&saved.server) == session::origin(&plex_server);
            if std::env::var_os("PLEX_URL").is_some() && !matches {
                continue;
            }
            if let Ok(token) = std::env::var("PLEX_TOKEN") {
                if !matches && !single_plex {
                    continue;
                }
                saved.token = token.into();
            }
        }
        let Ok(source) = session::open(&saved) else {
            unopened.push(saved.server.clone());
            continue;
        };
        let store = metadata::Store::new(saved.kind.plugin().id, &saved.server);
        let overview = store.overview().map(|saved| saved.value);
        let art = overview
            .iter()
            .flat_map(|overview| &overview.continue_items)
            .filter_map(|item| {
                let path = continue_art(item)?;
                let image = source.cached_artwork(path)?;
                let size = [image.width() as usize, image.height() as usize];
                Some((
                    path.clone(),
                    egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                ))
            })
            .collect();
        servers.push(Restored {
            home: store.home().map(|saved| saved.value),
            session: saved,
            source,
            overview,
            art,
        });
    }
    let error = saved.error.or_else(|| {
        (!unopened.is_empty()).then(|| {
            format!(
                "Perfuse could not open the saved sign-in for {}. Sign in again in Settings.",
                unopened.join(", ")
            )
        })
    });
    Startup {
        servers,
        unreadable: saved.unreadable,
        error,
    }
}

impl App {
    /// Loads saved servers in the background. The window shows them, from their last
    /// snapshot, as soon as they are ready, then refreshes each one.
    pub(super) fn restore(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        self.restoring = Some(rx);
        let (server, ctx) = (self.sign_in.server.clone(), ctx.clone());
        tasks::spawn(move || {
            let _ = tx.send(restore_saved(server));
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_restore(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.restoring else {
            return;
        };
        let startup = match rx.try_recv() {
            Ok(startup) => startup,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Startup {
                servers: vec![],
                unreadable: vec![],
                error: Some("Perfuse could not load your saved servers.".into()),
            },
        };
        self.restoring = None;
        self.unreadable = startup.unreadable;
        if let Some(error) = startup.error {
            self.error = Some(error);
        }
        let first = self.servers.len();
        for restored in startup.servers {
            self.servers.push(Server::restored(restored, ctx));
        }
        self.servers[first..].sort_by_key(|s| s.prefs.position);
        // The snapshot is on screen. Refreshing each server is the later step.
        for server in &mut self.servers[first..] {
            server.check(ctx);
        }
        self.next_refresh = Instant::now() + REFRESH_INTERVAL;
        if first == 0 && !self.servers.is_empty() && self.active.is_none() {
            self.activate(0);
            self.overview = true;
        }
    }

    pub(super) fn poll_background(&mut self) {
        while let Ok(result) = self.background_rx.try_recv() {
            match result {
                Background::Notice(message) => self.notice = Some(message),
                Background::Remembered {
                    kind,
                    origin,
                    wanted,
                    result,
                } => {
                    if let Some(server) = self.servers.iter_mut().find(|s| {
                        s.session.kind == kind && session::origin(&s.session.server) == origin
                    }) {
                        server.remembered = if result.is_ok() { wanted } else { !wanted };
                    }
                    if let Err(error) = result {
                        if self.sign_in.form.added_any() {
                            self.sign_in.form.credential_error = Some(error);
                        } else {
                            self.notice = Some(error);
                        }
                    }
                }
            }
        }
    }

    /// Saves a new sign-in to Credential Manager without blocking the window.
    pub(super) fn remember_in_background(&self, session: session::Session) {
        let tx = self.background_tx.clone();
        tasks::spawn(move || {
            let result = session::save(&session);
            let _ = tx.send(Background::Remembered {
                kind: session.kind,
                origin: session::origin(&session.server),
                wanted: true,
                result,
            });
        });
    }

    /// Signs out on the server where it can, then forgets the saved sign-in, preferences
    /// and snapshots. Runs in the background and reports only what could not be done.
    fn forget_removed(&self, removed: Server, book: Option<player::Retired>) {
        let tx = self.background_tx.clone();
        let name = removed.name();
        let Server {
            session,
            source,
            remembered,
            store,
            ..
        } = removed;
        tasks::spawn(move || {
            if let Some(book) = book {
                if book.wait_until(Instant::now() + Duration::from_secs(8)) {
                    drop(book);
                } else {
                    book.detach();
                }
            }
            let mut problems = Vec::new();
            if source.capabilities().sign_out && source.sign_out().is_err() {
                problems.push(format!(
                    "Perfuse could not sign out of {name} on the server."
                ));
            }
            match remembered.then(|| session::clear(&session)) {
                Some(Err(_)) => problems.push(format!(
                    "Windows could not delete the saved sign-in for {name}, so it will reappear next time. Remove it again in Settings."
                )),
                _ => {
                    prefs::forget_server(&session);
                    store.forget();
                }
            }
            if !problems.is_empty() {
                let _ = tx.send(Background::Notice(problems.join(" ")));
            }
        });
    }

    /// Opens the add-server form for an expired server, keeping its place and name.
    pub(super) fn sign_in_again(&mut self, i: usize) {
        let server = &self.servers[i];
        let (kind, address) = (server.session.kind, server.session.server.clone());
        self.connection_open = false;
        self.adding = true;
        self.choose_source(Some(kind));
        self.sign_in.server = address;
        self.sign_in.form.replace(kind, &self.sign_in.server);
    }

    /// Forgets saved sign-ins that no longer open, after the user confirms.
    pub(super) fn forget_unreadable(&mut self) {
        let entries = std::mem::take(&mut self.unreadable);
        let tx = self.background_tx.clone();
        tasks::spawn(move || {
            if entries
                .iter()
                .any(|entry| session::forget_unreadable(entry).is_err())
            {
                let _ = tx.send(Background::Notice(
                    "Windows could not delete every unreadable sign-in. Try again next time."
                        .into(),
                ));
            }
        });
    }
}

pub(super) fn library_icon(lib: &Library) -> Icon {
    match lib.kind {
        LibraryKind::Shows => Icon::TvShows,
        LibraryKind::Audiobooks => Icon::Audiobooks,
        LibraryKind::Movies => Icon::Movies,
    }
}

fn chip_width(ui: &egui::Ui, title: &str, font_size: f32, icon: f32) -> f32 {
    let label = ui.painter().layout_no_wrap(
        title.replace(['\r', '\n'], " "),
        egui::FontId::proportional(font_size),
        TEXT,
    );
    (label.size().x + 2.0 * ui.spacing().button_padding.x + icon + ui.spacing().icon_spacing).ceil()
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
