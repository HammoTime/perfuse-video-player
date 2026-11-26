#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icons;
mod mpv;
mod player;
mod prefs;
mod runtime;
mod servers;
mod session;
use servers::Server;
mod updates;
mod video;

#[cfg(test)]
mod ui_tests;

use eframe::egui::{self, Color32, RichText, Vec2};
use icons::Icon;
use perfuse::{
    media::{Browse, Item, Kind, Library, Page, Source, Stream},
    plugins::plex::{self, auth},
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ACCENT: Color32 = Color32::from_rgb(61, 155, 255);
const MUTED: Color32 = Color32::from_rgb(152, 152, 159);
const PANEL: Color32 = Color32::from_rgb(28, 28, 30);
const GREEN: Color32 = Color32::from_rgb(126, 226, 160);
const ERROR_TEXT: Color32 = Color32::from_rgb(255, 160, 150);
const TEXT: Color32 = Color32::from_rgb(235, 235, 245);
const SIDEBAR_TEXT_INSET: f32 = 40.0;
const SIDEBAR_ICON_INSET: f32 = 10.0;
const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
// Button state fills from assets/buttons.
const CONTROL: Color32 = Color32::from_rgb(44, 44, 48);
const CONTROL_HOVER: Color32 = Color32::from_rgb(62, 62, 67);
const CONTROL_PRESSED: Color32 = Color32::from_rgb(31, 58, 92);
const FULLSCREEN_CONTROLS_SECONDS: f64 = 2.0;
const FULLSCREEN_REVEAL_SECONDS: f64 = 1.0;
// Allow for the native video's 200ms pointer sampling cadence.
const FULLSCREEN_MOTION_GAP_SECONDS: f64 = 0.35;
const MIN_BROWSE_SIZE: Vec2 = Vec2::new(860.0, 620.0);
const MIN_PLAYBACK_SIZE: Vec2 = Vec2::new(240.0, 135.0);
const MINI_PLAYER_WIDTH: f32 = 540.0;
const MINI_PLAYER_HEIGHT: f32 = 380.0;
const RESIZE_BORDER: f32 = 6.0;
const RESIZE_CORNER: f32 = 12.0;
const HOME_CARD_WIDTH: f32 = 232.0;

fn main() -> eframe::Result {
    thread::spawn(perfuse::trim_artwork_cache);
    let mut viewport = egui::ViewportBuilder::default();
    if let Some(icon) = icons::app_icon() {
        viewport = viewport.with_icon(icon);
    }
    eframe::run_native(
        "Perfuse",
        eframe::NativeOptions {
            viewport: viewport
                .with_inner_size([1440.0, 900.0])
                .with_min_inner_size(MIN_BROWSE_SIZE)
                .with_decorations(false)
                // Only the controls viewport is transparent. The default GL
                // config still has 8 alpha bits for that viewport's rendering.
                .with_transparent(false),
            ..Default::default()
        },
        Box::new(|cc| {
            #[cfg(windows)]
            icons::use_large_icon(cc);
            let mut app = App::new(&cc.egui_ctx);
            if runtime::license_accepted() {
                app.initialize(&cc.egui_ctx, false);
            } else {
                app.runtime_prompt = Some(RuntimePrompt::FirstRun {
                    download: runtime::needed(),
                    error: None,
                });
            }
            Ok(Box::new(app))
        }),
    )
}

#[derive(Clone)]
struct Location {
    title: String,
    browse: Browse,
    search: String,
    offset: usize,
    previous_pages: Vec<usize>,
    library: bool,
    parent: Option<Item>,
}

enum Reply {
    Connected(Source, Vec<Library>, session::Session),
    /// A page plus optional Home Continue Watching and Recently Released rows.
    Page {
        page: Page,
        home_rows: Option<(Vec<Item>, Vec<Item>)>,
        notice: Option<String>,
    },
    Details(Item),
    Cover(String, egui::ColorImage),
    ArtworkDone,
    Failed(String),
}

#[derive(PartialEq)]
enum RuntimeAction {
    Setup,
    Inspect,
    Check,
    Upgrade,
}

enum RuntimePrompt {
    FirstRun {
        download: bool,
        error: Option<String>,
    },
    Setup,
    Upgrade {
        libraries: Box<runtime::Available>,
        client: Option<updates::Available>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum SettingsTab {
    Servers,
    Playback,
    Updates,
}

struct App {
    settings_tab: SettingsTab,
    servers: Vec<Server>,
    active: Option<usize>,
    overview: bool,
    adding: bool,
    removing: Option<usize>,
    next_health_check: Instant,
    status_popover: servers::StatusPopover,
    server: String,
    token: String,
    client_id: String,
    source: Option<Source>,
    sections: Vec<Library>,
    selected_library: Option<String>,
    location: Option<Location>,
    history: Vec<Location>,
    search: String,
    page: Option<Page>,
    home_loaded: bool,
    on_deck: Vec<Item>,
    released: Vec<Item>,
    carousel_offsets: HashMap<&'static str, (f32, f32)>,
    covers: Artwork,
    detail: Option<Item>,
    version: usize,
    resume: bool,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    connection_open: bool,
    generation: Arc<AtomicU64>,
    tx: Sender<(u64, Reply)>,
    rx: Receiver<(u64, Reply)>,
    player: Option<player::Player>,
    surface: Option<video::Surface>,
    playing_title: String,
    playing_seconds: f64,
    playing_duration: f64,
    buffered: f64,
    volume: f64,
    seek_position: Option<f64>,
    fullscreen: bool,
    always_on_top: bool,
    overlay_focused: bool,
    overlay_hovered: bool,
    last_playback_interaction: f64,
    video_pointer: Option<(i32, i32)>,
    reveal_motion: Option<(f64, f64)>,
    controls_opacity: f32,
    paused: bool,
    playing_format: String,
    speed: f64,
    login: Option<auth::Login>,
    login_url: Option<String>,
    available_servers: Vec<auth::Server>,
    remember: bool,
    runtime_rx: Option<Receiver<runtime::Update>>,
    runtime_prompt: Option<RuntimePrompt>,
    runtime_status: Option<String>,
    runtime_failed: bool,
    runtime_action: RuntimeAction,
    runtime_installed: runtime::Installed,
    runtime_available: Option<runtime::Available>,
    runtime_result: Option<Result<String, String>>,
    client_updates: updates::Updates,
    next_update_check: Option<Instant>,
    dismissed_updates: Option<(Option<String>, Option<String>)>,
    client_after_runtime: Option<updates::Available>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        icons::install(ctx);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::BLACK;
        visuals.window_fill = PANEL;
        visuals.override_text_color = Some(TEXT);
        visuals.selection.bg_fill = Color32::from_rgb(24, 49, 82);
        visuals.selection.stroke.color = ACCENT;
        visuals.disabled_alpha = 0.38;
        visuals.widgets.inactive.bg_fill = CONTROL;
        visuals.widgets.hovered.bg_fill = CONTROL_HOVER;
        visuals.widgets.active.bg_fill = CONTROL_PRESSED;
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.corner_radius = egui::CornerRadius::same(14);
            widget.bg_stroke = egui::Stroke::NONE;
        }
        ctx.set_visuals(visuals);
        ctx.style_mut(|style| {
            style.spacing.item_spacing = Vec2::new(14.0, 14.0);
            style.spacing.button_padding = Vec2::new(16.0, 12.0);
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        });
        let (tx, rx) = mpsc::channel();
        Self {
            settings_tab: SettingsTab::Servers,
            servers: vec![],
            active: None,
            overview: false,
            adding: false,
            removing: None,
            next_health_check: Instant::now() + Duration::from_secs(60),
            status_popover: servers::StatusPopover::default(),
            server: std::env::var("PLEX_URL")
                .unwrap_or_else(|_| plex::PLUGIN.default_address.into()),
            token: std::env::var("PLEX_TOKEN").unwrap_or_default(),
            client_id: format!(
                "perfuse-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            source: None,
            sections: vec![],
            selected_library: None,
            location: None,
            history: vec![],
            search: String::new(),
            page: None,
            home_loaded: false,
            on_deck: vec![],
            released: vec![],
            carousel_offsets: HashMap::new(),
            covers: Artwork::default(),
            detail: None,
            version: 0,
            resume: true,
            busy: false,
            error: None,
            notice: None,
            connection_open: false,
            generation: Arc::new(AtomicU64::new(0)),
            tx,
            rx,
            player: None,
            surface: None,
            playing_title: String::new(),
            playing_seconds: 0.0,
            playing_duration: 0.0,
            buffered: 0.0,
            volume: 100.0,
            seek_position: None,
            fullscreen: false,
            always_on_top: false,
            overlay_focused: true,
            overlay_hovered: false,
            last_playback_interaction: 0.0,
            video_pointer: None,
            reveal_motion: None,
            controls_opacity: 1.0,
            paused: false,
            playing_format: String::new(),
            speed: 1.0,
            login: None,
            login_url: None,
            available_servers: vec![],
            remember: cfg!(windows),
            runtime_rx: None,
            runtime_prompt: None,
            runtime_status: None,
            runtime_failed: false,
            runtime_action: RuntimeAction::Setup,
            runtime_installed: runtime::Installed::default(),
            runtime_available: None,
            runtime_result: None,
            client_updates: updates::Updates::default(),
            next_update_check: None,
            dismissed_updates: None,
            client_after_runtime: None,
        }
    }

    fn initialize(&mut self, ctx: &egui::Context, download_approved: bool) {
        if cfg!(all(windows, target_arch = "x86_64")) {
            self.next_update_check = Some(Instant::now());
        }
        self.restore(ctx);
        if !runtime::needed() {
            self.runtime_action = RuntimeAction::Inspect;
            self.runtime_rx = Some(runtime::inspect(ctx.clone()));
        } else if download_approved {
            self.install_runtime(ctx);
        } else {
            self.runtime_prompt = Some(RuntimePrompt::Setup);
        }
    }

    fn restore(&mut self, ctx: &egui::Context) {
        let sessions = match session::load() {
            Ok(sessions) => sessions,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        let single = sessions.len() == 1;
        for mut saved in sessions {
            let matches = session::origin(&saved.server) == session::origin(&self.server);
            if std::env::var_os("PLEX_URL").is_some() && !matches {
                continue;
            }
            if let Ok(token) = std::env::var("PLEX_TOKEN") {
                if !matches && !single {
                    continue;
                }
                saved.token = token;
            }
            if let Ok(source) = session::open(&saved) {
                self.client_id = saved.client_id.clone();
                let mut server = Server::new(saved, source, vec![], true);
                server.check(ctx);
                self.servers.push(server);
            }
        }
        self.servers.sort_by_key(|s| s.prefs.position);
        if !self.servers.is_empty() {
            self.activate(0);
            self.overview = true;
        }
    }

    fn request_runtime_setup(&mut self) {
        if self.runtime_rx.is_none() && runtime::needed() {
            self.runtime_prompt = Some(RuntimePrompt::Setup);
        }
    }

    fn runtime_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &self.runtime_prompt else {
            return;
        };
        let first_run = matches!(prompt, RuntimePrompt::FirstRun { .. });
        let (title, confirm_label) = match prompt {
            RuntimePrompt::FirstRun { download, .. } => (
                "Perfuse and playback licences",
                if *download {
                    "Accept and download"
                } else {
                    "Accept and continue"
                },
            ),
            RuntimePrompt::Setup => ("Download playback libraries?", "Download and install"),
            RuntimePrompt::Upgrade { .. } => ("Update playback libraries?", "Download and install"),
        };
        let dialog = egui::Modal::new(egui::Id::new("runtime_licences"))
            .frame(egui::Frame::popup(&ctx.style()).fill(PANEL).corner_radius(18).inner_margin(24))
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.heading(title);
                if matches!(prompt, RuntimePrompt::FirstRun { download: false, .. }) {
                    ui.label("Review the licences for Perfuse and its playback libraries to continue.");
                } else {
                    ui.label("These third-party libraries enable playback. Perfuse will download them from shinchiro’s Windows builds on GitHub and install them for your Windows account.");
                }
                if let RuntimePrompt::Upgrade { client: Some(_), .. } = prompt {
                    ui.label("Perfuse will also download its app update and restart.");
                }
                ui.add_space(10.0);
                ui.strong("Licences and source");
                ui.hyperlink_to("Perfuse (AGPL-3.0-only)", "https://www.gnu.org/licenses/agpl-3.0.html");
                for (name, url) in runtime::LICENSE_LINKS {
                    ui.hyperlink_to(*name, *url);
                }
                if let RuntimePrompt::FirstRun { error: Some(error), .. } = prompt {
                    ui.colored_label(ERROR_TEXT, error);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let confirm = ui.add(egui::Button::new(confirm_label).fill(CONTROL_PRESSED)).clicked();
                    let cancel = ui.button(if first_run { "Decline and exit" } else { "Cancel" }).clicked();
                    (confirm, cancel)
                }).inner
            });
        let (confirm, cancel) = dialog.inner;
        if confirm
            && self.runtime_rx.is_none()
            && let Some(prompt) = self.runtime_prompt.take()
        {
            self.confirm_runtime_prompt(ctx, prompt);
        } else if cancel || (!first_run && dialog.should_close()) {
            self.runtime_prompt = None;
            if first_run {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn confirm_runtime_prompt(&mut self, ctx: &egui::Context, prompt: RuntimePrompt) {
        match prompt {
            RuntimePrompt::FirstRun { download, .. } => match runtime::accept_license() {
                Ok(()) => self.initialize(ctx, download),
                Err(error) => {
                    self.runtime_prompt = Some(RuntimePrompt::FirstRun {
                        download,
                        error: Some(error),
                    });
                }
            },
            RuntimePrompt::Setup => {
                if runtime::needed() {
                    self.install_runtime(ctx);
                }
            }
            RuntimePrompt::Upgrade { libraries, client } => {
                self.client_after_runtime = client;
                self.runtime_action = RuntimeAction::Upgrade;
                self.runtime_failed = false;
                self.runtime_status = Some("Preparing library update…".into());
                self.runtime_rx = Some(runtime::upgrade(ctx.clone(), *libraries));
            }
        }
    }

    fn install_runtime(&mut self, ctx: &egui::Context) {
        self.runtime_action = RuntimeAction::Setup;
        self.runtime_failed = false;
        self.runtime_status = Some("Preparing playback libraries…".into());
        self.runtime_rx = Some(runtime::start(ctx.clone()));
    }

    fn begin(&mut self) -> u64 {
        self.busy = true;
        self.error = None;
        // Any earlier artwork worker is cancelled with its generation.
        self.covers.loading = false;
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn connect(&mut self, ctx: &egui::Context) {
        if self
            .servers
            .iter()
            .any(|s| session::origin(&self.server) == session::origin(&s.session.server))
        {
            self.error = Some("This server has already been added.".into());
            return;
        }
        let generation = self.begin();
        let session = session::Session {
            server: self.server.clone(),
            token: self.token.clone(),
            client_id: self.client_id.clone(),
        };
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        thread::spawn(move || {
            let result = session::open(&session)
                .and_then(|source| {
                    source
                        .libraries()
                        .map(|sections| Reply::Connected(source, sections, session))
                })
                .unwrap_or_else(Reply::Failed);
            let _ = tx.send((generation, result));
            ctx.request_repaint();
        });
    }

    fn load_page(&mut self, ctx: &egui::Context, recently_added_only: bool) {
        let (Some(source), Some(location)) = (self.source.clone(), self.location.clone()) else {
            return;
        };
        let generation = self.begin();
        self.page = None;
        self.detail = None;
        if recently_added_only {
            let keep: std::collections::HashSet<String> = self
                .on_deck
                .iter()
                .chain(self.released.iter())
                .flat_map(|item| {
                    let art = (item.kind == Kind::Movie)
                        .then(|| item.art.clone())
                        .flatten();
                    [item.thumb.clone(), art].into_iter().flatten()
                })
                .collect();
            self.covers.retain(&keep);
        } else {
            self.covers.clear();
            self.home_loaded = false;
            self.on_deck.clear();
            self.released.clear();
            self.carousel_offsets.clear();
        }
        self.covers.loading = true;
        let sections = self.sections.clone();
        let (tx, ctx, current) = (self.tx.clone(), ctx.clone(), self.generation.clone());
        thread::spawn(move || {
            match source.browse(&location.browse, &location.search, location.offset) {
                Ok(page) => {
                    let mut unavailable = vec![];
                    let home_rows = if matches!(location.browse, Browse::RecentlyAdded)
                        && !recently_added_only
                    {
                        let on_deck = source
                            .browse(&Browse::ContinueWatching, "", 0)
                            .map(|page| page.items)
                            .unwrap_or_else(|_| {
                                unavailable.push("Continue Watching");
                                vec![]
                            });
                        let released = source.recently_released(&sections).unwrap_or_else(|_| {
                            unavailable.push("Recently Released");
                            vec![]
                        });
                        Some((on_deck, released))
                    } else {
                        None
                    };
                    let notice = (!unavailable.is_empty()).then(|| {
                        format!("{} is unavailable from this server. You can still browse your libraries.", unavailable.join(" and "))
                    });
                    let mut art: Vec<_> = page
                        .items
                        .iter()
                        .filter_map(|item| item.thumb.clone())
                        .collect();
                    if let Some((on_deck, released)) = &home_rows {
                        art.extend(
                            on_deck
                                .iter()
                                .chain(released.iter())
                                .filter_map(|item| item.thumb.clone()),
                        );
                        // Landscape Home cards prefer movie backdrops.
                        art.extend(
                            on_deck
                                .iter()
                                .chain(released.iter())
                                .filter(|item| item.kind == Kind::Movie)
                                .filter_map(|item| item.art.clone()),
                        );
                    }
                    if let Some(parent) = &location.parent {
                        art.extend(
                            [parent.thumb.clone(), parent.art.clone()]
                                .into_iter()
                                .flatten(),
                        );
                    }
                    let mut seen = std::collections::HashSet::new();
                    art.retain(|path| seen.insert(path.clone()));
                    if tx
                        .send((
                            generation,
                            Reply::Page {
                                page,
                                home_rows,
                                notice,
                            },
                        ))
                        .is_err()
                    {
                        return;
                    }
                    ctx.request_repaint();
                    load_artwork(&source, art, generation, &current, &tx, &ctx);
                }
                Err(error) => {
                    let _ = tx.send((generation, Reply::Failed(error)));
                    ctx.request_repaint();
                }
            }
        });
    }

    fn library(&mut self, section: &Library, ctx: &egui::Context) {
        self.selected_library = Some(section.id.clone());
        self.history.clear();
        self.search.clear();
        self.location = Some(Location {
            title: section.title.clone(),
            browse: Browse::Library(section.clone()),
            search: String::new(),
            offset: 0,
            previous_pages: vec![],
            library: true,
            parent: None,
        });
        self.load_page(ctx, false);
    }

    fn open_item(&mut self, item: &Item, ctx: &egui::Context) {
        if item.playable() {
            let Some(source) = self.source.clone() else {
                return;
            };
            let generation = self.begin();
            self.covers.loading = true;
            let (tx, ctx, id, current) = (
                self.tx.clone(),
                ctx.clone(),
                item.id.clone(),
                self.generation.clone(),
            );
            thread::spawn(move || {
                match source.item(&id) {
                    Ok(item) => {
                        let art: Vec<_> = [item.art.clone(), item.thumb.clone()]
                            .into_iter()
                            .flatten()
                            .collect();
                        let _ = tx.send((generation, Reply::Details(item)));
                        ctx.request_repaint();
                        load_artwork(&source, art, generation, &current, &tx, &ctx);
                    }
                    Err(error) => {
                        let _ = tx.send((generation, Reply::Failed(error)));
                    }
                }
                ctx.request_repaint();
            });
        } else if matches!(item.kind, Kind::Series | Kind::Season) {
            if let Some(location) = self.location.take() {
                self.history.push(location);
            }
            self.search.clear();
            self.location = Some(Location {
                title: item.title.clone(),
                browse: Browse::Children(item.id.clone()),
                search: String::new(),
                offset: 0,
                previous_pages: vec![],
                library: false,
                parent: Some(item.clone()),
            });
            self.load_page(ctx, false);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_servers(ctx);
        let mut runtime_finished = None;
        if let Some(rx) = &self.runtime_rx {
            while let Ok(update) = rx.try_recv() {
                match update {
                    runtime::Update::Status(message) => self.runtime_status = Some(message),
                    runtime::Update::Libraries(installed) => self.runtime_installed = installed,
                    runtime::Update::Available(available) => {
                        self.runtime_result = Some(Ok(match &available {
                            Some(available) => format!("Update available: {}", available.version),
                            None => "Libraries are up to date.".into(),
                        }));
                        self.runtime_available = available;
                    }
                    runtime::Update::Finished(result) => runtime_finished = Some(result),
                }
            }
        }
        if let Some(result) = runtime_finished {
            self.runtime_rx = None;
            self.runtime_failed = result.is_err();
            self.runtime_status = match result {
                Err(error) if self.runtime_action == RuntimeAction::Check => {
                    self.runtime_result = Some(Err(error));
                    None
                }
                result => result.err(),
            };
            if let Some(available) = self.client_after_runtime.take()
                && !self.runtime_failed
            {
                self.client_updates.install(ctx, available);
            }
        }
        while let Ok((generation, reply)) = self.rx.try_recv() {
            if generation != self.generation.load(Ordering::Relaxed) {
                continue;
            }
            match reply {
                Reply::Connected(source, sections, session) => {
                    self.busy = false;
                    let save_error = if self.remember {
                        session::save(&session).err()
                    } else {
                        None
                    };
                    let mut entry = Server::new(
                        session,
                        source,
                        sections,
                        self.remember && save_error.is_none(),
                    );
                    if entry.prefs.name.is_empty()
                        && let Some(discovered) = self.available_servers.iter().find(|s| {
                            s.connections.iter().any(|c| {
                                session::origin(&c.uri) == session::origin(&entry.session.server)
                            })
                        })
                    {
                        entry.prefs.name = discovered.name.clone();
                    }
                    entry.prefs.position = self.servers.len();
                    entry.persist();
                    entry.check(ctx);
                    self.servers.push(entry);
                    self.activate(self.servers.len() - 1);
                    self.overview = true;
                    self.adding = false;
                    self.connection_open = false;
                    self.token.clear();
                    self.available_servers.clear();
                    if let Some(error) = save_error {
                        self.error = Some(error);
                    }
                }
                Reply::Page {
                    page,
                    home_rows,
                    notice,
                } => {
                    self.busy = false;
                    if let Some(location) = &mut self.location {
                        location.offset = page.offset;
                    }
                    let visible = |item: &Item| {
                        !self.active.is_some_and(|i| {
                            item.library_id
                                .as_ref()
                                .is_some_and(|id| self.servers[i].prefs.hidden.contains(id))
                        })
                    };
                    self.page = Some(page);
                    if let Some((on_deck, released)) = home_rows {
                        self.home_loaded = true;
                        self.on_deck = on_deck.into_iter().filter(visible).collect();
                        self.released = released.into_iter().filter(visible).collect();
                        self.notice = notice;
                    } else if self.selected_library.is_some() {
                        self.notice = notice;
                    }
                }
                Reply::Details(item) => {
                    self.busy = false;
                    self.detail = Some(item);
                    self.version = 0;
                    self.resume = true;
                }
                Reply::Cover(path, image) => {
                    let texture = ctx.load_texture(&path, image, egui::TextureOptions::LINEAR);
                    self.covers.arrive(path, texture, ctx.input(|i| i.time));
                }
                Reply::ArtworkDone => self.covers.loading = false,
                Reply::Failed(error) => {
                    self.busy = false;
                    self.covers.loading = false;
                    self.error = Some(error);
                }
            }
        }
        let auth_events: Vec<_> = self
            .login
            .as_ref()
            .map(|login| login.events.try_iter().collect())
            .unwrap_or_default();
        for event in auth_events {
            match event {
                auth::Event::Browser(url) => {
                    self.login_url = Some(url.clone());
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
                auth::Event::Failed(error) => {
                    self.login = None;
                    self.login_url = None;
                    self.error = Some(error);
                }
                auth::Event::SignedIn(servers) => {
                    self.login = None;
                    self.login_url = None;
                    if servers.is_empty() {
                        self.error = Some("Signed in, but Plex returned no compatible movie or TV servers for this account.".into());
                    } else {
                        let matching = servers.iter().find_map(|server| {
                            server
                                .connections
                                .iter()
                                .find(|connection| {
                                    reqwest::Url::parse(&connection.uri).ok()
                                        == reqwest::Url::parse(&self.server).ok()
                                })
                                .map(|c| {
                                    (
                                        c.uri.clone(),
                                        server.access_token.clone().unwrap_or_default(),
                                    )
                                })
                        });
                        let selected = matching.or_else(|| {
                            (servers.len() == 1).then(|| {
                                (
                                    servers[0].connections[0].uri.clone(),
                                    servers[0].access_token.clone().unwrap_or_default(),
                                )
                            })
                        });
                        self.available_servers = servers;
                        if let Some((server, token)) = selected {
                            self.server = server;
                            self.token = token;
                            self.connect(ctx);
                        }
                    }
                }
            }
        }
        let mut finished = None;
        if let Some(player) = &self.player {
            while let Ok(event) = player.events.try_recv() {
                match event {
                    player::Event::Playing {
                        seconds,
                        duration,
                        buffered,
                        paused,
                        volume,
                    } => {
                        self.playing_seconds = seconds;
                        self.playing_duration = duration;
                        self.buffered = buffered;
                        self.volume = volume;
                        self.paused = paused;
                    }
                    player::Event::Warning(message) => self.notice = Some(message),
                    player::Event::Finished(result) => finished = Some(result),
                }
            }
        }
        if let Some(result) = finished {
            self.player = None;
            self.surface = None;
            self.overlay_focused = true;
            self.overlay_hovered = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(MIN_BROWSE_SIZE));
            let size = ctx.content_rect().size();
            if size.x < MIN_BROWSE_SIZE.x || size.y < MIN_BROWSE_SIZE.y {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                    size.x.max(MIN_BROWSE_SIZE.x),
                    size.y.max(MIN_BROWSE_SIZE.y),
                )));
            }
            if self.fullscreen {
                self.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            match result {
                Ok(()) => {
                    // Re-fetch on the next opening so resume uses Plex's latest saved position.
                    self.detail = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn connection_fields(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.login.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Finish signing in in your browser…");
            });
            if let Some(url) = &self.login_url {
                ui.hyperlink_to("Open Plex sign-in again", url);
            }
            if ui
                .add(Pill::secondary(Icon::Cancel, "Cancel sign-in"))
                .clicked()
            {
                self.login = None;
                self.login_url = None;
            }
        } else if ui
            .add_enabled(
                !self.busy && self.player.is_none(),
                Pill::primary(Icon::ServerPlex, "Sign in with Plex").min_width(210.0),
            )
            .clicked()
        {
            self.error = None;
            self.available_servers.clear();
            self.login = Some(auth::Login::start(self.client_id.clone()));
        }
        ui.label(
            RichText::new("Sign in securely in your browser, then choose your server.")
                .small()
                .color(MUTED),
        );
        ui.add_enabled(
            cfg!(windows) && !self.busy,
            egui::Checkbox::new(&mut self.remember, "Remember me on this computer"),
        );
        ui.label(
            RichText::new("Remembered sign-ins are stored in Windows Credential Manager.")
                .small()
                .color(MUTED),
        );
        let mut selected = None;
        for (index, server) in self.available_servers.iter().enumerate() {
            ui.add_space(10.0);
            if self.servers.iter().any(|s| {
                server
                    .connections
                    .iter()
                    .any(|c| session::origin(&c.uri) == session::origin(&s.session.server))
            }) {
                ui.label(format!("{} · Already added", server.name));
                continue;
            }
            if ui
                .add_enabled(
                    !self.busy,
                    Pill::secondary(Icon::Connect, &format!("Connect to {}", server.name)),
                )
                .clicked()
            {
                selected = Some((
                    server.connections[0].uri.clone(),
                    server.access_token.clone().unwrap_or_default(),
                ));
            }
            ui.push_id(index, |ui| {
                ui.collapsing("Other server addresses", |ui| {
                    for connection in &server.connections {
                        if ui
                            .add_enabled(!self.busy, Pill::secondary(Icon::Server, &connection.uri))
                            .clicked()
                        {
                            selected = Some((
                                connection.uri.clone(),
                                server.access_token.clone().unwrap_or_default(),
                            ));
                        }
                    }
                });
            });
        }
        if let Some((server, token)) = selected {
            self.server = server;
            self.token = token;
            self.connect(ctx);
        }
        ui.add_space(10.0);
        ui.collapsing("Manual server connection", |ui| {
            ui.add_enabled_ui(!self.busy && self.login.is_none(), |ui| {
                let server_label = ui.label("Plex server");
                ui.add(egui::TextEdit::singleline(&mut self.server).desired_width(400.0).hint_text("http://192.168.1.10:32400")).labelled_by(server_label.id);
                let token_label = ui.label("Plex token");
                ui.add(egui::TextEdit::singleline(&mut self.token).password(true).desired_width(400.0)).labelled_by(token_label.id);
                if ui.add(Pill::secondary(Icon::Connect, "Connect to server")).clicked() { self.connect(ctx); }
            });
            ui.hyperlink_to("How to find a Plex token", "https://support.plex.tv/articles/204059436-finding-an-authentication-token-x-plex-token/");
        });
        if self.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Connecting…");
            });
        }
    }

    fn home(&mut self, ctx: &egui::Context) {
        self.overview = false;
        self.selected_library = None;
        self.history.clear();
        self.search.clear();
        self.connection_open = false;
        self.location = Some(Location {
            title: self
                .active
                .map(|i| self.servers[i].name())
                .unwrap_or_else(|| "Home".into()),
            browse: Browse::RecentlyAdded,
            search: String::new(),
            offset: 0,
            previous_pages: vec![],
            library: false,
            parent: None,
        });
        self.load_page(ctx, false);
    }

    fn back(&mut self, ctx: &egui::Context) {
        if self.connection_open {
            self.connection_open = false;
        } else if self.detail.is_some() {
            self.detail = None;
            if self.location.is_none() {
                self.overview = true;
            }
        } else if let Some(location) = self.history.pop() {
            self.search = location.search.clone();
            self.location = Some(location);
            self.load_page(ctx, false);
        } else if self.selected_library.is_some() {
            self.home(ctx);
        } else {
            self.overview = true;
        }
    }

    fn set_always_on_top(&mut self, ctx: &egui::Context, enabled: bool) {
        self.always_on_top = enabled;
        ctx.send_viewport_cmd_to(
            egui::ViewportId::ROOT,
            egui::ViewportCommand::WindowLevel(if enabled {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            }),
        );
    }

    fn is_mini_player(&self, ctx: &egui::Context) -> bool {
        if self.player.is_none() || self.fullscreen {
            return false;
        }
        let size = ctx.content_rect().size();
        size.x < MINI_PLAYER_WIDTH || size.y < MINI_PLAYER_HEIGHT
    }

    fn page_previous(&mut self, ctx: &egui::Context) {
        if let Some(location) = &mut self.location {
            location.offset = location.previous_pages.pop().unwrap_or(0);
        }
        self.load_page(ctx, self.selected_library.is_none());
    }

    fn page_next(&mut self, ctx: &egui::Context) {
        if let (Some(location), Some(page)) = (&mut self.location, &self.page) {
            location.previous_pages.push(location.offset);
            location.offset = page.offset + page.items.len();
        }
        self.load_page(ctx, self.selected_library.is_none());
    }

    fn titlebar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("titlebar")
            .frame(
                egui::Frame::new()
                    .fill(Color32::BLACK)
                    .inner_margin(egui::Margin::symmetric(20, 0)),
            )
            .exact_height(72.0)
            .show(ctx, |ui| {
                let bar = ui.max_rect();
                let drag_rect = egui::Rect::from_min_max(
                    bar.min + Vec2::new(84.0, RESIZE_BORDER),
                    bar.max - Vec2::new(196.0, 0.0),
                );
                if drag_rect.is_positive() {
                    let drag = ui.interact(
                        drag_rect,
                        ui.id().with("drag"),
                        egui::Sense::click_and_drag(),
                    );
                    if drag.drag_started() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                    if drag.double_clicked() {
                        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                }
                ui.horizontal_centered(|ui| {
                    if self.player.is_none() {
                        if ui
                            .add_enabled(
                                !self.busy
                                    && (self.detail.is_some()
                                        || self.connection_open
                                        || !self.history.is_empty()
                                        || (!self.servers.is_empty() && !self.overview)),
                                IconButton::new(Icon::Back, "Back"),
                            )
                            .clicked()
                        {
                            self.back(ctx);
                        }
                    } else if ui
                        .add(IconButton::new(
                            Icon::Close,
                            "Close player and save progress",
                        ))
                        .clicked()
                        && let Some(player) = &self.player
                    {
                        player.stop();
                    }
                    ui.add(Icon::Mark.image(18.0, Color32::WHITE));
                    ui.label(RichText::new("PERFUSE").strong().color(ACCENT));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(IconButton::new(Icon::Close, "Close window"))
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        if ui
                            .add(if maximized {
                                IconButton::new(Icon::Restore, "Restore")
                            } else {
                                IconButton::new(Icon::Maximize, "Maximize")
                            })
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                        }
                        if ui
                            .add(IconButton::new(Icon::Minimize, "Minimize"))
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        let pin_label = if self.always_on_top {
                            "Disable always on top"
                        } else {
                            "Always on top"
                        };
                        if ui
                            .add(IconButton::new(Icon::Pin, pin_label).active(self.always_on_top))
                            .clicked()
                        {
                            self.set_always_on_top(ctx, !self.always_on_top);
                        }
                    });
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(244.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(20, 20, 23))
                    .inner_margin(egui::Margin::same(28)),
            )
            .show(ctx, |ui| {
                egui::Frame::new().show(ui, |ui| {
                    ui.set_min_height(ui.available_height());
                    ui.set_width(ui.available_width());
                    if nav(
                        ui,
                        Icon::Home,
                        "Home",
                        self.overview && !self.connection_open,
                    )
                    .clicked()
                        && !self.busy
                    {
                        self.overview = true;
                        self.connection_open = false;
                        self.detail = None;
                    }
                    ui.add_space(24.0);
                    egui::ScrollArea::vertical()
                        .id_salt("server_groups")
                        .max_height((ui.available_height() - 130.0).max(60.0))
                        .show(ui, |ui| {
                            for i in 0..self.servers.len() {
                                ui.push_id(i, |ui| {
                                    self.server_group(ui, ctx, i);
                                });
                                ui.add_space(12.0);
                            }
                        });
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        if nav(
                            ui,
                            Icon::Settings,
                            "Servers & settings",
                            self.connection_open,
                        )
                        .clicked()
                        {
                            self.connection_open = true;
                        }
                        self.status_button(ui, ctx);
                    });
                });
            });
    }

    fn server_group(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, i: usize) {
        let mut sections = self.servers[i].libraries();
        let mut requested_sort = None;
        control_row(ui, 28.0, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            self.group_heading(ui, i, ctx);
            if sections.len() > 1 && self.servers[i].online {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let sort_btn = ui.add(
                        IconButton::new(Icon::Sort, "Sort libraries")
                            .size(24.0)
                            .active(self.servers[i].prefs.sort != prefs::LibrarySort::Server),
                    );
                    egui::Popup::menu(&sort_btn).show(|ui| {
                        for (mode, label) in [
                            (prefs::LibrarySort::Server, "Server order"),
                            (prefs::LibrarySort::NameAsc, "Name (A–Z)"),
                            (prefs::LibrarySort::NameDesc, "Name (Z–A)"),
                            (prefs::LibrarySort::TypeThenName, "Movies, then TV"),
                        ] {
                            if ui
                                .selectable_label(self.servers[i].prefs.sort == mode, label)
                                .clicked()
                            {
                                requested_sort = Some(mode);
                            }
                        }
                        if self.servers[i].prefs.sort == prefs::LibrarySort::Custom {
                            let _ = ui.selectable_label(true, "Custom order");
                        }
                        ui.separator();
                        ui.label(
                            RichText::new("Drag libraries to reorder")
                                .small()
                                .color(MUTED),
                        );
                    });
                });
            }
        });
        let mut pending_move: Option<(usize, usize)> = None;
        let mut open_section: Option<Library> = None;
        let can_reorder = sections.len() > 1;
        let section_count = sections.len();
        if self.servers[i].prefs.collapsed {
            return;
        }
        let server_home = self.active == Some(i)
            && !self.overview
            && !self.connection_open
            && self.selected_library.is_none()
            && self.detail.is_none()
            && self
                .location
                .as_ref()
                .is_some_and(|location| matches!(location.browse, Browse::RecentlyAdded));
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 16,
                ..Default::default()
            })
            .show(ui, |ui| {
                let home = nav(ui, Icon::Home, "Home", server_home);
                let label = format!("{} Home", self.servers[i].name());
                home.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        server_home,
                        &label,
                    )
                });
                if home.on_hover_text(label).clicked() {
                    self.open_server(i, ctx);
                }
                let mut row_rects = Vec::with_capacity(section_count);
                for (index, section) in sections.iter().enumerate() {
                    let selected = !self.overview
                        && self.active == Some(i)
                        && !self.connection_open
                        && self.selected_library.as_ref() == Some(&section.id);
                    let icon = if section.kind == Kind::Series {
                        Icon::TvShows
                    } else {
                        Icon::Movies
                    };
                    let dragging_this = self.servers[i].dragging_section == Some(index);
                    let response = nav_item(
                        ui,
                        icon,
                        &section.title,
                        selected,
                        can_reorder,
                        dragging_this,
                    );
                    row_rects.push(response.rect);
                    if can_reorder && response.drag_started_by(egui::PointerButton::Primary) {
                        self.servers[i].dragging_section = Some(index);
                    }
                    if response.clicked()
                        && self.servers[i].dragging_section.is_none()
                        && !self.busy
                    {
                        open_section = Some(section.clone());
                    }
                    if can_reorder {
                        response.context_menu(|ui| {
                            if ui
                                .add_enabled(index > 0, egui::Button::new("Move up"))
                                .clicked()
                            {
                                pending_move = Some((index, index - 1));
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    index + 1 < section_count,
                                    egui::Button::new("Move down"),
                                )
                                .clicked()
                            {
                                pending_move = Some((index, index + 1));
                                ui.close();
                            }
                            ui.separator();
                            if ui.button("Sort A–Z").clicked() {
                                requested_sort = Some(prefs::LibrarySort::NameAsc);
                                ui.close();
                            }
                            if ui.button("Sort Z–A").clicked() {
                                requested_sort = Some(prefs::LibrarySort::NameDesc);
                                ui.close();
                            }
                            if ui.button("Server order").clicked() {
                                requested_sort = Some(prefs::LibrarySort::Server);
                                ui.close();
                            }
                        });
                    }
                }
                if let Some(from) = self.servers[i].dragging_section {
                    if from >= row_rects.len() {
                        self.servers[i].dragging_section = None;
                    } else {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        let pointer_y = ui
                            .ctx()
                            .input(|i| i.pointer.latest_pos())
                            .map(|p| p.y)
                            .unwrap_or_else(|| row_rects[from].center().y);
                        let drop_slot = row_rects
                            .iter()
                            .position(|rect| pointer_y < rect.center().y)
                            .unwrap_or(row_rects.len());
                        if drop_slot != from && drop_slot != from + 1 {
                            let y = if drop_slot == 0 {
                                row_rects[0].top()
                            } else if drop_slot == row_rects.len() {
                                row_rects[drop_slot - 1].bottom()
                            } else {
                                (row_rects[drop_slot - 1].bottom() + row_rects[drop_slot].top())
                                    * 0.5
                            };
                            ui.painter().hline(
                                row_rects[0].x_range(),
                                y,
                                egui::Stroke::new(2.0_f32, ACCENT),
                            );
                        }
                        if !ui.ctx().input(|i| i.pointer.primary_down()) {
                            let to = if drop_slot > from {
                                drop_slot - 1
                            } else {
                                drop_slot
                            };
                            if to != from {
                                pending_move = Some((from, to));
                            }
                            self.servers[i].dragging_section = None;
                        }
                    }
                }
            });
        if let Some(sort) = requested_sort {
            self.servers[i].prefs.sort = sort;
        }
        if let Some((from, to)) = pending_move {
            let moved = sections.remove(from);
            sections.insert(to, moved);
            self.servers[i].prefs.sort = prefs::LibrarySort::Custom;
            self.servers[i].prefs.custom_keys = sections.iter().map(|lib| lib.id.clone()).collect();
        }
        if requested_sort.is_some() || pending_move.is_some() {
            self.servers[i].persist();
            if self.active == Some(i) {
                self.sections = self.servers[i].libraries();
            }
        }
        if let Some(section) = open_section {
            if self.active != Some(i) {
                self.activate(i);
            }
            self.connection_open = false;
            self.overview = false;
            self.library(&section, ctx);
        }
    }

    fn search_library(&mut self, ctx: &egui::Context) {
        if let Some(location) = &mut self.location {
            location.search = self.search.clone();
            location.offset = 0;
            location.previous_pages.clear();
        }
        self.load_page(ctx, false);
    }

    fn horizontal_carousel_row(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        heading: &str,
        items: &[Item],
        render_card: fn(&mut egui::Ui, &Item, &Artwork) -> bool,
    ) -> Option<Item> {
        let viewport = ui.available_width();
        let spacing = ui.spacing().item_spacing.x;
        let estimated_content = if items.is_empty() {
            0.0
        } else {
            items.len() as f32 * HOME_CARD_WIDTH + (items.len() - 1) as f32 * spacing
        };
        let (offset_x, content_width) = self
            .carousel_offsets
            .get(id)
            .copied()
            .unwrap_or((0.0, estimated_content));
        let max_offset = (content_width.max(estimated_content) - viewport).max(0.0);
        let scrollable = max_offset > 1.0;
        let mut scroll_delta = 0.0_f32;
        control_row(ui, 32.0, |ui| {
            ui.label(RichText::new(heading).size(22.0).strong());
            if scrollable {
                let pitch = HOME_CARD_WIDTH + spacing;
                let step = (viewport / pitch).floor().max(1.0) * pitch;
                let back_label = format!("Scroll {heading} back");
                if ui
                    .add_enabled(
                        offset_x > 1.0,
                        IconButton::new(Icon::PagePrevious, &back_label),
                    )
                    .clicked()
                {
                    scroll_delta = step.min(offset_x.max(0.0));
                }
                let forward_label = format!("Scroll {heading} forward");
                if ui
                    .add_enabled(
                        offset_x < max_offset - 1.0,
                        IconButton::new(Icon::PageNext, &forward_label),
                    )
                    .clicked()
                {
                    scroll_delta = -step.min((max_offset - offset_x).max(0.0));
                }
            }
        });
        let mut picked = None;
        let output = egui::ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
            if scroll_delta != 0.0 {
                ui.scroll_with_delta(Vec2::new(scroll_delta, 0.0));
            }
            ui.horizontal_top(|ui| {
                for item in items {
                    if render_card(ui, item, &self.covers) && !self.busy {
                        picked = Some(item.clone());
                    }
                }
            });
        });
        self.carousel_offsets
            .insert(id, (output.state.offset.x, output.content_size.x));
        picked
    }

    fn collection(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.spacing_mut().item_spacing.y = 12.0;
        let home = self.selected_library.is_none();
        let title = self
            .location
            .as_ref()
            .map(|l| l.title.as_str())
            .unwrap_or("Your library")
            .to_owned();
        ui.horizontal(|ui| {
            let title_width = (ui.available_width() - 110.0).max(80.0);
            ui.allocate_ui_with_layout(
                Vec2::new(title_width, 40.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(egui::Label::new(RichText::new(&title).size(30.0).strong()).truncate())
                        .on_hover_text(&title);
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(!self.busy, IconButton::new(Icon::Refresh, "Refresh"))
                    .clicked()
                {
                    self.load_page(ctx, false);
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });
        if let Some(i) = self.active {
            ui.horizontal(|ui| {
                ui.add(Icon::ServerPlex.image(15.0, MUTED));
                ui.colored_label(
                    MUTED,
                    if home {
                        format!("Plex server · {} libraries", self.sections.len())
                    } else {
                        format!("Plex / {}", self.servers[i].name())
                    },
                );
            });
        }
        ui.add_space(34.0);
        if self.location.as_ref().is_some_and(|l| l.library) {
            ui.horizontal(|ui| {
                let (search, clear) = Pill::pair(
                    ui,
                    Pill::secondary(Icon::Search, "Search"),
                    Pill::secondary(Icon::Clear, "Clear"),
                );
                let clearing = !self.search.is_empty();
                let shown: &[&Pill] = if clearing {
                    &[&search, &clear]
                } else {
                    &[&search]
                };
                // Keep the field usable; narrower rows drop the button labels.
                let compact = !Pill::labels_fit(ui, shown, 264.0);
                let (search, clear) = (search.icon_only(compact), clear.icon_only(compact));
                let spacing = ui.spacing().item_spacing.x;
                let mut buttons = search.width(ui) + spacing;
                if clearing {
                    buttons += clear.width(ui) + spacing;
                }
                // The text edit adds its 12pt side margins to the desired width.
                let search_width = (ui.available_width() - buttons - 24.0).max(80.0);
                let response = ui.add_enabled(
                    !self.busy,
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search this library")
                        .desired_width(search_width)
                        .margin(Vec2::new(12.0, 10.0)),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        ui.is_enabled(),
                        "Search library",
                    )
                });
                let submitted =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.add_enabled(!self.busy, search).clicked() || submitted {
                    self.search_library(ctx);
                }
                if clearing && ui.add_enabled(!self.busy, clear).clicked() {
                    self.search.clear();
                    self.search_library(ctx);
                }
            });
            ui.add_space(28.0);
        }
        if self.sections.is_empty() {
            ui.heading("No movie or TV libraries available");
            ui.label("Check this user's access in Plex, then reconnect in settings.");
            return;
        }
        let mut picked = None;
        let mut page_action: i8 = 0;
        let show_home_rows = home
            && (self.home_loaded
                || self.page.is_some()
                || !self.on_deck.is_empty()
                || !self.released.is_empty());
        egui::ScrollArea::vertical()
            .id_salt(("collection", &title))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(parent) = self.location.as_ref().and_then(|l| l.parent.as_ref()) {
                    hero(ui, parent, &self.covers, "TV LIBRARY");
                    ui.add_space(38.0);
                }
                if show_home_rows {
                    if self.on_deck.is_empty() {
                        ui.label(RichText::new("Continue Watching").size(22.0).strong());
                        ui.label(
                            RichText::new(
                                "Nothing to continue. Start a film or episode from your library.",
                            )
                            .color(MUTED),
                        );
                    } else {
                        let on_deck = self.on_deck.clone();
                        if let Some(item) = self.horizontal_carousel_row(
                            ui,
                            "continue",
                            "Continue Watching",
                            &on_deck,
                            continue_card,
                        ) {
                            picked = Some(item);
                        }
                    }
                    ui.add_space(48.0);
                    if !self.released.is_empty() {
                        let released = self.released.clone();
                        if let Some(item) = self.horizontal_carousel_row(
                            ui,
                            "released",
                            "Recently Released",
                            &released,
                            released_card,
                        ) {
                            picked = Some(item);
                        }
                        ui.add_space(48.0);
                    }
                }
                if let Some(page) = &self.page {
                    let has_previous = self
                        .location
                        .as_ref()
                        .is_some_and(|l| !l.previous_pages.is_empty());
                    let has_next = page.has_next();
                    control_row(ui, 32.0, |ui| {
                        ui.label(
                            RichText::new(if home {
                                "Recently Added"
                            } else if self.location.as_ref().is_some_and(|l| !l.library) {
                                "Seasons & Episodes"
                            } else if self.location.as_ref().is_some_and(|l| !l.search.is_empty()) {
                                "Search Results"
                            } else {
                                "All Titles"
                            })
                            .size(22.0)
                            .strong(),
                        );
                        if !page.items.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "{}–{}{}",
                                    page.offset + 1,
                                    page.offset + page.items.len(),
                                    page.total.map(|n| format!(" of {n}")).unwrap_or_default()
                                ))
                                .size(13.0)
                                .color(MUTED),
                            );
                        }
                        if home && (has_previous || has_next) {
                            if ui
                                .add_enabled(
                                    !self.busy && has_previous,
                                    IconButton::new(
                                        Icon::PagePrevious,
                                        "Previous Recently Added page",
                                    ),
                                )
                                .clicked()
                            {
                                page_action = -1;
                            }
                            if ui
                                .add_enabled(
                                    !self.busy && has_next,
                                    IconButton::new(Icon::PageNext, "Next Recently Added page"),
                                )
                                .clicked()
                            {
                                page_action = 1;
                            }
                        }
                    });
                    ui.add_space(14.0);
                    let items: Vec<_> = page
                        .items
                        .iter()
                        .filter(|item| {
                            !self.active.is_some_and(|i| {
                                item.library_id
                                    .as_ref()
                                    .is_some_and(|id| self.servers[i].prefs.hidden.contains(id))
                            })
                        })
                        .collect();
                    if items.is_empty() {
                        ui.add_space(34.0);
                        ui.heading("No titles found");
                        ui.label(
                            RichText::new("Try another search, or choose a different library.")
                                .color(MUTED),
                        );
                    }
                    let episodes =
                        !items.is_empty() && items.iter().all(|i| i.kind == Kind::Episode);
                    if episodes {
                        for &item in &items {
                            egui::Frame::new()
                                .fill(PANEL)
                                .corner_radius(18)
                                .inner_margin(24)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        if cover(ui, item, &self.covers, Vec2::new(112.0, 64.0))
                                            .clicked()
                                            && !self.busy
                                        {
                                            picked = Some(item.clone());
                                        }
                                        ui.vertical(|ui| {
                                            ui.set_width(ui.available_width());
                                            ui.label(
                                                RichText::new(item.subtitle())
                                                    .small()
                                                    .color(ACCENT),
                                            );
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new(&item.title).strong(),
                                                    )
                                                    .frame(false)
                                                    .wrap_mode(egui::TextWrapMode::Truncate),
                                                )
                                                .clicked()
                                                && !self.busy
                                            {
                                                picked = Some(item.clone());
                                            }
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&item.summary)
                                                        .small()
                                                        .color(MUTED),
                                                )
                                                .truncate(),
                                            );
                                        });
                                    });
                                });
                        }
                    } else {
                        let columns =
                            ((ui.available_width() + 18.0) / 176.0).floor().max(1.0) as usize;
                        let width = ((ui.available_width() - 18.0 * (columns - 1) as f32)
                            / columns as f32)
                            .min(210.0);
                        egui::Grid::new("posters")
                            .num_columns(columns)
                            .spacing([18.0, 22.0])
                            .show(ui, |ui| {
                                for (index, item) in items.into_iter().enumerate() {
                                    if poster(ui, item, &self.covers, width) && !self.busy {
                                        picked = Some(item.clone());
                                    }
                                    if (index + 1) % columns == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    }
                    ui.add_space(38.0);
                } else if self.busy {
                    if show_home_rows {
                        ui.label(RichText::new("Recently Added").size(22.0).strong());
                        ui.add_space(14.0);
                        ui.label(RichText::new("Loading Recently Added…").color(MUTED));
                    } else {
                        ui.label(RichText::new("Loading your library…").color(MUTED));
                    }
                } else {
                    ui.label("Use Refresh to try loading this page again, or select a library.");
                }
            });
        if page_action < 0 {
            self.page_previous(ctx);
        } else if page_action > 0 {
            self.page_next(ctx);
        }
        if let Some(item) = picked {
            self.open_item(&item, ctx);
        }
    }

    fn details(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(item) = self.detail.clone() else {
            return false;
        };
        let mut play = false;
        egui::ScrollArea::vertical()
            .id_salt("detail_page")
            .show(ui, |ui| {
                hero(ui, &item, &self.covers, "ORIGINAL QUALITY");
                ui.add_space(28.0);
                ui.horizontal_wrapped(|ui| {
                    if !item.versions.is_empty() {
                        let offset = item.resume_ms.unwrap_or(0);
                        let (resume, restart) = Pill::pair(
                            ui,
                            if offset > 0 {
                                Pill::primary(Icon::Resume, "Resume")
                            } else {
                                Pill::primary(Icon::Play, "Play")
                            }
                            .min_width(150.0),
                            Pill::secondary(Icon::PlayFromBeginning, "Play from beginning"),
                        );
                        let shown: &[&Pill] = if offset > 0 {
                            &[&resume, &restart]
                        } else {
                            &[&resume]
                        };
                        // Leave room for the Watched badge beside the buttons.
                        let compact = !Pill::labels_fit(ui, shown, 110.0);
                        let resume = if offset > 0 {
                            resume.icon_only(compact)
                        } else {
                            resume.min_width(150.0)
                        };
                        if ui.add(resume).clicked() {
                            self.resume = true;
                            play = true;
                        }
                        if offset > 0 && ui.add(restart.icon_only(compact)).clicked() {
                            self.resume = false;
                            play = true;
                        }
                    } else {
                        ui.label("No media file is available for this item.");
                    }
                    if item.watched {
                        badge(ui, Icon::Watched, "Watched", GREEN);
                    }
                });
                if let (Some(offset), Some(duration)) = (item.resume_ms, item.duration_ms)
                    && offset > 0
                    && duration > 0
                {
                    ui.add_space(16.0);
                    ui.add(
                        egui::ProgressBar::new((offset as f32 / duration as f32).clamp(0.0, 1.0))
                            .fill(ACCENT)
                            .desired_width(220.0)
                            .text(format!(
                                "{} left",
                                clock(duration.saturating_sub(offset) as f64 / 1000.0)
                            )),
                    );
                }
                ui.add_space(48.0);
                ui.label(RichText::new("About").size(22.0).strong());
                ui.label(if item.summary.is_empty() {
                    "No synopsis available."
                } else {
                    &item.summary
                });
                ui.add_space(44.0);
                ui.label(RichText::new("Playback").size(22.0).strong());
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(18)
                    .inner_margin(28)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if !item.versions.is_empty() {
                            self.version = self.version.min(item.versions.len() - 1);
                            egui::ComboBox::from_label("Version")
                                .selected_text(item.versions[self.version].label())
                                .show_ui(ui, |ui| {
                                    for (index, media) in item.versions.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.version,
                                            index,
                                            format!("{} · {}", index + 1, media.label()),
                                        );
                                    }
                                });
                            ui.separator();
                            let media = &item.versions[self.version];
                            info_row(
                                ui,
                                "Video",
                                media.video_codec.as_deref().unwrap_or("Not provided"),
                            );
                            info_row(
                                ui,
                                "Audio",
                                media.audio_codec.as_deref().unwrap_or("Not provided"),
                            );
                            info_row(
                                ui,
                                "Container",
                                media.container.as_deref().unwrap_or("Not provided"),
                            );
                            if matches!(media.stream, Stream::Multipart) {
                                ui.colored_label(
                                    ACCENT,
                                    "This version has multiple parts and cannot be played yet.",
                                );
                            }
                        }
                        ui.separator();
                        info_row(ui, "Playback", "Original file · Direct Play");
                        info_row(
                            ui,
                            "Read ahead",
                            "Up to 5 minutes · fills in the background",
                        );
                        ui.label(
                            RichText::new(
                                "Change embedded audio and subtitle tracks during playback.",
                            )
                            .small()
                            .color(MUTED),
                        );
                    });
            });
        play
    }

    fn start_playback(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if runtime::needed() {
            self.request_runtime_setup();
            return;
        }
        if self.runtime_rx.is_some() {
            return;
        }
        let (Some(item), Some(source)) = (self.detail.clone(), self.source.clone()) else {
            return;
        };
        if self.player.is_some() {
            return;
        }
        let surface = match video::Surface::new(frame) {
            Ok(surface) => surface,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };
        self.playing_title = item.title.clone();
        self.playing_format = item
            .versions
            .get(self.version)
            .map(|m| m.label())
            .unwrap_or_default();
        self.playing_seconds = if self.resume {
            item.resume_ms.unwrap_or(0) as f64 / 1000.0
        } else {
            0.0
        };
        self.paused = false;
        self.speed = 1.0;
        self.playing_duration = item.duration_ms.unwrap_or(0) as f64 / 1000.0;
        self.buffered = 0.0;
        self.seek_position = None;
        self.notice = None;
        self.error = None;
        self.overlay_focused = true;
        self.overlay_hovered = false;
        self.player = Some(player::Player::start(
            Some(surface.id()),
            source,
            item,
            self.version,
            self.resume,
        ));
        self.surface = Some(surface);
        self.connection_open = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(MIN_PLAYBACK_SIZE));
    }

    fn settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.set_max_width(ui.available_width().min(980.0));
        ui.label(RichText::new("Settings").size(34.0).strong());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (tab, icon, text) in [
                (SettingsTab::Servers, Icon::Server, "Servers"),
                (SettingsTab::Playback, Icon::Play, "Playback"),
                (SettingsTab::Updates, Icon::Refresh, "Updates & about"),
            ] {
                let active = self.settings_tab == tab;
                let color = if active { TEXT } else { MUTED };
                let button = egui::Button::image_and_text(
                    icon.image(16.0, if active { ACCENT } else { MUTED }),
                    RichText::new(text).size(15.0).color(color),
                )
                .min_size(Vec2::new(
                    if tab == SettingsTab::Updates {
                        154.0
                    } else {
                        104.0
                    },
                    38.0,
                ))
                .frame(false);
                let response = ui.add(button);
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        true,
                        active,
                        text,
                    )
                });
                if active {
                    ui.painter().hline(
                        response.rect.x_range(),
                        response.rect.bottom() + 3.0,
                        egui::Stroke::new(2.0_f32, ACCENT),
                    );
                }
                if response.clicked() {
                    self.settings_tab = tab;
                }
            }
        });
        ui.separator();
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt(("settings", self.settings_tab))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.settings_tab == SettingsTab::Servers {
                    self.server_settings(ui, ctx);
                    return;
                }
                if self.settings_tab == SettingsTab::Playback {
                    ui.label(RichText::new("Playback configuration").size(23.0).strong());
                    egui::Frame::new()
                        .fill(PANEL)
                        .corner_radius(14)
                        .inner_margin(20)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for (n, (label, value)) in [
                                ("Streaming quality", "Original · Direct Play"),
                                ("Hardware decoding", "Automatic · safe modes"),
                                ("Video output", "DirectX 11 · embedded in Perfuse"),
                                ("Read ahead", "300 seconds · temporary disk cache"),
                            ]
                            .into_iter()
                            .enumerate()
                            {
                                if n > 0 {
                                    ui.separator();
                                }
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(label).size(15.0));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            ui.colored_label(MUTED, value);
                                        },
                                    );
                                });
                            }
                        });
                    ui.colored_label(
                        MUTED,
                        "Change audio, subtitles, volume and speed in the player.",
                    );
                    return;
                }
                self.third_party_libraries(ui, ctx);
                ui.add_space(22.0);
                ui.heading("About");
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Perfuse {}", updates::VERSION));
                    ui.hyperlink_to(
                        "GitHub",
                        "https://github.com/HammoTime/perfuse-video-player",
                    );
                    match &self.client_updates.status {
                        Some(Ok(message)) => {
                            let color = if self.client_updates.busy() {
                                MUTED
                            } else {
                                GREEN
                            };
                            ui.colored_label(color, message);
                        }
                        Some(Err(error)) => {
                            ui.colored_label(ERROR_TEXT, error);
                        }
                        None => {}
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            cfg!(all(windows, target_arch = "x86_64"))
                                && !self.client_updates.busy()
                                && self.client_after_runtime.is_none(),
                            egui::Button::new("Check for Perfuse updates"),
                        )
                        .clicked()
                    {
                        self.dismissed_updates = None;
                        self.client_updates.check(ctx);
                    }
                    if self.client_updates.available.is_some()
                        && ui
                            .add_enabled(
                                self.runtime_rx.is_none() && !self.client_updates.busy(),
                                egui::Button::new("Update Perfuse"),
                            )
                            .clicked()
                    {
                        self.update_available(ctx, true);
                    }
                });
            });
    }

    fn third_party_libraries(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("3rd party libraries");
        egui::Frame::new().fill(PANEL).corner_radius(18).inner_margin(28).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if self.runtime_installed.libraries.is_empty() {
                ui.label(if self.runtime_rx.is_some() { "Reading installed libraries…" } else { "No playback libraries found." });
            } else {
                egui::Grid::new("third_party_libraries").num_columns(2).striped(true)
                    .max_col_width((ui.available_width() * 0.55).max(80.0)).show(ui, |ui| {
                        ui.strong("Name"); ui.strong("Version"); ui.end_row();
                        for library in &self.runtime_installed.libraries {
                            ui.label(&library.name);
                            ui.label(RichText::new(&library.version).color(MUTED));
                            ui.end_row();
                        }
                    });
            }
            ui.add_space(16.0);
            ui.horizontal_wrapped(|ui| {
                ui.style_mut().override_font_id = Some(egui::FontId::proportional(13.0));
                ui.colored_label(MUTED, "Playback libraries update together as one runtime.");
                match &self.runtime_result {
                    Some(Ok(message)) => { ui.colored_label(GREEN, message); }
                    Some(Err(error)) => { ui.colored_label(ERROR_TEXT, error); }
                    None if self.runtime_action == RuntimeAction::Check && self.runtime_rx.is_some() => {
                        ui.colored_label(MUTED, "Checking for library updates…");
                    }
                    None => {}
                }
            });
            let overridden = std::env::var_os("PERFUSE_LIBMPV").is_some();
            if overridden {
                ui.label("Updates are managed by PERFUSE_LIBMPV. Remove the override to update here.");
            }
            ui.add_space(16.0);
            let idle = self.runtime_rx.is_none() && self.player.is_none();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(idle && cfg!(windows), egui::Button::new("Check for Updates")).clicked() {
                    self.dismissed_updates = None;
                    self.check_runtime(ctx);
                }
                if ui.add_enabled(idle && !self.client_updates.downloading && cfg!(windows) && !overridden && self.runtime_available.is_some(), egui::Button::new("Update")).clicked() {
                    self.update_available(ctx, false);
                }
            });
        });
    }

    fn check_runtime(&mut self, ctx: &egui::Context) {
        self.runtime_action = RuntimeAction::Check;
        self.runtime_result = None;
        self.runtime_status = None;
        self.runtime_failed = false;
        self.runtime_rx = Some(runtime::check(ctx.clone()));
    }

    fn update_available(&mut self, ctx: &egui::Context, include_client: bool) {
        let client = self
            .client_updates
            .available
            .clone()
            .filter(|_| include_client);
        if let Some(libraries) = self.runtime_available.clone()
            && std::env::var_os("PERFUSE_LIBMPV").is_none()
        {
            self.runtime_prompt = Some(RuntimePrompt::Upgrade {
                libraries: Box::new(libraries),
                client,
            });
        } else if let Some(available) = client {
            self.client_updates.install(ctx, available);
        }
    }

    fn poll_updates(&mut self, ctx: &egui::Context) {
        if self.client_updates.poll() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if let Some(next) = self.next_update_check {
            let now = Instant::now();
            if now < next {
                ctx.request_repaint_after(next - now);
            } else if self.runtime_prompt.is_none()
                && self.runtime_rx.is_none()
                && !self.client_updates.busy()
            {
                self.next_update_check = Some(now + UPDATE_INTERVAL);
                ctx.request_repaint_after(UPDATE_INTERVAL);
                self.client_updates.check(ctx);
                if std::env::var_os("PERFUSE_LIBMPV").is_none() && !runtime::needed() {
                    self.check_runtime(ctx);
                }
            }
        }
    }

    fn update_notification(&mut self, ctx: &egui::Context, offset: f32) {
        let client = self
            .client_updates
            .available
            .as_ref()
            .map(|update| update.version.clone());
        let libraries = self
            .runtime_available
            .as_ref()
            .filter(|_| std::env::var_os("PERFUSE_LIBMPV").is_none())
            .map(|update| update.version.clone());
        let title = match (&client, &libraries) {
            (Some(version), Some(_)) => format!("Perfuse {version} and library updates available"),
            (Some(version), None) => format!("Perfuse {version} is available"),
            (None, Some(_)) => "Playback library update available".into(),
            (None, None) => return,
        };
        let key = (client, libraries);
        if self.dismissed_updates.as_ref() == Some(&key) {
            return;
        }
        egui::Area::new(egui::Id::new("update_notification"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_TOP, [-16.0, offset])
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .fill(PANEL)
                    .inner_margin(16)
                    .show(ui, |ui| {
                        ui.strong(title);
                        if self.client_updates.downloading {
                            if let Some(Ok(status)) = &self.client_updates.status {
                                ui.label(status);
                            }
                        } else if self.runtime_action == RuntimeAction::Upgrade
                            && let Some(status) = &self.runtime_status
                        {
                            let color = if self.runtime_failed {
                                ERROR_TEXT
                            } else {
                                TEXT
                            };
                            ui.colored_label(color, status);
                        } else if key.0.is_some()
                            && let Some(Err(error)) = &self.client_updates.status
                        {
                            ui.colored_label(ERROR_TEXT, error);
                        }
                        ui.horizontal(|ui| {
                            let action = match (&key.0, &key.1) {
                                (Some(_), Some(_)) => "Update both and restart",
                                (Some(_), None) => "Update Perfuse and restart",
                                _ => "Update libraries",
                            };
                            let idle = self.runtime_rx.is_none() && !self.client_updates.busy();
                            if ui.add_enabled(idle, egui::Button::new(action)).clicked() {
                                // The root context owns shutdown even when the notification
                                // is drawn in the native playback overlay.
                                self.update_available(ctx, key.0.is_some());
                            }
                            if ui.button("Later").clicked() {
                                self.dismissed_updates = Some(key.clone());
                            }
                        });
                    });
            });
    }

    fn playback_controls_visible(&mut self, ctx: &egui::Context) -> bool {
        // The embedded Win32 child receives mouse movement instead of egui.
        let pointer = self
            .surface
            .as_ref()
            .and_then(video::Surface::pointer_position);
        let native_moved = pointer.is_some() && pointer != self.video_pointer;
        self.video_pointer = pointer;
        let (now, moved, active) = ctx.input(|i| {
            let moved = i.pointer.delta() != Vec2::ZERO;
            let active = i.pointer.any_down()
                || i.events.iter().any(|event| {
                    matches!(
                        event,
                        egui::Event::PointerButton { .. }
                            | egui::Event::Key { .. }
                            | egui::Event::MouseWheel { .. }
                            | egui::Event::Text(_)
                    )
                });
            (i.time, moved || native_moved, active)
        });
        if !self.fullscreen {
            self.last_playback_interaction = now;
            self.reveal_motion = None;
            // Unfocused always-on-top windows hide controls until hovered so
            // picture-in-picture stays unobstructed while using other apps.
            if self.surface.is_some() && self.always_on_top && !self.overlay_focused {
                return self.overlay_hovered
                    || active
                    || egui::Popup::is_any_open(ctx)
                    || self.runtime_prompt.is_some();
            }
            return true;
        }
        let visible = now - self.last_playback_interaction < FULLSCREEN_CONTROLS_SECONDS;
        let reveal = if visible {
            self.reveal_motion = None;
            moved
        } else if moved {
            let start = match self.reveal_motion {
                Some((start, last)) if now - last <= FULLSCREEN_MOTION_GAP_SECONDS => start,
                _ => now,
            };
            self.reveal_motion = Some((start, now));
            now - start >= FULLSCREEN_REVEAL_SECONDS
        } else {
            if self
                .reveal_motion
                .is_some_and(|(_, last)| now - last > FULLSCREEN_MOTION_GAP_SECONDS)
            {
                self.reveal_motion = None;
            }
            false
        };
        if reveal || active || egui::Popup::is_any_open(ctx) || self.runtime_prompt.is_some() {
            self.last_playback_interaction = now;
            self.reveal_motion = None;
        }
        now - self.last_playback_interaction < FULLSCREEN_CONTROLS_SECONDS
    }

    fn playback_controls(&mut self, ctx: &egui::Context, mini: bool) {
        let controls_visible = self.playback_controls_visible(ctx);
        self.controls_opacity = ctx.animate_value_with_time(
            egui::Id::new("playback_controls_fade"),
            if controls_visible { 1.0 } else { 0.0 },
            if controls_visible { 0.3 } else { 0.375 },
        );
        let opacity = self.controls_opacity;
        let bounds = ctx.available_rect();
        if self.surface.is_none() {
            // The headless app-loop tests use the same overlay layout.
            self.playback_overlay(ctx, bounds, opacity, mini);
        } else {
            // A native video child covers its parent's GL drawing. Use an owned,
            // transparent viewport for controls, keeping the video embedded.
            let title = format!("Perfuse playback controls {}", std::process::id());
            let origin = ctx.input(|i| i.viewport().inner_rect.map_or(egui::Pos2::ZERO, |r| r.min));
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("playback_overlay"),
                egui::ViewportBuilder::default()
                    .with_title(&title)
                    .with_decorations(false)
                    .with_transparent(true)
                    .with_has_shadow(false)
                    .with_taskbar(false)
                    .with_active(false)
                    .with_visible(false)
                    .with_position(origin + bounds.min.to_vec2())
                    .with_inner_size(bounds.size()),
                |overlay, class| {
                    if class == egui::ViewportClass::Embedded {
                        self.playback_overlay(overlay, bounds, opacity, mini);
                    } else {
                        let state = self
                            .surface
                            .as_ref()
                            .map(|s| s.prepare_overlay(&title))
                            .unwrap_or_default();
                        self.overlay_focused = state.focused;
                        self.overlay_hovered = state.hovered;
                        // Keep the transparent overlay mounted while always-on-top
                        // (windowed) so hovering PiP never steals focus via ShowWindow.
                        let keep_pip_overlay = state.topmost && !self.fullscreen;
                        overlay.send_viewport_cmd(egui::ViewportCommand::Visible(
                            state.ready && (opacity > 0.0 || keep_pip_overlay),
                        ));
                        self.playback_controls_visible(overlay);
                        self.playback_overlay(overlay, overlay.content_rect(), opacity, mini);
                        if !mini && opacity > 0.0 {
                            self.update_notification(overlay, 82.0);
                        }
                    }
                    self.runtime_dialog(overlay);
                    if class != egui::ViewportClass::Embedded && self.runtime_prompt.is_none() {
                        self.playback_shortcuts(overlay);
                    }
                },
            );
        }
        if self.runtime_prompt.is_none() {
            self.playback_shortcuts(ctx);
        }
    }

    fn playback_overlay(
        &mut self,
        ctx: &egui::Context,
        bounds: egui::Rect,
        opacity: f32,
        mini: bool,
    ) {
        let Some(player) = &self.player else {
            return;
        };
        if self.surface.is_some() {
            self.handle_window_resize(ctx, egui::ViewportId::ROOT, mini);
        }
        if mini {
            let drag_rect = bounds.shrink(RESIZE_BORDER);
            if drag_rect.is_positive() {
                egui::Area::new(egui::Id::new("mini_drag"))
                    .order(egui::Order::Background)
                    .movable(false)
                    .fixed_pos(drag_rect.min)
                    .show(ctx, |ui| {
                        let (_, response) =
                            ui.allocate_exact_size(drag_rect.size(), egui::Sense::drag());
                        if response.drag_started_by(egui::PointerButton::Primary) {
                            ctx.send_viewport_cmd_to(
                                egui::ViewportId::ROOT,
                                egui::ViewportCommand::StartDrag,
                            );
                        }
                    });
            }
            if opacity > 0.0 {
                egui::Area::new(egui::Id::new("mini_play"))
                    .order(egui::Order::Foreground)
                    .movable(false)
                    .pivot(egui::Align2::CENTER_CENTER)
                    .fixed_pos(bounds.center())
                    .show(ctx, |ui| {
                        ui.set_opacity(opacity);
                        let size = (bounds.height().min(bounds.width()) * 0.32).clamp(40.0, 56.0);
                        let (icon, label) = if self.paused {
                            (Icon::Play, "Play")
                        } else {
                            (Icon::Pause, "Pause")
                        };
                        if ui.add(MiniPlayButton { icon, label, size }).clicked() {
                            player.pause();
                        }
                    });
            }
            return;
        }
        if opacity > 0.0 {
            egui::Area::new(egui::Id::new("playing_header"))
                .order(egui::Order::Foreground)
                .movable(false)
                .fixed_pos(bounds.left_top())
                .show(ctx, |ui| {
                    ui.set_width(bounds.width());
                    ui.set_opacity(opacity);
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(56, 28))
                        .show(ui, |ui| {
                            // Reserve the fullscreen button before the title takes the remaining width.
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let button = if self.fullscreen {
                                        IconButton::new(Icon::FullscreenExit, "Exit fullscreen")
                                    } else {
                                        IconButton::new(Icon::Fullscreen, "Fullscreen")
                                    };
                                    if ui.add(button.size(40.0)).clicked() {
                                        self.fullscreen = !self.fullscreen;
                                        ctx.send_viewport_cmd_to(
                                            egui::ViewportId::ROOT,
                                            egui::ViewportCommand::Fullscreen(self.fullscreen),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&self.playing_title)
                                                        .size(20.0)
                                                        .strong(),
                                                )
                                                .truncate(),
                                            );
                                            ui.label(
                                                RichText::new(if self.paused {
                                                    "Paused"
                                                } else {
                                                    "Now Playing"
                                                })
                                                .small()
                                                .color(MUTED),
                                            );
                                        },
                                    );
                                },
                            );
                        });
                });
            egui::Area::new(egui::Id::new("transport"))
                .order(egui::Order::Foreground)
                .movable(false)
                .pivot(egui::Align2::LEFT_BOTTOM)
                .fixed_pos(bounds.left_bottom())
                .show(ctx, |ui| {
                    ui.set_width(bounds.width());
                    ui.set_opacity(opacity);
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(56, 32))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing = Vec2::new(14.0, 16.0);
                            control_row(ui, 24.0, |ui| {
                                let position = self.seek_position.unwrap_or(self.playing_seconds);
                                ui.label(RichText::new(clock(position)).size(13.0));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "−{}",
                                                clock(
                                                    (self.playing_duration - self.playing_seconds)
                                                        .max(0.0)
                                                )
                                            ))
                                            .size(13.0)
                                            .color(MUTED),
                                        );
                                        let mut position = position;
                                        let seek = seekbar(
                                            ui,
                                            &mut position,
                                            self.playing_duration,
                                            self.playing_seconds + self.buffered,
                                        );
                                        if seek.dragged() {
                                            self.seek_position = Some(position);
                                        }
                                        if seek.drag_stopped()
                                            || (seek.changed() && !seek.dragged())
                                        {
                                            player.seek(position);
                                            self.seek_position = None;
                                        }
                                    },
                                );
                            });
                            control_row(ui, 44.0, |ui| {
                                if ui
                                    .add(IconButton::new(Icon::Back10, "Back 10s").size(44.0))
                                    .clicked()
                                {
                                    player.seek(self.playing_seconds - 10.0);
                                }
                                let button = if self.paused {
                                    Pill::primary(Icon::Play, "Play")
                                } else {
                                    Pill::primary(Icon::Pause, "Pause")
                                };
                                let button = button.min_width(130.0);
                                // Both seek buttons and the playback details keep their room.
                                let compact = !Pill::labels_fit(ui, &[&button], 370.0);
                                if ui.add(button.icon_only(compact)).clicked() {
                                    player.pause();
                                }
                                if ui
                                    .add(IconButton::new(Icon::Forward10, "Forward 10s").size(44.0))
                                    .clicked()
                                {
                                    player.seek(self.playing_seconds + 10.0);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "Buffered {} / 5:00",
                                                clock(self.buffered)
                                            ))
                                            .size(12.0)
                                            .color(MUTED),
                                        );
                                        badge(ui, Icon::StatusConnected, "Direct Play", GREEN);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&self.playing_format)
                                                    .size(12.0)
                                                    .color(MUTED),
                                            )
                                            .truncate(),
                                        );
                                    },
                                );
                            });
                            control_row(ui, 36.0, |ui| {
                                let (audio, subtitles) = Pill::pair(
                                    ui,
                                    Pill::secondary(Icon::AudioTrack, "Audio track"),
                                    Pill::secondary(Icon::Subtitles, "Subtitles"),
                                );
                                // The speed menu, chapter buttons and volume keep their room.
                                let compact = !Pill::labels_fit(ui, &[&audio, &subtitles], 360.0);
                                let (audio, subtitles) =
                                    (audio.icon_only(compact), subtitles.icon_only(compact));
                                if ui
                                    .add(audio)
                                    .on_hover_text("Cycle embedded audio tracks")
                                    .clicked()
                                {
                                    player.audio();
                                }
                                if ui
                                    .add(subtitles)
                                    .on_hover_text("Cycle embedded subtitles, including off")
                                    .clicked()
                                {
                                    player.subtitles();
                                }
                                let speed = ui
                                    .add(Pill::secondary(Icon::Speed, &format!("{}×", self.speed)))
                                    .on_hover_text("Playback speed");
                                egui::Popup::menu(&speed).show(|ui| {
                                    for speed in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0] {
                                        if ui
                                            .selectable_value(
                                                &mut self.speed,
                                                speed,
                                                format!("{speed}×"),
                                            )
                                            .clicked()
                                        {
                                            player.speed(speed);
                                        }
                                    }
                                });
                                if ui
                                    .add(
                                        IconButton::new(Icon::ChapterPrevious, "Previous chapter")
                                            .size(36.0),
                                    )
                                    .clicked()
                                {
                                    player.chapter(-1);
                                }
                                if ui
                                    .add(
                                        IconButton::new(Icon::ChapterNext, "Next chapter")
                                            .size(36.0),
                                    )
                                    .clicked()
                                {
                                    player.chapter(1);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.spacing_mut().slider_width = 110.0;
                                        ui.visuals_mut().selection.bg_fill = ACCENT;
                                        let slider = ui.add(
                                            egui::Slider::new(&mut self.volume, 0.0..=100.0)
                                                .show_value(false)
                                                .trailing_fill(true),
                                        );
                                        let icon = if self.volume <= 0.0 {
                                            Icon::VolumeMuted
                                        } else if self.volume < 50.0 {
                                            Icon::VolumeLow
                                        } else {
                                            Icon::VolumeHigh
                                        };
                                        let label =
                                            ui.add(icon.image(20.0, TEXT).alt_text("Volume"));
                                        if slider
                                            .labelled_by(label.id)
                                            .on_hover_text(format!("Volume {:.0}%", self.volume))
                                            .changed()
                                        {
                                            player.volume(self.volume);
                                        }
                                    },
                                );
                            });
                        });
                });
        }
    }

    fn playback_shortcuts(&mut self, ctx: &egui::Context) {
        let Some(player) = &self.player else {
            return;
        };
        // Keep shortcuts active even while the fullscreen panels are hidden.
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            self.fullscreen = !self.fullscreen;
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Fullscreen(self.fullscreen),
            );
        }
        if self.fullscreen && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.fullscreen = false;
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Fullscreen(false),
            );
        }
        if !ctx.wants_keyboard_input() {
            ctx.input(|i| {
                if i.key_pressed(egui::Key::Space) {
                    player.pause();
                }
                if i.key_pressed(egui::Key::ArrowLeft) {
                    player.seek(self.playing_seconds - 10.0);
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    player.seek(self.playing_seconds + 10.0);
                }
            });
        }
    }
    fn handle_window_resize(
        &self,
        ctx: &egui::Context,
        target: egui::ViewportId,
        allow_top: bool,
    ) -> bool {
        if self.fullscreen || ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return false;
        }
        if ctx.dragged_id().is_some() {
            return false;
        }
        let pos = ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()));
        let Some(dir) = pos.and_then(|pos| resize_direction(ctx.content_rect(), pos, allow_top))
        else {
            return false;
        };
        ctx.set_cursor_icon(resize_cursor(dir));
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd_to(target, egui::ViewportCommand::BeginResize(dir));
            return true;
        }
        false
    }
}

