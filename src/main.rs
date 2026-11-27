#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod artwork;
use artwork::Artwork;
mod audiobooks;
mod connection;
mod icons;
mod mpv;
mod player;
mod prefs;
mod runtime;
mod screen_fade;
mod servers;
mod session;
use servers::Server;
mod tasks;
mod updates;
mod video;

#[cfg(test)]
mod ui_tests;

use eframe::egui::{self, Color32, RichText, Vec2};
use icons::Icon;
use perfuse::{
    media::{self, Browse, Item, Kind, Library, LibraryKind, Page, Source, SourceError, Stream},
    metadata,
    plugins::{
        ServerKind,
        plex::{self, auth},
    },
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

const ACCENT: Color32 = Color32::from_rgb(61, 155, 255);
const MUTED: Color32 = Color32::from_rgb(152, 152, 159);
const PANEL: Color32 = Color32::from_rgb(28, 28, 30);
const GREEN: Color32 = Color32::from_rgb(126, 226, 160);
const ERROR_TEXT: Color32 = Color32::from_rgb(255, 160, 150);
const TEXT: Color32 = Color32::from_rgb(235, 235, 245);
// Sidebar grid: every top-level row shares one icon column (20) and one text
// column (40) from the panel's 16-point edge. Library rows indent by one step,
// so their icons sit on the top-level text column.
const SIDEBAR: Color32 = Color32::from_rgb(20, 20, 23);
const SIDEBAR_EDGE: i8 = 16;
const SIDEBAR_TEXT_INSET: f32 = 40.0;
const SIDEBAR_INDENT: i8 = 20;
const SIDEBAR_NAV_MARGIN: egui::Margin = egui::Margin {
    left: SIDEBAR_INDENT,
    right: 0,
    top: 0,
    bottom: 0,
};
const SIDEBAR_ICON_INSET: f32 = 10.0;
/// Corner radius for buttons, inputs, menu rows and popovers: square with a
/// softened edge.
const EDGE: u8 = 2;
const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
// Button state fills from assets/buttons.
const CONTROL: Color32 = Color32::from_rgb(44, 44, 48);
const CONTROL_HOVER: Color32 = Color32::from_rgb(62, 62, 67);
const CONTROL_PRESSED: Color32 = Color32::from_rgb(31, 58, 92);
const FULLSCREEN_CONTROLS_SECONDS: f64 = 2.0;
const FULLSCREEN_REVEAL_SECONDS: f64 = 1.0;
// Allow for the native video's 200ms pointer sampling cadence.
const FULLSCREEN_MOTION_GAP_SECONDS: f64 = 0.35;
// 70% black behind the playback controls, eased out toward the picture.
const SCRIM: Color32 = Color32::from_black_alpha(179);
const SCRIM_FEATHER: f32 = 48.0;
const MIN_BROWSE_SIZE: Vec2 = Vec2::new(860.0, 620.0);
const MIN_PLAYBACK_SIZE: Vec2 = Vec2::new(240.0, 135.0);
const MINI_PLAYER_WIDTH: f32 = 540.0;
const MINI_PLAYER_HEIGHT: f32 = 380.0;
const RESIZE_BORDER: f32 = 6.0;
const RESIZE_CORNER: f32 = 12.0;
const VIDEO_WIDTH: (f32, f32) = (240.0, 320.0);
const BOOK_WIDTH: (f32, f32) = (176.0, 224.0);
const POSTER_WIDTH: (f32, f32) = (160.0, 210.0);
const ART_GAP: f32 = 16.0;
const COMPACT_HOME_HEIGHT: f32 = 740.0;

fn main() -> eframe::Result {
    // Before any other thread starts or any library loads.
    mpv::harden_dll_search();
    thread::spawn(|| {
        perfuse::trim_artwork_cache();
        player::trim_cache();
    });
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
                app.runtime.prompt = Some(RuntimePrompt::FirstRun {
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
    /// Visible items on earlier pages. Differs from `offset` when hidden items were skipped.
    shown: usize,
    previous_shown: Vec<usize>,
    library: bool,
    parent: Option<Item>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PageLoad {
    Navigate,
    Paginate,
    Refresh,
}

/// Replies to navigation requests. Sign-in replies use their own channel.
enum Reply {
    /// A page plus optional Home Continue Watching and Recently Released rows.
    Page {
        page: Page,
        home_rows: Option<(Vec<Item>, Vec<Item>)>,
        notice: Option<String>,
        /// A fresh first Home page, kept to show the next time this Home opens.
        snapshot: Option<metadata::Home>,
    },
    Details(Item),
    Failed(SourceError),
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

/// Playback engine, video surface and transport state shared by both players.
struct PlaybackState {
    player: Option<player::Player>,
    surface: Option<video::Surface>,
    /// Stopped players still saving progress, each with the video surface it renders into.
    retired: Vec<(player::Retired, Option<video::Surface>)>,
    playing_title: String,
    playing_seconds: f64,
    playing_duration: f64,
    buffered: f64,
    volume: prefs::Volume,
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
}

/// Managed playback runtime setup, inspection and upgrades.
struct RuntimeState {
    rx: Option<Receiver<runtime::Update>>,
    prompt: Option<RuntimePrompt>,
    status: Option<String>,
    failed: bool,
    action: RuntimeAction,
    installed: runtime::Installed,
    available: Option<runtime::Available>,
    result: Option<Result<String, String>>,
}

/// Perfuse app updates and their schedule.
#[derive(Default)]
struct UpdateState {
    client: updates::Updates,
    /// Whether this executable is the installed copy that an update replaces.
    self_install: bool,
    next_check: Option<Instant>,
    dismissed: Option<(Option<String>, Option<String>)>,
    after_runtime: Option<updates::Available>,
}

/// Servers and navigation. Sign-in, playback and runtime state live in their own groups,
/// and navigation replies use a channel that sign-in never shares.
struct App {
    settings_tab: SettingsTab,
    servers: Vec<Server>,
    active: Option<usize>,
    overview: bool,
    adding: bool,
    removing: Option<(usize, egui::Id)>,
    next_refresh: Instant,
    /// Saved servers still loading in the background.
    restoring: Option<Receiver<servers::Startup>>,
    /// Saved sign-ins that no longer open and can only be forgotten.
    unreadable: Vec<session::Unreadable>,
    background_tx: Sender<servers::Background>,
    background_rx: Receiver<servers::Background>,
    status_popover: servers::HoverPopover,
    sign_in: connection::SignIn,
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
    carousel_offsets: HashMap<&'static str, f32>,
    covers: Artwork,
    artwork_scheduler: artwork::Scheduler,
    screen_fade: screen_fade::ScreenFade,
    detail: Option<Item>,
    version: usize,
    resume: bool,
    busy: bool,
    /// A snapshot is on screen while its fresh replacement loads. Unlike `busy`,
    /// the user can keep browsing.
    refreshing: bool,
    error: Option<String>,
    notice: Option<String>,
    connection_open: bool,
    generation: Arc<AtomicU64>,
    tx: Sender<(u64, Reply)>,
    rx: Receiver<(u64, Reply)>,
    playback: PlaybackState,
    book: audiobooks::BookPlayer,
    runtime: RuntimeState,
    updates: UpdateState,
}

impl App {
    fn artwork_frame(&mut self, ctx: &egui::Context) {
        let sources = self
            .source
            .as_ref()
            .map(|source| (&self.covers, source))
            .into_iter()
            .chain(self.servers.iter().map(|s| (&s.artwork, &s.source)))
            .chain(self.book.artwork_source());
        self.artwork_scheduler.frame(ctx, sources);
    }

    fn new(ctx: &egui::Context) -> Self {
        icons::install(ctx);
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::BLACK;
        let popup = popover_frame();
        visuals.window_fill = popup.fill;
        visuals.window_stroke = popup.stroke;
        visuals.menu_corner_radius = popup.corner_radius;
        visuals.popup_shadow = popup.shadow;
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
            widget.corner_radius = egui::CornerRadius::same(EDGE);
            widget.bg_stroke = egui::Stroke::NONE;
        }
        ctx.set_visuals(visuals);
        ctx.style_mut(|style| {
            style.spacing.item_spacing = Vec2::new(14.0, 14.0);
            style.spacing.menu_margin = popup.inner_margin;
            style.spacing.button_padding = Vec2::new(16.0, 12.0);
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        });
        let (tx, rx) = mpsc::channel();
        let (background_tx, background_rx) = mpsc::channel();
        Self {
            settings_tab: SettingsTab::Servers,
            servers: vec![],
            active: None,
            overview: false,
            adding: false,
            removing: None,
            next_refresh: Instant::now() + servers::REFRESH_INTERVAL,
            restoring: None,
            unreadable: Vec::new(),
            background_tx,
            background_rx,
            status_popover: servers::HoverPopover::default(),
            sign_in: connection::SignIn::new(
                std::env::var("PLEX_URL").unwrap_or_else(|_| plex::PLUGIN.default_address.into()),
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
            artwork_scheduler: artwork::Scheduler::default(),
            screen_fade: screen_fade::ScreenFade::default(),
            detail: None,
            version: 0,
            resume: true,
            busy: false,
            refreshing: false,
            error: None,
            notice: None,
            connection_open: false,
            generation: Arc::new(AtomicU64::new(0)),
            tx,
            rx,
            playback: PlaybackState {
                player: None,
                surface: None,
                retired: Vec::new(),
                playing_title: String::new(),
                playing_seconds: 0.0,
                playing_duration: 0.0,
                buffered: 0.0,
                volume: prefs::Volume::default(),
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
            },
            book: Default::default(),
            runtime: RuntimeState {
                rx: None,
                prompt: None,
                status: None,
                failed: false,
                action: RuntimeAction::Setup,
                installed: runtime::Installed::default(),
                available: None,
                result: None,
            },
            updates: UpdateState {
                self_install: updates::self_install_supported(),
                ..UpdateState::default()
            },
        }
    }

    fn initialize(&mut self, ctx: &egui::Context, download_approved: bool) {
        if cfg!(all(windows, target_arch = "x86_64")) {
            self.updates.next_check = Some(Instant::now());
        }
        self.restore(ctx);
        if !runtime::needed() {
            self.runtime.action = RuntimeAction::Inspect;
            self.runtime.rx = Some(runtime::inspect(ctx.clone()));
        } else if download_approved {
            self.install_runtime(ctx);
        } else {
            self.runtime.prompt = Some(RuntimePrompt::Setup);
        }
    }

    fn request_runtime_setup(&mut self) {
        if self.runtime.rx.is_none() && runtime::needed() {
            self.runtime.prompt = Some(RuntimePrompt::Setup);
        }
    }

    fn runtime_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &self.runtime.prompt else {
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
            .frame(egui::Frame::popup(&ctx.style()).fill(PANEL).corner_radius(0).inner_margin(24))
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
                    let (accept, cancel) = Pill::pair(ui, Pill::primary(Icon::Check, confirm_label),
                        Pill::secondary(Icon::Cancel, if first_run { "Decline and exit" } else { "Cancel" }));
                    let confirm = ui.add(accept).clicked();
                    let cancel = ui.add(cancel).clicked();
                    (confirm, cancel)
                }).inner
            });
        let (confirm, cancel) = dialog.inner;
        if confirm
            && self.runtime.rx.is_none()
            && let Some(prompt) = self.runtime.prompt.take()
        {
            self.confirm_runtime_prompt(ctx, prompt);
        } else if cancel || (!first_run && dialog.should_close()) {
            self.runtime.prompt = None;
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
                    self.runtime.prompt = Some(RuntimePrompt::FirstRun {
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
                self.updates.after_runtime = client;
                self.runtime.action = RuntimeAction::Upgrade;
                self.runtime.failed = false;
                self.runtime.status = Some("Preparing library update…".into());
                self.runtime.rx = Some(runtime::upgrade(ctx.clone(), *libraries));
            }
        }
    }

    fn install_runtime(&mut self, ctx: &egui::Context) {
        self.runtime.action = RuntimeAction::Setup;
        self.runtime.failed = false;
        self.runtime.status = Some("Preparing playback libraries…".into());
        self.runtime.rx = Some(runtime::start(ctx.clone()));
    }

    fn begin(&mut self) -> u64 {
        self.busy = true;
        // A snapshot refresh still in flight belongs to the view being left.
        self.refreshing = false;
        self.error = None;
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Identify the page being drawn, independent of scroll, typing, artwork and refresh.
    fn browse_screen(&self, video_playing: bool) -> Option<egui::Id> {
        let source = self.source.as_ref()?;
        if video_playing || self.adding || self.removing.is_some() {
            return None;
        }
        let server = egui::Id::new(("browse", Arc::as_ptr(source) as *const ()));
        if self.book.full {
            return Some(egui::Id::new("book player"));
        }
        if self.connection_open {
            return Some(egui::Id::new(("settings", self.settings_tab)));
        }
        if self.overview {
            return Some(egui::Id::new("all servers home"));
        }
        if self.active.is_some_and(|i| !self.servers[i].online) {
            return None;
        }
        if let Some(item) = &self.detail {
            return Some(server.with(("detail", &item.id)));
        }
        Some(server.with((
            self.selected_library.as_deref(),
            self.location.as_ref().map(|location| {
                (
                    location.parent.as_ref().map(|item| &item.id),
                    &location.search,
                    location.offset,
                )
            }),
        )))
    }

    fn load_page(&mut self, ctx: &egui::Context, mode: PageLoad) {
        let (Some(source), Some(location)) = (self.source.clone(), self.location.clone()) else {
            return;
        };
        let generation = self.begin();
        let recently_added_only = mode == PageLoad::Paginate && self.selected_library.is_none();
        let keep_page = mode == PageLoad::Refresh && self.page.is_some();
        if keep_page {
            self.busy = false;
            self.refreshing = true;
        } else {
            self.page = None;
        }
        self.detail = None;
        if !keep_page && !recently_added_only {
            self.home_loaded = false;
            self.on_deck.clear();
            self.released.clear();
            self.carousel_offsets.clear();
        }
        let full_home = matches!(location.browse, Browse::RecentlyAdded)
            && !recently_added_only
            && location.offset == 0
            && location.search.is_empty();
        let server = self.active.and_then(|i| self.servers.get(i));
        let hidden = server.map(|s| s.prefs.hidden.clone()).unwrap_or_default();
        let store = server
            .filter(|s| full_home && s.remembered)
            .map(|s| s.store.clone());
        // Show the last Home seen for this server while the fresh one loads.
        if let Some(home) = server
            .filter(|_| full_home && !keep_page)
            .and_then(|s| s.home.clone())
        {
            self.busy = false;
            self.refreshing = true;
            self.home_loaded = true;
            let visible = |item: &Item| {
                !item
                    .library_id
                    .as_ref()
                    .is_some_and(|id| hidden.contains(id))
            };
            self.on_deck = home.on_deck.into_iter().filter(visible).collect();
            self.released = home.released.into_iter().filter(visible).collect();
            self.page = Some(home.page);
        }
        let sections = self.sections.clone();
        let continue_heading = self.continue_heading();
        let (tx, ctx, current) = (self.tx.clone(), ctx.clone(), self.generation.clone());
        tasks::spawn(move || {
            if current.load(Ordering::Relaxed) != generation {
                return;
            }
            let visible = |item: &Item| {
                !item
                    .library_id
                    .as_ref()
                    .is_some_and(|id| hidden.contains(id))
            };
            let browsed = media::browse_visible(
                source.as_ref(),
                &location.browse,
                &location.search,
                location.offset,
                &visible,
            );
            match browsed {
                Ok(page) => {
                    let mut unavailable = vec![];
                    let home_rows = if matches!(location.browse, Browse::RecentlyAdded)
                        && !recently_added_only
                    {
                        let on_deck = source
                            .browse(&Browse::ContinueWatching, "", 0)
                            .map(|page| page.items)
                            .unwrap_or_else(|_| {
                                unavailable.push(continue_heading);
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
                    let snapshot =
                        home_rows
                            .as_ref()
                            .filter(|_| full_home)
                            .map(|(on_deck, released)| metadata::Home {
                                page: page.clone(),
                                on_deck: on_deck.clone(),
                                released: released.clone(),
                            });
                    if let (Some(store), Some(home)) = (&store, &snapshot) {
                        store.save_home(home);
                    }
                    if tx
                        .send((
                            generation,
                            Reply::Page {
                                page,
                                home_rows,
                                notice,
                                snapshot,
                            },
                        ))
                        .is_err()
                    {
                        return;
                    }
                    ctx.request_repaint();
                }
                Err(error) => {
                    let _ = tx.send((generation, Reply::Failed(error)));
                    ctx.request_repaint();
                }
            }
        });
    }

    fn library(&mut self, section: &Library, ctx: &egui::Context) {
        self.book.full = false;
        self.selected_library = Some(section.id.clone());
        self.history.clear();
        self.search.clear();
        self.location = Some(Location {
            title: section.title.clone(),
            browse: Browse::Library(section.clone()),
            search: String::new(),
            offset: 0,
            previous_pages: vec![],
            shown: 0,
            previous_shown: vec![],
            library: true,
            parent: None,
        });
        self.load_page(ctx, PageLoad::Navigate);
    }

    fn open_item(&mut self, item: &Item, ctx: &egui::Context) {
        if item.playable() {
            let Some(source) = self.source.clone() else {
                return;
            };
            let generation = self.begin();
            let (tx, ctx, id, current) = (
                self.tx.clone(),
                ctx.clone(),
                item.id.clone(),
                self.generation.clone(),
            );
            tasks::spawn(move || {
                if current.load(Ordering::Relaxed) != generation {
                    return;
                }
                match source.item(&id) {
                    Ok(item) => {
                        let _ = tx.send((generation, Reply::Details(item)));
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
                shown: 0,
                previous_shown: vec![],
                library: false,
                parent: Some(item.clone()),
            });
            self.load_page(ctx, PageLoad::Navigate);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_servers(ctx);
        let mut runtime_finished = None;
        if let Some(rx) = &self.runtime.rx {
            while let Ok(update) = rx.try_recv() {
                match update {
                    runtime::Update::Status(message) => self.runtime.status = Some(message),
                    runtime::Update::Libraries(installed) => self.runtime.installed = installed,
                    runtime::Update::Available(available) => {
                        self.runtime.result = Some(Ok(match &available {
                            Some(available) => format!("Update available: {}", available.version),
                            None => "Libraries are up to date.".into(),
                        }));
                        self.runtime.available = available;
                    }
                    runtime::Update::Finished(result) => runtime_finished = Some(result),
                }
            }
        }
        if let Some(result) = runtime_finished {
            self.runtime.rx = None;
            self.runtime.failed = result.is_err();
            self.runtime.status = match result {
                Err(error) if self.runtime.action == RuntimeAction::Check => {
                    self.runtime.result = Some(Err(error));
                    None
                }
                result => result.err(),
            };
            if let Some(available) = self.updates.after_runtime.take()
                && !self.runtime.failed
            {
                self.updates.client.install(ctx, available);
            }
        }
        while let Ok((generation, reply)) = self.rx.try_recv() {
            if generation != self.generation.load(Ordering::Relaxed) {
                continue;
            }
            match reply {
                Reply::Page {
                    page,
                    home_rows,
                    notice,
                    snapshot,
                } => {
                    self.busy = false;
                    self.refreshing = false;
                    if let Some(home) = snapshot
                        && let Some(i) = self.active
                    {
                        self.servers[i].home = Some(home);
                    }
                    if let Some(location) = &mut self.location {
                        location.offset = page.offset;
                    }
                    let visible = |item: &Item| {
                        self.active
                            .is_none_or(|i| self.servers[i].item_visible(item))
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
                Reply::Failed(error) => {
                    self.busy = false;
                    self.refreshing = false;
                    // Retrying cannot fix a rejected sign-in, so show the way to sign in again.
                    if error.is_unauthorized()
                        && let Some(i) = self.active
                    {
                        self.servers[i].online = false;
                        self.servers[i].expired = true;
                    } else if self.location.is_none() && self.detail.is_none() {
                        // An item opened from combined Home has no collection underneath it.
                        self.overview = true;
                    }
                    self.error = Some(error.into());
                }
            }
        }
        self.poll_sign_in(ctx);
        self.poll_restore(ctx);
        self.poll_background();
        self.poll_retired(ctx);
        let mut finished = None;
        while let Some(event) = self
            .playback
            .player
            .as_ref()
            .and_then(|player| player.events.try_recv().ok())
        {
            match event {
                player::Event::Playing {
                    seconds,
                    duration,
                    buffered,
                    paused,
                    ..
                } => {
                    self.playback.playing_seconds = seconds;
                    self.playback.playing_duration = duration;
                    self.playback.buffered = buffered;
                    self.playback.paused = paused;
                    if !paused || seconds > 0.0 {
                        self.book.completed = false;
                    }
                }
                player::Event::BookCompleted => self.book_completed(ctx),
                player::Event::ChapterSleepExpired(end) => {
                    self.book_chapter_sleep_expired(ctx, end)
                }
                player::Event::Warning(message) => self.notice = Some(message),
                player::Event::Finished(result) => finished = Some(result),
            }
        }
        if let Some(result) = finished {
            // Finished is the worker's last act after releasing libmpv, so this join is
            // immediate. The surface then goes at once instead of covering the UI.
            self.playback.player = None;
            self.playback.surface = None;
            if self.book.is_loaded() {
                match result {
                    Ok(()) => self.book_ended(ctx),
                    Err(error) => {
                        self.book_finished();
                        self.error = Some(error);
                    }
                }
                return;
            }
            self.playback.overlay_focused = true;
            self.playback.overlay_hovered = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(MIN_BROWSE_SIZE));
            let size = ctx.content_rect().size();
            if size.x < MIN_BROWSE_SIZE.x || size.y < MIN_BROWSE_SIZE.y {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                    size.x.max(MIN_BROWSE_SIZE.x),
                    size.y.max(MIN_BROWSE_SIZE.y),
                )));
            }
            if self.playback.fullscreen {
                self.playback.fullscreen = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            match result {
                Ok(()) => {
                    // Re-fetch on the next opening to get the server's latest saved position.
                    self.detail = None;
                }
                Err(error) => self.error = Some(error),
            }
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
            shown: 0,
            previous_shown: vec![],
            library: false,
            parent: None,
        });
        self.load_page(ctx, PageLoad::Navigate);
    }

    fn back(&mut self, ctx: &egui::Context) {
        if self.busy && self.location.is_none() && !self.connection_open && !self.book.full {
            self.generation.fetch_add(1, Ordering::Relaxed);
            self.busy = false;
        }
        if self.book.full {
            self.book.full = false;
        } else if self.connection_open {
            self.connection_open = false;
        } else if self.detail.is_some() {
            self.detail = None;
            if self.location.is_none() {
                self.overview = true;
            }
        } else if let Some(location) = self.history.pop() {
            self.search = location.search.clone();
            self.location = Some(location);
            self.load_page(ctx, PageLoad::Navigate);
        } else if self.selected_library.is_some() {
            self.home(ctx);
        } else {
            self.overview = true;
        }
    }

    fn set_always_on_top(&mut self, ctx: &egui::Context, enabled: bool) {
        self.playback.always_on_top = enabled;
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
        if self.playback.player.is_none() || self.book.is_loaded() || self.playback.fullscreen {
            return false;
        }
        let size = ctx.content_rect().size();
        size.x < MINI_PLAYER_WIDTH || size.y < MINI_PLAYER_HEIGHT
    }

    fn page_previous(&mut self, ctx: &egui::Context) {
        if let Some(location) = &mut self.location {
            location.offset = location.previous_pages.pop().unwrap_or(0);
            location.shown = location.previous_shown.pop().unwrap_or(0);
        }
        self.load_page(ctx, PageLoad::Paginate);
    }

    fn page_next(&mut self, ctx: &egui::Context) {
        if let (Some(location), Some(page)) = (&mut self.location, &self.page)
            && let Some(next) = page.next
        {
            location.previous_pages.push(location.offset);
            location.previous_shown.push(location.shown);
            location.offset = next;
            location.shown += page.items.len();
        }
        self.load_page(ctx, PageLoad::Paginate);
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
                    if self.playback.player.is_none() || self.book.is_loaded() {
                        if ui
                            .add_enabled(
                                !self.busy
                                    && (self.book.full
                                        || self.detail.is_some()
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
                        && let Some(player) = &self.playback.player
                    {
                        player.stop();
                    }
                    ui.add(Icon::Mark.image(28.0, Color32::WHITE));
                    ui.label(
                        RichText::new("PERFUSE")
                            .size(26.0)
                            .strong()
                            .extra_letter_spacing(1.5)
                            .color(ACCENT),
                    );
                });
            });
        self.caption_buttons(ctx);
    }

    /// Always on top, minimize, maximize and close, flush with the window's
    /// top-right corner in the Windows 11 caption button style.
    fn caption_buttons(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
        egui::Area::new(egui::Id::new("caption_buttons"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::pos2(
                screen.right() - 4.0 * CaptionButton::WIDTH,
                screen.top(),
            ))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.horizontal(|ui| {
                    let pin_label = if self.playback.always_on_top {
                        "Disable always on top"
                    } else {
                        "Always on top"
                    };
                    if ui
                        .add(
                            CaptionButton::new(Caption::Pin, pin_label)
                                .active(self.playback.always_on_top),
                        )
                        .clicked()
                    {
                        self.set_always_on_top(ctx, !self.playback.always_on_top);
                    }
                    if ui
                        .add(CaptionButton::new(Caption::Minimize, "Minimize"))
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                    }
                    let (glyph, label) = if maximized {
                        (Caption::Restore, "Restore")
                    } else {
                        (Caption::Maximize, "Maximize")
                    };
                    if ui.add(CaptionButton::new(glyph, label)).clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    }
                    if ui
                        .add(CaptionButton::new(Caption::Close, "Close window"))
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(244.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(egui::Margin {
                left: SIDEBAR_EDGE,
                right: SIDEBAR_EDGE,
                top: 28,
                bottom: 28,
            }))
            .show(ctx, |ui| {
                // Keep every row's right edge clear of the floating scroll bar so
                // the group controls line up with the rows above and below.
                let scroll = &ui.spacing().scroll;
                let gutter = (scroll.bar_width + scroll.bar_outer_margin + 4.0).round() as i8;
                egui::Frame::new()
                    .inner_margin(egui::Margin {
                        right: gutter,
                        ..Default::default()
                    })
                    .show(ui, |ui| {
                        ui.set_min_height(ui.available_height());
                        ui.set_width(ui.available_width());
                        let row_spacing = ui.spacing().item_spacing.y;
                        ui.spacing_mut().item_spacing.y = 24.0;
                        if nav(
                            ui,
                            Icon::Home,
                            "Home",
                            self.overview && !self.connection_open && !self.book.full,
                        )
                        .clicked()
                            && !self.busy
                        {
                            self.book.full = false;
                            self.overview = true;
                            self.connection_open = false;
                            self.detail = None;
                        }
                        ui.spacing_mut().item_spacing.y = row_spacing;
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
                                self.connection_open && !self.book.full,
                            )
                            .clicked()
                            {
                                self.book.full = false;
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
            let show_sort = sections.len() > 1 && self.servers[i].online;
            self.group_heading(ui, i, ctx, show_sort);
            if show_sort {
                let sort_btn = ui.add(
                    IconButton::new(Icon::Sort, "Sort libraries")
                        .size(24.0)
                        .active(self.servers[i].prefs.sort != prefs::LibrarySort::Server),
                );
                egui::Popup::menu(&sort_btn)
                    .frame(popover_frame())
                    .show(|ui| {
                        for (mode, label) in [
                            (prefs::LibrarySort::Server, "Server order"),
                            (prefs::LibrarySort::NameAsc, "Name (A–Z)"),
                            (prefs::LibrarySort::NameDesc, "Name (Z–A)"),
                            (prefs::LibrarySort::TypeThenName, "Movies, then TV"),
                        ] {
                            if ui
                                .selectable_label(self.servers[i].prefs.sort == mode, label)
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
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
            }
            let server = &mut self.servers[i];
            let (icon, label) = if server.prefs.collapsed {
                (Icon::PageNext, "Expand libraries")
            } else {
                (Icon::ChevronDown, "Collapse libraries")
            };
            if ui.add(IconButton::new(icon, label).size(24.0)).clicked() {
                server.prefs.collapsed = !server.prefs.collapsed;
                server.persist();
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
            && !self.book.full
            && !self.overview
            && !self.connection_open
            && self.selected_library.is_none()
            && self.detail.is_none()
            && self
                .location
                .as_ref()
                .is_some_and(|location| matches!(location.browse, Browse::RecentlyAdded));
        egui::Frame::new()
            .inner_margin(SIDEBAR_NAV_MARGIN)
            .show(ui, |ui| {
                let rail = ui.painter().add(egui::Shape::Noop);
                let home = nav_item(ui, Icon::Home, "Home", server_home, false, false);
                let home_rect = home.rect;
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
                        && !self.book.full
                        && self.active == Some(i)
                        && !self.connection_open
                        && self.selected_library.as_ref() == Some(&section.id);
                    let icon = servers::library_icon(section);
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
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
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
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                pending_move = Some((index, index + 1));
                                ui.close();
                            }
                            ui.separator();
                            if ui
                                .button("Sort A–Z")
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                requested_sort = Some(prefs::LibrarySort::NameAsc);
                                ui.close();
                            }
                            if ui
                                .button("Sort Z–A")
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                requested_sort = Some(prefs::LibrarySort::NameDesc);
                                ui.close();
                            }
                            if ui
                                .button("Server order")
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                requested_sort = Some(prefs::LibrarySort::Server);
                                ui.close();
                            }
                        });
                    }
                }
                // A thin rail ties each server's library menu together.
                let last = row_rects
                    .last()
                    .map_or(home_rect.bottom(), |rect| rect.bottom());
                ui.painter().set(
                    rail,
                    egui::Shape::rect_filled(
                        egui::Rect::from_x_y_ranges(
                            home_rect.left()..=home_rect.left() + 1.0,
                            home_rect.top()..=last,
                        ),
                        0,
                        ACCENT.gamma_multiply(0.35),
                    ),
                );
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
            location.shown = 0;
            location.previous_shown.clear();
        }
        self.load_page(ctx, PageLoad::Navigate);
    }

    fn audiobook_server(&self) -> bool {
        self.active
            .is_some_and(|i| self.servers[i].session.kind.plugin().audio_only())
    }

    fn continue_heading(&self) -> &'static str {
        if self.audiobook_server() {
            "Continue Listening"
        } else {
            "Continue Watching"
        }
    }

    fn library_search_fields(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
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
        let spacing = ui.spacing().item_spacing.x;
        let title_space = 168.0 + spacing;
        // Preserve room for the title and field before adding button labels.
        let compact = !Pill::labels_fit(ui, shown, title_space + 168.0);
        let (search, clear) = (search.icon_only(compact), clear.icon_only(compact));
        let mut buttons = search.width(ui) + spacing;
        if clearing {
            buttons += clear.width(ui) + spacing;
        }
        let margin = Vec2::new(12.0, 10.0);
        let field_width =
            (ui.available_width() - title_space - buttons).clamp(2.0 * margin.x, 360.0);
        ui.allocate_ui_with_layout(
            Vec2::new(field_width + buttons, 40.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                let response = ui.add_enabled(
                    !self.busy,
                    egui::TextEdit::singleline(&mut self.search)
                        .hint_text("Search this library")
                        .desired_width(field_width - 2.0 * margin.x)
                        .margin(margin),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::TextEdit,
                        response.enabled(),
                        "Search library",
                    )
                });
                let submitted =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.add_enabled(!self.busy, search).clicked() || submitted {
                    self.search_library(ctx);
                } else if clearing && ui.add_enabled(!self.busy, clear).clicked() {
                    self.search.clear();
                    self.search_library(ctx);
                }
            },
        );
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
        let searchable = self.location.as_ref().is_some_and(|l| l.library);
        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), 40.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                if ui
                    .add_enabled(
                        !self.busy && !self.refreshing,
                        IconButton::new(Icon::Refresh, "Refresh"),
                    )
                    .clicked()
                {
                    // Also refresh the server's libraries and Continue row now, rather
                    // than at the next background refresh.
                    if let Some(i) = self.active {
                        self.servers[i].check(ctx);
                    }
                    self.load_page(ctx, PageLoad::Refresh);
                }
                if self.busy || self.refreshing {
                    ui.spinner();
                }
                if searchable {
                    self.library_search_fields(ui, ctx);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&title).size(30.0).strong()).truncate())
                        .on_hover_text(&title);
                });
            },
        );
        if let Some(i) = self.active {
            let server = &self.servers[i];
            let kind = server.session.kind.plugin().name;
            ui.horizontal(|ui| {
                ui.add(server.icon().image(15.0, MUTED));
                ui.colored_label(
                    MUTED,
                    if home {
                        let count = self.sections.len();
                        let noun = if count == 1 { "library" } else { "libraries" };
                        format!("{kind} server · {count} {noun}")
                    } else {
                        format!("{kind} / {}", server.name())
                    },
                );
            });
        }
        ui.add_space(34.0);
        if self.sections.is_empty() {
            let nouns = self.active.map_or_else(
                || "movie or TV".into(),
                |i| self.servers[i].session.kind.plugin().library_nouns(),
            );
            ui.heading(format!("No {nouns} libraries available"));
            ui.label("Check this user's library access on the server, then reconnect in settings.");
            return;
        }
        let mut picked = None;
        let mut page_action: i8 = 0;
        let show_home_rows = home
            && (self.home_loaded
                || self.page.is_some()
                || !self.on_deck.is_empty()
                || !self.released.is_empty());
        let home_spacing =
            (ctx.content_rect().height() - MIN_BROWSE_SIZE.y + 32.0).clamp(32.0, 48.0);
        let audiobook_server = self.audiobook_server();
        let continue_heading = self.continue_heading();
        let book_status = |item: &Item| {
            self.source
                .as_ref()
                .filter(|source| self.book.is_current(source, &item.id))
                .map(|_| BookCardPlayback {
                    paused: self.playback.paused,
                    elapsed_ms: (self.playback.playing_seconds * 1000.0) as u64,
                    finished: self.book.completed,
                })
        };
        egui::ScrollArea::vertical()
            .id_salt(("collection", &title))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(parent) = self.location.as_ref().and_then(|l| l.parent.as_ref()) {
                    hero(ui, parent, &self.covers, "TV LIBRARY");
                    ui.add_space(38.0);
                }
                if show_home_rows && (!audiobook_server || !self.on_deck.is_empty()) {
                    if self.on_deck.is_empty() {
                        ui.label(RichText::new(continue_heading).size(22.0).strong());
                        ui.label(
                            RichText::new(
                                "Nothing to continue. Start a film or episode from your library.",
                            )
                            .color(MUTED),
                        );
                    } else {
                        let shelf = if audiobook_server {
                            Shelf::Books { count: None }
                        } else {
                            Shelf::Video {
                                rows: 1,
                                source: false,
                            }
                        };
                        home_carousel(
                            ui,
                            ("continue", continue_heading),
                            self.carousel_offsets.entry("continue").or_default(),
                            &self.on_deck,
                            shelf,
                            |item| egui::Id::new((self.active, &item.id)),
                            |ui, item, width| {
                                let clicked = if audiobook_server {
                                    book_card(ui, item, &self.covers, width, book_status(item))
                                } else {
                                    continue_card(ui, item, &self.covers, width)
                                };
                                if clicked && !self.busy {
                                    picked = Some(item.clone());
                                }
                            },
                        );
                    }
                    ui.add_space(home_spacing);
                    if !self.released.is_empty() {
                        home_carousel(
                            ui,
                            ("released", "Recently Released"),
                            self.carousel_offsets.entry("released").or_default(),
                            &self.released,
                            Shelf::Video {
                                rows: 1,
                                source: false,
                            },
                            |item| egui::Id::new((self.active, &item.id)),
                            |ui, item, width| {
                                if released_card(ui, item, &self.covers, width) && !self.busy {
                                    picked = Some(item.clone());
                                }
                            },
                        );
                        ui.add_space(home_spacing);
                    }
                }
                if let Some(page) = &self.page {
                    let shown = self.location.as_ref().map_or(page.offset, |l| l.shown);
                    let has_previous = self
                        .location
                        .as_ref()
                        .is_some_and(|l| !l.previous_pages.is_empty());
                    let has_next = page.has_next();
                    let items: Vec<_> = page
                        .items
                        .iter()
                        .filter(|item| {
                            self.active
                                .is_none_or(|i| self.servers[i].item_visible(item))
                        })
                        .collect();
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
                                    shown + 1,
                                    shown + page.items.len(),
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
                        if !home && (has_previous || has_next) {
                            // Library pages turn from the right of the heading row.
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add_enabled(
                                            !self.busy && has_next,
                                            IconButton::new(Icon::PageNext, "Next page"),
                                        )
                                        .clicked()
                                    {
                                        page_action = 1;
                                    }
                                    if ui
                                        .add_enabled(
                                            !self.busy && has_previous,
                                            IconButton::new(Icon::PagePrevious, "Previous page"),
                                        )
                                        .clicked()
                                    {
                                        page_action = -1;
                                    }
                                },
                            );
                        }
                    });
                    ui.add_space(14.0);
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
                    let grid_id =
                        egui::Id::new(("library cards", self.active, &title, page.offset));
                    library_cards(ui, grid_id, &items, episodes, |ui, item, width| {
                        if episodes {
                            egui::Frame::new()
                                .fill(PANEL)
                                .corner_radius(0)
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
                                                .on_hover_cursor(egui::CursorIcon::PointingHand)
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
                        } else {
                            let clicked = if item.kind == Kind::Audiobook {
                                book_card(ui, item, &self.covers, width, book_status(item))
                            } else {
                                poster(ui, item, &self.covers, width)
                            };
                            if clicked && !self.busy {
                                picked = Some(item.clone());
                            }
                        }
                    });
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
        let Some(item) = self.detail.take() else {
            return false;
        };
        if item.kind == Kind::Audiobook {
            let play = self.book_details(ui, &item);
            self.detail = Some(item);
            return play;
        }
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
                    .corner_radius(0)
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
        self.detail = Some(item);
        play
    }

    /// Stops the current player without waiting for it. Its video surface stays alive
    /// until the worker has saved progress and released libmpv.
    fn retire_player(&mut self) {
        let surface = self.playback.surface.take();
        if let Some(player) = self.playback.player.take() {
            self.playback.retired.push((player.retire(), surface));
        }
    }

    fn poll_retired(&mut self, ctx: &egui::Context) {
        // Each entry drops its finished worker before the surface that worker rendered to.
        self.playback
            .retired
            .retain(|(retired, _)| !retired.is_finished());
        if !self.playback.retired.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn start_playback(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        if runtime::needed() {
            self.request_runtime_setup();
            return;
        }
        if self.runtime.rx.is_some() {
            return;
        }
        let (Some(mut item), Some(source)) = (self.detail.clone(), self.source.clone()) else {
            return;
        };
        if self.playback.player.is_some() && !self.book.is_loaded() {
            return;
        }
        let audiobook = item.kind == Kind::Audiobook;
        let surface = if audiobook {
            if let Some(ms) = self.book.start_at_ms.take() {
                item.resume_ms = Some(ms);
                self.resume = true;
            }
            None
        } else {
            match video::Surface::new(frame) {
                Ok(surface) => Some(surface),
                Err(error) => {
                    self.error = Some(error);
                    return;
                }
            }
        };
        if self.book.is_loaded() {
            // The previous worker still saves its final progress to the original source.
            self.retire_player();
            self.book_finished();
        }
        self.playback.playing_title = item.title.clone();
        self.playback.playing_format = item
            .versions
            .get(self.version)
            .map(|m| m.label())
            .unwrap_or_default();
        self.playback.playing_seconds = if self.resume {
            item.resume_ms.unwrap_or(0) as f64 / 1000.0
        } else {
            0.0
        };
        self.playback.paused = false;
        self.playback.speed = 1.0;
        self.playback.playing_duration = item.duration_ms.unwrap_or(0) as f64 / 1000.0;
        self.playback.buffered = 0.0;
        self.playback.seek_position = None;
        self.notice = None;
        self.error = None;
        self.playback.overlay_focused = true;
        self.playback.overlay_hovered = false;
        if audiobook {
            self.book_started(&item, &source);
        }
        let player = player::Player::start(
            surface.as_ref().map(video::Surface::id),
            source,
            item,
            player::StartOptions {
                version: self.version,
                resume: self.resume,
                volume: self.playback.volume.gain(),
            },
        );
        self.playback.player = Some(player);
        self.playback.surface = surface;
        self.connection_open = false;
        if !audiobook {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(MIN_PLAYBACK_SIZE));
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let shown = self.settings_tab;
        ui.set_max_width(ui.available_width().min(980.0));
        ui.label(RichText::new("Settings").size(34.0).strong());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (tab, icon, text) in [
                (SettingsTab::Servers, Icon::Server, "Servers"),
                (SettingsTab::Playback, Icon::Play, "Playback"),
                (SettingsTab::Updates, Icon::Refresh, "Updates & about"),
            ] {
                let active = shown == tab;
                let color = if active { TEXT } else { MUTED };
                let button = egui::Button::image_and_text(
                    icon.image(16.0, if active { ACCENT } else { MUTED }),
                    RichText::new(text).size(15.0).color(color),
                )
                .min_size(Vec2::new(154.0, 38.0))
                .frame(false);
                let response = ui
                    .add(button)
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
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
                    ctx.request_repaint();
                }
            }
        });
        ui.separator();
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt(("settings", shown))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if shown == SettingsTab::Servers {
                    self.server_settings(ui, ctx);
                    return;
                }
                if shown == SettingsTab::Playback {
                    ui.label(RichText::new("Playback configuration").size(23.0).strong());
                    egui::Frame::new()
                        .fill(PANEL)
                        .corner_radius(0)
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
                    match &self.updates.client.status {
                        Some(Ok(message)) => {
                            let color = if self.updates.client.busy() {
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
                    let (check, update) = Pill::pair(
                        ui,
                        Pill::secondary(Icon::Refresh, "Check for Perfuse updates"),
                        Pill::secondary(
                            Icon::Refresh,
                            if self.updates.self_install {
                                "Update Perfuse"
                            } else {
                                "Download Perfuse update"
                            },
                        ),
                    );
                    if ui
                        .add_enabled(
                            cfg!(all(windows, target_arch = "x86_64"))
                                && !self.updates.client.busy()
                                && self.updates.after_runtime.is_none(),
                            check,
                        )
                        .clicked()
                    {
                        self.updates.dismissed = None;
                        self.updates.client.check(ctx);
                    }
                    if self.updates.client.available.is_some()
                        && ui
                            .add_enabled(
                                self.runtime.rx.is_none() && !self.updates.client.busy(),
                                update,
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
        egui::Frame::new().fill(PANEL).corner_radius(0).inner_margin(28).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if self.runtime.installed.libraries.is_empty() {
                ui.label(if self.runtime.rx.is_some() { "Reading installed libraries…" } else { "No playback libraries found." });
            } else {
                egui::Grid::new("third_party_libraries").num_columns(2).striped(true)
                    .max_col_width((ui.available_width() * 0.55).max(80.0)).show(ui, |ui| {
                        ui.strong("Name"); ui.strong("Version"); ui.end_row();
                        for library in &self.runtime.installed.libraries {
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
                match &self.runtime.result {
                    Some(Ok(message)) => { ui.colored_label(GREEN, message); }
                    Some(Err(error)) => { ui.colored_label(ERROR_TEXT, error); }
                    None if self.runtime.action == RuntimeAction::Check && self.runtime.rx.is_some() => {
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
            let idle = self.runtime.rx.is_none() && self.playback.player.is_none();
            ui.horizontal_wrapped(|ui| {
                let (check, update) = Pill::pair(ui, Pill::secondary(Icon::Refresh, "Check for Updates"), Pill::secondary(Icon::Refresh, "Update"));
                if ui.add_enabled(idle && cfg!(windows), check).clicked() {
                    self.updates.dismissed = None;
                    self.check_runtime(ctx);
                }
                if ui.add_enabled(idle && !self.updates.client.downloading && cfg!(windows) && !overridden && self.runtime.available.is_some(), update).clicked() {
                    self.update_available(ctx, false);
                }
            });
        });
    }

    fn check_runtime(&mut self, ctx: &egui::Context) {
        self.runtime.action = RuntimeAction::Check;
        self.runtime.result = None;
        self.runtime.status = None;
        self.runtime.failed = false;
        self.runtime.rx = Some(runtime::check(ctx.clone()));
    }

    fn update_available(&mut self, ctx: &egui::Context, include_client: bool) {
        let mut client = self
            .updates
            .client
            .available
            .clone()
            .filter(|_| include_client);
        // A development build or portable copy cannot replace itself. Offer the download.
        if !self.updates.self_install
            && let Some(available) = client.take()
        {
            ctx.open_url(egui::OpenUrl::new_tab(available.release_page()));
        }
        if let Some(libraries) = self.runtime.available.clone()
            && std::env::var_os("PERFUSE_LIBMPV").is_none()
        {
            self.runtime.prompt = Some(RuntimePrompt::Upgrade {
                libraries: Box::new(libraries),
                client,
            });
        } else if let Some(available) = client {
            self.updates.client.install(ctx, available);
        }
    }

    fn poll_updates(&mut self, ctx: &egui::Context) {
        if self.updates.client.poll() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if let Some(next) = self.updates.next_check {
            let now = Instant::now();
            if now < next {
                ctx.request_repaint_after(next - now);
            } else if self.runtime.prompt.is_none()
                && self.runtime.rx.is_none()
                && !self.updates.client.busy()
            {
                self.updates.next_check = Some(now + UPDATE_INTERVAL);
                ctx.request_repaint_after(UPDATE_INTERVAL);
                self.updates.client.check(ctx);
                if std::env::var_os("PERFUSE_LIBMPV").is_none() && !runtime::needed() {
                    self.check_runtime(ctx);
                }
            }
        }
    }

    fn update_notification(&mut self, ctx: &egui::Context, offset: f32) {
        let client = self
            .updates
            .client
            .available
            .as_ref()
            .map(|update| update.version.clone());
        let libraries = self
            .runtime
            .available
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
        if self.updates.dismissed.as_ref() == Some(&key) {
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
                        if self.updates.client.downloading {
                            if let Some(Ok(status)) = &self.updates.client.status {
                                ui.label(status);
                            }
                        } else if self.runtime.action == RuntimeAction::Upgrade
                            && let Some(status) = &self.runtime.status
                        {
                            let color = if self.runtime.failed {
                                ERROR_TEXT
                            } else {
                                TEXT
                            };
                            ui.colored_label(color, status);
                        } else if key.0.is_some()
                            && let Some(Err(error)) = &self.updates.client.status
                        {
                            ui.colored_label(ERROR_TEXT, error);
                        }
                        ui.horizontal(|ui| {
                            let action = match (&key.0, &key.1, self.updates.self_install) {
                                (Some(_), Some(_), true) => "Update both and restart",
                                (Some(_), Some(_), false) => {
                                    "Update libraries and download Perfuse"
                                }
                                (Some(_), None, true) => "Update Perfuse and restart",
                                (Some(_), None, false) => "Download Perfuse",
                                _ => "Update libraries",
                            };
                            let idle = self.runtime.rx.is_none() && !self.updates.client.busy();
                            if ui.add_enabled(idle, egui::Button::new(action)).clicked() {
                                // The root context owns shutdown even when the notification
                                // is drawn in the native playback overlay.
                                self.update_available(ctx, key.0.is_some());
                            }
                            if ui.button("Later").clicked() {
                                self.updates.dismissed = Some(key.clone());
                            }
                        });
                    });
            });
    }

    fn playback_controls_visible(&mut self, ctx: &egui::Context) -> bool {
        // The embedded Win32 child receives mouse movement instead of egui.
        let pointer = self
            .playback
            .surface
            .as_ref()
            .and_then(video::Surface::pointer_position);
        let native_moved = pointer.is_some() && pointer != self.playback.video_pointer;
        self.playback.video_pointer = pointer;
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
        if !self.playback.fullscreen {
            self.playback.last_playback_interaction = now;
            self.playback.reveal_motion = None;
            // Unfocused always-on-top windows hide controls until hovered so
            // picture-in-picture stays unobstructed while using other apps.
            if self.playback.surface.is_some()
                && self.playback.always_on_top
                && !self.playback.overlay_focused
            {
                return self.playback.overlay_hovered
                    || active
                    || egui::Popup::is_any_open(ctx)
                    || self.runtime.prompt.is_some();
            }
            return true;
        }
        let visible = now - self.playback.last_playback_interaction < FULLSCREEN_CONTROLS_SECONDS;
        let reveal = if visible {
            self.playback.reveal_motion = None;
            moved
        } else if moved {
            let start = match self.playback.reveal_motion {
                Some((start, last)) if now - last <= FULLSCREEN_MOTION_GAP_SECONDS => start,
                _ => now,
            };
            self.playback.reveal_motion = Some((start, now));
            now - start >= FULLSCREEN_REVEAL_SECONDS
        } else {
            if self
                .playback
                .reveal_motion
                .is_some_and(|(_, last)| now - last > FULLSCREEN_MOTION_GAP_SECONDS)
            {
                self.playback.reveal_motion = None;
            }
            false
        };
        if reveal || active || egui::Popup::is_any_open(ctx) || self.runtime.prompt.is_some() {
            self.playback.last_playback_interaction = now;
            self.playback.reveal_motion = None;
        }
        now - self.playback.last_playback_interaction < FULLSCREEN_CONTROLS_SECONDS
    }

    fn playback_controls(&mut self, ctx: &egui::Context, mini: bool) {
        let controls_visible = self.playback_controls_visible(ctx);
        self.playback.controls_opacity = ctx.animate_value_with_time(
            egui::Id::new("playback_controls_fade"),
            if controls_visible { 1.0 } else { 0.0 },
            if controls_visible { 0.3 } else { 0.375 },
        );
        let opacity = self.playback.controls_opacity;
        let bounds = ctx.available_rect();
        if self.playback.surface.is_none() {
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
                            .playback
                            .surface
                            .as_ref()
                            .map(|s| s.prepare_overlay(&title))
                            .unwrap_or_default();
                        self.playback.overlay_focused = state.focused;
                        self.playback.overlay_hovered = state.hovered;
                        // Keep the transparent overlay mounted while always-on-top
                        // (windowed) so hovering PiP never steals focus via ShowWindow.
                        let keep_pip_overlay = state.topmost && !self.playback.fullscreen;
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
                    if class != egui::ViewportClass::Embedded && self.runtime.prompt.is_none() {
                        self.playback_shortcuts(overlay);
                    }
                },
            );
        }
        if self.runtime.prompt.is_none() {
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
        let Some(player) = &self.playback.player else {
            return;
        };
        if self.playback.surface.is_some() {
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
                        let (icon, label) = if self.playback.paused {
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
                    // The scrim is this layer's first shape: above the video, below the controls.
                    let scrim = ui.painter().add(egui::Shape::Noop);
                    let header = egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(56, 28))
                        .show(ui, |ui| {
                            // Reserve the fullscreen button before the title takes the remaining width.
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let button = if self.playback.fullscreen {
                                        IconButton::new(Icon::FullscreenExit, "Exit fullscreen")
                                    } else {
                                        IconButton::new(Icon::Fullscreen, "Fullscreen")
                                    };
                                    if ui.add(button.size(40.0)).clicked() {
                                        self.playback.fullscreen = !self.playback.fullscreen;
                                        ctx.send_viewport_cmd_to(
                                            egui::ViewportId::ROOT,
                                            egui::ViewportCommand::Fullscreen(
                                                self.playback.fullscreen,
                                            ),
                                        );
                                    }
                                    ui.with_layout(
                                        egui::Layout::top_down(egui::Align::Min),
                                        |ui| {
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(&self.playback.playing_title)
                                                        .size(20.0)
                                                        .strong(),
                                                )
                                                .truncate(),
                                            );
                                            ui.label(
                                                RichText::new(if self.playback.paused {
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
                        })
                        .response
                        .rect;
                    ui.painter().set(
                        scrim,
                        controls_scrim(
                            egui::Rect::from_x_y_ranges(
                                bounds.x_range(),
                                bounds.top()..=header.bottom(),
                            ),
                            header.bottom() + SCRIM_FEATHER,
                        ),
                    );
                });
            egui::Area::new(egui::Id::new("transport"))
                .order(egui::Order::Foreground)
                .movable(false)
                .pivot(egui::Align2::LEFT_BOTTOM)
                .fixed_pos(bounds.left_bottom())
                .show(ctx, |ui| {
                    ui.set_width(bounds.width());
                    ui.set_opacity(opacity);
                    let scrim = ui.painter().add(egui::Shape::Noop);
                    let transport = egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(56, 32))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing = Vec2::new(14.0, 16.0);
                            control_row(ui, 24.0, |ui| {
                                let position = self
                                    .playback
                                    .seek_position
                                    .unwrap_or(self.playback.playing_seconds);
                                ui.label(RichText::new(clock(position)).size(13.0));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "−{}",
                                                clock(
                                                    (self.playback.playing_duration
                                                        - self.playback.playing_seconds)
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
                                            self.playback.playing_duration,
                                            Some(
                                                self.playback.playing_seconds
                                                    + self.playback.buffered,
                                            ),
                                        );
                                        if seek.dragged() {
                                            self.playback.seek_position = Some(position);
                                        }
                                        if seek.drag_stopped()
                                            || (seek.changed() && !seek.dragged())
                                        {
                                            player.seek(position);
                                            self.playback.seek_position = None;
                                        }
                                    },
                                );
                            });
                            control_row(ui, 44.0, |ui| {
                                if ui
                                    .add(IconButton::new(Icon::Back10, "Back 10s").size(44.0))
                                    .clicked()
                                {
                                    player.seek(self.playback.playing_seconds - 10.0);
                                }
                                let button = if self.playback.paused {
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
                                    player.seek(self.playback.playing_seconds + 10.0);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "Buffered {} / 5:00",
                                                clock(self.playback.buffered)
                                            ))
                                            .size(12.0)
                                            .color(MUTED),
                                        );
                                        badge(ui, Icon::StatusConnected, "Direct Play", GREEN);
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(&self.playback.playing_format)
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
                                    .add(Pill::secondary(
                                        Icon::Speed,
                                        &format!("{}×", self.playback.speed),
                                    ))
                                    .on_hover_text("Playback speed");
                                egui::Popup::menu(&speed).frame(popover_frame()).show(|ui| {
                                    for speed in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0] {
                                        if ui
                                            .selectable_value(
                                                &mut self.playback.speed,
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
                                        let mut gain = self.playback.volume.gain();
                                        let slider = ui.add(
                                            egui::Slider::new(&mut gain, 0.0..=100.0)
                                                .show_value(false)
                                                .trailing_fill(true),
                                        );
                                        let icon = if gain <= 0.0 {
                                            Icon::VolumeMuted
                                        } else if gain < 50.0 {
                                            Icon::VolumeLow
                                        } else {
                                            Icon::VolumeHigh
                                        };
                                        let label =
                                            ui.add(icon.image(20.0, TEXT).alt_text("Volume"));
                                        let slider = slider
                                            .labelled_by(label.id)
                                            .on_hover_text(format!("Volume {gain:.0}%"));
                                        if slider.changed() && self.playback.volume.set(gain) {
                                            player.volume(gain);
                                        }
                                        if !slider.dragged() {
                                            self.playback.volume.save();
                                        }
                                    },
                                );
                            });
                        })
                        .response
                        .rect;
                    ui.painter().set(
                        scrim,
                        controls_scrim(
                            egui::Rect::from_x_y_ranges(
                                bounds.x_range(),
                                transport.top()..=bounds.bottom(),
                            ),
                            transport.top() - SCRIM_FEATHER,
                        ),
                    );
                });
        }
    }

    fn playback_shortcuts(&mut self, ctx: &egui::Context) {
        let Some(player) = &self.playback.player else {
            return;
        };
        // Keep shortcuts active even while the fullscreen panels are hidden.
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) {
            self.playback.fullscreen = !self.playback.fullscreen;
            ctx.send_viewport_cmd_to(
                egui::ViewportId::ROOT,
                egui::ViewportCommand::Fullscreen(self.playback.fullscreen),
            );
        }
        if self.playback.fullscreen && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.playback.fullscreen = false;
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
                    player.seek(self.playback.playing_seconds - 10.0);
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    player.seek(self.playback.playing_seconds + 10.0);
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
        if self.playback.fullscreen || ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
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
    fn on_exit(&mut self, _: Option<&eframe::glow::Context>) {
        self.playback.volume.save();
        // Flush progress and join each worker before destroying its video surface. A worker
        // still waiting on the network after the deadline ends with the process, and its
        // surface is left to Windows rather than destroyed underneath libmpv.
        self.retire_player();
        let deadline = Instant::now() + Duration::from_secs(8);
        for (retired, surface) in self.playback.retired.drain(..) {
            if retired.wait_until(deadline) {
                drop(retired);
                drop(surface);
            } else {
                retired.detach();
                std::mem::forget(surface);
            }
        }
        prefs::flush();
    }

    fn clear_color(&self, _: &egui::Visuals) -> [f32; 4] {
        // Root window gaps/margins must stay opaque. eframe clears immediate
        // viewports (the playback overlay) separately to transparent RGBA.
        [0.0, 0.0, 0.0, 1.0]
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.book_tick(ctx);
        self.poll_updates(ctx);
        let video_playing = self.playback.player.is_some() && !self.book.is_loaded();
        if self.playback.player.is_some()
            || self.busy
            || self.sign_in.busy
            || self.sign_in.login.is_some()
            || self.runtime.rx.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        let mini = self.is_mini_player(ctx);
        if !self.playback.fullscreen && !mini {
            self.titlebar(ctx);
        }
        if self.source.is_some() && !video_playing {
            self.sidebar(ctx);
        }
        if self.book.is_loaded() && !self.book.full {
            self.book_dock(ctx);
        }
        let show_global_error = self.source.is_some() && !self.adding;
        if !mini
            && ((show_global_error && self.error.is_some())
                || self.notice.is_some()
                || !self.unreadable.is_empty()
                || self.runtime.status.is_some())
        {
            egui::TopBottomPanel::bottom("messages")
                .frame(
                    egui::Frame::new()
                        .fill(PANEL)
                        .inner_margin(egui::Margin::same(28)),
                )
                .show(ctx, |ui| {
                    if let Some(status) = self.runtime.status.clone() {
                        ui.horizontal_wrapped(|ui| {
                            if self.runtime.rx.is_some() {
                                ui.spinner();
                            }
                            ui.label(status);
                            if self.runtime.failed
                                && self.runtime.action == RuntimeAction::Setup
                                && ui.button("Retry playback setup").clicked()
                            {
                                self.request_runtime_setup();
                            }
                        });
                    }
                    if show_global_error && let Some(error) = self.error.clone() {
                        ui.horizontal_wrapped(|ui| {
                            ui.add(Icon::Alert.image(18.0, ERROR_TEXT));
                            ui.colored_label(ERROR_TEXT, error);
                            if ui.add(IconButton::new(Icon::Dismiss, "Dismiss")).clicked() {
                                self.error = None;
                            }
                        });
                    }
                    if !self.unreadable.is_empty() {
                        let labels: Vec<_> =
                            self.unreadable.iter().map(session::Unreadable::label).collect();
                        let message = match labels.len() {
                            1 => format!("Perfuse could not open the saved sign-in for {}. Forget it, then sign in again.", labels[0]),
                            _ => format!("Perfuse could not open {} saved sign-ins: {}. Forget them, then sign in again.", labels.len(), labels.join(", ")),
                        };
                        ui.horizontal_wrapped(|ui| {
                            ui.add(Icon::Alert.image(18.0, ERROR_TEXT));
                            ui.colored_label(ERROR_TEXT, message);
                            if ui.add(Pill::secondary(Icon::Forget, "Forget")).clicked() {
                                self.forget_unreadable();
                            }
                            if ui.add(IconButton::new(Icon::Dismiss, "Dismiss")).clicked() {
                                self.unreadable.clear();
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
        if video_playing {
            if let Some(surface) = &self.playback.surface {
                surface.place(ctx.available_rect(), ctx.pixels_per_point());
                surface.reclaim_keyboard();
            }
            self.playback_controls(ctx, mini);
        }
        let mut play = false;
        let pane = ctx.available_rect();
        let screen = self.browse_screen(video_playing);
        if screen.is_none() {
            self.screen_fade = Default::default();
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(if self.source.is_some() {
                        Color32::from_rgb(13, 13, 16)
                    } else {
                        Color32::BLACK
                    })
                    .inner_margin(if video_playing {
                        egui::Margin::ZERO
                    } else if self.book.full
                        || self
                            .detail
                            .as_ref()
                            .is_some_and(|item| item.kind == Kind::Audiobook)
                    {
                        let padding = if ctx.content_rect().width() <= 1000.0 {
                            32
                        } else {
                            48
                        };
                        egui::Margin {
                            bottom: if self.book.full { 0 } else { padding },
                            ..egui::Margin::same(padding)
                        }
                    } else if self.source.is_some() {
                        let mut margin = egui::Margin::symmetric(48, 38);
                        if self.overview
                            && !self.connection_open
                            && ctx.content_rect().height() <= COMPACT_HOME_HEIGHT
                        {
                            margin.top = 24;
                        }
                        margin
                    } else {
                        egui::Margin::same(20)
                    }),
            )
            .show(ctx, |ui| {
                let waiting = self.busy
                    && self.page.is_none()
                    && self.detail.is_none()
                    && !self.overview
                    && !self.connection_open
                    && !self.book.full;
                if waiting && screen.is_some() {
                    if self.screen_fade.hold(ui, pane) {
                        return;
                    }
                    self.screen_fade = Default::default();
                }
                let capture = screen
                    .filter(|_| !waiting)
                    .map(|key| (key, screen_fade::ScreenFade::first_shape(ui)));
                if video_playing {
                    let rect = ui.available_rect_before_wrap();
                    ui.painter().rect_filled(rect, 0.0, Color32::BLACK);
                } else if self.book.full {
                    self.book_full_player(ui);
                } else if self.source.is_none() && self.restoring.is_some() {
                    // Saved servers appear in a moment. Avoid flashing the welcome screen.
                    ui.centered_and_justified(|ui| ui.spinner());
                } else if self.source.is_none() {
                    self.onboarding(ui, ctx);
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
                if let Some((key, first)) = capture {
                    self.screen_fade.present(ui, pane, key, first);
                }
            });
        if play {
            self.start_playback(ctx, frame);
        }
        self.show_book_toast(ctx);
        if self.playback.surface.is_none() {
            if !mini {
                self.update_notification(ctx, 58.0);
            }
            self.server_dialogs(ctx);
            self.runtime_dialog(ctx);
        }
        self.handle_window_resize(ctx, egui::ViewportId::ROOT, true);
        self.artwork_frame(ctx);
    }
}

fn nav(ui: &mut egui::Ui, icon: Icon, title: &str, selected: bool) -> egui::Response {
    nav_item(ui, icon, title, selected, false, false)
}

/// A square-edged sidebar row, marked in ACCENT when selected.
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
    let color = ACCENT;
    if selected || dragging || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            EDGE,
            SIDEBAR.lerp_to_gamma(
                color,
                if dragging {
                    0.26
                } else if selected {
                    0.18
                } else {
                    0.07
                },
            ),
        );
    }
    let hovered = response.hovered() || response.has_focus();
    if selected || dragging || hovered {
        // Selection is marked by shape as well as colour. Hover keeps a softer
        // edge so the blue line on the left never disappears under the fill.
        let marker = egui::Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height()));
        let strength = if selected || dragging { 1.0 } else { 0.6 };
        ui.painter()
            .rect_filled(marker, 0, color.gamma_multiply(strength));
    }
    if response.has_focus() || dragging {
        ui.painter().rect_stroke(
            rect,
            EDGE,
            egui::Stroke::new(1.0_f32, color),
            egui::StrokeKind::Inside,
        );
    }
    icon.paint(
        ui,
        rect.left_center() + Vec2::new(SIDEBAR_ICON_INSET + 10.0, 0.0),
        20.0,
        if selected || dragging { color } else { MUTED },
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
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PillStyle {
    Primary,
    Secondary,
    Danger,
}

/// Labelled square-edged button following assets/buttons/button-primary.svg and
/// button-secondary.svg. When a row is too narrow for labels it can show only
/// its icon in a square button; the label stays as tooltip and accessible name.
struct Pill<'a> {
    icon: Icon,
    label: &'a str,
    style: PillStyle,
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
            style: PillStyle::Primary,
            tall: false,
            icon_only: false,
            min_width: 0.0,
        }
    }

    fn secondary(icon: Icon, label: &'a str) -> Self {
        Self {
            style: PillStyle::Secondary,
            ..Self::primary(icon, label)
        }
    }

    fn danger(icon: Icon, label: &'a str) -> Self {
        Self {
            style: PillStyle::Danger,
            ..Self::secondary(icon, label)
        }
    }

    fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }

    fn icon_only(mut self, icon_only: bool) -> Self {
        self.icon_only = icon_only;
        self
    }

    /// Gives two side-by-side buttons one size: the wider width, and the
    /// primary height when either is primary.
    fn pair(ui: &egui::Ui, first: Self, second: Self) -> (Self, Self) {
        let width = first.width(ui).max(second.width(ui));
        let tall = first.height().max(second.height()) == 44.0;
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
        if self.style == PillStyle::Primary || self.tall {
            44.0
        } else {
            36.0
        }
    }

    fn padding(&self) -> f32 {
        if self.style == PillStyle::Primary {
            20.0
        } else {
            14.0
        }
    }

    fn ink(&self) -> Color32 {
        match self.style {
            PillStyle::Primary => Color32::BLACK,
            PillStyle::Secondary => TEXT,
            PillStyle::Danger => Color32::from_rgb(255, 217, 212),
        }
    }

    fn galley(&self, ui: &egui::Ui) -> std::sync::Arc<egui::Galley> {
        let text = RichText::new(self.label).color(self.ink());
        egui::WidgetText::from(if self.style == PillStyle::Primary {
            text.strong()
        } else {
            text
        })
        .into_galley(
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
        let radius = f32::from(EDGE);
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
            let fill = if self.style == PillStyle::Danger {
                if pressed {
                    Color32::from_rgb(73, 23, 21)
                } else if response.hovered() {
                    Color32::from_rgb(116, 39, 31)
                } else {
                    Color32::from_rgb(90, 31, 28)
                }
            } else if self.style == PillStyle::Secondary {
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
                let (icon_x, text_x) = (left + ICON / 2.0, left + ICON + GAP);
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

/// Square icon-only button following assets/buttons/button-icon.svg. The
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
            ui.painter().rect_filled(rect, EDGE, fill);
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.expand(3.0),
                    EDGE,
                    egui::Stroke::new(2.0_f32, ACCENT),
                    egui::StrokeKind::Middle,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Caption {
    Pin,
    Minimize,
    Maximize,
    Restore,
    Close,
}

/// A Windows 11 caption button: a flat 46 × 32 cell with a thin 10-point
/// glyph, a faint wash on hover and the system red behind Close.
struct CaptionButton<'a> {
    glyph: Caption,
    label: &'a str,
    active: bool,
}

impl<'a> CaptionButton<'a> {
    const WIDTH: f32 = 46.0;
    const HEIGHT: f32 = 32.0;

    fn new(glyph: Caption, label: &'a str) -> Self {
        Self {
            glyph,
            label,
            active: false,
        }
    }

    /// Shows a toggle as on, such as always on top.
    fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
}

impl egui::Widget for CaptionButton<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(Self::WIDTH, Self::HEIGHT), egui::Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), self.label)
        });
        if ui.is_rect_visible(rect) {
            let pressed = response.is_pointer_button_down_on();
            let hovered = response.hovered() || response.has_focus();
            let close = self.glyph == Caption::Close;
            let fill = match (close, pressed, hovered) {
                (true, true, _) => Color32::from_rgb(148, 20, 30),
                (true, false, true) => Color32::from_rgb(196, 43, 28),
                (false, true, _) => Color32::from_white_alpha(10),
                (false, false, true) => Color32::from_white_alpha(15),
                _ if self.active => Color32::from_white_alpha(10),
                _ => Color32::TRANSPARENT,
            };
            let painter = ui.painter();
            painter.rect_filled(rect, 0, fill);
            if response.has_focus() {
                painter.rect_stroke(
                    rect.shrink(1.0),
                    0,
                    egui::Stroke::new(2.0_f32, ACCENT),
                    egui::StrokeKind::Inside,
                );
            }
            let ink = if close && (pressed || hovered) {
                Color32::WHITE
            } else if self.active {
                ACCENT
            } else {
                TEXT
            };
            let stroke = egui::Stroke::new(1.0_f32, ink);
            let c = rect.center();
            let half = 5.0;
            match self.glyph {
                Caption::Pin => {
                    // The 24-point pin outline at 14 points draws a hairline
                    // stroke that matches the caption glyphs.
                    Icon::Pin.paint(ui, c, 14.0, ink);
                }
                Caption::Minimize => {
                    painter.hline(c.x - half..=c.x + half, c.y, stroke);
                }
                Caption::Maximize => {
                    painter.rect_stroke(
                        egui::Rect::from_center_size(c, Vec2::splat(2.0 * half)),
                        1,
                        stroke,
                        egui::StrokeKind::Middle,
                    );
                }
                Caption::Restore => {
                    let front = egui::Rect::from_min_size(
                        c + Vec2::new(-half, -half + 2.0),
                        Vec2::splat(2.0 * half - 2.0),
                    );
                    painter.rect_stroke(front, 1, stroke, egui::StrokeKind::Middle);
                    // The rear window shows only above and to the right of the front one.
                    let top = c.y - half;
                    let right = c.x + half;
                    painter.line_segment(
                        [egui::pos2(front.left() + 2.0, top), egui::pos2(right, top)],
                        stroke,
                    );
                    painter.line_segment(
                        [
                            egui::pos2(right, top),
                            egui::pos2(right, front.bottom() - 2.0),
                        ],
                        stroke,
                    );
                }
                Caption::Close => {
                    painter.line_segment(
                        [c + Vec2::new(-half, -half), c + Vec2::new(half, half)],
                        stroke,
                    );
                    painter.line_segment(
                        [c + Vec2::new(half, -half), c + Vec2::new(-half, half)],
                        stroke,
                    );
                }
            }
        }
        response.on_hover_text(self.label)
    }
}