impl eframe::App for App {
    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        // Root window gaps/margins must stay opaque. eframe clears immediate
        // viewports (the playback overlay) separately to transparent RGBA.
        [0.0, 0.0, 0.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.poll_updates(ctx);
        self.covers.now = ctx.input(|i| i.time);
        if self.player.is_none() && self.covers.animating() {
            ctx.request_repaint();
        }
        if self.player.is_some() || self.busy || self.login.is_some() || self.runtime_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let mini = self.is_mini_player(ctx);
        if !self.fullscreen && !mini {
            self.titlebar(ctx);
        }
        if self.source.is_some() && self.player.is_none() {
            self.sidebar(ctx);
        }
        if !mini && (self.error.is_some() || self.notice.is_some() || self.runtime_status.is_some())
        {
            egui::TopBottomPanel::bottom("messages")
                .frame(
                    egui::Frame::new()
                        .fill(PANEL)
                        .inner_margin(egui::Margin::same(28)),
                )
                .show(ctx, |ui| {
                    if let Some(status) = self.runtime_status.clone() {
                        ui.horizontal_wrapped(|ui| {
                            if self.runtime_rx.is_some() {
                                ui.spinner();
                            }
                            ui.label(status);
                            if self.runtime_failed
                                && self.runtime_action == RuntimeAction::Setup
                                && ui.button("Retry playback setup").clicked()
                            {
                                self.request_runtime_setup();
                            }
                        });
                    }
                    if let Some(error) = self.error.clone() {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(Icon::Alert.image(18.0, ERROR_TEXT));
                            ui.colored_label(ERROR_TEXT, error);
                            if ui.add(IconButton::new(Icon::Dismiss, "Dismiss")).clicked() {
                                self.error = None;
                            }
                        });
                    }
                    if let Some(notice) = self.notice.clone() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(notice);
                            if ui.add(IconButton::new(Icon::Dismiss, "Dismiss")).clicked() {
                                self.notice = None;
                            }
                        });
                    }
                });
        }
        if self.player.is_some() {
            if let Some(surface) = &self.surface {
                surface.place(ctx.available_rect(), ctx.pixels_per_point());
                surface.reclaim_keyboard();
            }
            self.playback_controls(ctx, mini);
        } else if self.source.is_some()
            && !self.connection_open
            && !self.overview
            && self.detail.is_none()
            && (self.page.as_ref().is_some_and(Page::has_next)
                || self
                    .location
                    .as_ref()
                    .is_some_and(|l| !l.previous_pages.is_empty()))
        {
            egui::TopBottomPanel::bottom("pagination")
                .frame(egui::Frame::new().inner_margin(egui::Margin::same(28)))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let previous = self
                            .location
                            .as_ref()
                            .is_some_and(|l| !l.previous_pages.is_empty());
                        let next = self.page.as_ref().is_some_and(Page::has_next);
                        let (previous_button, next_button) = Pill::pair(
                            ui,
                            Pill::secondary(Icon::PagePrevious, "Previous page"),
                            Pill::secondary(Icon::PageNext, "Next page").trailing(),
                        );
                        let compact = !Pill::labels_fit(ui, &[&previous_button, &next_button], 0.0);
                        let (previous_button, next_button) = (
                            previous_button.icon_only(compact),
                            next_button.icon_only(compact),
                        );
                        if ui
                            .add_enabled(!self.busy && previous, previous_button)
                            .clicked()
                        {
                            self.page_previous(ctx);
                        }
                        if ui.add_enabled(!self.busy && next, next_button).clicked() {
                            self.page_next(ctx);
                        }
                    });
                });
        }
        let mut play = false;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(if self.source.is_some() {
                        Color32::from_rgb(13, 13, 16)
                    } else {
                        Color32::BLACK
                    })
                    .inner_margin(if self.player.is_some() {
                        egui::Margin::ZERO
                    } else if self.source.is_some() {
                        egui::Margin::symmetric(48, 38)
                    } else {
                        egui::Margin::same(20)
                    }),
            )
            .show(ctx, |ui| {
                if self.player.is_some() {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
                } else if self.source.is_none() {
                    // Short windows tighten the card so discovered servers stay in view.
                    let roomy = ui.available_height() > 640.0;
                    let space =
                        |roomy_space: f32, compact: f32| if roomy { roomy_space } else { compact };
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.add_space(space(32.0, 4.0));
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 480.0).max(0.0) / 2.0);
                            egui::Frame::new()
                                .fill(PANEL)
                                .corner_radius(26)
                                .inner_margin(space(42.0, 28.0) as i8)
                                .show(ui, |ui| {
                                    // A frame inherits its parent's layout; the centering row
                                    // is horizontal, while the card's contents must stack.
                                    ui.vertical(|ui| {
                                        ui.set_width(420.0);
                                        ui.horizontal(|ui| {
                                            ui.add(Icon::Mark.image(38.0, Color32::WHITE));
                                            ui.label(
                                                RichText::new("PERFUSE")
                                                    .size(38.0)
                                                    .strong()
                                                    .color(ACCENT),
                                            );
                                        });
                                        ui.add_space(space(20.0, 10.0));
                                        ui.heading("Make yourself at home.");
                                        ui.label(
                                            RichText::new(
                                                "Connect a Plex server to watch your films and shows. You can add more servers later.",
                                            )
                                            .color(MUTED),
                                        );
                                        ui.add_space(space(34.0, 18.0));
                                        self.connection_fields(ui, ctx);
                                    });
                                });
                        });
                    });
                } else {
                    egui::Frame::new().show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        if self.connection_open {
                            self.settings(ui, ctx);
                        } else if self.overview {
                            self.servers_home(ui, ctx);
                        } else if self.active.is_some_and(|i| !self.servers[i].online) {
                            self.offline_view(ui, ctx);
                        } else if self.detail.is_some() {
                            play = self.details(ui);
                        } else {
                            self.collection(ui, ctx);
                        }
                    });
                }
            });
        if play {
            self.start_playback(ctx, frame);
        }
        if self.surface.is_none() {
            if !mini {
                self.update_notification(ctx, 58.0);
            }
            self.server_dialogs(ctx);
            self.runtime_dialog(ctx);
        }
        self.handle_window_resize(ctx, egui::ViewportId::ROOT, true);
    }
}

/// Sends cached artwork straight away, then fetches the rest from the server.
/// The worker stops as soon as navigation starts a newer generation.
fn load_artwork(
    source: &Source,
    paths: Vec<String>,
    generation: u64,
    current: &AtomicU64,
    tx: &Sender<(u64, Reply)>,
    ctx: &egui::Context,
) {
    let send = |path: String, image: image::RgbaImage| {
        let size = [image.width() as usize, image.height() as usize];
        let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        ctx.request_repaint();
        tx.send((generation, Reply::Cover(path, pixels))).is_ok()
    };
    let mut missing = vec![];
    for path in paths {
        if current.load(Ordering::Relaxed) != generation {
            return;
        }
        match source.cached_artwork(&path) {
            Some(image) => {
                if !send(path, image) {
                    return;
                }
            }
            None => missing.push(path),
        }
    }
    for path in missing {
        if current.load(Ordering::Relaxed) != generation {
            return;
        }
        if let Ok(image) = source.artwork(&path)
            && !send(path, image)
        {
            return;
        }
    }
    let _ = tx.send((generation, Reply::ArtworkDone));
    ctx.request_repaint();
}

const ART_FADE_SECONDS: f64 = 1.0;

/// Artwork textures for the current view, with when each arrived so new art
/// fades in, and whether more is still loading so its slot shows a placeholder.
#[derive(Default, Clone)]
struct Artwork {
    textures: HashMap<String, (egui::TextureHandle, f64)>,
    loading: bool,
    now: f64,
}