/// Centered square primary Play/Pause button for compact picture-in-picture playback.
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
            let fill = if response.is_pointer_button_down_on() {
                Color32::from_rgb(212, 212, 219)
            } else if response.hovered() {
                Color32::from_rgb(237, 237, 242)
            } else {
                Color32::WHITE
            };
            ui.painter()
                .rect_filled(rect.expand(4.0), EDGE, Color32::from_black_alpha(96));
            ui.painter().rect_filled(rect, EDGE, fill);
            if response.has_focus() {
                ui.painter().rect_stroke(
                    rect.expand(3.0),
                    EDGE,
                    egui::Stroke::new(2.0_f32, ACCENT),
                    egui::StrokeKind::Middle,
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

fn popover_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke((1.0, Color32::from_rgb(42, 42, 46)))
        .corner_radius(EDGE)
        .inner_margin(16)
        .shadow(egui::epaint::Shadow {
            offset: [0, 16],
            blur: 40,
            spread: 0,
            color: Color32::from_black_alpha(140),
        })
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
        .corner_radius(0)
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
    cover_view(ui, item, covers, size, true)
}

fn cover_view(
    ui: &mut egui::Ui,
    item: &Item,
    covers: &Artwork,
    size: Vec2,
    interactive: bool,
) -> egui::Response {
    cover_view_rounded(ui, item, covers, size, interactive, 0)
}

fn cover_view_rounded(
    ui: &mut egui::Ui,
    item: &Item,
    covers: &Artwork,
    size: Vec2,
    interactive: bool,
    radius: u8,
) -> egui::Response {
    let sense = if interactive {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let thumb = item.thumb.as_deref();
    let backdrop = (item.kind == Kind::Movie)
        .then_some(item.art.as_deref())
        .flatten();
    let (preferred, alternate) = if size.x > size.y {
        (backdrop.or(thumb), thumb)
    } else {
        (thumb.or(backdrop), backdrop)
    };
    let alternate = alternate.filter(|path| Some(*path) != preferred);
    if let Some(path) = preferred {
        covers.observe(ui, rect, path);
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            if interactive {
                egui::WidgetType::Button
            } else {
                egui::WidgetType::Image
            },
            ui.is_enabled(),
            if interactive {
                format!("Open {}", item.title)
            } else {
                format!("Cover of {}", item.title)
            },
        )
    });
    let paint = |texture: &egui::TextureHandle, opacity: f32| {
        let ratio = size.x / size.y / texture.aspect_ratio();
        let uv_size = Vec2::new(ratio.min(1.0), (1.0 / ratio).min(1.0));
        egui::Image::new(texture)
            .uv(egui::Rect::from_center_size(egui::pos2(0.5, 0.5), uv_size))
            .corner_radius(radius)
            .tint(Color32::WHITE.gamma_multiply(opacity))
            .paint_at(ui, rect);
    };
    let primary = preferred.and_then(|path| covers.get(path));
    let failed = preferred.is_none_or(|path| covers.failed(path));
    if primary.as_ref().is_none_or(|(_, opacity)| *opacity < 1.0) {
        ui.painter()
            .rect_filled(rect, radius, Color32::from_rgb(34, 36, 40));
        let mut show_initial = failed;
        if let Some(path) = alternate {
            let warm = covers.get(path);
            if warm.is_some() || failed {
                // Protect a resident alternate during the dissolve. A cold
                // alternate gets a request only if the preferred image failed.
                covers.observe(ui, rect, path);
            }
            if let Some((texture, opacity)) = warm {
                paint(&texture, if failed { opacity } else { 1.0 });
            }
            show_initial &= covers.failed(path);
        }
        if show_initial {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                item.title.chars().next().unwrap_or('P'),
                egui::FontId::proportional(42.0),
                ACCENT,
            );
        }
    }
    if let Some((texture, opacity)) = primary {
        paint(&texture, opacity);
    }
    if interactive && (response.hovered() || response.has_focus()) {
        ui.painter().rect_stroke(
            rect,
            radius,
            egui::Stroke::new(2.0_f32, ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    if interactive {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

enum Shelf {
    Video { rows: usize, source: bool },
    Books { count: Option<usize> },
}

impl Shelf {
    fn card_height(&self, width: f32) -> f32 {
        // Captions, progress and source information keep their space as artwork arrives.
        match self {
            Self::Video { source, .. } => width * 9.0 / 16.0 + if *source { 76.0 } else { 52.0 },
            Self::Books { count } => width + if count.is_some() { 76.0 } else { 54.0 },
        }
    }
}

fn card_columns(available: f32, widths: (f32, f32)) -> (usize, f32) {
    fitted_columns(available, widths, ART_GAP)
}

fn fitted_columns(available: f32, (minimum, maximum): (f32, f32), gap: f32) -> (usize, f32) {
    let columns = ((available + gap) / (minimum + gap)).floor().max(1.0) as usize;
    let width = ((available + gap) / columns as f32 - gap).clamp(1.0, maximum);
    (columns, width)
}

#[derive(Clone, Copy)]
struct CardAnchor {
    columns: usize,
    rows: usize,
    width: f32,
    item: egui::Id,
    scroll: f32,
    backward: bool,
}

fn scrolling_back(previous: Option<CardAnchor>, position: f32) -> bool {
    previous.is_some_and(|anchor| {
        let delta = position - anchor.scroll;
        if delta.abs() < 0.5 {
            anchor.backward
        } else {
            delta < 0.0
        }
    })
}

/// Clip the layout, keeping one neighbour ready on either edge.
fn nearby_cells(start: f32, end: f32, pitch: f32, count: usize) -> std::ops::Range<usize> {
    let first = ((start / pitch).floor() as usize)
        .saturating_sub(1)
        .min(count);
    let last = ((end / pitch).ceil() as usize).saturating_add(1).min(count);
    first..last
}

fn card_cell(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    behind: bool,
    add: impl FnOnce(&mut egui::Ui),
) -> bool {
    // A global, shelf-scoped identity keeps the card's interactive children stable across reflow.
    let mut builder = egui::UiBuilder::new()
        .id(id)
        .max_rect(rect)
        .layout(egui::Layout::top_down(egui::Align::Min));
    if behind {
        builder = builder.ui_stack_info(egui::UiStackInfo::default().with_tag("prefetch-behind"));
    }
    let mut cell = ui.new_child(builder);
    add(&mut cell);
    ui.memory(|memory| memory.focused())
        .and_then(|focused| ui.ctx().read_response(focused))
        .is_some_and(|response| {
            let here = rect.contains(response.rect.center());
            if here && response.gained_focus() {
                response.scroll_to_me(None);
            }
            here
        })
}

fn library_cards(
    ui: &mut egui::Ui,
    id: egui::Id,
    items: &[&Item],
    episodes: bool,
    mut add: impl FnMut(&mut egui::Ui, &Item, f32),
) {
    if items.is_empty() {
        return;
    }
    let available = ui.available_width();
    let (columns, width, height) = if episodes {
        (1, available, 112.0)
    } else if items.iter().all(|item| item.kind == Kind::Audiobook) {
        let (columns, width) = card_columns(available, BOOK_WIDTH);
        (columns, width, width + 54.0)
    } else {
        let (columns, width) = card_columns(available, POSTER_WIDTH);
        (columns, width, width * 1.5 + 52.0)
    };
    let pitch = height + ART_GAP;
    let rows = items.len().div_ceil(columns);
    let (_, grid) = ui.allocate_space(Vec2::new(available, rows as f32 * pitch - ART_GAP));
    let clip = ui.clip_rect();
    let near = nearby_cells(
        clip.top() - grid.top(),
        clip.bottom() - grid.top(),
        pitch,
        rows,
    );
    let old = ui.data(|data| data.get_temp::<CardAnchor>(id));
    let scroll = clip.top() - grid.top();
    let backward = scrolling_back(old, scroll);
    let anchor_row = old.and_then(|anchor| {
        if anchor.columns == columns && (anchor.width - width).abs() < 0.25 {
            return None;
        }
        items
            .iter()
            .position(|item| id.with(&item.id) == anchor.item)
            .map(|index| index / columns)
    });
    let mut focus = None;
    for row in near
        .clone()
        .chain(anchor_row.filter(|row| !near.contains(row)))
    {
        let y = grid.top() + row as f32 * pitch;
        for (column, item) in items[row * columns..].iter().take(columns).enumerate() {
            let key = id.with(&item.id);
            let left = grid.left() + column as f32 * (width + ART_GAP);
            let rect = egui::Rect::from_min_size(egui::pos2(left, y), Vec2::new(width, height));
            let behind = if backward {
                rect.top() >= clip.bottom()
            } else {
                rect.bottom() <= clip.top()
            };
            if card_cell(ui, key, rect, behind, |ui| add(ui, item, width)) {
                focus = Some(key);
            }
            if anchor_row == Some(row) && old.is_some_and(|anchor| anchor.item == key) {
                ui.scroll_to_rect_animation(rect, None, egui::style::ScrollAnimation::none());
            }
        }
    }
    let top_row = ((clip.top() - grid.top()) / pitch).floor().max(0.0) as usize;
    if let Some(item) = focus.or_else(|| {
        (grid.intersects(clip) && grid.top() < clip.top())
            .then(|| id.with(&items[top_row.min(rows - 1) * columns].id))
    }) {
        ui.data_mut(|data| {
            data.insert_temp(
                id,
                CardAnchor {
                    columns,
                    rows,
                    width,
                    item,
                    scroll,
                    backward,
                },
            )
        });
    } else if old.is_some() {
        ui.data_mut(|data| data.remove::<CardAnchor>(id));
    }
}

fn home_carousel<T>(
    ui: &mut egui::Ui,
    (id, heading): (&'static str, &str),
    offset_x: &mut f32,
    items: &[T],
    shelf: Shelf,
    item_key: impl Fn(&T) -> egui::Id,
    mut card: impl FnMut(&mut egui::Ui, &T, f32),
) {
    let viewport = ui.available_width();
    let show_count = if let Shelf::Books { count } = shelf {
        count
    } else {
        None
    };
    let (rows, width_range) = match shelf {
        Shelf::Video { rows, .. } => (rows, VIDEO_WIDTH),
        Shelf::Books { .. } => (1, BOOK_WIDTH),
    };
    let (visible_columns, card_width) = card_columns(viewport, width_range);
    let pitch = card_width + ART_GAP;
    let row_height = shelf.card_height(card_width);
    let page_size = visible_columns * rows;
    let page_count = items.len().div_ceil(page_size).max(1);
    let page_pitch = viewport + ART_GAP;
    let column_at = |x: f32| {
        let x = x.max(0.0);
        let page = (x / page_pitch).floor() as usize;
        let column = ((x % page_pitch) / pitch).floor() as usize;
        page * visible_columns + column.min(visible_columns - 1)
    };
    let max_offset = page_count.saturating_sub(1) as f32 * page_pitch;
    let shelf_id = egui::Id::new(("home shelf", id));
    let old = ui.data(|data| data.get_temp::<CardAnchor>(shelf_id));
    let mut target = old.and_then(|anchor| {
        if anchor.columns == visible_columns
            && anchor.rows == rows
            && (anchor.width - viewport).abs() < 0.25
        {
            return None;
        }
        let index = items
            .iter()
            .position(|item| item_key(item) == anchor.item)?;
        Some((index / page_size) as f32 * page_pitch)
    });
    control_row(ui, 32.0, |ui| {
        if let Some(count) = show_count {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.label(RichText::new(heading).size(22.0));
            ui.label(RichText::new(count.to_string()).size(13.0).color(MUTED));
            ui.spacing_mut().item_spacing.x = 8.0;
        } else {
            ui.label(RichText::new(heading).size(22.0).strong());
        }
        if show_count.is_some() || max_offset > 1.0 {
            let size = if show_count.is_some() { 28.0 } else { 32.0 };
            if ui
                .add_enabled(
                    *offset_x > 1.0,
                    IconButton::new(Icon::PagePrevious, &format!("Scroll {heading} back"))
                        .size(size),
                )
                .clicked()
            {
                target =
                    Some(((*offset_x / page_pitch - 0.001).ceil() - 1.0).max(0.0) * page_pitch);
            }
            if ui
                .add_enabled(
                    *offset_x < max_offset - 1.0,
                    IconButton::new(Icon::PageNext, &format!("Scroll {heading} forward"))
                        .size(size),
                )
                .clicked()
            {
                target = Some(((*offset_x / page_pitch + 0.001).floor() + 1.0) * page_pitch);
            }
        }
    });
    let mut scroll = egui::ScrollArea::horizontal()
        .id_salt(id)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
    if let Some(offset) = target {
        scroll = scroll.horizontal_scroll_offset(offset.clamp(0.0, max_offset));
    }
    let mut focused = None;
    let mut painted_offset = *offset_x;
    let output = scroll.show_viewport(ui, |ui, view| {
        painted_offset = view.left();
        let backward = scrolling_back(old, painted_offset);
        let origin = ui.max_rect().min;
        let height = rows as f32 * (row_height + ART_GAP) - ART_GAP;
        ui.set_min_size(Vec2::new(max_offset + viewport, height));
        let near = ui.clip_rect().expand2(Vec2::new(0.0, row_height + ART_GAP));
        if origin.y + height < near.top() || origin.y > near.bottom() {
            return;
        }
        let columns = column_at(view.left()).saturating_sub(1)
            ..(column_at(view.right()) + 2).min(page_count * visible_columns);
        for row in 0..rows {
            for column in columns.clone() {
                let index = (column / visible_columns) * page_size
                    + row * visible_columns
                    + column % visible_columns;
                if let Some(item) = items.get(index) {
                    let key = item_key(item);
                    let x = (column / visible_columns) as f32 * page_pitch
                        + (column % visible_columns) as f32 * pitch;
                    let start = origin + Vec2::new(x, row as f32 * (row_height + ART_GAP));
                    let rect = egui::Rect::from_min_size(start, Vec2::new(card_width, row_height));
                    let behind = if backward {
                        x >= view.right()
                    } else {
                        x + card_width <= view.left()
                    };
                    if card_cell(ui, shelf_id.with(key), rect, behind, |ui| {
                        card(ui, item, card_width)
                    }) {
                        focused = Some(key);
                    }
                }
            }
        }
    });
    *offset_x = output.state.offset.x;
    let column = column_at(*offset_x);
    let index = column / visible_columns * page_size + column % visible_columns;
    if let Some(item) = focused.or_else(|| items.get(index).map(&item_key)) {
        ui.data_mut(|data| {
            data.insert_temp(
                shelf_id,
                CardAnchor {
                    columns: visible_columns,
                    rows,
                    width: viewport,
                    item,
                    scroll: painted_offset,
                    backward: scrolling_back(old, painted_offset),
                },
            )
        });
    }
}

#[derive(Clone, Copy)]
struct BookCardPlayback {
    paused: bool,
    elapsed_ms: u64,
    finished: bool,
}

fn book_card(
    ui: &mut egui::Ui,
    item: &Item,
    covers: &Artwork,
    width: f32,
    playing: Option<BookCardPlayback>,
) -> bool {
    let elapsed = playing.map_or(item.resume_ms.unwrap_or(0), |live| live.elapsed_ms);
    let finished = playing.map_or(item.watched, |live| live.finished);
    let mut clicked = false;
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 8.0;
        let response = cover(ui, item, covers, Vec2::splat(width));
        let rect = response.rect;
        clicked |= response.on_hover_text(&item.title).clicked();
        let corner = rect.min + Vec2::splat(10.0);
        if let Some(live) = playing {
            let text = if live.paused { "Paused" } else { "Playing" };
            let label = ui.painter().layout_no_wrap(
                text.into(),
                egui::FontId::proportional(11.0),
                Color32::WHITE,
            );
            let badge = egui::Rect::from_min_size(corner, Vec2::new(label.size().x + 32.0, 24.0));
            ui.painter()
                .rect_filled(badge, 0, Color32::from_black_alpha(184));
            Icon::NowPlaying.paint(ui, corner + Vec2::new(13.0, 12.0), 14.0, Color32::WHITE);
            ui.painter().galley(
                corner + Vec2::new(24.0, (24.0 - label.size().y) * 0.5),
                label,
                Color32::WHITE,
            );
        } else if finished {
            let center = corner + Vec2::splat(10.0);
            ui.painter().circle_filled(center, 10.0, Color32::WHITE);
            Icon::Check.paint(ui, center, 14.0, Color32::BLACK);
        }
        if !finished
            && elapsed > 0
            && let Some(duration) = item.duration_ms.filter(|&ms| ms > 0)
        {
            let track = egui::Rect::from_min_size(
                rect.left_bottom() + Vec2::new(8.0, -11.0),
                Vec2::new(width - 16.0, 3.0),
            );
            ui.painter()
                .rect_filled(track, 2, Color32::from_white_alpha(71));
            ui.painter().rect_filled(
                egui::Rect::from_min_size(
                    track.min,
                    Vec2::new(
                        track.width() * (elapsed as f32 / duration as f32).clamp(0.0, 1.0),
                        3.0,
                    ),
                ),
                2,
                ACCENT,
            );
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            clicked |= ui
                .add(
                    egui::Label::new(
                        RichText::new(&item.title)
                            .size(15.0)
                            .line_height(Some(22.5)),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
            let duration = if finished {
                Some("Finished".to_owned())
            } else {
                item.duration_ms.map(|ms| {
                    let text =
                        audiobooks::book_duration(ms.saturating_sub(elapsed) as f64 / 1000.0);
                    if elapsed > 0 {
                        format!("{text} left")
                    } else {
                        text
                    }
                })
            };
            let author = Some(item.book.author.as_str()).filter(|s| !s.is_empty());
            let subtitle = [author, duration.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
            clicked |= ui
                .add(
                    egui::Label::new(
                        RichText::new(subtitle)
                            .size(12.0)
                            .color(MUTED)
                            .line_height(Some(18.0)),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
        });
    });
    clicked
}

fn continue_card(ui: &mut egui::Ui, item: &Item, covers: &Artwork, width: f32) -> bool {
    let size = Vec2::new(width, width * 9.0 / 16.0);
    let mut clicked = false;
    ui.vertical(|ui| {
        ui.set_width(size.x);
        ui.spacing_mut().item_spacing.y = 6.0;
        let response = cover(ui, item, covers, size);
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
            .on_hover_cursor(egui::CursorIcon::PointingHand)
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

fn released_card(ui: &mut egui::Ui, item: &Item, covers: &Artwork, width: f32) -> bool {
    let mut clicked = false;
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 6.0;
        clicked |= cover(ui, item, covers, Vec2::new(width, width * 9.0 / 16.0))
            .on_hover_text(&item.title)
            .clicked();
        clicked |= ui
            .add(
                egui::Button::new(RichText::new(&item.title).strong())
                    .frame(false)
                    .wrap_mode(egui::TextWrapMode::Truncate),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();
        let subtitle = match item.release_date.as_deref().and_then(release_date) {
            Some(date) => format!("{}  ·  {date}", item.subtitle()),
            None => item.subtitle(),
        };
        ui.add(egui::Label::new(RichText::new(subtitle).size(12.0).color(MUTED)).truncate());
    });
    clicked
}

/// Formats a `YYYY-MM-DD` release date as e.g. `20 Nov 2026`.
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
                .rect_filled(badge, 0.0, Color32::from_black_alpha(200));
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
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();
        ui.add(egui::Label::new(RichText::new(item.subtitle()).size(12.0).color(MUTED)).truncate());
    });
    clicked
}

fn hero(ui: &mut egui::Ui, item: &Item, covers: &Artwork, eyebrow: &str) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 228.0), egui::Sense::hover());
    if let Some(path) = &item.art {
        covers.observe(ui, rect, path);
    }
    ui.painter().rect_filled(rect, 0.0, PANEL);
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
        egui::Image::new(&texture)
            .uv(uv)
            .corner_radius(0)
            .tint(Color32::WHITE.gamma_multiply(opacity))
            .paint_at(ui, rect);
        ui.painter().rect_filled(
            rect,
            0.0,
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
        cover_view(
            ui,
            item,
            covers,
            Vec2::new(art_width, art_width * 1.5),
            false,
        );
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

/// Fills `band` with the scrim, then fades it out between the band's inner edge and `clear_at`.
fn controls_scrim(band: egui::Rect, clear_at: f32) -> egui::Shape {
    const STEPS: u32 = 8;
    let edge = if clear_at > band.bottom() {
        band.bottom()
    } else {
        band.top()
    };
    let mut mesh = egui::Mesh::default();
    mesh.add_colored_rect(band, SCRIM);
    for step in 0..=STEPS {
        // Smoothstep leaves no visible line where the solid band ends.
        let t = step as f32 / STEPS as f32;
        let alpha = f32::from(SCRIM.a()) * (1.0 - t * t * (3.0 - 2.0 * t));
        let color = Color32::from_black_alpha(alpha.round() as u8);
        let y = egui::lerp(edge..=clear_at, t);
        mesh.colored_vertex(egui::pos2(band.left(), y), color);
        mesh.colored_vertex(egui::pos2(band.right(), y), color);
        if step > 0 {
            let last = mesh.vertices.len() as u32 - 4;
            mesh.add_triangle(last, last + 1, last + 2);
            mesh.add_triangle(last + 1, last + 2, last + 3);
        }
    }
    mesh.into()
}

fn seekbar(
    ui: &mut egui::Ui,
    position: &mut f64,
    duration: f64,
    buffered_end: Option<f64>,
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
        let step = if buffered_end.is_some() { 10.0 } else { 1.0 };
        let delta = ui.input(|i| {
            if i.key_pressed(egui::Key::ArrowRight) {
                step
            } else if i.key_pressed(egui::Key::ArrowLeft) {
                -step
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
    let height = if buffered_end.is_some() { 6.0 } else { 4.0 };
    let track = egui::Rect::from_center_size(rect.center(), Vec2::new(rect.width(), height));
    ui.painter().rect_filled(track, 3.0, Color32::from_gray(65));
    for (end, color) in buffered_end
        .into_iter()
        .map(|end| (end, Color32::from_gray(110)))
        .chain(std::iter::once((*position, ACCENT)))
    {
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                track.min,
                Vec2::new(
                    track.width() * (end / duration).clamp(0.0, 1.0) as f32,
                    height,
                ),
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
        if buffered_end.is_none() {
            7.0
        } else if focused || response.hovered() {
            8.0
        } else {
            6.0
        },
        Color32::WHITE,
    );
    response.on_hover_text(if buffered_end.is_some() {
        "Seek · use Left / Right to move 10 seconds"
    } else {
        "Seek in chapter · use Left / Right to move 1 second"
    })
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
            kind: LibraryKind::Movies,
        });
        app.selected_library = Some("1".into());
        app.page = Some(Page::new(vec![item.clone(); 5], 0, Some(5)));
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