impl Artwork {
    /// Adds art that is shown immediately, without fading in.
    fn insert(&mut self, path: String, texture: egui::TextureHandle) {
        self.textures.insert(path, (texture, f64::NEG_INFINITY));
    }

    fn arrive(&mut self, path: String, texture: egui::TextureHandle, now: f64) {
        self.textures.insert(path, (texture, now));
    }

    fn clear(&mut self) {
        self.textures.clear();
    }

    fn retain(&mut self, keep: &std::collections::HashSet<String>) {
        self.textures.retain(|path, _| keep.contains(path));
    }

    fn contains(&self, path: &str) -> bool {
        self.textures.contains_key(path)
    }

    /// The texture and its current fade-in opacity.
    fn get(&self, path: &str) -> Option<(&egui::TextureHandle, f32)> {
        self.textures.get(path).map(|(texture, arrived)| {
            let t = ((self.now - arrived) / ART_FADE_SECONDS).clamp(0.0, 1.0) as f32;
            (texture, 1.0 - (1.0 - t).powi(3))
        })
    }

    fn pending(&self, path: Option<&String>) -> bool {
        self.loading && path.is_some_and(|path| !self.contains(path))
    }

    fn animating(&self) -> bool {
        self.loading
            || self
                .textures
                .values()
                .any(|(_, arrived)| self.now - arrived < ART_FADE_SECONDS)
    }
}

/// A softly pulsing slot shown while artwork loads.
fn placeholder(ui: &egui::Ui, rect: egui::Rect, radius: f32) {
    let pulse = (ui.input(|i| i.time) * 2.4).sin() as f32 * 0.5 + 0.5;
    let base = Color32::from_rgb(34, 36, 40);
    ui.painter().rect_filled(
        rect,
        radius,
        base.lerp_to_gamma(Color32::from_rgb(52, 55, 61), pulse),
    );
}

fn nav(ui: &mut egui::Ui, icon: Icon, title: &str, selected: bool) -> egui::Response {
    nav_item(ui, icon, title, selected, false, false)
}

fn nav_item(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    selected: bool,
    draggable: bool,
    dragging: bool,
) -> egui::Response {
    let sense = if draggable {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 42.0), sense);
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            title,
        )
    });
    if selected || dragging || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            8.0,
            if dragging {
                CONTROL_PRESSED
            } else if selected {
                Color32::from_rgb(23, 42, 64)
            } else {
                PANEL
            },
        );
    }
    if response.has_focus() || dragging {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    icon.paint(
        ui,
        rect.left_center() + Vec2::new(SIDEBAR_ICON_INSET + 10.0, 0.0),
        20.0,
        if selected || dragging { ACCENT } else { MUTED },
    );
    ui.painter().with_clip_rect(rect.shrink(4.0)).text(
        rect.left_center() + Vec2::new(SIDEBAR_TEXT_INSET, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(14.0),
        if selected || dragging {
            Color32::WHITE
        } else {
            Color32::from_gray(195)
        },
    );
    response
}

/// Labelled pill button following assets/buttons/button-primary.svg and
/// button-secondary.svg. When a row is too narrow for labels it can show only
/// its icon in a round button; the label stays as tooltip and accessible name.
struct Pill<'a> {
    icon: Icon,
    label: &'a str,
    primary: bool,
    trailing: bool,
    tall: bool,
    icon_only: bool,
    min_width: f32,
}

impl<'a> Pill<'a> {
    const ICON: f32 = 18.0;
    const GAP: f32 = 8.0;

    fn primary(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            primary: true,
            trailing: false,
            tall: false,
            icon_only: false,
            min_width: 0.0,
        }
    }

    fn secondary(icon: Icon, label: &'a str) -> Self {
        Self {
            primary: false,
            ..Self::primary(icon, label)
        }
    }

    fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    /// Places the icon after the label, for forward actions.
    fn trailing(mut self) -> Self {
        self.trailing = true;
        self
    }

    fn icon_only(mut self, icon_only: bool) -> Self {
        self.icon_only = icon_only;
        self
    }

    /// Gives two side-by-side buttons one size: the wider width, and the
    /// primary height when either is primary.
    fn pair(ui: &egui::Ui, first: Self, second: Self) -> (Self, Self) {
        let width = first.natural_width(ui).max(second.natural_width(ui));
        let tall = first.primary || second.primary;
        let even = |pill: Self| Self {
            tall,
            ..pill.min_width(width)
        };
        (even(first), even(second))
    }

    /// Whether these buttons fit with their labels beside `reserved` width of
    /// other content in the rest of the row.
    fn labels_fit(ui: &egui::Ui, pills: &[&Self], reserved: f32) -> bool {
        let spacing = ui.spacing().item_spacing.x;
        let labelled: f32 = pills
            .iter()
            .map(|pill| pill.natural_width(ui).max(pill.min_width) + spacing)
            .sum();
        labelled + reserved <= ui.available_width()
    }

    fn height(&self) -> f32 {
        if self.primary || self.tall {
            44.0
        } else {
            36.0
        }
    }

    fn padding(&self) -> f32 {
        if self.primary { 20.0 } else { 14.0 }
    }

    fn ink(&self) -> Color32 {
        if self.primary { Color32::BLACK } else { TEXT }
    }

    fn galley(&self, ui: &egui::Ui) -> std::sync::Arc<egui::Galley> {
        let text = RichText::new(self.label).color(self.ink());
        egui::WidgetText::from(if self.primary { text.strong() } else { text }).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (ui.available_width() - 2.0 * self.padding() - Self::ICON - Self::GAP).max(0.0),
            egui::FontId::proportional(15.0),
        )
    }

    fn natural_width(&self, ui: &egui::Ui) -> f32 {
        self.galley(ui).size().x + Self::ICON + Self::GAP + 2.0 * self.padding()
    }

    /// The width this button will take in the row.
    fn width(&self, ui: &egui::Ui) -> f32 {
        if self.icon_only {
            self.height()
        } else {
            self.natural_width(ui).max(self.min_width)
        }
    }
}

impl egui::Widget for Pill<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        const ICON: f32 = Pill::ICON;
        const GAP: f32 = Pill::GAP;
        let height = self.height();
        let radius = if self.primary || self.tall || self.icon_only {
            height / 2.0
        } else {
            14.0
        };
        let ink = self.ink();
        let galley = self.galley(ui);
        let width = self.width(ui);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), self.label)
        });
        if ui.is_rect_visible(rect) {
            let pressed = response.is_pointer_button_down_on();
            let fill = if !self.primary {
                control_fill(&response)
            } else if pressed {
                Color32::from_rgb(212, 212, 219)
            } else if response.hovered() {
                Color32::from_rgb(237, 237, 242)
            } else {
                Color32::WHITE
            };
            ui.painter().rect_filled(rect, radius, fill);
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.expand(3.0),
                    radius + 3.0,
                    egui::Stroke::new(2.0_f32, ACCENT),
                    egui::StrokeKind::Middle,
                );
            }
            if self.icon_only {
                self.icon.paint(ui, rect.center(), ICON, ink);
            } else {
                let content = galley.size().x + ICON + GAP;
                let left = rect.center().x - content / 2.0;
                let (icon_x, text_x) = if self.trailing {
                    (left + galley.size().x + GAP + ICON / 2.0, left)
                } else {
                    (left + ICON / 2.0, left + ICON + GAP)
                };
                self.icon
                    .paint(ui, egui::pos2(icon_x, rect.center().y), ICON, ink);
                ui.painter().galley(
                    egui::pos2(text_x, rect.center().y - galley.size().y / 2.0),
                    galley,
                    ink,
                );
            }
        }
        let response = if self.icon_only {
            response.on_hover_text(self.label)
        } else {
            response
        };
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

/// Circular icon-only button following assets/buttons/button-icon.svg. The
/// label is announced to assistive technology and shown as a tooltip.
struct IconButton<'a> {
    icon: Icon,
    label: &'a str,
    size: f32,
    active: bool,
}

impl<'a> IconButton<'a> {
    fn new(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            size: 32.0,
            active: false,
        }
    }

    fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
}

impl egui::Widget for IconButton<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), self.label)
        });
        if ui.is_rect_visible(rect) {
            let radius = self.size / 2.0;
            let fill = if self.active {
                if response.hovered() {
                    Color32::from_rgb(39, 72, 114)
                } else {
                    CONTROL_PRESSED
                }
            } else {
                control_fill(&response)
            };
            let ink = if self.active { ACCENT } else { TEXT };
            ui.painter().circle_filled(rect.center(), radius, fill);
            if response.has_focus() {
                ui.painter().circle_stroke(
                    rect.center(),
                    radius + 3.0,
                    egui::Stroke::new(2.0_f32, ACCENT),
                );
            }
            self.icon
                .paint(ui, rect.center(), (self.size * 0.56).round(), ink);
        }
        response
            .on_hover_text(self.label)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

/// Centered circular primary Play/Pause button for compact picture-in-picture playback.
struct MiniPlayButton<'a> {
    icon: Icon,
    label: &'a str,
    size: f32,
}

impl egui::Widget for MiniPlayButton<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.size), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), self.label)
        });
        if ui.is_rect_visible(rect) {
            let radius = self.size / 2.0;
            let fill = if response.is_pointer_button_down_on() {
                Color32::from_rgb(212, 212, 219)
            } else if response.hovered() {
                Color32::from_rgb(237, 237, 242)
            } else {
                Color32::WHITE
            };
            ui.painter()
                .circle_filled(rect.center(), radius + 4.0, Color32::from_black_alpha(96));
            ui.painter().circle_filled(rect.center(), radius, fill);
            if response.has_focus() {
                ui.painter().circle_stroke(
                    rect.center(),
                    radius + 3.0,
                    egui::Stroke::new(2.0_f32, ACCENT),
                );
            }
            self.icon.paint(
                ui,
                rect.center(),
                (self.size * 0.48).round(),
                Color32::BLACK,
            );
        }
        response
            .on_hover_text(self.label)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

fn resize_direction(
    rect: egui::Rect,
    pos: egui::Pos2,
    allow_top: bool,
) -> Option<egui::ResizeDirection> {
    use egui::ResizeDirection::*;
    if !rect.contains(pos) {
        return None;
    }
    let left = pos.x - rect.left() < RESIZE_BORDER;
    let right = rect.right() - pos.x < RESIZE_BORDER;
    let top = allow_top && pos.y - rect.top() < RESIZE_BORDER;
    let bottom = rect.bottom() - pos.y < RESIZE_BORDER;
    if !(left || right || top || bottom) {
        return None;
    }
    let near_left = pos.x - rect.left() < RESIZE_CORNER;
    let near_right = rect.right() - pos.x < RESIZE_CORNER;
    let near_top = allow_top && pos.y - rect.top() < RESIZE_CORNER;
    let near_bottom = rect.bottom() - pos.y < RESIZE_CORNER;
    Some(
        match (
            left || near_left,
            right || near_right,
            top || near_top,
            bottom || near_bottom,
        ) {
            (true, _, true, _) => NorthWest,
            (_, true, true, _) => NorthEast,
            (true, _, _, true) => SouthWest,
            (_, true, _, true) => SouthEast,
            (true, _, _, _) => West,
            (_, true, _, _) => East,
            (_, _, true, _) => North,
            (_, _, _, true) => South,
            _ => return None,
        },
    )
}

fn resize_cursor(direction: egui::ResizeDirection) -> egui::CursorIcon {
    use egui::ResizeDirection::*;
    match direction {
        North | South => egui::CursorIcon::ResizeVertical,
        East | West => egui::CursorIcon::ResizeHorizontal,
        NorthWest | SouthEast => egui::CursorIcon::ResizeNwSe,
        NorthEast | SouthWest => egui::CursorIcon::ResizeNeSw,
    }
}

fn control_fill(response: &egui::Response) -> Color32 {
    if response.is_pointer_button_down_on() {
        CONTROL_PRESSED
    } else if response.hovered() {
        CONTROL_HOVER
    } else {
        CONTROL
    }
}

fn badge(ui: &mut egui::Ui, icon: Icon, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let label = RichText::new(text).size(12.0).strong().color(color);
                // Keep the icon first when placed in a right-to-left row.
                if ui.layout().prefer_right_to_left() {
                    ui.label(label);
                    ui.add(icon.image(14.0, color));
                } else {
                    ui.add(icon.image(14.0, color));
                    ui.label(label);
                }
            });
        });
}

fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        ui.label(RichText::new(value.to_uppercase()).color(MUTED));
    });
}

fn cover(ui: &mut egui::Ui, item: &Item, covers: &Artwork, size: Vec2) -> egui::Response {
    let path = if size.x > size.y && item.kind == Kind::Movie {
        item.art
            .as_ref()
            .filter(|path| covers.contains(path))
            .or(item.thumb.as_ref())
    } else {
        item.thumb.as_ref()
    };
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("Open {}", item.title),
        )
    });
    if let Some((texture, opacity)) = path.and_then(|path| covers.get(path)) {
        if opacity < 1.0 {
            ui.painter()
                .rect_filled(rect, 8.0, Color32::from_rgb(34, 36, 40));
        }
        let ratio = size.x / size.y / texture.aspect_ratio();
        let uv_size = Vec2::new(ratio.min(1.0), (1.0 / ratio).min(1.0));
        egui::Image::new(texture)
            .uv(egui::Rect::from_center_size(egui::pos2(0.5, 0.5), uv_size))
            .corner_radius(8)
            .tint(Color32::WHITE.gamma_multiply(opacity))
            .paint_at(ui, rect);
    } else if covers.pending(path) {
        placeholder(ui, rect, 8.0);
    } else {
        ui.painter()
            .rect_filled(rect, 8.0, Color32::from_rgb(34, 36, 40));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            item.title.chars().next().unwrap_or('P'),
            egui::FontId::proportional(42.0),
            ACCENT,
        );
    }
    if response.hovered() || response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(2.0_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
}

fn continue_card(ui: &mut egui::Ui, item: &Item, covers: &Artwork) -> bool {
    let mut clicked = false;
    ui.vertical(|ui| {
        ui.set_width(232.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        let response = cover(ui, item, covers, Vec2::new(232.0, 130.0));
        let rect = response.rect;
        clicked |= response
            .on_hover_text(format!("Resume {}", item.title))
            .clicked();
        if let (Some(offset), Some(duration)) = (item.resume_ms, item.duration_ms)
            && duration > 0
        {
            let track = egui::Rect::from_min_size(
                rect.left_bottom() + Vec2::new(8.0, -8.0),
                Vec2::new(rect.width() - 16.0, 3.0),
            );
            ui.painter()
                .rect_filled(track, 2.0, Color32::from_white_alpha(70));
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    track.min,
                    Vec2::new(
                        track.width() * (offset as f32 / duration as f32).clamp(0.0, 1.0),
                        3.0,
                    ),
                ),
                2.0,
                ACCENT,
            );
        }
        clicked |= ui
            .add(
                egui::Button::new(RichText::new(&item.title).strong())
                    .frame(false)
                    .wrap_mode(egui::TextWrapMode::Truncate),
            )
            .clicked();
        let subtitle = item
            .duration_ms
            .map(|duration| {
                format!(
                    "{}  ·  {} min left",
                    item.subtitle(),
                    duration
                        .saturating_sub(item.resume_ms.unwrap_or(0))
                        .div_ceil(60_000)
                )
            })
            .unwrap_or_else(|| item.subtitle());
        ui.add(egui::Label::new(RichText::new(subtitle).size(12.0).color(MUTED)).truncate());
    });
    clicked
}

fn released_card(ui: &mut egui::Ui, item: &Item, covers: &Artwork) -> bool {
    let mut clicked = false;
    ui.vertical(|ui| {
        ui.set_width(232.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        clicked |= cover(ui, item, covers, Vec2::new(232.0, 130.0))
            .on_hover_text(&item.title)
            .clicked();
        clicked |= ui
            .add(
                egui::Button::new(RichText::new(&item.title).strong())
                    .frame(false)
                    .wrap_mode(egui::TextWrapMode::Truncate),
            )
            .clicked();
        let subtitle = match item.release_date.as_deref().and_then(release_date) {
            Some(date) => format!("{}  ·  {date}", item.subtitle()),
            None => item.subtitle(),
        };
        ui.add(egui::Label::new(RichText::new(subtitle).size(12.0).color(MUTED)).truncate());
    });
    clicked
}

/// Formats Plex's `YYYY-MM-DD` as e.g. `20 Nov 2026`.
fn release_date(date: &str) -> Option<String> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = date.get(..10)?.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let month = MONTHS.get(month.parse::<usize>().ok()?.checked_sub(1)?)?;
    let day: u32 = day.parse().ok()?;
    (year.len() == 4 && year.parse::<u32>().is_ok()).then(|| format!("{day} {month} {year}"))
}

fn poster(ui: &mut egui::Ui, item: &Item, covers: &Artwork, width: f32) -> bool {
    let mut clicked = false;
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 6.0;
        let response = cover(ui, item, covers, Vec2::new(width, width * 1.5));
        let rect = response.rect;
        clicked |= response.on_hover_text(&item.title).clicked();
        if let Some(resolution) = item
            .versions
            .first()
            .and_then(|m| m.video_resolution.as_deref())
        {
            let text = resolution.to_uppercase();
            let galley =
                ui.painter()
                    .layout_no_wrap(text, egui::FontId::proportional(11.0), Color32::WHITE);
            let badge = egui::Rect::from_min_size(
                rect.right_top() + Vec2::new(-galley.size().x - 16.0, 8.0),
                galley.size() + Vec2::new(8.0, 6.0),
            );
            ui.painter()
                .rect_filled(badge, 4.0, Color32::from_black_alpha(200));
            ui.painter()
                .galley(badge.min + Vec2::new(4.0, 3.0), galley, Color32::WHITE);
        }
        if let (Some(offset), Some(duration)) = (item.resume_ms, item.duration_ms)
            && offset > 0
            && duration > 0
        {
            let track = egui::Rect::from_min_size(
                rect.left_bottom() + Vec2::new(8.0, -10.0),
                Vec2::new(width - 16.0, 4.0),
            );
            ui.painter()
                .rect_filled(track, 2.0, Color32::from_white_alpha(80));
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    track.min,
                    Vec2::new(
                        track.width() * (offset as f32 / duration as f32).clamp(0.0, 1.0),
                        4.0,
                    ),
                ),
                2.0,
                ACCENT,
            );
        }
        if item.watched {
            let center = rect.left_top() + Vec2::splat(18.0);
            // The icon's check is cut out, so give it a dark backing on bright art.
            ui.painter()
                .circle_filled(center, 8.0, Color32::from_black_alpha(200));
            Icon::Watched.paint(ui, center, 24.0, ACCENT);
        }
        clicked |= ui
            .add(
                egui::Button::new(RichText::new(&item.title).strong())
                    .frame(false)
                    .wrap_mode(egui::TextWrapMode::Truncate),
            )
            .clicked();
        ui.add(egui::Label::new(RichText::new(item.subtitle()).size(12.0).color(MUTED)).truncate());
    });
    clicked
}

fn hero(ui: &mut egui::Ui, item: &Item, covers: &Artwork, eyebrow: &str) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 228.0), egui::Sense::hover());
    if covers.pending(item.art.as_ref()) {
        placeholder(ui, rect, 22.0);
    } else {
        ui.painter().rect_filled(rect, 22.0, PANEL);
    }
    if let Some((texture, opacity)) = item.art.as_ref().and_then(|path| covers.get(path)) {
        // Crop to fill rather than stretching the server's backdrop.
        let ratio = rect.aspect_ratio() / texture.aspect_ratio();
        let uv = if ratio > 1.0 {
            let h = 1.0 / ratio;
            egui::Rect::from_min_max(
                egui::pos2(0.0, (1.0 - h) / 2.0),
                egui::pos2(1.0, (1.0 + h) / 2.0),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2((1.0 - ratio) / 2.0, 0.0),
                egui::pos2((1.0 + ratio) / 2.0, 1.0),
            )
        };
        egui::Image::new(texture)
            .uv(uv)
            .corner_radius(22)
            .tint(Color32::WHITE.gamma_multiply(opacity))
            .paint_at(ui, rect);
        ui.painter().rect_filled(
            rect,
            22.0,
            Color32::from_black_alpha(155).gamma_multiply(opacity),
        );
    }
    // This content paints inside an already allocated rectangle. A scope would
    // advance the parent cursor again using the shorter content's bounds.
    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(26.0))
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    content.set_clip_rect(ui.clip_rect().intersect(rect.shrink(20.0)));
    content.horizontal(|ui| {
        let art_width = 112.0;
        cover(ui, item, covers, Vec2::new(art_width, art_width * 1.5));
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(eyebrow).size(12.0).strong().color(ACCENT));
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(&item.title).size(28.0).strong()).truncate());
            ui.label(
                RichText::new(format!(
                    "{}{}",
                    item.subtitle(),
                    item.duration_ms
                        .map(|d| format!(" · {} min", d / 60_000))
                        .unwrap_or_default()
                ))
                .color(MUTED),
            );
            if let Some(media) = item.versions.first() {
                ui.label(RichText::new(media.label()).size(12.0).strong());
            }
            ui.add_space(8.0);
            ui.add(egui::Label::new(&item.summary).truncate());
        });
    });
}

/// A fixed-height, left-to-right row whose items share one vertical centre.
fn control_row<R>(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), height),
        egui::Layout::left_to_right(egui::Align::Center),
        add,
    )
    .inner
}

fn seekbar(
    ui: &mut egui::Ui,
    position: &mut f64,
    duration: f64,
    buffered_end: f64,
) -> egui::Response {
    let duration = duration.max(1.0);
    let (rect, mut response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(100.0), 24.0),
        egui::Sense::click_and_drag(),
    );
    let focused = response.has_focus();
    if response.clicked() {
        response.request_focus();
    }
    if (response.clicked() || response.dragged())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        *position = ((pointer.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64 * duration;
        response.mark_changed();
    }
    if focused {
        let delta = ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowRight) {
                10.0
            } else if i.key_pressed(egui::Key::ArrowLeft) {
                -10.0
            } else {
                0.0
            }
        });
        if delta != 0.0 {
            *position = (*position + delta).clamp(0.0, duration);
            response.mark_changed();
        }
    }
    response.widget_info(|| egui::WidgetInfo::slider(true, *position, "Playback position"));
    let track = egui::Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 6.0));
    ui.painter().rect_filled(track, 3.0, Color32::from_gray(65));
    for (end, color) in [(buffered_end, Color32::from_gray(110)), (*position, ACCENT)] {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                track.min,
                Vec2::new(track.width() * (end / duration).clamp(0.0, 1.0) as f32, 6.0),
            ),
            3.0,
            color,
        );
    }
    ui.painter().circle_filled(
        egui::pos2(
            track.left() + track.width() * (*position / duration).clamp(0.0, 1.0) as f32,
            track.center().y,
        ),
        if focused || response.hovered() {
            8.0
        } else {
            6.0
        },
        Color32::WHITE,
    );
    response.on_hover_text("Seek · use Left / Right to move 10 seconds")
}

fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_release_dates() {
        assert_eq!(release_date("2026-11-05").as_deref(), Some("5 Nov 2026"));
        assert_eq!(
            release_date("2026-01-31 00:00:00").as_deref(),
            Some("31 Jan 2026")
        );
        for invalid in ["2026-13-01", "2026-00-01", "26-11-05", "soon"] {
            assert_eq!(release_date(invalid), None);
        }
    }

    #[test]
    fn library_and_detail_fit_the_minimum_window() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx);
        let item = Item {
            title: "A long title that needs to wrap in a narrow window".into(),
            kind: Kind::Movie,
            summary: "A synopsis for checking the responsive native layout. ".repeat(8),
            duration_ms: Some(7_200_000),
            resume_ms: Some(600_000),
            ..Default::default()
        };
        app.sections.push(Library {
            id: "1".into(),
            title: "Movies".into(),
            kind: Kind::Movie,
        });
        app.selected_library = Some("1".into());
        app.page = Some(Page {
            items: vec![item.clone(); 5],
            offset: 0,
            total: Some(5),
        });
        for details in [false, true] {
            app.detail = details.then(|| item.clone());
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(860.0, 620.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    app.titlebar(ctx);
                    app.sidebar(ctx);
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let available = ui.available_width();
                        if details {
                            app.details(ui);
                        } else {
                            app.collection(ui, ctx);
                        }
                        assert!(
                            ui.min_rect().width() <= available + 1.0,
                            "content overflows the minimum window"
                        );
                    });
                },
            );
        }
        app.back(&ctx);
        assert!(app.detail.is_none());
        assert_eq!(app.selected_library.as_deref(), Some("1"));
    }
}
